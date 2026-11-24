"""Selected archive updates: prepare, publish once, verify on the remote.

`python3 -B -m controllers.archive prepare|publish|verify|enqueue` adds or updates only the
contests an operator selected, with the files they reviewed, in an existing archive repository
(`index.json` with a `contests` list and one tree per contest under `contests/`). Every other tree
and index row stays as it is. A push whose outcome is unknown is never repeated: the recorded
commit is looked for on the remote instead.

Nothing here launches, submits, closes or changes a contest; contests are read through `ach show`.
"""
import argparse
import hashlib
import json
import os
import re
import subprocess
import uuid
from pathlib import Path, PurePosixPath

from .common import Ach, AchError, DaemonLock, LockHeld, atomic_write, atomic_write_json, utcnow

ID = re.compile(r'[a-z0-9][a-z0-9-]*')
BRANCH = re.compile(r'[A-Za-z0-9][A-Za-z0-9/_.-]*')
REMOTE = re.compile(r'(?:https://|ssh://|git@|file:///)[A-Za-z0-9._/@:~-]+')
TARGET = re.compile(r'contests/[A-Za-z0-9][A-Za-z0-9_.-]*')
TOKEN = re.compile(r'[a-f0-9]{64}')
FIELD = re.compile(r'[a-z][a-z0-9_]{0,39}')
TEXT = {'.md', '.txt', '.rst', '.py', '.rs', '.js', '.ts', '.tsx', '.jsx', '.html', '.css', '.json', '.yml',
        '.yaml', '.toml', '.csv', '.tex'}
DENIED = {'.git', '.private', '.harness', '.github', 'node_modules', 'target', 'dist', 'build', '__pycache__'}
NAME = re.compile(r'(?i)signature|서명|증명|신분|주민|동의서|신청서|credential|secret|token|id_rsa|\.env')
PRIVATE = re.compile(r'(?i)[\w.+-]+@[\w-]+\.[\w.-]+|\b01[016789][- .]?\d{3,4}[- .]?\d{4}\b|'
                     r'\b\d{6}[- ]?[1-4]\d{6}\b|-----BEGIN .*PRIVATE KEY-----|'
                     r'(?:api[_-]?key|password|authorization|access[_-]?token|token|client_secret)\s*[:=]\s*\S+|'
                     r'\b(?:gh[pousr]_|sk-|AKIA)[A-Za-z0-9_-]{12,}')
PROTECTED = {'submitted', 'closed', 'followup_pending'}
ACTIVE = {'gate_pass', 'routed', 'running', 'plan_review', 'awaiting_approval'}
PHASES = ('assignment', 'checkpoint', 'manual')
METADATA = ('id', 'state', 'type', 'level', 'deadline', 'title', 'organizer', 'url')
MANAGED = set(METADATA) | {'folder', 'path', 'files', 'archive_basis'}
MAX_FILE, MAX_ENTRY = 1536 * 1024, 40 * 1024 * 1024
LOCAL, NETWORK = 60, 600
ERRORS = (OSError, ValueError, KeyError, TypeError, AttributeError, subprocess.TimeoutExpired)
# A push was started and its outcome is not settled yet / what the remote was last seen to say.
OPEN = ('pushing', 'unknown')
SETTLED = ('verified', 'superseded', 'not_published', 'abandoned')


class ArchiveError(RuntimeError):
    """A stable code only: source text, remote URLs and Git output never travel in it."""


def digest(data):
    return hashlib.sha256(data).hexdigest()


def git(repo, *args, runner=subprocess.run, timeout=LOCAL):
    """One Git command as an argv list: no hooks, no prompt, no signing."""
    env = dict(os.environ, GIT_TERMINAL_PROMPT='0')
    result = runner(['git', '-c', 'core.hooksPath=/dev/null', '-c', 'commit.gpgsign=false', '-c', 'core.quotepath=false',
                     '-C', str(repo), *args], capture_output=True, timeout=timeout, env=env)
    if result.returncode:
        raise ArchiveError('git_failed')
    return result.stdout.decode('utf-8', errors='strict').rstrip('\n')


