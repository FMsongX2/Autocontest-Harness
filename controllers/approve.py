"""`python3 -m controllers approve`: apply the CLI's scoped delegation policy.

The CLI owns revision, score, artifact, rule, blocker and current-approval checks; readiness is
never inferred from request prose. Each cycle stores a runtime observation with
`ach reconcile --apply` (observe and store only: it never recovers, launches or stops work),
approves eligible `awaiting_approval` requests with the exact request token, notifies the
existing session, closes sessions of finished contests and cleans submitted folders once the
core allows it. Warnings are durable alerts, logged only when new, changed or resolved.
"""
import argparse
import json
import os
import subprocess
import tempfile
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

APPROVAL_NOTE = '현재 준비 요청에 대한 사용자 상시 위임 승인. 위임 근거와 범위는 CLI 설정 기록을 따른다.\n'

HANDOFF = (
    'CLI가 현재 revision·구조화된 준비 요청·점수·파일 해시·공식 규칙 기준·사용자 위임 범위를 다시 검사하여 '
    '제출 승인을 기록했습니다. ach approval-status로 현재 승인과 personalization 허용 범위를 확인하세요. '
    '미해결 질문을 임의로 기본값 처리하지 않습니다. 승인된 제출 서식의 개인정보·기존 서명 입력이 허용된 '
    '경우에만 ach profile show --for로 읽어 별도 최종 사본을 만드세요. 검수한 원본은 보존하고, 변경된 '
    '최종 사본의 전 페이지를 재렌더·비전 검수한 뒤 ach readiness --file <검수된 요청 파일> --personalized로 '
    '실제 업로드 파일 해시를 연결합니다. 본문·내용 수정은 running으로 돌아가 재채점·재승인을 받습니다. '
    '제출은 거래 절차로만 합니다. 공고의 접수 기간·제출처를 확인한 뒤 ach submission intent <id> --file <요청>'
    '(승인된 공식 제출처와 현재 요청 토큰)으로 의도를 기록하고, 외부 전송 직전에 '
    'ach submission start <id> --attempt <A>로 dispatching을 남깁니다. 시간 초과·응답 유실처럼 결과가 불확실하면 '
    '다시 보내지 말고 ach submission unknown <id> --attempt <A> --file <근거>로 기록한 뒤 공식 접수 목록·메일로 '
    '확인합니다. 접수 증빙(영수·확인 메일·목록 화면)은 .private/에 저장하고 '
    'ach submit <id> --attempt <A> --file <증빙 JSON>으로 확정합니다. 미제출이 공식 근거로 확인된 경우에만 '
    'ach submission resolve로 닫고 새 의도를 기록합니다. 확정 직후 본선·발표 등 후속 제작 일정이 있으면 '
    'followup_pending으로 옮긴 뒤 커밋·push합니다. 결과 발표만 기다리는 경우는 submitted로 둡니다. '
    '새 계정·결제·새로운 동의나 승인 범위 밖 전송은 이 위임으로 진행하지 않습니다.'
)


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
    def __init__(self, ach, home=None, dry_run=False, sender=subprocess.run, clock=utcnow):
        self.ach, self.dry_run, self.sender, self.clock = ach, dry_run, sender, clock
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
        status, reasons = self.evaluate(entry)
        if status is None:
            if reasons:
                self.alerts.raise_(entry['id'], 'approval_blocked', reasons, band)
            return
        if self.dry_run:
            self.log.emit('would_approve', contest_id=entry['id'], request_token=status['request_token'])
        else:
            self.approve(entry, status)

    # -- actions --------------------------------------------------------------------------------

    def show(self, contest_id):
        return self.ach.json('show', contest_id)

    def evaluate(self, entry):
        """(status, []) when eligible; (None, []) when already approved; (None, reasons) otherwise."""
        status = self.ach.json('approval-status', entry['id'])
        if status.get('submit_approval_valid'):
            return None, []
        if not status.get('eligible'):
            return None, safe_codes(status.get('reasons')) or ['not_eligible']
        return status, []

    def approve(self, entry, status):
        # The CLI rechecks policy, readiness, rules and this token atomically, not this cached result.
        fd, path = tempfile.mkstemp(suffix='.txt', prefix='ach-approval-')
        try:
            with os.fdopen(fd, 'w', encoding='utf-8') as handle:
                handle.write(APPROVAL_NOTE)
            self.ach.run('approve', entry['id'], '--note-file', path, '--by', 'controller-delegated',
                         '--delegated', '--expected-request', status['request_token'])
        finally:
            Path(path).unlink(missing_ok=True)
        session = self.show(entry['id']).get('session') or {}
        accepted = self.notify(session)
        self.log.emit('approved', contest_id=entry['id'], request_token=status['request_token'], handoff=accepted)

    def notify(self, session):
        handle = session.get('handle')
        if not handle or session.get('workspace') != 'orca':
            return False
        try:
            sent = self.sender(['orca', 'terminal', 'send', '--terminal', handle, '--text', HANDOFF,
                                '--enter', '--json'], capture_output=True, text=True, timeout=30)
        except (subprocess.TimeoutExpired, OSError):
            return False
        if sent.returncode:
            return False
        try:
            return bool(json.loads(sent.stdout).get('result', {}).get('send', {}).get('accepted', False))
        except (ValueError, AttributeError):
            return False

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
