"""`python3 -m controllers approve`: apply the CLI's scoped delegation policy.

The CLI owns revision, score, artifact, rule, blocker and current-approval checks; readiness is
never inferred from request prose. Each cycle stores a runtime observation with
`ach reconcile --apply` (observe and store only: it never recovers, launches or stops work),
runs the CLI's delegated handoff (`ach handoff <id>`) for every `awaiting_approval` request,
closes sessions of finished contests and cleans submitted folders once the core allows it.
The handoff is the CLI's: it approves an eligible request with its exact token and mails the
recorded session through Orca orchestration, journaled so that nothing is repeated. This
controller types nothing into a terminal. Warnings are durable alerts, logged only when new,
changed or resolved.
"""
import argparse
import subprocess
import time
from datetime import timedelta
from pathlib import Path

from .common import (ACTIVE_STATES, Ach, AchError, Alerts, EventLog, deadline_band, error_code,
                     parse_time, safe_codes, serve, utcnow)

STATE_DIR = Path('data') / 'auto-approve'
LOCK = Path('data') / 'controller-approve.lock'
READY = 'AUTOAPPROVE_READY'
IDLE_STATES = {'submitted', 'followup_pending', 'closed', 'skipped', 'duplicate'}
RUNTIME_ATTENTION = {'missing', 'ambiguous', 'unknown'}
CLEAN_RETRY_SECONDS = 1800
CLEAN_GRACE = timedelta(minutes=30)
CLOSE_GRACE = timedelta(minutes=10)
ERRORS = (AchError, subprocess.TimeoutExpired, ValueError, KeyError, TypeError, OSError)

# `ach handoff` statuses that need nothing from anyone.
HANDOFF_SETTLED = {'already_handed_off', 'approved_elsewhere', 'not_applicable'}
# Statuses where the approval or the message itself went wrong and the controller must look.
HANDOFF_ATTENTION = {'session_unavailable', 'approve_refused', 'send_failed', 'handoff_unknown', 'error'}


def elapsed_in_state(entry, now=None):
    """Time since the last real state transition into the current state (zero when unknown)."""
    history = entry.get('history') or []
    if not history:
        return timedelta(0)
    # Session-close bookkeeping is not a state transition and must not reset the grace period.
    last_move = next((h for h in reversed(history) if h.get('from') != h.get('to')), None)
    if not last_move or last_move.get('to') != entry.get('state'):
        return timedelta(0)
    moved_at = parse_time(last_move.get('at'))
    if moved_at is None:
        return timedelta(0)
    return (now or utcnow()) - moved_at


def runtime_codes(observation):
    """Stable attention codes from one `ach reconcile` entry (empty when nothing needs a look)."""
    codes = list(safe_codes(observation.get('blocked_reasons')))
    state = observation.get('runtime_state')
    if state in RUNTIME_ATTENTION:
        codes.append(f'runtime:{state}')
    if observation.get('needs_user_input'):
        codes.append('needs_user_input')
    if observation.get('checkpoint_valid') is False:
        codes.append('checkpoint_invalid')
    if codes and observation.get('recoverable'):
        codes.append('recoverable')
    return codes


