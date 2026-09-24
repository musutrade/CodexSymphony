#!/usr/bin/env python3
"""Host-owned complete prepublication validation and source-bound admission.

No credentials are accepted. Only the controller calls this installed program;
receipts live outside the writable agent mount. The remote tree is resolved by
GitHub before the controller asks for admission.
"""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import environment_contract as contract

BASE = Path.home() / '.local/share/codexsymphony'
STATE = BASE / 'publication'
WORKSPACES = BASE / 'workspaces'
APPROVAL = BASE / 'gate-host/approval.json'


def atomic(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    pending = path.with_suffix('.new')
    with pending.open('w') as stream:
        json.dump(value, stream, indent=2)
        stream.flush()
        os.fsync(stream.fileno())
    pending.replace(path)


def workspace(issue):
    if not re.fullmatch(r'GH-[1-9][0-9]*', issue):
        raise ValueError('assigned GH issue required')
    root = WORKSPACES / issue
    if root.is_symlink() or root.resolve().parent != WORKSPACES.resolve():
        raise ValueError('unsafe workspace')
    return root.resolve(strict=True)


def source_tree(root):
    """Hash exactly the tested bytes, without source filters or index writes."""
    root=Path(root).resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix='publication-tree-') as temp:
        # Do not pass controller credentials to Git or read global Git settings.
        env={'PATH':'/usr/bin:/bin','HOME':temp,'LANG':'C.UTF-8',
             'GIT_CONFIG_NOSYSTEM':'1','GIT_CONFIG_GLOBAL':'/dev/null'}
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


def admit(root, receipt, tree):
    if receipt.get('status') != 'PASS' or receipt.get('scope') != 'complete-local-isolated-gate':
        raise ValueError('publication rejected: complete local validation has not passed')
    current = inputs(root)
    if current != receipt.get('inputs') or tree != current['tree']:
        raise ValueError('publication rejected: source/tree/environment changed; run local_gate again')
    report = Path(receipt['report'])
    if hashlib.sha256(report.read_bytes()).hexdigest() != receipt['report_sha256']:
        raise ValueError('publication rejected: retained report changed')
    value = json.loads(report.read_text())
    if not value.get('passed') or not value.get('evidence_complete'):
        raise ValueError('publication rejected: incomplete gate report')
    return {'status': 'PASS', **current}


def validate(issue):
    root = workspace(issue)
    state = STATE / issue
    state.mkdir(parents=True, exist_ok=True)
    with (state / 'lock').open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        atomic(state / 'receipt.json', {'status': 'RUNNING'})
        try:
            before = inputs(root)
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
            run = Path(result['run'])
            if source_tree(run/'workspace') != before['tree']:
                raise ValueError('validated snapshot tree differs from publication source')
            proof = json.loads((run / 'environment.json').read_text())
            if proof['fingerprint'] != before['environment']:
                raise ValueError('environment drift: verifier and publication fingerprints differ')
            report = run / 'workspace/.harness-gate/reports/test_result.json'
            receipt = {'status': 'PASS', 'scope': result['scope'], 'inputs': before,
                       'report': str(report), 'report_sha256': hashlib.sha256(report.read_bytes()).hexdigest()}
            admit(root, receipt, before['tree'])
            atomic(state / 'receipt.json', receipt)
        except Exception as error:
            atomic(state / 'receipt.json', {'status': 'FAIL', 'error': str(error),
                                           'logs': str(state)})
            raise


def start(issue):
    root = workspace(issue)
    inputs(root)  # fail before starting expensive work on a drifted environment
    unit = 'codexsymphony-prepublish-' + issue.lower()
    active = subprocess.run(['systemctl', '--user', 'is-active', '--quiet', unit + '.service'])
    if active.returncode == 0:
        return {'status': 'RUNNING'}
    state = STATE / issue
    atomic(state / 'receipt.json', {'status': 'PENDING'})
    subprocess.run(['systemd-run', '--user', '--collect', '--quiet', '--unit', unit,
                    '--property=RuntimeMaxSec=3400', '--property=UMask=0077',
                    '/usr/bin/python3', str(Path(__file__).resolve()), 'validate', '--issue', issue], check=True)
    return {'status': 'RUNNING'}


def main():
    os.umask(0o077)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation', choices=['start', 'status', 'check', 'validate'])
    parser.add_argument('--issue', required=True)
    parser.add_argument('--tree')
    args = parser.parse_args()
    root = workspace(args.issue)
    if args.operation == 'validate':
        validate(args.issue)
        return
    if args.operation == 'start':
        print(json.dumps(start(args.issue)))
        return
    path = STATE / args.issue / 'receipt.json'
    receipt = json.loads(path.read_text()) if path.exists() else {'status': 'MISSING'}
    if args.operation == 'check':
        print(json.dumps(admit(root, receipt, args.tree)))
    elif receipt.get('status') == 'PASS':
        try:
            print(json.dumps(admit(root, receipt, receipt['inputs']['tree'])))
        except ValueError as error:
            print(json.dumps({'status': 'STALE', 'error': str(error)}))
    else:
        if receipt.get('status')=='FAIL':
            logs={}
            for name in ['gate.stdout','gate.stderr']:
                log=path.parent/name
                if log.exists():
                    with log.open('rb') as stream:
                        stream.seek(max(0,log.stat().st_size-12000))
                        logs[name]=stream.read().decode(errors='replace')
            stdout=logs.get('gate.stdout','')
            for line in stdout.splitlines():
                if line.startswith('Retaining complete gate run: '):
                    run=Path(line.split(': ',1)[1])
                    if run.resolve().parent!=(BASE/'gate-host/runs').resolve():
                        continue
                    timing=run/'timings.jsonl'
                    failed=[]
                    if timing.exists():
                        failed=[json.loads(row)['phase'] for row in timing.read_text().splitlines() if json.loads(row).get('status')=='FAIL']
                    for phase in failed[-2:]:
                        if not re.fullmatch(r'[a-z-]+',phase): continue
                        for suffix in ['stdout','stderr']:
                            log=run/(phase+'.'+suffix)
                            if log.exists():
                                with log.open('rb') as stream:
                                    stream.seek(max(0,log.stat().st_size-8000))
                                    logs[phase+'.'+suffix]=stream.read().decode(errors='replace')
            receipt['diagnostics']=logs
        print(json.dumps(receipt))


if __name__ == '__main__':
    try:
        main()
    except Exception as error:
        print(json.dumps({'status': 'REJECTED', 'error': str(error)}))
        sys.exit(1)
