#!/usr/bin/env python3
"""Host-owned complete prepublication validation and source-bound admission.

No credentials are accepted. Only the controller calls this installed program;
receipts live outside the writable agent mount. The remote tree is resolved by
GitHub before the controller asks for admission.
"""
import argparse
from contextlib import ExitStack, contextmanager
import fcntl
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import sys
import tempfile

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import environment_contract as contract
import evidence_ledger as ledger

BASE = Path.home() / '.local/share/codexsymphony'
STATE = BASE / 'publication'
# Immutable outcomes; receipt.json is only a mutable status/validation_id pointer.
LEDGER = STATE / 'ledger'
WORKSPACES = BASE / 'workspaces'
APPROVAL = BASE / 'gate-host/approval.json'
CGROUP_ROOT = Path('/sys/fs/cgroup')
# Written by the installer from the installed approval's host release (bounded_layout.lease).
HOST_LEASES = Path(__file__).resolve().with_name('host-leases.json')
# Minimal verification evidence. Native raw profiles stay under capture retention
# and are deliberately not bound: their compaction must not invalidate a PASS.
REQUIRED_EVIDENCE = {3: ('environment.json', 'requests.json', 'source-archive.json', 'source.tar.gz', 'source-inputs.json')}
OPTIONAL_EVIDENCE = ('test-capture.json', 'complete-gate.json', 'verify-result.json', 'measurement-summary.json',
                     'capture-registration.json', 'independent-raw-inventory.json')


def atomic(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    pending = path.with_suffix('.new')
    with pending.open('w') as stream:
        json.dump(value, stream, indent=2)
        stream.flush()
        os.fsync(stream.fileno())
    pending.replace(path)


def fixed_binding(issue, approval):
    binding = STATE / 'active-workspace.json'
    if binding.is_symlink():
        raise ValueError('aliased workspace binding')
    selected = json.loads(binding.read_text())
    root = Path('/home/gem/CodexSymphony')
    volume = Path('/mnt/dev-ssd/codexsymphony-bounded/data')
    if selected != {'issue': issue, 'repository': str(root)} or approval['repository'] != str(root):
        raise ValueError('issue is not assigned to the fixed workspace')
    if not root.is_mount() or not volume.is_mount() or root.stat().st_dev != volume.stat().st_dev:
        raise ValueError('bounded workspace mounts required')
    return root


def workspace(issue):
    if not re.fullmatch(r'GH-[1-9][0-9]*', issue):
        raise ValueError('assigned GH issue required')
    approval = json.loads(APPROVAL.read_text())
    if approval.get('execution_version') == 3:
        return fixed_binding(issue, approval)
    root = WORKSPACES / issue
    if root.is_symlink() or root.resolve().parent != WORKSPACES.resolve():
        raise ValueError('unsafe workspace')
    return root.resolve(strict=True)


def snapshot_matches(run, root, approval):
    if approval.get('execution_version') != 3:
        return source_tree(run / 'workspace') == source_tree(root)
    archive = json.loads((run / 'source-archive.json').read_text())
    if hashlib.sha256((run / 'source.tar.gz').read_bytes()).hexdigest() != archive['sha256']:
        raise ValueError('retained source archive changed')
    path = Path(approval['host_release']) / 'fixed_workspace.py'
    if hashlib.sha256(path.read_bytes()).hexdigest() != approval['runtime_files'][str(path)]:
        raise ValueError('approved source reader changed')
    spec = importlib.util.spec_from_file_location('approved_source_reader', path)
    reader = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(reader)
    current = reader.sources(root)
    captured = json.loads((run / 'source-inputs.json').read_text())
    return current == archive['inputs'] and captured == {name: row['sha256'] for name, row in current.items()}


def temporary_root(root):
    if root != Path('/home/gem/CodexSymphony'):
        return None
    volume = Path('/mnt/dev-ssd/codexsymphony-bounded/data')
    if not volume.is_mount():
        raise ValueError('bounded temporary filesystem missing')
    return volume / 'tmp'


def git_environment(temp):
    return {'PATH': '/usr/bin:/bin', 'HOME': temp, 'LANG': 'C.UTF-8',
            'GIT_CONFIG_NOSYSTEM': '1', 'GIT_CONFIG_GLOBAL': '/dev/null'}


def head_commit(root):
    """Commit the host Gate names as the evidence context (rev-parse HEAD)."""
    with tempfile.TemporaryDirectory(prefix='publication-head-', dir=temporary_root(Path(root))) as temp:
        return subprocess.check_output(['/usr/bin/git', '-c', 'core.fsmonitor=false', '-C', str(root),
                                        'rev-parse', '--verify', 'HEAD^{commit}'],
                                       env=git_environment(temp), text=True).strip()


def commit_parents(root, commit):
    """Parents of the validated commit, recorded as an audit label; they are no equivalence condition."""
    with tempfile.TemporaryDirectory(prefix='publication-parents-', dir=temporary_root(Path(root))) as temp:
        row = subprocess.check_output(['/usr/bin/git', '-c', 'core.fsmonitor=false', '-C', str(root),
                                       'rev-list', '--parents', '--max-count=1', commit],
                                      env=git_environment(temp), text=True).split()
    if row[:1] != [commit]:
        raise ValueError('validated commit parents unavailable')
    return row[1:]


def source_tree(root):
    """Hash exactly the tested bytes, without source filters or index writes."""
    root=Path(root).resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix='publication-tree-', dir=temporary_root(root)) as temp:
        # Do not pass controller credentials to Git or read global Git settings.
        env=git_environment(temp)
        git=['/usr/bin/git','-c','core.fsmonitor=false','-c','core.hooksPath=/dev/null']
        names=subprocess.check_output([*git,'-C',str(root),'ls-files','-z',
                                       '--cached','--others','--exclude-standard'],env=env).split(b'\0')
        database=Path(temp)/'objects.git'
        subprocess.run([*git,'init','--bare','--object-format=sha1','--quiet',str(database)],env=env,check=True)
        command=[*git,'--git-dir='+str(database)]
        entries=[]
        for raw in sorted(set(filter(None,names))):
            name=raw.decode('utf-8')
            path=root/name
            if path.is_symlink() or not path.resolve().is_relative_to(root):
                raise ValueError('unsafe source path: '+name)
            if not path.exists(): continue  # tracked deletion
            if not path.is_file(): raise ValueError('unsupported source entry: '+name)
            oid=subprocess.check_output([*command,'hash-object','-w','--no-filters','--stdin'],
                                        input=path.read_bytes(),env=env).strip()
            mode=b'100755' if path.stat().st_mode & 0o100 else b'100644'
            entries.append(mode+b' '+oid+b'\t'+raw+b'\0')
        subprocess.run([*command,'update-index','-z','--index-info'],
                       input=b''.join(entries),env=env,check=True)
        return subprocess.check_output([*command,'write-tree'],env=env,text=True).strip()


