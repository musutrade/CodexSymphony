"""Explicit post-merge equivalence receipts; original full reports stay unchanged."""
import hashlib
import json
from pathlib import Path
import subprocess
from ci_policy import policy_identity


def sha(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def tree(root, revision):
    return subprocess.check_output(['git', '-C', root, 'rev-parse', revision + '^{tree}'],
                                   text=True, stderr=subprocess.PIPE).strip()


def identical_tree_result(root, run, config, approval, job):
    # Explicit manual runs always execute everything. Rollout enables reuse only
    # after a complete main run has seeded the trusted compilation cache.
    if (config.get('reuse_identical_tree') is not True or run['event'] != 'push'
            or run.get('head_branch') != 'main'):
        return None
    policy = policy_identity(config, approval)
    current_tree = tree(root, run['head_sha'])
    receipts = sorted((Path(config['state_root']) / 'jobs').glob('*/receipt.json'),
                      key=lambda p: p.stat().st_mtime, reverse=True)[:100]
    for path in receipts:
        if path.resolve() != path:
            raise ValueError('unsafe equivalence baseline')
        prior = json.loads(path.read_text())
        if not (prior.get('finished') and prior.get('status') == 'PASS' and prior.get('scope') == 'full'
                and prior.get('policy_identity') == policy and prior.get('event') == 'pull_request'):
            continue
        try:
            if tree(root, prior['source_sha']) != current_tree:
                continue
        except subprocess.CalledProcessError:
            continue
        report = Path(prior['run']) / 'workspace/.harness-gate/reports/test_result.json'
        if not report.is_file():
            continue  # Expired evidence requires a fresh full run.
        if report.resolve() != report or sha(report) != prior['report_sha256']:
            raise ValueError('full baseline report changed')
        value = json.loads(report.read_text())
        if (value.get('passed') is not True or value.get('evidence_complete') is not True
                or value.get('source_identity') not in ('commit:' + prior['source_sha'], 'working-tree:' + prior['source_sha'])):
            raise ValueError('incomplete full baseline report')
        evidence = value['quality']['evidence']
        if not evidence or any(e['context']['commit'] != prior['source_sha'] for e in evidence):
            raise ValueError('baseline evidence source differs')
        # A previous verdict never excuses checking today's installed runtime.
        for name, digest in approval['runtime_files'].items():
            p = Path(name)
            if p.resolve() != p or sha(p) != digest:
                raise ValueError('approved runtime changed: ' + name)
        result = {'scope': 'identical-tree', 'status': 'PASS', 'source_sha': run['head_sha'],
                  'identity': f"{run['id']}/{run['run_attempt']}", 'policy_identity': policy,
                  'tree': current_tree, 'baseline_sha': prior['source_sha'],
                  'baseline_identity': prior['identity'], 'baseline_report_sha256': prior['report_sha256'],
                  'full_suite_executed': False,
                  'checks': ['exact Git tree', 'complete approved policy', 'installed runtime digests',
                             'retained full report digest', 'complete source-bound baseline evidence']}
        output = job / 'tree-equivalence.json'
        output.write_text(json.dumps(result, indent=2) + '\n')
        return result | {'report_sha256': sha(output)}
    return None