def private_check(text):
    if PRIVATE.search(text) or '\x00' in text:
        raise ArchiveError('private_content_detected')


def source_file(folder, item):
    """`(name, bytes)` of one reviewed text file, or the reason the whole selection is refused."""
    if not isinstance(item, dict) or item.get('privacy_reviewed') is not True:
        raise ArchiveError('privacy_review_required')
    name = item.get('path')
    if not isinstance(name, str):
        raise ArchiveError('file_path_invalid')
    relative = PurePosixPath(name)
    if (relative.is_absolute() or not relative.parts or '..' in relative.parts or '\\' in name
            or any(ord(c) < 32 or ord(c) == 127 for c in name)
            or any(part.startswith('.') or part in DENIED for part in relative.parts)
            or NAME.search(name) or relative.suffix.lower() not in TEXT):
        raise ArchiveError('file_excluded')
    path = folder / name
    if any(p.is_symlink() for p in (path, *path.parents)) or not path.is_file():
        raise ArchiveError('file_not_regular')
    if not path.resolve().is_relative_to(folder) or path.stat().st_size > MAX_FILE:
        raise ArchiveError('file_outside_or_too_large')
    data = path.read_bytes()
    if digest(data) != item.get('sha256'):
        raise ArchiveError('source_changed')
    try:
        private_check(data.decode('utf-8'))
    except UnicodeDecodeError:
        raise ArchiveError('file_excluded') from None
    return name, data


def metadata(entry):
    """The public row fields of a contest: workflow facts only, never notes, receipts or paths."""
    value = {key: entry.get(key) for key in METADATA}
    private_check(json.dumps(value, ensure_ascii=False))
    return value


def journal(ach):
    """`<home>/data/archive-sync`, refused when it or its parts under the home are symlinks."""
    home = Path(ach.root).resolve()
    root = home / 'data' / 'archive-sync'
    if any(path.is_symlink() for path in (home / 'data', root, root / 'pending')):
        raise ArchiveError('journal_symlink')
    return root


def enqueue(ach, contest_id, phase):
    """Note that a contest was assigned or reached a checkpoint, for a later archive update.

    Does nothing unless `data/archive-sync/config.json` holds `{"enabled": true}`. Never raises: a
    failure here must not undo or repeat the launch or checkpoint that already succeeded."""
    try:
        if not isinstance(contest_id, str) or not ID.fullmatch(contest_id) or phase not in PHASES[:2]:
            raise ArchiveError('event_invalid')
        root = journal(ach)
        config = root / 'config.json'
        if not config.exists():
            return {'state': 'disabled'}
        settings = json.loads(config.read_text())
        if not isinstance(settings, dict) or type(settings.get('enabled', False)) is not bool:
            raise ArchiveError('archive_config_invalid')
        if settings.get('enabled') is not True:
            return {'state': 'disabled'}
        entry = ach.json('show', contest_id)
        if entry.get('state') not in ACTIVE or entry.get('submissions'):
            return {'state': 'protected'}
        record = {'id': contest_id, 'phase': phase, 'workflow_state': entry['state'],
                  'history': len(entry.get('history') or [])}
        record['token'] = digest(json.dumps(record, sort_keys=True).encode())
        record['state'] = 'pending'
        with DaemonLock(root / 'pending.lock'):
            path = root / 'pending' / (contest_id + '.json')
            previous = json.loads(path.read_text()) if path.exists() else {}
            if previous.get('token') != record['token']:
                atomic_write_json(path, record)
        return {'state': 'pending', 'token': record['token']}
    except (ArchiveError, AchError, LockHeld, *ERRORS):
        return {'state': 'error', 'reason': 'archive_enqueue_failed'}