def environment(root):
    value = contract.load(root)
    env = dict(os.environ, PATH=contract.tool_path(value), **contract.test_environment(value))
    return contract.fingerprint(root, env)


def inputs(root):
    approval = json.loads(APPROVAL.read_text())
    # Code/tool changes invalidate a result even when the approval path is stable.
    for name, expected in approval['runtime_files'].items():
        if hashlib.sha256(Path(name).read_bytes()).hexdigest() != expected:
            raise ValueError('environment drift: approved runtime changed: ' + name)
    return {'tree': source_tree(root), 'environment': environment(root)['fingerprint'],
            'approval': contract.digest(approval)}


def ledger_inputs(current, commit):
    return {'approval': current['approval'], 'commit': commit,
            'environment': current['environment'], 'tree': current['tree']}


def check_report(report):
    value = json.loads(report.read_text())
    if not value.get('passed') or not value.get('evidence_complete'):
        raise ValueError('publication rejected: incomplete gate report')


def admit(root, receipt, tree):
    """The receipt only points at a validation_id; the ledger decides admission."""
    if receipt.get('status') != 'PASS' or receipt.get('scope') != 'complete-local-isolated-gate':
        raise ValueError('publication rejected: complete local validation has not passed')
    current = inputs(root)
    if current != receipt.get('inputs') or tree != current['tree']:
        raise ValueError('publication rejected: source/tree/environment changed; run local_gate again')
    commit = head_commit(root)
    if commit != receipt.get('commit'):
        raise ValueError('publication rejected: evidence commit changed; run local_gate again')
    record = ledger.verify(LEDGER, receipt.get('validation_id'), ledger_inputs(current, commit))
    report = Path(record['evidence']['report']['path'])
    if str(report) != receipt.get('report') or record['evidence']['report']['sha256'] != receipt.get('report_sha256'):
        raise ValueError('publication rejected: receipt differs from the ledger record')
    check_report(report)
    return {'status': 'PASS', 'validation_id': record['validation_id'], 'commit': commit, **current}


def evidence_commits(run, report):
    """Commits the host Gate itself sealed as its evidence context."""
    requests = json.loads((run / 'requests.json').read_text())
    commits = {value['context']['commit'] for value in requests.values()}
    capture = run / 'test-capture.json'
    if capture.is_file():
        commits.add(json.loads(capture.read_text())['context']['commit'])
    commits.add(json.loads(report.read_text())['source_identity'].removeprefix('working-tree:'))
    return commits


