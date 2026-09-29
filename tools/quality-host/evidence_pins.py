"""Retention pins: protect the exact evidence a success publication names.

A pin protects one canonical bounded Gate run (evidence/gate/run-*) until it is
released or expires. It protects every file of that run except CLEANABLE
(rebuildable raw payloads and scratch), so cleanup may still reclaim those.
Only runs beside the pin root are accepted: legacy and non-v3 layouts cannot be
pinned.

Capacity: pins reserve bytes from the installed records budget, the same
authority bounded_records applies (compact_gate_evidence.maintain's budget
default), read at runtime from the digest-checked storage deployment. Pin admission
and record cleanup charge the same thing: the inode-deduplicated record bytes (every
file but the installed policy's payloads) of the pinned runs and of the current required record (current_record, the newest
completed capture exactly as cleanup ranks it), plus the state file. A pin that
does not fit is refused, so it never displaces current evidence; cleanup whose pins
and current record no longer fit fails explicitly instead of exceeding the budget.

State is one bounded file of live pins. Each pin carries its publication
(unconfirmed, then confirmed in place), so the publication and the merge-source
proof are charged with the pin and disappear with it; nothing else is written for
them. Released and expired pins are dropped on every write, including cleanup.
The state charge counts the file and its pending replacement and excludes the
sequence number (SEQUENCE_SLACK covers any width). Confirming shortens an entry
and extending keeps its width, so neither exceeds what the pin reserved.

Lock order: workspace lease -> ledger lock -> pin lock. A holder of the pin lock
never waits for the ledger or the workspace lease, and never reopens the pin lock:
holding() yields the locked root to callers that confirm() under that same lock.
Cleanup holds the pin lock across its pin check and every rename or delete; pin
creation, extension and release hold it too, so a run is never deleted between a
pin's check and its record.
"""
import ast
from contextlib import contextmanager
import fcntl
import hashlib
import json
import operator
import os
from pathlib import Path
import re
import stat
import time

SCHEMA = 'codexsymphony-evidence-pins/v2'
RUN = re.compile('run-[0-9a-f]{12}')
# Paths cleanup may still remove from a pinned run: the installed compaction allowlist
# (rebuildable payloads, legacy workspace reports) and scratch. v3 evidence never lies here.
CLEANABLE = ('probes/backend/raw', 'http-server', 'workspace/.harness-gate/reports', 'tmp', 'cargo-home', 'target')
# The installed policy's rebuildable payloads (compact_gate_evidence.payloads); record bytes are the rest.
PAYLOADS = ('probes/backend/raw', 'http-server')
OPERATORS = {ast.Add: operator.add, ast.Sub: operator.sub, ast.Mult: operator.mul, ast.Pow: operator.pow}
# Upper bound on the width of any sequence number; the state charge excludes the sequence and adds this.
SEQUENCE_SLACK = 20
# An unconfirmed publication's identity placeholder has the width of the confirmed identity.
PLACEHOLDER = '0' * 64


class PinError(ValueError):
    def __init__(self, category, message):
        super().__init__(message)
        self.category = category


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':')).encode()


def digest(data):
    return hashlib.sha256(data).hexdigest()


def open_regular(path, flags):
    """Open without following a final link; a FIFO must not hang and is refused."""
    try:
        descriptor = os.open(path, flags | os.O_NOFOLLOW | os.O_NONBLOCK, 0o600)
    except FileNotFoundError:
        raise
    except OSError:
        raise PinError('tampered', 'not a regular file: ' + str(path)) from None
    if not stat.S_ISREG(os.fstat(descriptor).st_mode):
        os.close(descriptor)
        raise PinError('tampered', 'not a regular file: ' + str(path))
    return descriptor


def read_json(path):
    with os.fdopen(open_regular(path, os.O_RDONLY), 'rb') as stream:
        return json.loads(stream.read())


