"""Host-owned immutable validation records and an append-only event index.

Each complete Gate outcome is a content-addressed record written once; its
validation_id is the SHA-256 of the record bytes. Only the hash-chained event
index decides whether a PASS stands. A later complete FAIL or revocation for the
same executed inputs (approval, environment, tree) blocks every earlier PASS of
those inputs, whatever commit it named. A complete Gate appends a start event
before it executes and concludes only that attempt, for exactly the inputs the
start announced, with its PASS or FAIL. Equivalence across commits is never a
rebinding of a record; it belongs to a separately audited attestation. While
any attempt of the same executed inputs is unconcluded no PASS of them stands,
so a running or crashed attempt is never masked by another attempt's result.
Verification only reads. Missing
evidence or a transport failure is never written as a FAIL, so the same PASS
verifies again once its evidence is available.

Boundary: the chain detects edits, reordering and partial appends inside the
index. It cannot detect truncation of trailing events or a rollback of the whole
ledger directory; that relies on the ledger living outside every agent-writable
mount, and callers that must detect rollback pass a previously observed head as
an anchor. Writers hold the exclusive ledger lock; a publisher holds hold() for
its whole verify-then-publish step so no FAIL or revocation can land in between.
"""
from contextlib import contextmanager
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import time

SCHEMA = 'codexsymphony-evidence-ledger/v1'
INPUT_KEYS = ('approval', 'commit', 'environment', 'tree')
EXECUTION_KEYS = ('approval', 'environment', 'tree')
EVENT_KEYS = {'sequence', 'kind', 'validation_id', 'attempt', 'input_key', 'execution_key', 'at_ms', 'reason', 'previous'}
KINDS = ('start', 'pass', 'fail', 'revoke')
IDENTITY = re.compile('[0-9a-f]{64}')
GENESIS = '0' * 64


class LedgerError(ValueError):
    """Carries category: missing, blocked, mismatch or tampered."""


def fail(category, message):
    error = LedgerError(category + ': ' + message)
    error.category = category
    return error


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=False).encode()


def digest(data):
    return hashlib.sha256(data).hexdigest()


def file_digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def keys(inputs):
    """Return (input_key over all inputs, execution_key over what the Gate executed)."""
    if not isinstance(inputs, dict) or sorted(inputs) != list(INPUT_KEYS):
        raise fail('mismatch', 'inputs must bind exactly ' + ', '.join(INPUT_KEYS))
    if not all(isinstance(inputs[name], str) and inputs[name] for name in INPUT_KEYS):
        raise fail('mismatch', 'input identities must be non-empty strings')
    executed = {name: inputs[name] for name in EXECUTION_KEYS}
    return digest(canonical(inputs)), digest(canonical(executed))


def canonical_root(root):
    root = Path(root)
    if not root.is_absolute() or root.resolve() != root or (root / 'records').is_symlink():
        raise fail('tampered', 'ledger root must be canonical')
    return root


def open_regular(path, flags):
    """Open without following a final link and require a regular file."""
    try:
        # O_NONBLOCK: a FIFO planted in place of a ledger file must not hang the open.
        descriptor = os.open(path, flags | os.O_NOFOLLOW | os.O_NONBLOCK, 0o600)
    except FileNotFoundError:
        raise
    except OSError:
        raise fail('tampered', 'ledger file is not a regular file: ' + path.name) from None
    if not stat.S_ISREG(os.fstat(descriptor).st_mode):
        os.close(descriptor)
        raise fail('tampered', 'ledger file is not a regular file: ' + path.name)
    return descriptor


@contextmanager
def locked(root, exclusive):
    flags = os.O_RDWR | os.O_CREAT if exclusive else os.O_RDONLY
    try:
        descriptor = open_regular(root / 'lock', flags)
    except FileNotFoundError:
        raise fail('missing', 'ledger has no records') from None
    try:
        fcntl.flock(descriptor, fcntl.LOCK_EX if exclusive else fcntl.LOCK_SH)
        yield
    finally:
        os.close(descriptor)


def hold(root):
    """Shared lock for a caller's whole verify-then-publish step."""
    return locked(canonical_root(root), False)


def sync_directory(path):
    descriptor = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def write_new(path, data):
    """Create path exactly once; an existing record is never replaced."""
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o400)
    with os.fdopen(descriptor, 'wb') as stream:
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())
    sync_directory(path.parent)


def evidence_rows(evidence):
    if not isinstance(evidence, dict):
        raise fail('mismatch', 'evidence must map names to files')
    rows = {}
    for name, value in sorted(evidence.items()):
        path = Path(value)
        if not path.is_absolute() or path.resolve() != path or not path.is_file():
            raise fail('mismatch', 'evidence must be a canonical regular file: ' + str(name))
        rows[name] = {'path': str(path), 'sha256': file_digest(path), 'bytes': path.stat().st_size}
    return rows