def check_commit(root, run, report, approval, commit):
    """The announced commit must still be HEAD and be the commit the Gate recorded."""
    if head_commit(root) != commit:
        raise ValueError('HEAD changed during local validation')
    if approval.get('execution_version') == 3 and evidence_commits(run, report) != {commit}:
        raise ValueError('Gate evidence context names another commit')


def retained_evidence(run, report, approval):
    names = REQUIRED_EVIDENCE.get(approval.get('execution_version'), ('environment.json',))
    evidence = {'report': report, **{name: run / name for name in names}}
    evidence.update({name: run / name for name in OPTIONAL_EVIDENCE if (run / name).is_file()})
    return {name: str(path) for name, path in evidence.items()}


def run_gate(root, state, before, commit):
    """Execute the installed complete Gate and return its checked retained report."""
    approval = json.loads(APPROVAL.read_text())
    approval['repository'] = str(root)
    atomic(state / 'gate-approval.json', approval)
    policy = contract.load(root)
    env = dict(os.environ, PATH=contract.tool_path(policy), **contract.test_environment(policy))
    with (state / 'gate.stdout').open('w') as out, (state / 'gate.stderr').open('w') as err:
        subprocess.run(['/usr/bin/python3', str(Path(approval['host_release']) / 'run.py'),
                        '--repository', str(root), '--approval', str(state / 'gate-approval.json')],
                       env=env, stdout=out, stderr=err, check=True, timeout=3300)
    result = json.loads((state / 'gate.stdout').read_text().splitlines()[-1])
    if result.get('status') != 'PASS' or result.get('scope') != 'complete-local-isolated-gate':
        raise ValueError('complete Gate acceptance missing')
    if inputs(root) != before:
        raise ValueError('source/environment changed during local validation')
    run = Path(result['run']).resolve(strict=True)
    if not snapshot_matches(run, root, approval):
        raise ValueError('validated snapshot tree differs from publication source')
    proof = json.loads((run / 'environment.json').read_text())
    if proof['fingerprint'] != before['environment']:
        raise ValueError('environment drift: verifier and publication fingerprints differ')
    report = run / ('reports/test_result.json' if approval.get('execution_version') == 3 else 'workspace/.harness-gate/reports/test_result.json')
    check_report(report)
    check_commit(root, run, report, approval, commit)
    return run, report, approval, result['scope']


def conclude_failure(state, attempt, identity, error):
    """Conclude this attempt as FAIL; if the ledger refuses, the receipt keeps identity for recover."""
    receipt = {'status': 'FAIL', 'error': str(error), 'logs': str(state), 'attempt': attempt, 'identity': identity}
    if attempt is not None:
        try:
            receipt['validation_id'] = ledger.record_fail(LEDGER, attempt, identity, {}, {'error': str(error)[:4000]})
        except (ledger.LedgerError, OSError) as refused:
            receipt['ledger_error'] = str(refused)
    atomic(state / 'receipt.json', receipt)


def read_receipt(state):
    path = state / 'receipt.json'
    return json.loads(path.read_text()) if path.exists() else {'status': 'MISSING'}


def orphaned(receipt):
    """Unconcluded attempts of the inputs an unfinished receipt announced."""
    identity = receipt.get('identity')
    if not identity or receipt.get('validation_id'):
        return []
    try:
        return ledger.unconcluded(LEDGER, identity)
    except ledger.LedgerError as error:
        if getattr(error, 'category', None) == 'missing':
            return []
        raise


def refuse_orphaned(state):
    """Never overwrite the only pointer to an unconcluded attempt."""
    if orphaned(read_receipt(state)):
        raise ValueError('unconcluded attempt: run recover after its execution has terminated')


@contextmanager
def scheduling(state):
    """Short blocking lock: every receipt transition by start, validate or recover takes it first."""
    state.mkdir(parents=True, exist_ok=True)
    with (state / 'schedule.lock').open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        yield


def execution(state):
    """Per-issue execution lock, taken without waiting while scheduling() is held.

    validate() keeps it for its whole run and is then the only receipt writer.
    start() and recover() hold it only inside scheduling(), so a validate() that
    waits on scheduling() never meets their transient hold.
    """
    lock = (state / 'lock').open('a')
    try:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        lock.close()
        raise
    return lock


