"""Managed controllers (`python3 -m controllers`): lifecycle guards, contract preflight, lifetime
locks, durable alerts and the queue's freeze/reload rules.

Everything runs against a temporary ACH_HOME and a scripted fake `ach`; no real CLI, contest,
session, terminal or network is touched.
"""
import json
import os
import stat
import subprocess
import sys
import tempfile
import unittest
from datetime import datetime, timedelta, timezone
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from controllers import __main__ as entry_point  # noqa: E402
from controllers import approve, common, queue  # noqa: E402

NOW = datetime(2026, 10, 25, 12, 0, tzinfo=timezone.utc)


class FakeAch:
    """Scripted `ach`: `responses[command]` is a value or a callable(args) -> value; every call is
    recorded. `run` returns a CompletedProcess; `rc[command]` sets its exit code."""

    def __init__(self, home, responses=None, rc=None):
        self.root = Path(home)
        self.responses = dict(responses or {})
        self.rc = dict(rc or {})
        self.calls = []

    def value(self, args):
        self.calls.append(list(args))
        response = self.responses.get(args[0], {})
        return response(args) if callable(response) else response

    def json(self, *args, timeout=180):
        value = self.value(args)
        if isinstance(value, Exception):
            raise value
        if self.rc.get(args[0]):
            raise common.AchError(args[0], 'ach_failed', self.rc[args[0]])
        return value

    def run(self, *args, timeout=180, check=True):
        value = self.value(args)
        code = self.rc.get(args[0], 0)
        if check and code:
            raise common.AchError(args[0], 'ach_failed', code)
        return subprocess.CompletedProcess([], code, stdout=json.dumps(value), stderr='')

    def commands(self):
        return [call[0] for call in self.calls]


def contest(state='submitted', age_minutes=40, **extra):
    at = (datetime.now(timezone.utc) - timedelta(minutes=age_minutes)).isoformat()
    value = {
        'id': 'fixture-contest', 'state': state, 'folder': None,
        'history': [{'from': 'awaiting_approval', 'to': state, 'at': at}],
        'session': {'handle': 'fixture-session', 'workspace_id': 'fixture-workspace'},
    }
    value.update(extra)
    return value


def events(path):
    path = Path(path)
    if not path.exists():
        return []
    return [json.loads(line) for line in path.read_text(encoding='utf-8').splitlines()]


