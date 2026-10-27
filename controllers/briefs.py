"""`python3 -m controllers briefs`: controller briefs (`.private/controller-brief.md`) for routed contests.

The common rules and the procedure section of a brief are generated text that lives between
managed-region markers; everything outside them (title, contest summary, contest-specific notes,
anything a user or session added) belongs to the contest and is never rewritten.

The generated text is data, not code: `data/queue-master/brief-overrides.json` (private, never
published) may set `version`, `common`, `procedure`, per-contest `special` sections and the queue's
`launch_handoff` text. Without it the generic public defaults below apply.

  briefs create <id> [<id> ...]
      Write a brief for each routed contest that has none (existing briefs are left alone).

  briefs refresh [--dry-run] [--adopt-legacy] (--id <id> ... | --file <brief> ...)
      Rewrite only the managed regions of existing briefs to the current generated text.
      A region whose recorded hash no longer matches its body was edited by hand and is
      reported as `modified`, not overwritten. A brief without markers is `legacy`: with
      --adopt-legacy its one recognised generated section is replaced by the managed region
      and the replaced text is saved next to the brief; otherwise it is only reported.
      Anything else (`unrecognized`, `malformed`) is reported for manual review.
      --id only touches contests whose registry state is still active.
      Output: one JSON object per brief; exit code 3 when any brief needs review.
"""
import argparse
import hashlib
import json
import re
import sys
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path

from .common import ACTIVE_STATES, Ach, AchError, atomic_write, read_json

BRIEF = Path('.private/controller-brief.md')
OVERRIDES = Path('data') / 'queue-master' / 'brief-overrides.json'
DEFAULT_VERSION = '0.8.0'
REVIEW = {'modified', 'malformed', 'unrecognized', 'legacy', 'missing'}

DEFAULT_COMMON = """## 필수 운영 규칙

- `data/local-rules.md`의 운영 규칙을 먼저 읽고 따른다.
- 공식 공고 원문이 우선한다. 규칙 변경은 `ach rules capture`·`ach rules review`로 기록한다.
- 제출 승인 전에는 접수·계정 가입·외부 업로드를 하지 않는다. 제출은 `ach submission intent`·`start`·`unknown`·`ach submit` 절차로만 한다.
- 비밀번호·토큰·개인정보는 어디에도 복제하지 않는다.
"""

DEFAULT_PROCEDURE = """## 절차

CONTEST.md, `.harness/gate.json`, `.private/gate-evidence/`를 먼저 읽는다. 해당 레벨의 조사·기획 채점·초안·최종 점수 루프를 수행하고, 최종 채점·`ach check`·`ach readiness` 뒤 `awaiting_approval`에서 멈춘다.
"""

# Managed regions in document order: name -> legacy heading prefix.
HEADINGS = {'common': '## 필수 운영 규칙', 'procedure': '## 절차'}
MARKER = re.compile(
    r'^<!-- controller-brief:(?P<name>[a-z]+) (?P<kind>begin|end)'
    r'(?: version=(?P<version>\S+) sha256=(?P<sha>[0-9a-f]{64}))? -->\n?',
    re.M,
)


@dataclass
class Template:
    """The generated text of briefs: managed region bodies, their version, per-contest sections."""
    version: str = DEFAULT_VERSION
    regions: dict = field(default_factory=lambda: {'common': DEFAULT_COMMON, 'procedure': DEFAULT_PROCEDURE})
    special: dict = field(default_factory=dict)


def load_overrides(home):
    """The private overrides object (empty when the file is absent)."""
    value = read_json(Path(home) / OVERRIDES, {})
    if not isinstance(value, dict):
        raise ValueError(f'{OVERRIDES} must hold a JSON object')
    return value