def claim(state):
    """Under scheduling: own execution and write RUNNING unless an attempt is orphaned."""
    with scheduling(state):
        lock = execution(state)
        try:
            refuse_orphaned(state)
            atomic(state / 'receipt.json', {'status': 'RUNNING'})
        except BaseException:
            lock.close()
            raise
    return lock


def validate(issue):
    root = workspace(issue)
    state = STATE / issue
    with claim(state):
        attempt = identity = None
        try:
            before = inputs(root)
            identity = ledger_inputs(before, head_commit(root))
            atomic(state / 'receipt.json', {'status': 'RUNNING', 'identity': identity})
            # Announced under the per-issue execution lock, before the Gate starts.
            attempt = ledger.begin(LEDGER, identity)
            atomic(state / 'receipt.json', {'status': 'RUNNING', 'identity': identity, 'attempt': attempt})
            run, report, approval, scope = run_gate(root, state, before, identity['commit'])
            details = {'issue': issue, 'scope': scope, 'run': str(run), 'execution_version': approval.get('execution_version'),
                       'parents': commit_parents(root, identity['commit'])}
            validation_id = ledger.record_pass(LEDGER, attempt, identity, retained_evidence(run, report, approval), details)
            atomic(state / 'receipt.json', {'status': 'PASS', 'scope': scope, 'inputs': before, 'commit': identity['commit'],
                                            'identity': identity, 'attempt': attempt, 'validation_id': validation_id,
                                            'report': str(report), 'report_sha256': ledger.file_digest(report)})
        except Exception as error:
            conclude_failure(state, attempt, identity, error)
            raise


def unit_name(issue):
    return 'codexsymphony-prepublish-' + issue.lower()


def unit_state(issue):
    """(ActiveState, ControlGroup) of the validation unit."""
    shown = subprocess.check_output(['systemctl', '--user', 'show', '--property=ActiveState',
                                     '--property=ControlGroup', unit_name(issue) + '.service'], text=True)
    fields = dict(line.split('=', 1) for line in shown.splitlines() if '=' in line)
    return fields.get('ActiveState', ''), fields.get('ControlGroup', '')


def populated(group):
    """cgroup v2 'populated' covers the group and every descendant; an absent group was collected."""
    if not group:
        return False
    directory = CGROUP_ROOT / group.lstrip('/')
    if not directory.exists():
        return False
    events = directory / 'cgroup.events'
    if not events.is_file():
        raise ValueError('recovery rejected: cgroup v2 occupancy unavailable')
    fields = dict(line.split(' ', 1) for line in events.read_text().splitlines() if ' ' in line)
    if fields.get('populated') not in ('0', '1'):
        raise ValueError('recovery rejected: unreadable cgroup occupancy')
    return fields['populated'] == '1'


def check_terminated(issue):
    """A settled unit whose cgroup subtree holds no process; 'failed' alone can leave children."""
    active, group = unit_state(issue)
    if active not in ('inactive', 'failed'):
        raise ValueError('recovery rejected: validation unit is ' + (active or 'unknown'))
    if populated(group):
        raise ValueError('recovery rejected: validation cgroup still has processes')


def required_leases():
    """Leases v3 runs hold for their whole execution, bound to the current approval identity."""
    approval = json.loads(APPROVAL.read_text())
    if approval.get('execution_version') != 3:
        return ()
    try:
        bound = json.loads(HOST_LEASES.read_text())
    except FileNotFoundError:
        raise ValueError('recovery rejected: installed lease binding missing') from None
    if bound.get('approval') != contract.digest(approval) or bound.get('host_release') != approval['host_release']:
        raise ValueError('recovery rejected: lease binding is for another approval; reinstall')
    if not bound.get('leases'):
        raise ValueError('recovery rejected: installed lease binding is empty')
    return tuple(Path(path) for path in bound['leases'])


@contextmanager
def execution_leases():
    """Hold the installed Gate's execution leases; a missing lease is not proof of termination."""
    with ExitStack() as stack:
        for path in required_leases():
            try:
                descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
            except OSError:
                raise ValueError('recovery rejected: Gate execution lease unavailable: ' + str(path)) from None
            stack.callback(os.close, descriptor)
            if not stat.S_ISREG(os.fstat(descriptor).st_mode):
                raise ValueError('recovery rejected: Gate execution lease unavailable: ' + str(path))
            try:
                fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                raise ValueError('recovery rejected: Gate execution lease is held: ' + str(path)) from None
        yield


def crashed_attempt(receipt):
    attempts = orphaned(receipt)
    named = receipt.get('attempt')
    if named in attempts:
        return named
    if named is None and len(attempts) == 1:
        return attempts[0]  # begin landed but the receipt never learned its id
    raise ValueError('recovery rejected: no single unconcluded attempt is named by the receipt')


