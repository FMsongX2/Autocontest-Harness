"""`python3 -m controllers queue`: launch only the queued routed contests within capacity.

Every cycle reloads `data/queue-master/queue.json` and `preset.json`, rereads the core
`new_work_frozen` setting and the registry, and starts at most what capacity allows. It only ever
launches `routed` contests named in the queue; it never restarts, resumes or recovers existing
work (that is `ach recover`, an explicit operator action). While new work is frozen nothing is
launched. Warnings are durable alerts, logged only when new, changed or resolved.

Priority: non-video before video; within a group contests due within 21 days first by deadline,
then the rest by gate priority, highest first.
"""
import argparse
import json
import subprocess
import time
from datetime import timedelta
from pathlib import Path

from . import briefs, preset
from .common import (Ach, AchError, Alerts, EventLog, atomic_write_json, deadline_band, error_code,
                     parse_time, read_json, serve, utcnow)

STATE_DIR = Path('data') / 'queue-master'
LOCK = Path('data') / 'controller-queue.lock'
READY = 'QUEUE_READY'
SETTLED = {'awaiting_approval', 'submitted', 'followup_pending', 'closed', 'skipped', 'rejected', 'duplicate'}
URGENT = timedelta(days=21)
ERRORS = (AchError, subprocess.TimeoutExpired, ValueError, KeyError, TypeError, OSError)

DEFAULT_HANDOFF = (
    '작업 시작 전에 .private/controller-brief.md와 .private/gate-evidence/를 읽고 하네스 실행 지시와 함께 '
    '반영하세요. 사용자 제출 승인은 아직 없습니다. 별도 공모전 세션을 시작하지 마세요.'
)


def priority_key(entry, now):
    is_video = 1 if entry.get('type') == 'video' else 0
    deadline = parse_time(entry.get('deadline'))
    due = deadline.timestamp() if deadline else float('inf')
    urgent = deadline is not None and deadline - now <= URGENT
    return (is_video, 0 if urgent else 1, due if urgent else -(entry.get('priority') or 0))