def file_digest(path):
    with os.fdopen(open_regular(path, os.O_RDONLY), 'rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


@contextmanager
def locked(root):
    """Exclusive pin lock; the pin root and its lock are installed, never created here."""
    root = Path(root)
    if not root.is_absolute() or root.resolve() != root:
        raise PinError('tampered', 'pin root must be canonical')
    try:
        descriptor = open_regular(root / 'lock', os.O_RDWR)
    except FileNotFoundError:
        raise PinError('missing', 'pin root is not installed') from None
    try:
        fcntl.flock(descriptor, fcntl.LOCK_EX)
        yield root
    finally:
        os.close(descriptor)


def empty():
    return {'schema': SCHEMA, 'sequence': 0, 'pins': {}}


def load(root):
    """The stored state; the caller holds the pin lock."""
    try:
        state = read_json(root / 'state.json')
    except FileNotFoundError:
        return empty()
    if state.get('schema') != SCHEMA or type(state.get('sequence')) is not int or not isinstance(state.get('pins'), dict):
        raise PinError('tampered', 'invalid pin state')
    return state


def save(root, state, pins):
    """Atomically replace the state with these pins; the caller holds the pin lock."""
    state = state | {'sequence': state['sequence'] + 1, 'pins': pins}
    pending = root / 'state.json.new'
    descriptor = os.open(pending, os.O_WRONLY | os.O_CREAT | os.O_TRUNC | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, 'wb') as stream:
        stream.write(canonical(state))
        stream.flush()
        os.fsync(stream.fileno())
    pending.replace(root / 'state.json')
    return state


def live(state, moment):
    return {key: value for key, value in state['pins'].items() if value['expires_at_ms'] > moment}


def now_ms(now=None):
    return int((time.time() if now is None else now) * 1000)


def pinnable(root, run):
    run = Path(run)
    if run.parent != Path(root).parent / 'gate' or not RUN.fullmatch(run.name) or run.resolve() != run:
        raise PinError('mismatch', 'only a canonical bounded Gate run can be pinned: ' + str(run))
    return run


def cleanable(run, path):
    return any(path.is_relative_to(run / name) for name in CLEANABLE)


def check_files(run, evidence):
    """Every ledger-bound file lies in the run's protected part and still has its bytes."""
    for name, row in evidence.items():
        path = Path(row['path'])
        if not path.is_relative_to(run) or path.resolve() != path or cleanable(run, path):
            raise PinError('mismatch', 'evidence is not a protected file of the pinned run: ' + name)
        try:
            actual = file_digest(path)
        except FileNotFoundError:
            raise PinError('missing', 'pinned evidence missing: ' + name) from None
        if actual != row['sha256']:
            raise PinError('tampered', 'pinned evidence changed: ' + name)


def files(run, excluded):
    """{(device, inode): bytes} of every file of run outside the excluded subtrees."""
    found = {}
    for directory, dirs, names in os.walk(run):
        relative = Path(directory).relative_to(run)
        dirs[:] = [name for name in dirs if str(relative / name) not in excluded]
        for name in names:
            info = os.lstat(Path(directory) / name)
            found[(info.st_dev, info.st_ino)] = info.st_size
    return found


def charge(runs):
    """Record bytes of these runs as the installed policy counts them (all but its payloads), each inode once.

    Scratch and legacy workspace reports are charged although cleanup may reclaim them."""
    found = {}
    for run in set(runs):
        found.update(files(run, PAYLOADS))
    return sum(found.values())


def state_bytes(state):
    """The state file and its pending replacement, for any sequence number."""
    return 2 * (len(canonical(state | {'sequence': 0})) + SEQUENCE_SLACK)


def measured(run, path):
    """A retained measurement result whose measurement file is a canonical, unchanged file of run."""
    result = read_json(path)
    measurement = Path(result['measurement'])
    if not measurement.is_absolute() or measurement.resolve() != measurement:
        raise PinError('tampered', 'canonical, non-symlink workspace path required')
    if not measurement.is_relative_to(run) or file_digest(measurement) != result['measurement_sha256']:
        raise PinError('tampered', 'retained measurement changed')
    return result['coverage_and_crap'] == 'PASS'


def completed(run):
    """A completed bounded capture: the one criterion both cleanup and pin admission rank records by."""
    if not (run / 'source-archive.json').exists() or not (run / 'capture-registration.json').exists():
        return False
    if file_digest(run / 'source.tar.gz') != read_json(run / 'source-archive.json')['sha256']:
        raise PinError('tampered', 'retained source archive changed')
    binding = read_json(run / 'capture-registration.json')
    if binding['root'] != str(run) or binding['bundle_sha256'] != file_digest(run / 'probes/backend/bundle.json'):
        raise PinError('tampered', 'retained capture registration changed')
    for name in ('measurement-summary.json', 'recovery-measurement.json'):
        if (run / name).exists():
            return measured(run, run / name)
    return False


def pending(slot):
    """Captures a Gate or manual capture announced in the slot and has not finished."""
    captures = set()
    for name in ('pending-capture.json', 'pending-gate.json'):
        path = slot / name
        if path.is_symlink():
            raise PinError('tampered', 'symlink pending capture')
        if path.exists():
            captures.add(read_json(path)['capture'])
    return captures


def archived_at(run):
    return (run / 'source-archive.json').stat().st_mtime


def candidates(parent, announced):
    """Completed, unannounced bounded runs, newest source archive first."""
    runs = []
    for run in parent.glob('run-*'):
        if not RUN.fullmatch(run.name) or run.resolve() != run:
            raise PinError('tampered', 'unsafe bounded evidence directory')
        if str(run) not in announced and completed(run):
            runs.append(run)
    return sorted(runs, key=archived_at, reverse=True)


def slot_of(root):
    """The Gate slot of the bounded volume the pin root belongs to (bounded_layout.slot)."""
    return Path(root).parent.parent / 'validation/gate'


def current_record(root):
    """The current required record, chosen exactly as cleanup chooses it: [] or [newest candidate]."""
    return candidates(Path(root).parent / 'gate', pending(slot_of(root)))[:1]


def runs(state):
    return frozenset(Path(value['run']) for value in state['pins'].values())


def reserved(state, current):
    """Bytes the pins, the current required record and the state itself reserve."""
    return charge([*runs(state), *current]) + state_bytes(state)


def pin(root, budget, record, subject, attempt, ttl_seconds, publication, now=None):
    """Reserve capacity for the record's run and its unconfirmed publication; success is published only afterwards."""
    moment = now_ms(now)
    run = pinnable(root, record['details'].get('run', ''))
    with locked(root) as root:
        state = load(root)
        check_files(run, record['evidence'])
        value = {'validation_id': record['validation_id'], 'subject': subject, 'attempt': attempt,
                 'run': str(run), 'created_at_ms': moment, 'expires_at_ms': moment + ttl_seconds * 1000}
        identity = digest(canonical(value))
        value |= {'publication': publication | {'status': 'unconfirmed'}, 'publication_id': PLACEHOLDER}
        pins = live(state, moment) | {identity: value}
        needed = reserved(state | {'pins': pins}, current_record(root))
        if needed > budget:
            raise PinError('capacity', f'pins and current required evidence need {needed} of {budget} record bytes')
        save(root, state, pins)
    return value | {'pin_id': identity}


def check_live(state, pin_id, validation_id, now=None):
    """The live pin of this validation in a state already read under the pin lock."""
    value = live(state, now_ms(now)).get(pin_id)
    if value is None or value['validation_id'] != validation_id:
        raise PinError('expired', 'retention pin missing or expired')
    return value


def active(root, pin_id, validation_id, now=None):
    """For callers outside the pin lock: the live pin, read under the lock."""
    with locked(root) as root:
        return check_live(load(root), pin_id, validation_id, now)


@contextmanager
def holding(root, pin_id, validation_id, now=None):
    """Hold the pin lock while the caller publishes; the pin must be live at entry. Yields the locked root."""
    with locked(root) as root:
        check_live(load(root), pin_id, validation_id, now)
        yield root


def confirm(root, pin_id, validation_id, now=None):
    """Inside holding(): re-read the state, require the pin still live and confirm its publication in place."""
    state = load(root)
    value = check_live(state, pin_id, validation_id, now)
    publication = value['publication'] | {'status': 'confirmed'}
    value = value | {'publication': publication, 'publication_id': digest(canonical(publication))}
    save(root, state, live(state, now_ms(now)) | {pin_id: value})
    return value | {'pin_id': pin_id}


def find(root, subject, now=None):
    """Live confirmed publications of exactly this subject, by pin identity; never ranked by time."""
    found = []
    for key, value in sorted(current(root, now).items()):
        publication = value['publication']
        if value['subject'] != subject or publication['status'] != 'confirmed':
            continue
        if digest(canonical(publication)) != value['publication_id']:
            raise PinError('tampered', 'publication bytes differ from its identity')
        found.append(value | {'pin_id': key})
    return found


def current(root, now=None):
    """Live pins; read under the lock so a concurrent write is never half-seen."""
    with locked(root) as root:
        return live(load(root), now_ms(now))


def extend(root, pin_id, until_ms, now=None):
    """Keep a live pin until at least until_ms; an expired or released pin is never revived."""
    moment = now_ms(now)
    with locked(root) as root:
        state = load(root)
        pins = live(state, moment)
        if pin_id not in pins:
            return False
        expires = max(pins[pin_id]['expires_at_ms'], until_ms)
        # The entry keeps the width it was charged with.
        if len(str(expires)) != len(str(pins[pin_id]['expires_at_ms'])):
            raise PinError('capacity', 'pin expiry width would change')
        save(root, state, pins | {pin_id: pins[pin_id] | {'expires_at_ms': expires}})
        return True


def release(root, pin_ids, now=None):
    """Drop the named pins (and every expired one); returns the dropped identities."""
    moment = now_ms(now)
    with locked(root) as root:
        state = load(root)
        pins = live(state, moment)
        kept = {key: value for key, value in pins.items() if key not in pin_ids}
        save(root, state, kept)
    return sorted(set(pins) - set(kept))


@contextmanager
def cleanup(root, now=None):
    """Exclusive pin lock across a cleanup pass; yields the live state, stored without expired pins.

    No installed root holds no pins."""
    root = Path(root)
    if not root.exists() and not root.is_symlink():
        yield empty()
        return
    with locked(root) as root:
        state = load(root)
        pins = live(state, now_ms(now))
        if pins != state['pins']:
            state = save(root, state, pins)
        yield state


def installed_policy(deployment):
    """The digest-checked installed retention policy source (the records budget authority)."""
    value = json.loads(Path(deployment).read_text())
    release = Path(value['release'])
    if not release.is_absolute() or release.resolve() != release:
        raise PinError('tampered', 'installed retention release must be canonical')
    for name, expected in value['files'].items():
        path = Path(name)
        if path.is_relative_to(release) and file_digest(path) != expected:
            raise PinError('tampered', 'installed retention policy changed')
    entry = release / 'compact_gate_evidence.py'
    if str(entry) not in value['files']:
        raise PinError('missing', 'retention policy is not approved')
    return entry.read_bytes()


def installed_default(deployment, name):
    """An installed maintain() default, evaluated without executing the policy."""
    names = {}
    for node in ast.parse(installed_policy(deployment)).body:
        if isinstance(node, ast.Assign) and len(node.targets) == 1 and isinstance(node.targets[0], ast.Name):
            names[node.targets[0].id] = node.value
        if isinstance(node, ast.FunctionDef) and node.name == 'maintain':
            return default(node, name, names)
    raise PinError('missing', 'installed retention policy has no maintain ' + name)


def records_budget(deployment):
    return installed_default(deployment, 'budget')


def retention_hours(deployment):
    return installed_default(deployment, 'hours')


def default(function, name, names):
    arguments = [argument.arg for argument in function.args.args]
    defaults = dict(zip(arguments[len(arguments) - len(function.args.defaults):], function.args.defaults))
    if name not in defaults:
        raise PinError('missing', 'installed retention policy has no maintain ' + name)
    return evaluate(defaults[name], names)


def evaluate(node, names):
    """Integer literals, module constants and + - * ** only; anything else is refused."""
    if isinstance(node, ast.Constant) and type(node.value) is int:
        return node.value
    if isinstance(node, ast.Name) and node.id in names:
        return evaluate(names[node.id], names)
    if isinstance(node, ast.BinOp) and type(node.op) in OPERATORS:
        left, right = evaluate(node.left, names), evaluate(node.right, names)
        if isinstance(node.op, ast.Pow) and not 0 <= right <= 64:
            raise PinError('mismatch', 'unsupported installed retention expression')
        return OPERATORS[type(node.op)](left, right)
    raise PinError('mismatch', 'unsupported installed retention expression')