def recover(issue):
    """Conclude a crashed attempt as FAIL once its execution is proven terminated.

    The per-issue lock excludes a live validate, the unit cgroup must be empty and
    the Gate's execution leases free. Nothing is restored to PASS; a new complete
    Gate is still required before anything can stand.
    """
    state = STATE / issue
    with scheduling(state), execution(state):
        check_terminated(issue)
        with execution_leases():
            receipt = read_receipt(state)
            attempt = crashed_attempt(receipt)
            validation_id = ledger.record_fail(LEDGER, attempt, receipt['identity'], {},
                                               {'issue': issue, 'error': 'execution terminated without conclusion'})
        concluded = {'status': 'FAIL', 'error': 'recovered: execution terminated without conclusion', 'logs': str(state),
                     'identity': receipt['identity'], 'attempt': attempt, 'validation_id': validation_id}
        atomic(state / 'receipt.json', concluded)
        return concluded


def unit_active(issue):
    return subprocess.run(['systemctl', '--user', 'is-active', '--quiet', unit_name(issue) + '.service']).returncode == 0


def start(issue):
    root = workspace(issue)
    inputs(root)  # fail before starting expensive work on a drifted environment
    state = STATE / issue
    with scheduling(state):
        try:
            probe = execution(state)
        except BlockingIOError:
            return {'status': 'RUNNING'}  # a validate owns the receipt; never overwrite it
        with probe:
            if unit_active(issue):
                return {'status': 'RUNNING'}  # scheduled validate is waiting on scheduling()
            refuse_orphaned(state)
            atomic(state / 'receipt.json', {'status': 'PENDING'})
            subprocess.run(['systemd-run', '--user', '--collect', '--quiet', '--unit', unit_name(issue),
                            '--property=RuntimeMaxSec=3400', '--property=UMask=0077',
                            '/usr/bin/python3', str(Path(__file__).resolve()), 'validate', '--issue', issue], check=True)
    return {'status': 'RUNNING'}


def tail(path, size):
    with path.open('rb') as stream:
        stream.seek(max(0, path.stat().st_size - size))
        return stream.read().decode(errors='replace')


def failed_phases(run):
    timing = run / 'timings.jsonl'
    if not timing.exists():
        return []
    rows = [json.loads(row) for row in timing.read_text().splitlines()]
    return [row['phase'] for row in rows if row.get('status') == 'FAIL']


def phase_logs(stdout):
    """Tails of the last failed phases of the retained run the Gate announced."""
    logs = {}
    for line in stdout.splitlines():
        if not line.startswith('Retaining complete gate run: '):
            continue
        run = Path(line.split(': ', 1)[1])
        if run.resolve().parent != (BASE / 'gate-host/runs').resolve():
            continue
        for phase in failed_phases(run)[-2:]:
            if not re.fullmatch(r'[a-z-]+', phase):
                continue
            for suffix in ['stdout', 'stderr']:
                log = run / (phase + '.' + suffix)
                if log.exists():
                    logs[phase + '.' + suffix] = tail(log, 8000)
    return logs


def diagnostics(state):
    logs = {name: tail(state / name, 12000) for name in ['gate.stdout', 'gate.stderr'] if (state / name).exists()}
    return logs | phase_logs(logs.get('gate.stdout', ''))


def status(root, issue, tree):
    receipt = read_receipt(STATE / issue)
    if receipt.get('status') == 'PASS':
        try:
            return admit(root, receipt, receipt['inputs']['tree'])
        except ValueError as error:
            return {'status': 'STALE', 'error': str(error)}
    if receipt.get('status') == 'FAIL':
        receipt['diagnostics'] = diagnostics(STATE / issue)
    return receipt


def check(root, issue, tree):
    return admit(root, read_receipt(STATE / issue), tree)


def start_operation(root, issue, tree):
    return start(issue)


def recover_operation(root, issue, tree):
    return recover(issue)


OPERATIONS = {'start': start_operation, 'recover': recover_operation, 'status': status, 'check': check}


def main():
    os.umask(0o077)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation', choices=['start', 'status', 'check', 'validate', 'recover'])
    parser.add_argument('--issue', required=True)
    parser.add_argument('--tree')
    args = parser.parse_args()
    root = workspace(args.issue)
    if args.operation == 'validate':
        validate(args.issue)
        return
    print(json.dumps(OPERATIONS[args.operation](root, args.issue, args.tree)))


if __name__ == '__main__':
    try:
        main()
    except Exception as error:
        print(json.dumps({'status': 'REJECTED', 'error': str(error)}))
        sys.exit(1)
