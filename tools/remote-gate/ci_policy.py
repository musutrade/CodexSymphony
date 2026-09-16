"""Host-owned, conservative documentation scope and process lifecycle helpers."""
import hashlib
import json
from pathlib import Path
import os
import signal
import subprocess
import time

# Explicit prose-only inputs. Never infer safety from a .md suffix: workflow,
# agent instructions, fixtures and executable documentation require full checks.
DOC_PATHS = frozenset({
    'README.md',
    'Personal_AI_Software_Factory_综合方案.md',
    'docs/evidence-lifecycle-impact.md',
})


def policy_identity(config, approval):
    value = {'repository': config['repository'], 'protected_files': config['protected_files'],
             'approval': approval}
    return hashlib.sha256(json.dumps(value, sort_keys=True).encode()).hexdigest()


def documentation_changes(root, baseline, head):
    ancestor = subprocess.run(['git', '-C', root, 'merge-base', '--is-ancestor', baseline, head],
                              stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    if ancestor.returncode:
        return None
    names = subprocess.check_output(['git', '-C', root, 'diff', '--no-renames', '--name-only',
                                     '-z', baseline, head]).decode().split('\0')
    names = [name for name in names if name]
    if not names or not set(names) <= DOC_PATHS:
        return None
    for name in names:
        path = Path(root) / name
        mode = subprocess.check_output(['git', '-C', root, 'ls-tree', head, '--', name]).decode()
        if not mode.startswith('100644 blob ') or path.is_symlink() or not path.is_file():
            return None  # deletions, executable files and symlinks get full validation
        if path.stat().st_size > 2 * 1024 * 1024:
            raise ValueError('documentation exceeds 2 MiB: ' + name)
        content = path.read_bytes().decode('utf-8')
        if '\x00' in content or any(line.startswith(('<<<<<<< ', '=======', '>>>>>>> '))
                                  for line in content.splitlines()):
            raise ValueError('invalid documentation or merge conflict: ' + name)
    subprocess.run(['git', '-C', root, 'diff', '--check', baseline, head], check=True,
                   stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    return names


def documentation_result(root, run, config, approval, job):
    if run['event'] == 'workflow_dispatch':
        return None  # an explicit manual run always requests the complete suite
    policy = policy_identity(config, approval)
    receipts = sorted((Path(config['state_root']) / 'jobs').glob('*/receipt.json'),
                      key=lambda p: p.stat().st_mtime, reverse=True)[:100]
    for receipt in receipts:
        if receipt.is_symlink() or receipt.parent.is_symlink():
            raise ValueError('symlink baseline receipt')
        prior = json.loads(receipt.read_text())
        if not (prior.get('finished') and prior.get('status') == 'PASS'
                and prior.get('scope') == 'full' and prior.get('policy_identity') == policy):
            continue
        names = documentation_changes(root, prior['source_sha'], run['head_sha'])
        if names is None:
            continue
        report = {'scope': 'documentation', 'status': 'PASS', 'source_sha': run['head_sha'],
                  'identity': f"{run['id']}/{run['run_attempt']}", 'policy_identity': policy,
                  'baseline_sha': prior['source_sha'], 'baseline_identity': prior['identity'],
                  'baseline_report_sha256': prior['report_sha256'], 'changed_paths': names,
                  'checks': ['approved prose paths', 'regular UTF-8 files', 'size limit',
                             'conflict markers', 'git diff --check'],
                  'full_suite_executed': False}
        path = Path(job) / 'documentation-result.json'
        path.write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
        return report | {'report_sha256': hashlib.sha256(path.read_bytes()).hexdigest()}
    return None


class SupersededRun(Exception):
    pass


def run_cancellable(command, stdout, stderr, cancelled, timeout=1500, interval=15):
    """Only the host callback can cancel; terminate the entire isolated process group."""
    process = subprocess.Popen(command, stdout=stdout, stderr=stderr, start_new_session=True)
    deadline = time.monotonic() + timeout
    try:
        while process.poll() is None:
            if cancelled():
                raise SupersededRun('Actions attempt cancelled or superseded')
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise subprocess.TimeoutExpired(command, timeout)
            try:
                process.wait(timeout=min(interval, remaining))
            except subprocess.TimeoutExpired:
                pass
        return process.returncode
    finally:
        # Also reap descendants left by a failed launcher. No orphan build survives
        # cancellation/API errors/timeouts and competes with the next run.
        try:
            os.killpg(process.pid, signal.SIGTERM)
            if process.poll() is None:
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    pass
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        process.wait()