class ApproveController:
    def __init__(self, ach, home=None, dry_run=False, clock=utcnow):
        self.ach, self.dry_run, self.clock = ach, dry_run, clock
        home = Path(home) if home else ach.root
        state = home / STATE_DIR
        self.log = EventLog(state / 'events.jsonl', dry_run, clock)
        self.alerts = Alerts(state / 'alerts.json', self.log, dry_run, clock)
        self.retry = {}

    # -- one cycle ------------------------------------------------------------------------------

    def reconcile(self):
        """Observe runtimes; `--apply` stores the snapshot only. Returns {id: observation} or None."""
        args = ['reconcile'] if self.dry_run else ['reconcile', '--apply']
        try:
            result = self.ach.json(*args)
        except ERRORS as error:
            self.alerts.raise_('*', 'reconcile_failed', [error_code(error)])
            self.alerts.keep('runtime_attention')
            return None
        observed = {}
        for item in result.get('entries') or []:
            if isinstance(item, dict) and isinstance(item.get('id'), str):
                observed[item['id']] = item
        return observed

    def cycle(self):
        now = self.clock()
        observed = self.reconcile()
        entries = self.ach.json('list')
        present = {'*'} | {entry['id'] for entry in entries}
        checked = {'*'}
        for entry in entries:
            contest_id = entry['id']
            if contest_id.startswith('legacy'):
                continue
            band = deadline_band(entry.get('deadline'), now)
            try:
                if observed is not None:
                    self.runtime_alert(entry, observed.get(contest_id), band)
                self.handle(entry, band, now)
                checked.add(contest_id)
            except ERRORS as error:
                self.alerts.raise_(contest_id, 'contest_error', [error_code(error)], band)
        self.alerts.resolve(checked, present)
        self.alerts.save()
        return True

    def runtime_alert(self, entry, observation, band):
        if not observation or entry.get('state') not in ACTIVE_STATES:
            return
        codes = runtime_codes(observation)
        if codes:
            self.alerts.raise_(entry['id'], 'runtime_attention', codes, band)

    def handle(self, entry, band, now):
        state = entry['state']
        if state in IDLE_STATES:
            self.close_finished_session(entry, band, now)
        if state == 'submitted':
            self.clean_submitted(entry, band, now)
            return
        if state != 'awaiting_approval':
            return
        self.handoff(entry, band)

    # -- actions --------------------------------------------------------------------------------

    def show(self, contest_id):
        return self.ach.json('show', contest_id)

    def handoff(self, entry, band='none'):
        """Run the CLI's delegated handoff for one waiting request and record what it says.

        A request that needs the user's own approval (`user_approval_request`) is a different
        alert from one that waits on a reason, so the two are never reported as the same thing.
        """
        args = ['handoff', entry['id'], '--trigger', 'controller']
        if self.dry_run:
            args.append('--dry-run')
        result = self.ach.json(*args)
        status = result.get('status') if isinstance(result.get('status'), str) else 'error'
        reasons = safe_codes(result.get('reasons'))
        token = result.get('request_token')
        if status == 'handed_off':
            record = result.get('record') or {}
            self.log.emit('handed_off', contest_id=entry['id'], request_token=token,
                          message_id=record.get('message_id'))
        elif status in ('would_approve_and_hand_off', 'would_hand_off'):
            self.log.emit(status, contest_id=entry['id'], request_token=token)
        elif status in HANDOFF_SETTLED:
            return
        elif result.get('user_approval_request') is True:
            self.alerts.raise_(entry['id'], 'user_approval_request', [status] + reasons, band)
        elif status in HANDOFF_ATTENTION:
            self.alerts.raise_(entry['id'], 'handoff_attention', [status] + reasons, band)
        else:
            self.alerts.raise_(entry['id'], 'approval_blocked', reasons or [status], band)

    def clean_submitted(self, entry, band='none', now=None):
        full = self.show(entry['id'])
        if full.get('state') != 'submitted' or not full.get('folder') or not Path(full['folder']).is_dir():
            return
        # Missing legacy readiness is not proof that no future work remains.
        request = full.get('submission_readiness') or {}
        if request.get('followup_required') is not False:
            return
        if elapsed_in_state(full, now) < CLEAN_GRACE:
            return
        if time.time() - self.retry.get(entry['id'], 0) < CLEAN_RETRY_SECONDS:
            self.alerts.keep('clean_refused', entry['id'])
            return
        if self.dry_run:
            self.log.emit('would_clean', contest_id=entry['id'])
            return
        result = self.ach.run('clean', entry['id'], check=False)
        self.retry[entry['id']] = time.time()
        if result.returncode == 0:
            self.log.emit('cleaned', contest_id=entry['id'])
        else:
            self.alerts.raise_(entry['id'], 'clean_refused', [f'rc:{result.returncode}'], band)

    def close_finished_session(self, entry, band='none', now=None):
        full = self.show(entry['id'])
        if full.get('state') not in IDLE_STATES or elapsed_in_state(full, now) < CLOSE_GRACE:
            return
        session = full.get('session') or {}
        if not session.get('handle') and not session.get('workspace_id'):
            return
        if self.dry_run:
            self.log.emit('would_close_session', contest_id=entry['id'], state=full['state'])
            return
        # The CLI rechecks these under its lifecycle/registry locks. Failures remain retryable.
        result = self.ach.run('session-close', entry['id'], '--expected-state', full['state'],
                              '--expected-history', str(len(full.get('history') or [])), check=False)
        if result.returncode == 0:
            self.log.emit('session_closed', contest_id=entry['id'], state=full['state'])
        else:
            self.alerts.raise_(entry['id'], 'session_close_refused', [f'rc:{result.returncode}'], band)


def main(argv, ach=None):
    parser = argparse.ArgumentParser(prog='python3 -m controllers approve')
    parser.add_argument('--once', action='store_true', help='run one cycle and exit')
    parser.add_argument('--dry-run', action='store_true', help='report actions; take no lock, write nothing')
    parser.add_argument('--interval', type=float, default=120, help='seconds between cycles')
    options = parser.parse_args(argv)
    ach = ach or Ach()
    return serve(lambda client: ApproveController(client, dry_run=options.dry_run), ach,
                 ach.root / LOCK, READY, options.interval, options.once, options.dry_run)