def settle_pending(directory, receipt):
    """Mark the pending notes this run answered. A newer note (another token) stays pending."""
    root = directory.parent
    try:
        if (root / 'pending').is_symlink():
            raise ArchiveError('pending_symlink')
        with DaemonLock(root / 'pending.lock'):
            for contest_id, token in receipt.get('pending_tokens', {}).items():
                if not ID.fullmatch(contest_id) or not isinstance(token, str) or not TOKEN.fullmatch(token):
                    raise ArchiveError('pending_token_invalid')
                path = root / 'pending' / (contest_id + '.json')
                if path.is_file():
                    current = json.loads(path.read_text())
                    if current.get('token') == token:
                        current.update(state='verified', archive_run=receipt['run_id'],
                                       remote_commit=receipt.get('remote_commit') or receipt.get('base'))
                        atomic_write_json(path, current)
        receipt.pop('pending_acknowledgement', None)
    except (ArchiveError, LockHeld, *ERRORS):
        # The verified remote receipt stands even when the note could not be marked.
        receipt['pending_acknowledgement'] = 'incomplete'


def selected_sources(ach, selection):
    """`{id: {metadata, history, files}}` for the selection, each contest read once through `ach show`."""
    if not isinstance(selection, list) or not selection:
        raise ArchiveError('selection_required')
    result = {}
    for item in selection:
        contest_id = item.get('id') if isinstance(item, dict) else None
        if not isinstance(contest_id, str) or not ID.fullmatch(contest_id) or contest_id in result:
            raise ArchiveError('selection_invalid')
        entry = ach.json('show', contest_id)
        if not isinstance(entry, dict) or entry.get('id') != contest_id:
            raise ArchiveError('contest_identity_mismatch')
        if entry.get('state') not in ACTIVE or entry.get('submissions'):
            raise ArchiveError('submitted_or_inactive_protected')
        if not isinstance(entry.get('folder'), str) or not Path(entry['folder']).is_dir():
            raise ArchiveError('source_folder_missing')
        folder = Path(entry['folder']).resolve()
        files = item.get('files')
        if not isinstance(files, list) or not files:
            raise ArchiveError('reviewed_files_required')
        approved = {}
        for spec in files:
            name, data = source_file(folder, spec)
            if name in approved:
                raise ArchiveError('duplicate_file')
            approved[name] = data
        if sum(len(data) for data in approved.values()) > MAX_ENTRY:
            raise ArchiveError('selection_too_large')
        result[contest_id] = {'metadata': metadata(entry), 'history': len(entry.get('history') or []),
                              'files': approved}
    return result


def basis(selected):
    """What a publish must still find as it was at prepare: the public fields and the history length."""
    return {contest_id: dict(source['metadata'], history=source['history']) for contest_id, source in selected.items()}


def index_rows(repo, runner):
    raw = json.loads(git(repo, 'show', 'HEAD:index.json', runner=runner, timeout=NETWORK))
    rows = raw.get('contests') if isinstance(raw, dict) else None
    if not isinstance(rows, list) or any(not isinstance(row, dict) for row in rows):
        raise ArchiveError('archive_index_invalid')
    if any(row.get('id') is not None and not isinstance(row.get('id'), str) for row in rows):
        raise ArchiveError('archive_index_invalid')
    # Rows that are not selected may lack an id or carry fields unknown here; they are kept as they are.
    return raw, rows


def row_defaults(request):
    """Extra fields for a new row (an archive's own columns), as plain values. Never for a managed field."""
    defaults = request.get('row_defaults', {})
    if not isinstance(defaults, dict) or any(
            not isinstance(key, str) or not FIELD.fullmatch(key) or key in MANAGED
            or not (value is None or isinstance(value, (str, int, float, bool))) for key, value in defaults.items()):
        raise ArchiveError('row_defaults_invalid')
    private_check(json.dumps(defaults, ensure_ascii=False))
    return defaults