class Home(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.home = Path(self.tmp.name)
        (self.home / 'data').mkdir()
        self.quiet = patch('builtins.print')
        self.quiet.start()

    def tearDown(self):
        self.quiet.stop()
        self.tmp.cleanup()

    def files(self):
        return sorted(str(p.relative_to(self.home)) for p in self.home.rglob('*') if p.is_file())


class ApproveLifecycleTests(Home):
    def controller(self, ach, dry_run=False):
        return approve.ApproveController(ach, home=self.home, dry_run=dry_run)

    def test_close_rechecks_resumed_state_instead_of_stale_listing(self):
        ach = FakeAch(self.home, {'show': contest('running')})
        self.controller(ach).close_finished_session(contest('followup_pending'))
        self.assertEqual(ach.commands(), ['show'])

    def test_failed_close_is_retried_and_never_marked_complete(self):
        current = contest('closed')
        attempts = []

        def close(args):
            attempts.append(args)
            return {}

        ach = FakeAch(self.home, {'show': current, 'session-close': close}, rc={'session-close': 1})
        controller = self.controller(ach)
        controller.close_finished_session(current)
        ach.rc['session-close'] = 0
        controller.close_finished_session(current)
        self.assertEqual(len(attempts), 2)
        kinds = [(e['event'], e.get('code')) for e in events(self.home / 'data/auto-approve/events.jsonl')]
        self.assertEqual(kinds, [('alert', 'session_close_refused'), ('session_closed', None)])

    def test_missing_or_positive_followup_never_triggers_folder_cleanup(self):
        with tempfile.TemporaryDirectory() as folder:
            for request in [None, {}, {'followup_required': True}]:
                current = contest(folder=folder, submission_readiness=request)
                ach = FakeAch(self.home, {'show': current})
                self.controller(ach).clean_submitted(current)
                self.assertEqual(ach.commands(), ['show'])

    def test_grace_period_is_not_reset_by_session_bookkeeping(self):
        current = contest(age_minutes=40)
        current['history'].append({'from': 'submitted', 'to': 'submitted',
                                   'at': datetime.now(timezone.utc).isoformat()})
        self.assertGreater(approve.elapsed_in_state(current), timedelta(minutes=39))
        current['history'][0]['at'] = '2026-01-01T00:00:00'
        self.assertEqual(approve.elapsed_in_state(current), timedelta(0))

    def test_dry_run_does_not_close_sessions_or_clean_folders_or_write(self):
        with tempfile.TemporaryDirectory() as folder:
            current = contest(folder=folder, submission_readiness={'followup_required': False})
            ach = FakeAch(self.home, {'show': current})
            controller = self.controller(ach, dry_run=True)
            controller.close_finished_session(current)
            controller.clean_submitted(current)
            self.assertEqual(ach.commands(), ['show', 'show'])
            self.assertTrue(Path(folder).is_dir())
        self.assertEqual(self.files(), [])

    def test_reconcile_is_applied_as_observation_only(self):
        ach = FakeAch(self.home, {'reconcile': {'entries': [], 'changed': False}, 'list': []})
        self.controller(ach).cycle()
        self.assertEqual(ach.calls, [['reconcile', '--apply'], ['list']])
        dry = FakeAch(self.home, {'reconcile': {'entries': [], 'changed': False}, 'list': []})
        self.controller(dry, dry_run=True).cycle()
        self.assertEqual(dry.calls[0], ['reconcile'])
        for forbidden in ('recover', 'restart', 'resume', 'launch'):
            self.assertNotIn(forbidden, ach.commands() + dry.commands())

    def test_eligible_request_is_approved_with_its_exact_token(self):
        waiting = contest('awaiting_approval', session={})
        ach = FakeAch(self.home, {
            'reconcile': {'entries': []}, 'list': [waiting], 'show': waiting,
            'approval-status': {'eligible': True, 'request_token': 'tok-1'}, 'approve': {},
        })
        self.controller(ach).cycle()
        approved = next(call for call in ach.calls if call[0] == 'approve')
        self.assertIn('--delegated', approved)
        self.assertEqual(approved[approved.index('--expected-request') + 1], 'tok-1')
        self.assertIn('submission intent', approve.HANDOFF)
        self.assertIn('submission start', approve.HANDOFF)
        self.assertIn('submission unknown', approve.HANDOFF)
        self.assertIn('ach submit <id> --attempt', approve.HANDOFF)


class PreflightAndLockTests(Home):
    def runner(self, protocol, seen):
        def run(argv, **kwargs):
            seen.append(argv[1:])
            if argv[1] == 'contract':
                body = {'protocol': protocol, 'version': '0.8.0', 'capabilities': []}
                return subprocess.CompletedProcess(argv, 0, stdout=json.dumps(body), stderr='')
            return subprocess.CompletedProcess(argv, 0, stdout='[]', stderr='')
        return run

    def test_protocol_mismatch_fails_before_any_side_effect(self):
        for mode in (['approve', '--once'], ['queue', '--once'], ['preset', 'X', '--all'],
                     ['briefs', 'refresh', '--file', 'brief.md']):
            seen = []
            ach = common.Ach(home=self.home, binary='ach-fixture', runner=self.runner(1, seen))
            self.assertEqual(entry_point.main(mode, ach=ach), common.EXIT_CONTRACT, mode)
            self.assertEqual(seen, [['contract', '--json']], mode)
            self.assertEqual(self.files(), [], mode)

    def test_ach_runs_with_argv_and_the_discovered_root(self):
        seen, captured = [], {}

        def run(argv, **kwargs):
            captured.update(kwargs)
            return self.runner(2, seen)(argv, **kwargs)

        ach = common.Ach(home=self.home, binary='ach-fixture', runner=run)
        self.assertEqual(common.preflight(ach)['protocol'], 2)
        self.assertEqual(captured['env']['ACH_HOME'], str(self.home))
        self.assertEqual(captured['cwd'], self.home)
        self.assertNotIn('shell', captured)

    def test_root_is_ach_home_or_the_package_checkout(self):
        with patch.dict(os.environ, {'ACH_HOME': str(self.home)}):
            self.assertEqual(common.root(), self.home)
        with patch.dict(os.environ, {'ACH_HOME': ''}):
            self.assertEqual(common.root(), ROOT)

    def test_duplicate_daemon_is_refused_while_the_lock_is_held(self):
        lock = self.home / approve.LOCK
        cycles = []

        class Controller:
            log = None

            def cycle(self):
                cycles.append(1)
                return False

        ach = FakeAch(self.home, {'contract': {'protocol': 2}})
        with common.DaemonLock(lock):
            code = common.serve(lambda _: Controller(), ach, lock, 'READY', 0, True, False)
            self.assertEqual(code, common.EXIT_LOCKED)
            self.assertEqual(cycles, [])
        self.assertEqual(stat.S_IMODE(lock.stat().st_mode), 0o600)
        # The kernel lock is gone with its holder: the next start runs.
        self.assertEqual(common.serve(lambda _: Controller(), ach, lock, 'READY', 0, True, False), 0)
        self.assertEqual(cycles, [1])

    def test_dry_run_takes_no_lock(self):
        lock = self.home / queue.LOCK

        class Controller:
            log = None

            def cycle(self):
                return False

        ach = FakeAch(self.home, {'contract': {'protocol': 2}})
        with common.DaemonLock(lock):
            self.assertEqual(common.serve(lambda _: Controller(), ach, lock, 'READY', 0, True, True), 0)

    def test_contract_change_mid_run_stops_before_the_next_cycle(self):
        replies = iter([{'protocol': 2}, {'protocol': 3}])
        ach = FakeAch(self.home, {'contract': lambda _: next(replies)})
        cycles = []

        class Controller:
            log = None

            def cycle(self):
                cycles.append(1)
                return True

        code = common.serve(lambda _: Controller(), ach, self.home / 'data/x.lock', 'READY', 0, False, False,
                            sleep=lambda _: None)
        self.assertEqual(code, common.EXIT_CONTRACT)
        self.assertEqual(cycles, [1])


class AlertTests(Home):
    def book(self, clock=lambda: NOW):
        log = common.EventLog(self.home / 'data/auto-approve/events.jsonl', clock=clock)
        return common.Alerts(self.home / 'data/auto-approve/alerts.json', log, clock=clock)

    def logged(self):
        return [(e['event'], e.get('code'), e.get('change')) for e in
                events(self.home / 'data/auto-approve/events.jsonl')]

    def test_identical_warning_is_suppressed_across_restarts(self):
        for _ in range(3):
            book = self.book()
            book.raise_('c1', 'approval_blocked', ['score_missing'], 'gte7d')
            book.resolve({'c1'}, {'c1'})
            book.save()
        self.assertEqual(self.logged(), [('alert', 'approval_blocked', 'new')])
        state = self.home / 'data/auto-approve/alerts.json'
        self.assertEqual(stat.S_IMODE(state.stat().st_mode), 0o600)
        self.assertEqual(sorted(os.listdir(state.parent)), ['alerts.json', 'events.jsonl'])

    def test_changed_status_or_deadline_band_is_logged_again(self):
        book = self.book()
        book.raise_('c1', 'approval_blocked', ['score_missing'], 'gte7d')
        book.save()
        book = self.book()
        book.raise_('c1', 'approval_blocked', ['score_missing', 'rules_pending'], 'gte7d')
        book.save()
        book = self.book()
        book.raise_('c1', 'approval_blocked', ['rules_pending', 'score_missing'], 'lt72h')
        book.save()
        self.assertEqual(self.logged(), [('alert', 'approval_blocked', 'new'),
                                         ('alert', 'approval_blocked', 'changed'),
                                         ('alert', 'approval_blocked', 'changed')])

    def test_resolution_is_logged_once_and_only_for_checked_contests(self):
        book = self.book()
        book.raise_('c1', 'clean_refused', ['rc:1'])
        book.raise_('c2', 'clean_refused', ['rc:1'])
        book.save()
        book = self.book()
        book.resolve(checked={'c1'}, present={'c1', 'c2'})  # c2 errored: not evidence of resolution
        book.save()
        book = self.book()
        book.resolve(checked={'c1'}, present={'c1', 'c2'})
        book.save()
        self.assertEqual(self.logged(), [('alert', 'clean_refused', 'new'), ('alert', 'clean_refused', 'new'),
                                         ('alert_resolved', 'clean_refused', None)])
        stored = json.loads((self.home / 'data/auto-approve/alerts.json').read_text())['alerts']
        self.assertEqual(list(stored), ['c2:clean_refused'])

    def test_vanished_contest_alerts_are_resolved(self):
        book = self.book()
        book.raise_('gone', 'approval_blocked', ['x'])
        book.resolve(checked=set(), present={'*'})
        self.assertEqual(book.state, {})

    def test_private_prose_never_enters_fingerprints_or_logs(self):
        book = self.book()
        book.raise_('c1', 'approval_blocked', ['ok_code', '개인 메모가 들어간\n긴 사유', 'x' * 200])
        self.assertEqual(book.state['c1:approval_blocked']['status'], ['ok_code'])

    def test_deadline_bands(self):
        def at(**delta):
            return (NOW + timedelta(**delta)).isoformat()
        self.assertEqual(common.deadline_band(None, NOW), 'none')
        self.assertEqual(common.deadline_band('2026-11-01', NOW), 'invalid')
        self.assertEqual(common.deadline_band(at(hours=-1), NOW), 'expired')
        self.assertEqual(common.deadline_band(at(hours=5), NOW), 'lt24h')
        self.assertEqual(common.deadline_band(at(hours=48), NOW), 'lt72h')
        self.assertEqual(common.deadline_band(at(days=5), NOW), 'lt7d')
        self.assertEqual(common.deadline_band(at(days=30), NOW), 'gte7d')

    def test_controller_restart_repeats_no_unchanged_approval_warning(self):
        waiting = contest('awaiting_approval', deadline=(NOW + timedelta(days=30)).isoformat())
        responses = {'reconcile': {'entries': []}, 'list': [waiting], 'show': waiting,
                     'approval-status': {'eligible': False, 'reasons': ['score_missing']}}
        for _ in range(2):
            approve.ApproveController(FakeAch(self.home, responses), home=self.home, clock=lambda: NOW).cycle()
        self.assertEqual(self.logged(), [('alert', 'approval_blocked', 'new')])
        responses['approval-status'] = {'eligible': False, 'reasons': ['rules_untracked']}
        approve.ApproveController(FakeAch(self.home, responses), home=self.home, clock=lambda: NOW).cycle()
        responses['approval-status'] = {'submit_approval_valid': True}
        approve.ApproveController(FakeAch(self.home, responses), home=self.home, clock=lambda: NOW).cycle()
        self.assertEqual(self.logged(), [('alert', 'approval_blocked', 'new'),
                                         ('alert', 'approval_blocked', 'changed'),
                                         ('alert_resolved', 'approval_blocked', None)])


class QueueTests(Home):
    def setUp(self):
        super().setUp()
        self.state = self.home / 'data/queue-master'
        self.state.mkdir(parents=True)
        self.folder = self.home / 'contest'
        (self.folder / '.private').mkdir(parents=True)
        (self.folder / '.private/controller-brief.md').write_text('brief\n', encoding='utf-8')
        self.write_queue(['c1'])
        (self.state / 'preset.json').write_text(json.dumps({'non_video': 'PresetA'}), encoding='utf-8')
        self.frozen = False
        self.entries = [{'id': 'c1', 'state': 'routed', 'level': 2, 'folder': str(self.folder),
                         'deadline': (NOW + timedelta(days=30)).isoformat()},
                        {'id': 'c2', 'state': 'routed', 'level': 2, 'folder': str(self.folder),
                         'deadline': (NOW + timedelta(days=10)).isoformat()}]

    def write_queue(self, contests):
        (self.state / 'queue.json').write_text(json.dumps({'window': 3, 'preset': 'PresetV', 'contests': contests}),
                                               encoding='utf-8')

    def ach(self):
        return FakeAch(self.home, {
            'config': lambda args: self.frozen, 'list': lambda args: self.entries,
            'capacity': {'running': 0, 'slots': 2, 'slots_by_level': {'2': 2}, 'ram': {'pressure': 'ok'}},
            'launch': lambda args: {'handle': None, 'preset': args[args.index('--preset') + 1]},
        })

    def controller(self, ach):
        sent = lambda *a, **k: subprocess.CompletedProcess([], 1, stdout='', stderr='')  # noqa: E731
        return queue.QueueController(ach, home=self.home, sender=sent, sleep=lambda _: None, clock=lambda: NOW)

    def launched(self, ach):
        return [call[1] for call in ach.calls if call[0] == 'launch']

    def test_freeze_is_honored_every_cycle_and_reread_before_launch(self):
        self.frozen = True
        ach = self.ach()
        controller = self.controller(ach)
        self.assertTrue(controller.cycle())
        self.assertTrue(controller.cycle())
        self.assertEqual(self.launched(ach), [])
        alerts = [e for e in events(self.state / 'events.jsonl') if e['event'] == 'alert']
        self.assertEqual([e['code'] for e in alerts], ['new_work_frozen'])
        self.frozen = False
        controller.cycle()
        self.assertEqual(self.launched(ach), ['c1'])
        self.assertIn('alert_resolved', [e['event'] for e in events(self.state / 'events.jsonl')])

    def test_queue_and_presets_are_reloaded_every_cycle(self):
        ach = self.ach()
        controller = self.controller(ach)
        self.entries[0]['state'] = 'running'
        controller.cycle()
        self.assertEqual(self.launched(ach), [])
        self.write_queue(['c1', 'c2'])
        (self.state / 'preset.json').write_text(json.dumps({'non_video': 'PresetB'}), encoding='utf-8')
        controller.cycle()
        launch = next(call for call in ach.calls if call[0] == 'launch')
        self.assertEqual(launch, ['launch', 'c2', '--preset', 'PresetB'])

    def test_no_hidden_recovery_of_existing_work(self):
        for state in ('running', 'hold', 'plan_review', 'awaiting_approval'):
            self.entries[0]['state'] = state
            ach = self.ach()
            self.controller(ach).cycle()
            self.assertFalse({'launch', 'restart', 'resume', 'recover'} & set(ach.commands()), state)

    def test_unknown_contest_and_missing_brief_are_isolated_alerts(self):
        self.write_queue(['missing-id', 'c2', 'c1'])
        (self.folder / '.private/controller-brief.md').unlink()
        ach = self.ach()
        self.controller(ach).cycle()
        codes = sorted(e['code'] for e in events(self.state / 'events.jsonl') if e['event'] == 'alert')
        self.assertEqual(codes, ['brief_pending', 'brief_pending', 'queue_unknown_contest'])
        self.assertEqual(self.launched(ach), [])

    def test_status_snapshot_is_private_and_only_logged_when_changed(self):
        self.entries[0]['state'] = 'running'
        controller = self.controller(self.ach())
        controller.cycle()
        controller.cycle()
        checkpoints = [e for e in events(self.state / 'events.jsonl') if e['event'] == 'checkpoint']
        self.assertEqual(len(checkpoints), 1)
        self.assertEqual(stat.S_IMODE((self.state / 'status.json').stat().st_mode), 0o600)

    def test_dry_run_launches_nothing_and_writes_nothing(self):
        before = self.files()
        ach = self.ach()
        controller = queue.QueueController(ach, home=self.home, dry_run=True, clock=lambda: NOW)
        controller.cycle()
        self.assertEqual(self.launched(ach), [])
        self.assertEqual(self.files(), before)

    def test_auto_preset_needs_private_mapping(self):
        (self.state / 'preset.json').write_text(json.dumps({'non_video': 'auto'}), encoding='utf-8')
        ach = self.ach()
        self.controller(ach).cycle()
        self.assertEqual(self.launched(ach), [])
        codes = [e['code'] for e in events(self.state / 'events.jsonl') if e['event'] == 'alert']
        self.assertEqual(codes, ['contest_error'])


class OldPathsTests(unittest.TestCase):
    def test_no_executable_controller_remains_under_data(self):
        for old in ('data/auto-approve/run.py', 'data/queue-master/run.py', 'data/queue-master/autopreset.py',
                    'data/queue-master/make_briefs.py', 'data/queue-master/switch_preset.py'):
            self.assertFalse((ROOT / old).exists(), old)


if __name__ == '__main__':
    unittest.main()
