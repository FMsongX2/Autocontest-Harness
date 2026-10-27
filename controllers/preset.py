"""`python3 -m controllers preset`: switch contest sessions between omp model presets.

Usage:
  preset <PresetName> <contest-id> [<contest-id> ...]   restart those sessions under the preset
  preset <PresetName> --all                              every live non-video session
  preset <PresetName> --video                            every live video session
  preset --pick                                          print the preset `auto` resolves to now
  add --resume to also reopen named submitted/followup_pending contests (`ach resume`, moves them to running)
  add --dry-run to print what `ach` would run without touching any session or file

Every switch goes through `ach restart <id> --preset <name>` (running, awaiting_approval, plan_review),
which closes the recorded session through its workspace backend, restores the saved conversation and
keeps ACH_SESSION, PATH, runner args and the recorded runner/workspace; the core validates the
preset again. This command never types into terminals, never starts omp itself and never edits omp
settings. The chosen presets for --all/--video are written to data/queue-master/preset.json, which
the queue controller reads before every new launch. Preset names are checked against `modelPresets`
in ~/.omp/agent/config.yml (read only) before anything is changed.
"""
import re
import sys
from pathlib import Path

from .common import Ach, AchError, atomic_write_json, read_json

STATE = Path('data') / 'queue-master' / 'preset.json'
NAME = re.compile(r'[A-Za-z0-9][A-Za-z0-9_.-]*')
# `auto` (queue presets) is resolved with preset.json's private `auto_providers` mapping
# ({provider: preset}) and optional `auto_default`: the provider with more headroom wins.


def omp_config():
    return Path.home() / '.omp' / 'agent' / 'config.yml'


def preset_names(path=None):
    """Top-level keys of the `modelPresets:` mapping (a block mapping, read without a YAML library)."""
    text = Path(path or omp_config()).read_text(encoding='utf-8')
    names, inside, indent = set(), False, None
    for line in text.splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith('#'):
            continue
        depth = len(line) - len(line.lstrip(' '))
        if depth == 0:
            inside = stripped.split('#', 1)[0].rstrip() == 'modelPresets:'
            indent = None
            continue
        if not inside:
            continue
        if indent is None:
            indent = depth
        if depth != indent:
            continue
        key = stripped.split(':', 1)[0].strip()
        if len(key) >= 2 and key[0] == key[-1] and key[0] in '"\'':
            key = key[1:-1]
        if ':' in stripped and key:
            names.add(key)
    return names


def pick(ach, settings):
    """The `auto` preset: the provider whose tightest short window has the most remaining.

    `settings` is preset.json; without an `auto_providers` mapping `auto` is not configured."""
    providers = (settings or {}).get('auto_providers')
    if not isinstance(providers, dict) or not providers:
        raise ValueError('auto preset is not configured (preset.json auto_providers)')
    default = settings.get('auto_default') or next(iter(providers.values()))
    try:
        windows = ach.json('budget', timeout=60)['windows']
    except (AchError, KeyError, TypeError, ValueError):
        return default
    scores = {}
    for provider in providers:
        remaining = [w['remaining'] for w in windows
                     if isinstance(w, dict) and w.get('provider') == provider
                     and isinstance(w.get('remaining'), (int, float))
                     and not str(w.get('window', '')).startswith('30 day')
                     and 'Fable' not in str(w.get('window', ''))]
        if remaining:
            scores[provider] = min(remaining)
    if not scores:
        return default
    return providers[max(scores, key=scores.get)]


def call(ach, *args):
    """(ok, parsed JSON or error text) for an interactive command."""
    try:
        return True, ach.json(*args)
    except AchError as error:
        return False, error.stderr or str(error)


def switch(ach, contest_id, name, resume, dry_run):
    ok, entry = call(ach, 'show', contest_id)
    if not ok:
        return f'{contest_id}: {entry}'
    state = entry.get('state')
    if state in RESTART_STATES:
        command = 'restart'
    elif state in RESUME_STATES and resume:
        command = 'resume'
    else:
        hint = ' (--resume to reopen it)' if state in RESUME_STATES else ''
        return f'{contest_id}: skipped, state {state}{hint}'
    args = [command, contest_id, '--preset', name]
    if dry_run:
        args.append('--dry-run')
    ok, result = call(ach, *args)
    if not ok:
        return f'{contest_id}: {command} failed: {result}'
    return (f"{contest_id}: {command} {name} mode={result.get('mode')} "
            f"session={str(result.get('session_id') or '-')[:8]} handle={result.get('handle')} "
            f"state={result.get('state')}")


def live(ach, video):
    ok, entries = call(ach, 'list')
    if not ok:
        raise SystemExit(f'ach list failed: {entries}')
    out = []
    for entry in entries:
        if entry['id'].startswith('legacy') or entry.get('state') not in RESTART_STATES:
            continue
        if (entry.get('type') == 'video') != video:
            continue
        if (entry.get('session') or {}).get('handle'):
            out.append(entry['id'])
    return out


def main(argv, ach=None, names=None):
    ach = ach or Ach()
    flags = {a for a in argv if a in ('--resume', '--dry-run')}
    rest = [a for a in argv if a not in flags]
    if rest == ['--pick']:
        try:
            print(pick(ach, read_json(ach.root / STATE, {})), flush=True)
        except ValueError as error:
            print(error, file=sys.stderr)
            return 2
        return 0
    if not rest:
        print(__doc__, file=sys.stderr)
        return 2
    name, args = rest[0], rest[1:]
    known = names if names is not None else preset_names()
    if not NAME.fullmatch(name) or name not in known:
        print(f'preset not found: {name}', file=sys.stderr)
        return 2
    if args and args[0] in ('--all', '--video'):
        video = args[0] == '--video'
        if '--dry-run' not in flags:
            path = ach.root / STATE
            state = read_json(path, {}) or {}
            state['video' if video else 'non_video'] = name
            atomic_write_json(path, state)
        targets = live(ach, video)
    else:
        targets = args
    for contest in targets:
        if contest.startswith('-'):
            print(f'{contest}: not a contest id', flush=True)
            continue
        print(switch(ach, contest, name, '--resume' in flags, '--dry-run' in flags), flush=True)
    return 0