def prepare(ach, request, runner=subprocess.run):
    """Build the candidate commit in a clone of its own; nothing is pushed."""
    if not isinstance(request, dict) or request.get('private_remote_confirmed') is not True:
        raise ArchiveError('private_remote_review_required')
    remote, branch = request.get('remote'), request.get('branch', 'main')
    if (not isinstance(remote, str) or not remote or remote.startswith('-') or '\x00' in remote
            or re.search(r'https?://[^/]*@', remote) or not isinstance(branch, str) or not BRANCH.fullmatch(branch)
            or '..' in branch):
        raise ArchiveError('remote_or_branch_invalid')
    if not (Path(remote).is_absolute() or REMOTE.fullmatch(remote)):
        raise ArchiveError('remote_transport_not_allowed')
    try:
        run_id = str(uuid.UUID(request['run_id']))
    except ERRORS:
        raise ArchiveError('run_id_invalid') from None
    phase = request.get('phase')
    if phase not in PHASES:
        raise ArchiveError('phase_invalid')
    defaults = row_defaults(request)
    selected = selected_sources(ach, request.get('selection'))
    directory = journal(ach) / run_id
    if directory.is_symlink():
        raise ArchiveError('journal_symlink')
    directory.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    try:
        directory.mkdir(mode=0o700)
    except FileExistsError:
        raise ArchiveError('run_already_exists') from None
    receipt = {'version': 1, 'run_id': run_id, 'phase': phase, 'state': 'preparing',
               'branch': branch, 'remote_key': digest((remote + '\n' + branch).encode()),
               'selected_ids': sorted(selected), 'created_at': utcnow().isoformat(), 'pending_tokens': {}}
    for contest_id in selected:
        pending = directory.parent / 'pending' / (contest_id + '.json')
        if pending.is_file() and not pending.is_symlink():
            try:
                token = json.loads(pending.read_text()).get('token')
            except ERRORS:
                token = None
            if isinstance(token, str) and TOKEN.fullmatch(token):
                receipt['pending_tokens'][contest_id] = token
    atomic_write_json(directory / 'receipt.json', receipt)
    atomic_write_json(directory / 'request.json', request)
    repo = directory / 'repo'
    try:
        # Only the latest commit and no file contents: blobs come on demand, nothing is checked out.
        git(directory, 'clone', '--no-checkout', '--single-branch', '--depth', '1', '--filter=blob:none',
            '--branch', branch, '--', remote, str(repo), runner=runner, timeout=NETWORK)
        base = git(repo, 'rev-parse', 'HEAD', runner=runner)
        index, rows = index_rows(repo, runner)
        original_rows = json.loads(json.dumps(rows))
        unselected = [row for row in rows if row.get('id') not in selected]
        occupied = [row.get('path') for row in rows]
        expected = {}
        git(repo, 'read-tree', 'HEAD', runner=runner)
        for contest_id, source in selected.items():
            matching = [row for row in rows if row.get('id') == contest_id]
            if len(matching) > 1:
                raise ArchiveError('archive_index_ambiguous')
            old = matching[0] if matching else None
            if old and (old.get('state') in PROTECTED or old.get('submitted_at')):
                raise ArchiveError('submitted_archive_protected')
            target = old.get('path') if old else 'contests/' + contest_id
            if not isinstance(target, str) or not TARGET.fullmatch(target) or '..' in target:
                raise ArchiveError('archive_path_invalid')
            if old is None and target in occupied:
                raise ArchiveError('archive_path_occupied')
            if old is not None and occupied.count(target) != 1:
                raise ArchiveError('archive_path_ambiguous')
            # Drop only this contest's tree from the index, then add the reviewed files as blobs.
            tracked = git(repo, 'ls-tree', '-r', '-z', '--name-only', 'HEAD', '--', target, runner=runner).split('\0')
            for path in tracked:
                if not path:
                    continue
                if not path.startswith(target + '/') or any(ord(c) < 32 for c in path):
                    raise ArchiveError('archive_path_unsafe')
                git(repo, 'update-index', '--force-remove', '--', path, runner=runner)
            for name, data in source['files'].items():
                local = directory / 'files' / contest_id / name
                local.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
                local.write_bytes(data)
                local.chmod(0o600)
                blob = git(repo, 'hash-object', '-w', '--', str(local), runner=runner)
                git(repo, 'update-index', '--add', '--cacheinfo', f'100644,{blob},{target}/{name}', runner=runner)
            row = dict(defaults) if old is None else dict(old)
            row.update(source['metadata'], folder=target.split('/', 1)[1], path=target, files=len(source['files']))
            row['archive_basis'] = digest(json.dumps(source['metadata'], sort_keys=True).encode())
            if old is None:
                rows.append(row)
            else:
                rows[rows.index(old)] = row
            expected[contest_id] = row
        if [row for row in rows if row.get('id') not in selected] != unselected:
            raise ArchiveError('unselected_metadata_changed')
        head_tree = git(repo, 'rev-parse', 'HEAD^{tree}', runner=runner)
        if rows == original_rows and git(repo, 'write-tree', '--missing-ok', runner=runner) == head_tree:
            # The same files and fields are already there: no commit for a new timestamp alone.
            receipt.update(state='unchanged', base=base)
            settle_pending(directory, receipt)
            atomic_write_json(directory / 'receipt.json', receipt)
            return receipt
        index['generated_at'] = utcnow().isoformat()
        atomic_write(directory / 'index.json', json.dumps(index, ensure_ascii=False, indent=2) + '\n')
        blob = git(repo, 'hash-object', '-w', '--', str(directory / 'index.json'), runner=runner)
        git(repo, 'update-index', '--add', '--cacheinfo', f'100644,{blob},index.json', runner=runner)
        # The README keeps its content; one block at its top, owned by this tool, names the last update.
        try:
            readme = git(repo, 'show', 'HEAD:README.md', runner=runner, timeout=NETWORK)
        except ArchiveError:
            readme = '# Contest archive'
        readme = re.sub(r'\A<!-- archive-sync:start -->.*?<!-- archive-sync:end -->\n*', '', readme, flags=re.S)
        header = ('<!-- archive-sync:start -->\nLatest selected update: ' + ', '.join(sorted(selected))
                  + '\nCurrent metadata: [index.json](index.json).\n<!-- archive-sync:end -->\n\n')
        atomic_write(directory / 'README.md', header + readme + '\n')
        blob = git(repo, 'hash-object', '-w', '--', str(directory / 'README.md'), runner=runner)
        git(repo, 'update-index', '--add', '--cacheinfo', f'100644,{blob},README.md', runner=runner)
        tree = git(repo, 'write-tree', '--missing-ok', runner=runner)
        changed = git(repo, 'diff-tree', '--no-commit-id', '--name-only', '-r', base, tree, runner=runner).splitlines()
        targets = [row['path'] + '/' for row in expected.values()]
        if any(path not in ('index.json', 'README.md') and not any(path.startswith(t) for t in targets) for path in changed):
            raise ArchiveError('unselected_path_changed')
        commit = git(repo, 'commit-tree', tree, '-p', base, '-m', 'Update selected archive entries', runner=runner)
        receipt.update(state='prepared', base=base, commit=commit, tree=tree, expected_rows=expected,
                       changed_paths=changed, basis=basis(selected))
        atomic_write_json(directory / 'receipt.json', receipt)
        return receipt
    except (ArchiveError, *ERRORS):
        receipt['state'] = 'prepare_failed'
        atomic_write_json(directory / 'receipt.json', receipt)
        raise