def check_evidence(rows):
    for name, row in sorted(rows.items()):
        path = Path(row['path'])
        if path.resolve() != path:
            raise fail('tampered', 'evidence path replaced by a link: ' + name)
        if not path.is_file():
            raise fail('missing', 'evidence unavailable: ' + name)
        if file_digest(path) != row['sha256']:
            raise fail('tampered', 'evidence changed: ' + name)


def parse_event(line, sequence, previous):
    try:
        event = json.loads(line)
    except json.JSONDecodeError:
        raise fail('tampered', 'malformed event at sequence ' + str(sequence)) from None
    if not isinstance(event, dict) or set(event) != EVENT_KEYS or event['kind'] not in KINDS:
        raise fail('tampered', 'invalid event at sequence ' + str(sequence))
    if event['sequence'] != sequence or event['previous'] != previous:
        raise fail('tampered', 'event chain broken at sequence ' + str(sequence))
    return event


def read_events(root):
    """Return (events, per-line digests) of the index; see the module boundary."""
    try:
        descriptor = open_regular(root / 'events.jsonl', os.O_RDONLY)
    except FileNotFoundError:
        return [], []
    with os.fdopen(descriptor, 'rb') as stream:
        data = stream.read()
    if data and not data.endswith(b'\n'):
        raise fail('tampered', 'incomplete event append')
    events, digests = [], []
    for line in data.splitlines():
        events.append(parse_event(line, len(events), digests[-1] if digests else GENESIS))
        digests.append(digest(line))
    return events, digests


def head_of(digests):
    return {'sequence': len(digests), 'digest': digests[-1] if digests else GENESIS}


def check_anchor(digests, anchor):
    """Reject an index that no longer extends a previously observed head."""
    if anchor is None:
        return
    count = anchor.get('sequence') if isinstance(anchor, dict) else None
    if type(count) is not int or count < 0 or count > len(digests):
        raise fail('tampered', 'index is shorter than the anchored head')
    if head_of(digests[:count]) != anchor:
        raise fail('tampered', 'index diverges from the anchored head')


def append(root, kind, value, reason):
    """Append one event; the caller holds the exclusive ledger lock."""
    _, digests = read_events(root)
    event = {'sequence': len(digests), 'kind': kind, 'validation_id': value['validation_id'],
             'attempt': value['attempt'], 'input_key': value['input_key'], 'execution_key': value['execution_key'],
             'at_ms': int(time.time() * 1000), 'reason': reason, 'previous': head_of(digests)['digest']}
    descriptor = open_regular(root / 'events.jsonl', os.O_WRONLY | os.O_APPEND | os.O_CREAT)
    with os.fdopen(descriptor, 'ab') as stream:
        stream.write(canonical(event) + b'\n')
        stream.flush()
        os.fsync(stream.fileno())
    return event


def pending(events, execution_key):
    """Attempts of these executed inputs that started and have not concluded."""
    started, concluded = set(), set()
    for event in events:
        if event['execution_key'] != execution_key:
            continue
        if event['kind'] == 'start':
            started.add(event['attempt'])
        elif event['kind'] != 'revoke':
            concluded.add(event['attempt'])
    return started - concluded


def prepared(root):
    root = canonical_root(root)
    (root / 'records').mkdir(mode=0o700, parents=True, exist_ok=True)
    return root


def begin(root, inputs):
    """Announce a complete Gate for inputs before it executes; returns the attempt id."""
    root = prepared(root)
    input_key, execution_key = keys(inputs)
    attempt = digest(canonical({'inputs': inputs, 'nonce': os.urandom(16).hex()}))
    value = {'validation_id': attempt, 'attempt': attempt, 'input_key': input_key, 'execution_key': execution_key}
    with locked(root, True):
        append(root, 'start', value, 'complete Gate started')
    return attempt


def started(events, input_key):
    """Pending attempts whose start announced exactly these inputs, commit included."""
    return {event['attempt'] for event in events if event['kind'] == 'start' and event['input_key'] == input_key}


def check_attempt(events, attempt, input_key, execution_key):
    """A conclusion binds the exact inputs its start announced; it can never rebind another commit."""
    if attempt not in pending(events, execution_key) & started(events, input_key):
        raise fail('mismatch', 'attempt is not a pending start of exactly these inputs')


def unconcluded(root, inputs):
    """Read-only: attempts started for exactly these inputs that never concluded."""
    root = canonical_root(root)
    input_key, execution_key = keys(inputs)
    with locked(root, False):
        events = read_events(root)[0]
    return sorted(pending(events, execution_key) & started(events, input_key))