def template_from(overrides):
    base = Template()
    regions = dict(base.regions)
    for name in HEADINGS:
        text = overrides.get(name)
        if text is not None:
            if not isinstance(text, str) or not text.startswith(HEADINGS[name]):
                raise ValueError(f'brief override `{name}` must be text starting with {HEADINGS[name]!r}')
            regions[name] = text
    special = overrides.get('special') or {}
    if not isinstance(special, dict) or not all(isinstance(v, str) for v in special.values()):
        raise ValueError('brief override `special` must map contest ids to text')
    version = overrides.get('version') or DEFAULT_VERSION
    if not isinstance(version, str) or not re.fullmatch(r'\S+', version):
        raise ValueError('brief override `version` must be one word')
    return Template(version=version, regions=regions, special=special)


def load_template(home):
    return template_from(load_overrides(home))


def digest(body):
    return hashlib.sha256(body.encode('utf-8')).hexdigest()


def region(name, body, version):
    """The managed region for `name`: markers around `body`, hash of `body` in the begin marker."""
    return (f'<!-- controller-brief:{name} begin version={version} sha256={digest(body)} -->\n'
            f'{body}<!-- controller-brief:{name} end -->\n')


def refresh_region(text, name, generated, version, adopt_legacy):
    """Refresh one managed region of a brief's text.

    Returns (status, new_text, replaced) where new_text is None when nothing may change and
    replaced is the legacy text that an adoption removed (to be saved for review).
    """
    marks = [m for m in MARKER.finditer(text) if m.group('name') == name]
    if marks:
        kinds = [m.group('kind') for m in marks]
        if kinds != ['begin', 'end'] or marks[0].group('sha') is None:
            return 'malformed', None, None
        begin, end = marks
        body = text[begin.end():end.start()]
        if digest(body) != begin.group('sha'):
            return 'modified', None, None
        if body == generated and begin.group('version') == version:
            return 'current', None, None
        return 'updated', text[:begin.start()] + region(name, generated, version) + text[end.end():], None
    lines = text.splitlines(keepends=True)
    starts = [i for i, line in enumerate(lines) if line.startswith(HEADINGS[name])]
    if len(starts) != 1:
        return 'unrecognized', None, None
    start = starts[0]
    stop = next((i for i in range(start + 1, len(lines)) if lines[i].startswith('## ')), len(lines))
    section = ''.join(lines[start:stop])
    if not adopt_legacy:
        return 'legacy', None, None
    # Keep the blank line that separated the old section from what followed it.
    trailing = '\n' if section.endswith('\n\n') else ''
    new = ''.join(lines[:start]) + region(name, generated, version) + trailing + ''.join(lines[stop:])
    return 'adopted', new, section


def refresh_text(text, template, adopt_legacy=False):
    """Refresh every managed region. Returns (statuses, new_text or None, replaced sections)."""
    statuses, replaced, current = {}, {}, text
    for name in HEADINGS:
        status, new, old = refresh_region(current, name, template.regions[name], template.version, adopt_legacy)
        statuses[name] = status
        if new is not None:
            current = new
        if old is not None:
            replaced[name] = old
    return statuses, (current if current != text else None), replaced


def refresh_file(path, template, dry_run=False, adopt_legacy=False, now=None):
    """Refresh one brief file. Returns a JSON-able report; writes only when not dry_run."""
    path = Path(path)
    report = {'file': str(path), 'regions': {}, 'changed': False, 'backup': None, 'review': False}
    if not path.is_file() or path.is_symlink():
        report['regions'] = {name: 'missing' for name in HEADINGS}
        report['review'] = True
        return report
    text = path.read_text(encoding='utf-8')
    statuses, new, replaced = refresh_text(text, template, adopt_legacy)
    report['regions'] = statuses
    report['review'] = any(s in REVIEW for s in statuses.values())
    report['changed'] = new is not None
    if new is None or dry_run:
        return report
    if replaced:
        stamp = (now or datetime.now(timezone.utc)).strftime('%Y%m%dT%H%M%SZ')
        backup = path.with_name(f'{path.stem}.legacy-{stamp}.md')
        if backup.exists():
            raise SystemExit(f'오류: 백업 파일이 이미 있다: {backup}')
        parts = [f'<!-- {name}: refresh 전 원문 -->\n{old}' for name, old in replaced.items()]
        atomic_write(backup, '\n'.join(parts), 0o600)
        report['backup'] = str(backup)
    atomic_write(path, new, path.stat().st_mode & 0o777)
    return report