def is_ancestor(repo, commit, tip, runner):
    """Whether `commit` is in the history of `tip`. Any other answer of Git is an error, not a no."""
    result = runner(['git', '-C', str(repo), 'merge-base', '--is-ancestor', commit, tip], capture_output=True,
                    timeout=LOCAL, env=dict(os.environ, GIT_TERMINAL_PROMPT='0'))
    if result.returncode not in (0, 1):
        raise ArchiveError('git_failed')
    return result.returncode == 0


def _verify(directory, runner=subprocess.run):
    receipt = json.loads((directory / 'receipt.json').read_text())
    state = receipt.get('state')
    if state in SETTLED:
        # What was seen then stays on record; it is not presented as a new observation.
        if state == 'verified':
            settle_pending(directory, receipt)
            atomic_write_json(directory / 'receipt.json', receipt)
        return receipt
    if state not in OPEN:
        raise ArchiveError('no_push_to_verify')
    repo = directory / 'repo'
    receipt.pop('reason', None)
    try:
        git(repo, 'fetch', '--no-tags', 'origin', 'refs/heads/' + receipt['branch'], runner=runner, timeout=NETWORK)
        remote = git(repo, 'rev-parse', 'FETCH_HEAD', runner=runner)
        landed = is_ancestor(repo, receipt['commit'], remote, runner)
        same = landed
        if landed:
            index = json.loads(git(repo, 'show', remote + ':index.json', runner=runner, timeout=NETWORK))
            for contest_id, row in receipt['expected_rows'].items():
                expected = git(repo, 'rev-parse', receipt['commit'] + ':' + row['path'], runner=runner)
                same = (same and [item for item in index['contests'] if item.get('id') == contest_id] == [row]
                        and git(repo, 'rev-parse', remote + ':' + row['path'], runner=runner) == expected)
    except (ArchiveError, *ERRORS):
        receipt.update(state='unknown', reason='remote_unreachable')
        atomic_write_json(directory / 'receipt.json', receipt)
        return receipt
    receipt.update(observed_remote=remote, observed_at=utcnow().isoformat())
    if landed and same:
        receipt.update(state='verified', remote_commit=remote, verified_at=receipt['observed_at'])
        settle_pending(directory, receipt)
    elif landed:
        # The commit is in the branch; a later commit changed the selected rows or trees again.
        receipt.update(state='superseded', remote_commit=remote, reason='selected_content_changed_later')
    elif remote != receipt['base']:
        # The branch moved on without the commit, so the leased push can no longer be accepted.
        receipt.update(state='not_published', reason='remote_advanced_without_commit')
    else:
        receipt.update(state='unknown', reason='commit_not_on_remote')
    atomic_write_json(directory / 'receipt.json', receipt)
    return receipt