def record(root, outcome, attempt, inputs, evidence, details):
    """Conclude one pending attempt with an immutable record admitted to the index."""
    root = prepared(root)
    input_key, execution_key = keys(inputs)
    if not isinstance(details, dict):
        raise fail('mismatch', 'details must be an object')
    value = {'schema': SCHEMA, 'outcome': outcome, 'attempt': attempt, 'inputs': inputs, 'input_key': input_key,
             'execution_key': execution_key, 'evidence': evidence_rows(evidence), 'details': details,
             'created_at_ms': int(time.time() * 1000), 'nonce': os.urandom(16).hex()}
    data = canonical(value)
    validation_id = digest(data)
    with locked(root, True):
        check_attempt(read_events(root)[0], attempt, input_key, execution_key)
        write_new(root / 'records' / (validation_id + '.json'), data)
        append(root, outcome, value | {'validation_id': validation_id}, outcome)
    return validation_id


def record_pass(root, attempt, inputs, evidence, details):
    if not evidence:
        raise fail('mismatch', 'a PASS requires retained evidence')
    return record(root, 'pass', attempt, inputs, evidence, details)


def record_fail(root, attempt, inputs, evidence, details):
    return record(root, 'fail', attempt, inputs, evidence, details)


def load_record(root, validation_id):
    if not isinstance(validation_id, str) or not IDENTITY.fullmatch(validation_id):
        raise fail('mismatch', 'invalid validation_id')
    path = canonical_root(root) / 'records' / (validation_id + '.json')
    try:
        descriptor = open_regular(path, os.O_RDONLY)
    except FileNotFoundError:
        raise fail('missing', 'unknown validation_id') from None
    with os.fdopen(descriptor, 'rb') as stream:
        data = stream.read()
    if digest(data) != validation_id:
        raise fail('tampered', 'record bytes differ from validation_id')
    return json.loads(data)


def revoke(root, validation_id, reason):
    root = canonical_root(root)
    if not isinstance(reason, str) or not reason:
        raise fail('mismatch', 'revocation requires a reason')
    with locked(root, True):
        value = load_record(root, validation_id)
        if value['outcome'] != 'pass':
            raise fail('mismatch', 'only a PASS can be revoked')
        return append(root, 'revoke', value | {'validation_id': validation_id}, reason)


def standing(events, validation_id):
    """Return the admitting PASS event unless a later start/FAIL/revoke of its executed inputs exists."""
    admitted = None
    for event in events:
        if event['kind'] == 'pass' and event['validation_id'] == validation_id:
            admitted = event
        elif admitted is not None and event['kind'] != 'pass' and event['execution_key'] == admitted['execution_key']:
            raise fail('blocked', event['kind'] + ' at sequence ' + str(event['sequence']) + ' supersedes this PASS')
    if admitted is None:
        raise fail('missing', 'PASS was never admitted to the index')
    if pending(events, admitted['execution_key']):
        raise fail('blocked', 'an attempt of these executed inputs has not concluded')
    return admitted


def check_record(value, inputs, input_key):
    if value.get('schema') != SCHEMA or value.get('outcome') != 'pass':
        raise fail('mismatch', 'record is not a PASS of this ledger')
    if value['inputs'] != inputs or value['input_key'] != input_key:
        raise fail('mismatch', 'PASS was produced for different inputs')


def verify_events(root, validation_id, inputs, events):
    input_key, execution_key = keys(inputs)
    value = load_record(root, validation_id)
    check_record(value, inputs, input_key)
    admitted = standing(events, validation_id)
    if (admitted['input_key'], admitted['execution_key']) != (input_key, execution_key):
        raise fail('tampered', 'index binds the PASS to different inputs')
    check_evidence(value['evidence'])
    return value | {'validation_id': validation_id, 'sequence': admitted['sequence']}


def verify(root, validation_id, inputs, anchor=None):
    """Read-only admission check of one PASS against the caller's exact inputs."""
    root = canonical_root(root)
    with locked(root, False):
        events, digests = read_events(root)
        check_anchor(digests, anchor)
        return verify_events(root, validation_id, inputs, events) | {'head': head_of(digests)}


def latest_pass(events, input_key):
    latest = None
    for event in events:
        if event['kind'] == 'pass' and event['input_key'] == input_key:
            latest = event['validation_id']
    return latest


def current(root, inputs, anchor=None):
    """Verify the newest PASS for inputs; standing() rejects it if anything later or unconcluded blocks it."""
    root = canonical_root(root)
    input_key, _ = keys(inputs)
    with locked(root, False):
        events, digests = read_events(root)
        check_anchor(digests, anchor)
        latest = latest_pass(events, input_key)
        if latest is None:
            raise fail('blocked', 'no standing PASS for these inputs')
        return verify_events(root, latest, inputs, events) | {'head': head_of(digests)}