def show(ach, contest_id):
    if not re.fullmatch(r'[a-z0-9-]+', contest_id):
        raise SystemExit(f'오류: 레지스트리 id 형식이 아니다: {contest_id!r}')
    try:
        return ach.json('show', contest_id, timeout=120)
    except AchError as error:
        raise SystemExit(f'오류: ach show {contest_id} 실패: {error.stderr or error}') from error


def render(entry, contest_id, template):
    gate = entry.get('gate') or {}
    deliverables = '\n'.join(f"  - {d.get('name')} ({d.get('format')}): {str(d.get('limits'))[:420]}"
                             for d in gate.get('deliverables', []))
    criteria = '\n'.join(f"  - {c.get('stage')}: {c.get('name')} {c.get('weight')}" for c in gate.get('criteria', []))
    return f"""# 실행 인계: {entry['title']}

{region('common', template.regions['common'], template.version)}
## 공모전 요약(게이트 기록, 데이터)

- 유형 {entry.get('type')}, 레벨 {entry.get('level')}. 마감 {entry.get('deadline')}. 공식 URL {entry.get('url')}
- 상금: {gate.get('prize')}
- 제출물:
{deliverables}
- 심사 기준:
{criteria}
- 게이트 판정 근거 전문은 `.private/gate-evidence/gate.json`에 있다.

{template.special.get(contest_id, '')}
{region('procedure', template.regions['procedure'], template.version)}"""


def create(ach, template, ids):
    for contest_id in ids:
        entry = show(ach, contest_id)
        folder = Path(entry['folder'])
        brief = folder / BRIEF
        if brief.exists():
            print('exists', contest_id)
            continue
        evidence = folder / '.private/gate-evidence'
        evidence.mkdir(parents=True, exist_ok=True)
        atomic_write(evidence / 'gate.json', json.dumps(entry.get('gate') or {}, ensure_ascii=False, indent=2))
        body = render(entry, contest_id, template)
        atomic_write(brief, body)
        print('wrote', contest_id, len(body))


def refresh(ach, template, ids, files, dry_run, adopt_legacy):
    targets = [(None, Path(f)) for f in files]
    reports = []
    for contest_id in ids:
        entry = show(ach, contest_id)
        if entry.get('state') not in ACTIVE_STATES or not entry.get('folder'):
            reports.append({'id': contest_id, 'skipped': f"상태 {entry.get('state')}: 갱신하지 않는다"})
            continue
        targets.append((contest_id, Path(entry['folder']) / BRIEF))
    for contest_id, path in targets:
        report = refresh_file(path, template, dry_run=dry_run, adopt_legacy=adopt_legacy)
        if contest_id:
            report['id'] = contest_id
        reports.append(report)
    for report in reports:
        print(json.dumps(report, ensure_ascii=False))
    return 3 if any(r.get('review') for r in reports) else 0


def main(argv, ach=None):
    parser = argparse.ArgumentParser(prog='python3 -m controllers briefs',
                                     description='Controller briefs for routed contests.')
    sub = parser.add_subparsers(dest='command', required=True)
    p_create = sub.add_parser('create', help='write missing briefs')
    p_create.add_argument('ids', nargs='+')
    p_refresh = sub.add_parser('refresh', help='rewrite only the managed regions of existing briefs')
    p_refresh.add_argument('--dry-run', action='store_true')
    p_refresh.add_argument('--adopt-legacy', action='store_true')
    group = p_refresh.add_mutually_exclusive_group(required=True)
    group.add_argument('--id', dest='ids', nargs='+', default=[])
    group.add_argument('--file', dest='files', nargs='+', default=[])
    args = parser.parse_args(argv)
    ach = ach or Ach()
    try:
        template = load_template(ach.root)
    except (ValueError, OSError) as error:
        print(f'오류: {OVERRIDES}: {error}', file=sys.stderr)
        return 2
    if args.command == 'create':
        create(ach, template, args.ids)
        return 0
    return refresh(ach, template, args.ids, args.files, args.dry_run, args.adopt_legacy)