def verify(directory, runner=subprocess.run):
    """Look for the recorded commit and the selected rows and trees on the remote as it is now."""
    with DaemonLock(directory / 'publish.lock'):
        return _verify(directory, runner)


def abandon(directory, runner=subprocess.run):
    """Give up a push whose outcome stayed unknown, after one more look at the remote. Nothing is
    pushed or undone; the run only stops holding back later runs for the same contests. Should the
    push arrive after all, a later run meets it as a remote that moved and is refused by its lease."""
    with DaemonLock(directory / 'publish.lock'):
        receipt = _verify(directory, runner)
        if receipt['state'] == 'unknown':
            receipt.update(state='abandoned', abandoned_at=utcnow().isoformat())
            atomic_write_json(directory / 'receipt.json', receipt)
        return receipt


def _publish(ach, directory, runner=subprocess.run):
    with DaemonLock(directory / 'publish.lock'):
        receipt = json.loads((directory / 'receipt.json').read_text())
        if receipt.get('state') != 'prepared':
            raise ArchiveError('push_not_replayable')
        for path in directory.parent.glob('*/receipt.json'):
            if path.parent == directory:
                continue
            try:
                previous = json.loads(path.read_text())
            except ERRORS:
                continue
            if (previous.get('remote_key') == receipt['remote_key'] and previous.get('state') in OPEN
                    and set(previous.get('selected_ids') or []) & set(receipt['selected_ids'])):
                raise ArchiveError('prior_push_unresolved')
        request = json.loads((directory / 'request.json').read_text())
        if basis(selected_sources(ach, request['selection'])) != receipt['basis']:
            raise ArchiveError('workflow_changed')
        repo = directory / 'repo'
        head = git(repo, 'ls-remote', 'origin', 'refs/heads/' + receipt['branch'], runner=runner, timeout=NETWORK).split()
        if not head or head[0] != receipt['base']:
            receipt.update(state='conflict', reason='remote_advanced_before_push')
            atomic_write_json(directory / 'receipt.json', receipt)
            return receipt
        # The candidate is a direct child of the base, and the lease lets the push through only while
        # the branch still is that base: a compare-and-swap, never a rewrite of history.
        if git(repo, 'rev-parse', receipt['commit'] + '^', runner=runner) != receipt['base']:
            raise ArchiveError('parent_mismatch')
        receipt.update(state='pushing', push_started_at=utcnow().isoformat())
        atomic_write_json(directory / 'receipt.json', receipt)
        try:
            git(repo, 'push', '--force-with-lease=refs/heads/' + receipt['branch'] + ':' + receipt['base'],
                'origin', receipt['commit'] + ':refs/heads/' + receipt['branch'], runner=runner, timeout=NETWORK)
        except (ArchiveError, OSError, subprocess.TimeoutExpired):
            pass  # Outcome unknown: never a second push, only a look at the remote for this commit.
        return _verify(directory, runner)