class QueueController:
    def __init__(self, ach, home=None, dry_run=False, sender=subprocess.run, sleep=time.sleep, clock=utcnow):
        self.ach, self.dry_run, self.sender, self.sleep, self.clock = ach, dry_run, sender, sleep, clock
        self.home = Path(home) if home else ach.root
        self.state = self.home / STATE_DIR
        self.log = EventLog(self.state / 'events.jsonl', dry_run, clock)
        self.alerts = Alerts(self.state / 'alerts.json', self.log, dry_run, clock)

    def frozen(self):
        return self.ach.json('config', 'get', 'new_work_frozen') is True

    def finish(self, checked, present, result):
        self.alerts.resolve(checked, present)
        self.alerts.save()
        return result

    def cycle(self):
        now = self.clock()
        try:
            queue = read_json(self.state / 'queue.json')
            if not isinstance(queue, dict) or not isinstance(queue.get('contests'), list):
                raise ValueError('queue.json needs a contests list')
            window = int(queue['window'])
        except (ValueError, KeyError, TypeError, OSError) as error:
            self.alerts.raise_('*', 'queue_config_invalid', [error_code(error)])
            # Nothing else was checked: keep every other alert as it is.
            present = {'*'} | {value.get('contest_id') for value in self.alerts.state.values()}
            return self.finish(set(), present, True)
        contests = [c for c in queue['contests'] if isinstance(c, str)]
        present = {'*'} | set(contests)
        checked = {'*'}
        frozen = self.frozen()
        entries = {entry['id']: entry for entry in self.ach.json('list')}
        pending, states = [], {}
        for contest_id in contests:
            entry = entries.get(contest_id)
            if entry is None:
                states[contest_id] = 'unknown'
                self.alerts.raise_(contest_id, 'queue_unknown_contest')
                checked.add(contest_id)
                continue
            state = entry.get('state')
            states[contest_id] = state
            band = deadline_band(entry.get('deadline'), now)
            if state in SETTLED:
                checked.add(contest_id)
                continue
            if band == 'expired':
                states[contest_id] = 'deadline_expired'
                self.alerts.raise_(contest_id, 'deadline_expired', [f'state:{state}'], band)
                checked.add(contest_id)
                continue
            pending.append(entry)
        pending.sort(key=lambda entry: priority_key(entry, now))

        capacity = self.ach.json('capacity')
        snapshot = {'window': window, 'states': states, 'new_work_frozen': frozen,
                    'running': capacity.get('running'), 'slots': capacity.get('slots'),
                    'pressure': (capacity.get('ram') or {}).get('pressure')}
        self.checkpoint(snapshot)
        if not pending:
            self.log.emit('batch_settled')
            return self.finish(checked, present, False)
        if frozen:
            self.alerts.raise_('*', 'new_work_frozen', [f'pending:{len(pending)}'])
            return self.finish(checked, present, True)

        for entry in pending:
            contest_id = entry['id']
            band = deadline_band(entry.get('deadline'), now)
            if entry.get('state') != 'routed':
                checked.add(contest_id)
                continue
            try:
                outcome = self.launch(entry, queue, window, band)
            except ERRORS as error:
                self.alerts.raise_(contest_id, 'contest_error', [error_code(error)], band)
                continue
            if outcome == 'stop':
                break
            checked.add(contest_id)

        if all(entry.get('state') == 'hold' for entry in pending):
            self.log.emit('batch_blocked', contests=[entry['id'] for entry in pending])
            return self.finish(checked, present, False)
        return self.finish(checked, present, True)

    def checkpoint(self, snapshot):
        """status.json (0600) and a `checkpoint` event, only when the snapshot changed."""
        path = self.state / 'status.json'
        try:
            previous = read_json(path)
        except (ValueError, OSError):
            previous = None
        if previous == snapshot:
            return
        if not self.dry_run:
            atomic_write_json(path, snapshot)
        self.log.emit('checkpoint', **snapshot)

    def launch(self, entry, queue, window, band):
        """'stop' ends this cycle's launches (capacity full or launch unverified), else 'next'."""
        contest_id = entry['id']
        folder = Path(entry.get('folder') or '')
        if not entry.get('folder') or not (folder / briefs.BRIEF).is_file():
            self.alerts.raise_(contest_id, 'brief_pending', [], band)
            return 'next'
        capacity = self.ach.json('capacity')
        if capacity['running'] >= window or capacity['slots'] < 1:
            self.alerts.raise_(contest_id, 'capacity_wait', [], band)
            return 'stop'
        if capacity.get('slots_by_level', {}).get(str(entry.get('level')), 0) < 1:
            self.alerts.raise_(contest_id, 'level_capacity_wait', [f"level:{entry.get('level')}"], band)
            return 'next'
        # The freeze can be set while this cycle runs: check again right before starting new work.
        if self.frozen():
            self.alerts.raise_('*', 'new_work_frozen', ['pending'])
            return 'stop'
        name = self.preset_for(entry, queue)
        if self.dry_run:
            self.log.emit('would_launch', contest_id=contest_id, preset=name)
            return 'next'
        try:
            result = self.ach.json('launch', contest_id, '--preset', name)
        except ERRORS as error:
            # Whether a session started is unknown: never retry blindly in the same cycle.
            self.alerts.raise_(contest_id, 'launch_unverified', [error_code(error)], band)
            return 'stop'
        self.log.emit('launched', contest_id=contest_id, handle=result.get('handle'),
                      workspace_id=result.get('workspace_id'), preset=result.get('preset'))
        handle = result.get('handle')
        if not handle:
            self.log.emit('handoff_unverified', contest_id=contest_id, reason='missing_handle')
            return 'next'
        self.log.emit('handoff', contest_id=contest_id, accepted=self.handoff(handle))
        return 'next'

    def preset_for(self, entry, queue):
        settings = read_json(self.state / 'preset.json', {}) or {}
        if entry.get('type') == 'video':
            name = settings.get('video') or queue['preset']
        else:
            name = settings.get('non_video') or queue.get('preset_non_video') or queue['preset']
        if name == 'auto':
            name = preset.pick(self.ach, settings)
        if not isinstance(name, str) or not preset.NAME.fullmatch(name):
            raise ValueError('invalid preset name')
        return name

    def handoff(self, handle):
        overrides = briefs.load_overrides(self.home)
        text = overrides.get('launch_handoff') or DEFAULT_HANDOFF
        for attempt in range(3):
            try:
                receipt = self.sender(['orca', 'terminal', 'send', '--terminal', handle, '--text', text,
                                       '--enter', '--json'], cwd=self.home, capture_output=True,
                                      text=True, timeout=30)
                delivered = json.loads(receipt.stdout) if receipt.returncode == 0 else {}
                if delivered.get('result', {}).get('send', {}).get('accepted', False):
                    return True
            except (subprocess.TimeoutExpired, ValueError, OSError, AttributeError):
                pass
            if attempt < 2:
                self.sleep(10)
        return False


def main(argv, ach=None):
    parser = argparse.ArgumentParser(prog='python3 -m controllers queue')
    parser.add_argument('--once', action='store_true', help='run one cycle and exit')
    parser.add_argument('--dry-run', action='store_true', help='report launches; take no lock, write nothing')
    parser.add_argument('--interval', type=float, default=60, help='seconds between cycles')
    options = parser.parse_args(argv)
    ach = ach or Ach()
    return serve(lambda client: QueueController(client, dry_run=options.dry_run), ach,
                 ach.root / LOCK, READY, options.interval, options.once, options.dry_run)
