import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from ci_policy import policy_identity
from tree_reuse import identical_tree_result


class TreeReuse(unittest.TestCase):
    def test_equal_tree_binds_new_attempt_without_relabelling_prior_report(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary); repo = root / 'repo'; repo.mkdir()
            def git(*args):
                return subprocess.check_output(['git', '-C', repo, *args], stderr=subprocess.PIPE, text=True).strip()
            git('init', '-q'); git('config', 'user.email', 'fixture@example.invalid'); git('config', 'user.name', 'Fixture')
            (repo / 'code').write_text('same code'); git('add', '.'); git('commit', '-qm', 'PR')
            prior_sha = git('rev-parse', 'HEAD')
            git('commit', '--allow-empty', '-qm', 'merge')
            head = git('rev-parse', 'HEAD')
            runtime = root / 'runtime'; runtime.write_text('reviewed runtime')
            approval = {'execution_version': 2, 'runtime_files': {str(runtime): hashlib.sha256(runtime.read_bytes()).hexdigest()}}
            config = {'reuse_identical_tree': True, 'repository': 'owner/repo', 'protected_files': {}, 'state_root': str(root)}
            run_dir = root / 'retained'
            report = run_dir / 'workspace/.harness-gate/reports/test_result.json'; report.parent.mkdir(parents=True)
            report.write_text(json.dumps({'passed': True, 'evidence_complete': True,
                'source_identity': 'commit:' + prior_sha, 'quality': {'evidence': [{'context': {'commit': prior_sha}}]}}))
            original = report.read_bytes()
            cache = run_dir / 'cache-restore.json'; cache.write_text(json.dumps({'hit': True}))
            prior = {'finished': True, 'scope': 'full', 'status': 'PASS', 'event': 'pull_request',
                     'policy_identity': policy_identity(config, approval), 'source_sha': prior_sha,
                     'identity': '1/1', 'report_sha256': hashlib.sha256(original).hexdigest(), 'run': str(run_dir)}
            receipt = root / 'jobs/1-1/receipt.json'; receipt.parent.mkdir(parents=True); receipt.write_text(json.dumps(prior))
            job = root / 'jobs/2-1'; job.mkdir()
            run = {'event': 'push', 'head_branch': 'main', 'head_sha': head, 'id': 2, 'run_attempt': 1}
            result = identical_tree_result(repo, run, config, approval, job)
            self.assertEqual(result['scope'], 'identical-tree')
            self.assertEqual(result['source_sha'], head)
            self.assertEqual(result['baseline_sha'], prior_sha)
            self.assertFalse(result['full_suite_executed'])
            self.assertEqual(report.read_bytes(), original)
            for hit in (False, 'true', None):
                cache.write_text(json.dumps({'hit': hit}))
                self.assertIsNone(identical_tree_result(repo, run, config, approval, job))
            # Explicitly disabling caching does not require warming a seed.
            self.assertIsNotNone(identical_tree_result(repo, run, config | {'cache_max_bytes': 0}, approval, job))
            cache.unlink()
            self.assertIsNone(identical_tree_result(repo, run, config, approval, job))
            cache.write_text(json.dumps({'hit': True}))
            failed = root / 'jobs/failed/receipt.json'; failed.parent.mkdir()
            failed.write_text(json.dumps({'finished': True, 'status': 'FAIL', 'source_sha': prior_sha}))
            self.assertIsNone(identical_tree_result(repo, run, config, approval, job))
            failed.unlink()
            for event in ('pull_request', 'workflow_dispatch'):
                self.assertIsNone(identical_tree_result(repo, run | {'event': event}, config, approval, job))
            for change in ({'scope': 'identical-tree'}, {'policy_identity': 'old'}, {'finished': False}, {'status': 'FAIL'}):
                receipt.write_text(json.dumps(prior | change))
                self.assertIsNone(identical_tree_result(repo, run, config, approval, job))
            receipt.write_text(json.dumps(prior))
            runtime.write_text('changed')
            with self.assertRaisesRegex(ValueError, 'runtime changed'):
                identical_tree_result(repo, run, config, approval, job)
            runtime.write_text('reviewed runtime')
            report.write_bytes(original + b' ')
            with self.assertRaisesRegex(ValueError, 'report changed'):
                identical_tree_result(repo, run, config, approval, job)
            report.write_bytes(original)
            (repo / 'code').write_text('different code'); git('add', '.'); git('commit', '-qm', 'changed')
            self.assertIsNone(identical_tree_result(repo, run | {'head_sha': git('rev-parse', 'HEAD')}, config, approval, job))