def publish(ach, directory, runner=subprocess.run):
    """Push the prepared commit once, then verify. Sources and workflow state must be as prepared."""
    receipt = json.loads((directory / 'receipt.json').read_text())
    key = receipt.get('remote_key')
    if not isinstance(key, str) or not TOKEN.fullmatch(key):
        raise ArchiveError('remote_identity_missing')
    # One publish at a time per remote and branch in this home, so the check for an unresolved earlier
    # push and the start of this one cannot interleave. Writers elsewhere are held off by the lease.
    with DaemonLock(directory.parent / ('publish-' + key + '.lock')):
        return _publish(ach, directory, runner)


def summary(receipt):
    return {key: receipt.get(key) for key in ('run_id', 'phase', 'state', 'selected_ids', 'base', 'commit',
                                              'remote_commit', 'observed_remote', 'changed_paths', 'reason',
                                              'pending_acknowledgement')}


def main(argv=None):
    parser = argparse.ArgumentParser(prog='controllers.archive')
    actions = parser.add_subparsers(dest='action', required=True)
    p = actions.add_parser('prepare')
    p.add_argument('--file', type=Path, required=True)
    p = actions.add_parser('enqueue')
    p.add_argument('--id', required=True)
    p.add_argument('--phase', choices=PHASES[:2], required=True)
    for action in ('publish', 'verify', 'abandon'):
        p = actions.add_parser(action)
        p.add_argument('--run-id', required=True)
    args = parser.parse_args(argv)
    ach = Ach()
    try:
        if args.action == 'enqueue':
            result = enqueue(ach, args.id, args.phase)
            print(json.dumps(result))
            return 1 if result['state'] == 'error' else 0
        if args.action == 'prepare':
            receipt = prepare(ach, json.loads(args.file.read_text()))
        else:
            directory = journal(ach) / str(uuid.UUID(args.run_id))
            receipt = (publish(ach, directory) if args.action == 'publish'
                       else abandon(directory) if args.action == 'abandon' else verify(directory))
        print(json.dumps(summary(receipt), ensure_ascii=False))
        return 0 if receipt['state'] in ('prepared', 'verified', 'unchanged') else 1
    except (ArchiveError, AchError, LockHeld, *ERRORS) as error:
        reason = str(error) if isinstance(error, ArchiveError) else 'archive_operation_incomplete'
        print(json.dumps({'state': 'blocked', 'reason': reason}))
        return 2


if __name__ == '__main__':
    raise SystemExit(main())
