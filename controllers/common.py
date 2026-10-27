"""Shared plumbing: root discovery, the `ach` runner, contract preflight, the lifetime lock,
atomic private state files, the event log and durable alert fingerprints."""
import errno
import fcntl
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
import time
from datetime import datetime, timedelta, timezone
from pathlib import Path

from . import PROTOCOL

EXIT_CONTRACT = 2
EXIT_LOCKED = 75
ACTIVE_STATES = {'routed', 'running', 'awaiting_approval', 'plan_review'}
GLOBAL = '*'


def root():
    """The harness root: `ACH_HOME` when set, otherwise the checkout that holds this package."""
    home = os.environ.get('ACH_HOME')
    if home:
        return Path(home)
    return Path(__file__).resolve().parents[1]


def utcnow():
    return datetime.now(timezone.utc)


class ContractError(RuntimeError):
    """`ach contract` is unavailable or speaks another protocol: nothing may be changed."""


class LockHeld(RuntimeError):
    """Another controller process of the same kind holds the lifetime lock."""


class AchError(RuntimeError):
    """A failed `ach` call. `code` is a stable reason; stderr is kept out of `str()` (and logs)."""

    def __init__(self, command, code, returncode=None, stderr=''):
        self.command, self.code, self.returncode, self.stderr = command, code, returncode, stderr
        detail = f' rc={returncode}' if returncode is not None else ''
        super().__init__(f'ach {command}: {code}{detail}')


def error_code(error):
    """A stable, private-text-free code for an exception, for logs and alert fingerprints."""
    if isinstance(error, AchError):
        return f'{error.code}:{error.command}'
    if isinstance(error, subprocess.TimeoutExpired):
        return 'timeout'
    if isinstance(error, ValueError):
        return 'invalid_json' if isinstance(error, json.JSONDecodeError) else 'invalid_value'
    return type(error).__name__.lower()


class Ach:
    """Runs `<root>/bin/ach <args> --json` (or `ACH_BIN`) as an argv list with `ACH_HOME=<root>`."""

    def __init__(self, home=None, binary=None, runner=subprocess.run):
        self.root = Path(home) if home else root()
        self.binary = binary or os.environ.get('ACH_BIN') or str(self.root / 'bin' / 'ach')
        self.runner = runner

    def run(self, *args, timeout=180, check=True):
        env = dict(os.environ, ACH_HOME=str(self.root))
        try:
            result = self.runner([self.binary, *args, '--json'], cwd=self.root, env=env,
                                 capture_output=True, text=True, timeout=timeout)
        except subprocess.TimeoutExpired as error:
            raise AchError(args[0], 'ach_timeout') from error
        except OSError as error:
            raise AchError(args[0], 'ach_unavailable') from error
        if check and result.returncode:
            raise AchError(args[0], 'ach_failed', result.returncode, (result.stderr or '').strip())
        return result

    def json(self, *args, timeout=180):
        result = self.run(*args, timeout=timeout)
        try:
            return json.loads(result.stdout)
        except ValueError as error:
            raise AchError(args[0], 'ach_invalid_json') from error


def preflight(ach):
    """Require `ach contract` protocol 2 before any side effect. Returns the contract."""
    try:
        contract = ach.json('contract', timeout=30)
    except AchError as error:
        raise ContractError(f'contract_unavailable:{error.code}') from error
    if not isinstance(contract, dict) or contract.get('protocol') != PROTOCOL:
        found = contract.get('protocol') if isinstance(contract, dict) else None
        raise ContractError(f'protocol_mismatch: expected {PROTOCOL}, found {found!r}')
    return contract


class DaemonLock:
    """A nonblocking `flock` held for the whole process lifetime; the kernel drops it on exit or
    crash, so a stale file never blocks a later start. The file itself is never deleted."""

    def __init__(self, path):
        self.path = Path(path)
        self.fd = None

    def acquire(self):
        self.path.parent.mkdir(parents=True, exist_ok=True)
        fd = os.open(self.path, os.O_RDWR | os.O_CREAT, 0o600)
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError as error:
            os.close(fd)
            if error.errno in (errno.EWOULDBLOCK, errno.EAGAIN, errno.EACCES):
                raise LockHeld(str(self.path)) from error
            raise
        os.ftruncate(fd, 0)
        os.write(fd, f'{os.getpid()}\n'.encode())
        self.fd = fd
        return self

    def release(self):
        if self.fd is not None:
            os.close(self.fd)
            self.fd = None

    def __enter__(self):
        return self.acquire()

    def __exit__(self, *_):
        self.release()


def atomic_write(path, text, mode=0o600):
    """Replace `path` with `text` in one rename after fsync; the result has `mode`."""
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp = tempfile.mkstemp(dir=path.parent, prefix=f'.{path.name}.', suffix='.tmp')
    try:
        with os.fdopen(fd, 'w', encoding='utf-8') as handle:
            handle.write(text)
            handle.flush()
            os.fsync(handle.fileno())
        os.chmod(tmp, mode)
        os.replace(tmp, path)
    except BaseException:
        if os.path.exists(tmp):
            os.unlink(tmp)
        raise
    directory = os.open(path.parent, os.O_RDONLY)
    try:
        os.fsync(directory)
    finally:
        os.close(directory)


def atomic_write_json(path, value):
    atomic_write(path, json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + '\n')


def read_json(path, default=None):
    path = Path(path)
    if not path.exists():
        return default
    return json.loads(path.read_text(encoding='utf-8'))


class EventLog:
    """JSON lines to stdout and, unless `dry_run`, appended to `path` (created 0600)."""

    def __init__(self, path, dry_run=False, clock=utcnow):
        self.path, self.dry_run, self.clock = Path(path), dry_run, clock

    def emit(self, kind, **fields):
        record = {'at': self.clock().isoformat(timespec='seconds'), 'event': kind, **fields}
        if self.dry_run:
            record['dry_run'] = True
        line = json.dumps(record, ensure_ascii=False)
        print(line, flush=True)
        if self.dry_run:
            return
        self.path.parent.mkdir(parents=True, exist_ok=True)
        fd = os.open(self.path, os.O_WRONLY | os.O_APPEND | os.O_CREAT, 0o600)
        with os.fdopen(fd, 'a', encoding='utf-8') as out:
            out.write(line + '\n')


def parse_time(value):
    """An aware datetime from an RFC 3339 string, or None when absent, naive or unparsable."""
    if not isinstance(value, str) or not value:
        return None
    try:
        moment = datetime.fromisoformat(value.replace('Z', '+00:00'))
    except ValueError:
        return None
    return moment if moment.tzinfo else None


def deadline_band(deadline, now):
    """Deadline risk band: none | invalid | expired | lt24h | lt72h | lt7d | gte7d."""
    if not deadline:
        return 'none'
    moment = parse_time(deadline)
    if moment is None:
        return 'invalid'
    left = moment - now
    if left < timedelta(0):
        return 'expired'
    if left < timedelta(hours=24):
        return 'lt24h'
    if left < timedelta(hours=72):
        return 'lt72h'
    if left < timedelta(days=7):
        return 'lt7d'
    return 'gte7d'


def fingerprint(code, status, band):
    body = json.dumps({'code': code, 'status': list(status), 'band': band}, sort_keys=True)
    return hashlib.sha256(body.encode('utf-8')).hexdigest()


CODE = re.compile(r'[A-Za-z0-9_.:/-]{1,80}')


def safe_codes(values):
    """Sorted, deduplicated stable codes (ASCII identifiers); anything else, such as prose that
    might carry private notes, is dropped."""
    return sorted({value for value in values or [] if isinstance(value, str) and CODE.fullmatch(value)})


class Alerts:
    """Warnings keyed `<contest>:<code>` with a fingerprint of (code, status codes, deadline band).

    A warning is logged only when it is new or its fingerprint changed, and `alert_resolved` is
    logged once it stops being raised for a contest that was fully checked. The last state lives
    in `path` (atomic, 0600), so a restarted controller does not repeat a warning it already gave.
    """

    def __init__(self, path, log, dry_run=False, clock=utcnow):
        self.path, self.log, self.dry_run, self.clock = Path(path), log, dry_run, clock
        stored = read_json(self.path, {}) or {}
        self.state = dict(stored.get('alerts') or {})
        self.seen = set()
        self.dirty = False

    def raise_(self, contest_id, code, status=(), band='none'):
        status = safe_codes(status)
        key = f'{contest_id}:{code}'
        self.seen.add(key)
        mark = fingerprint(code, status, band)
        previous = self.state.get(key)
        if previous and previous.get('fingerprint') == mark:
            return False
        self.state[key] = {'contest_id': contest_id, 'code': code, 'status': status,
                           'deadline_band': band, 'fingerprint': mark,
                           'since': self.clock().isoformat(timespec='seconds')}
        self.dirty = True
        self.log.emit('alert', contest_id=contest_id, code=code, status=status, deadline_band=band,
                      change='changed' if previous else 'new', fingerprint=mark[:16])
        return True

    def keep(self, code, contest_id=None):
        """Treat stored alerts with `code` (of `contest_id`, or all) as still raised: their check
        could not run this cycle, which is not evidence that the condition went away."""
        self.seen.update(key for key, value in self.state.items() if value.get('code') == code
                         and (contest_id is None or value.get('contest_id') == contest_id))

    def resolve(self, checked, present):
        """Resolve unraised alerts of contests in `checked`, and of contests no longer `present`."""
        for key in sorted(self.state):
            contest_id = self.state[key].get('contest_id')
            if key in self.seen and contest_id in present:
                continue
            if contest_id in checked or contest_id not in present:
                value = self.state.pop(key)
                self.dirty = True
                self.log.emit('alert_resolved', contest_id=contest_id, code=value.get('code'),
                              fingerprint=str(value.get('fingerprint', ''))[:16])
        self.seen = set()

    def save(self):
        if self.dirty and not self.dry_run:
            atomic_write_json(self.path, {'version': 1, 'alerts': self.state})
        self.dirty = False


def serve(controller, ach, lock_path, ready, interval, once, dry_run, sleep=time.sleep):
    """Run a daemon controller: preflight, lifetime lock, cycles. Returns the exit code.

    `controller(ach)` builds the controller only after preflight and lock, so a protocol mismatch
    or a duplicate instance leaves no file behind. Dry runs take no lock and write no files.
    """
    try:
        preflight(ach)
    except ContractError as error:
        print(f'contract check failed: {error}', file=sys.stderr, flush=True)
        return EXIT_CONTRACT
    lock = None
    if not dry_run:
        try:
            lock = DaemonLock(lock_path).acquire()
        except LockHeld:
            print(f'already running: {lock_path} is locked by another process', file=sys.stderr, flush=True)
            return EXIT_LOCKED
    try:
        instance = controller(ach)
        print(ready, flush=True)
        first = True
        while True:
            if not first:
                try:
                    preflight(ach)
                except ContractError as error:
                    print(f'contract check failed: {error}', file=sys.stderr, flush=True)
                    return EXIT_CONTRACT
            first = False
            try:
                keep_running = instance.cycle()
            except (AchError, subprocess.TimeoutExpired, ValueError, KeyError, TypeError, OSError) as error:
                instance.log.emit('cycle_error', error=error_code(error))
                if once:
                    return 1
                keep_running = True
            if once or not keep_running:
                return 0
            sleep(interval)
    finally:
        if lock:
            lock.release()
