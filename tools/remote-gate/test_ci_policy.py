import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

import host
from ci_policy import documentation_changes, documentation_result, policy_identity, run_cancellable, SupersededRun


class DocumentationScope(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name) / 'repo'
        self.root.mkdir()
        self.git('init', '-q')
        self.git('config', 'user.email', 'fixture@example.invalid')
        self.git('config', 'user.name', 'Fixture')
        (self.root / 'README.md').write_text('# Example\n')
        (self.root / 'code.rs').write_text('fn main() {}\n')
        self.base = self.commit()
        self.config = {'repository': 'owner/repo', 'protected_files': {'workflow': 'digest'},
                       'state_root': self.tmp.name}
        self.approval = {'trusted_files': {'host': 'digest'}}
        self.receipt = Path(self.tmp.name) / 'jobs/1-1/receipt.json'
        self.receipt.parent.mkdir(parents=True)
        self.prior = {'finished': True, 'status': 'PASS', 'scope': 'full',
                      'source_sha': self.base, 'identity': '1/1', 'report_sha256': 'digest',
                      'policy_identity': policy_identity(self.config, self.approval)}
        self.receipt.write_text(json.dumps(self.prior))
        self.job = Path(self.tmp.name) / 'jobs/2-1'
        self.job.mkdir()

    def git(self, *args):
        return subprocess.check_output(['git', '-C', self.root, *args], stderr=subprocess.PIPE).decode().strip()

    def commit(self):
        self.git('add', '-A')
        self.git('commit', '-qm', 'fixture')
        return self.git('rev-parse', 'HEAD')

    def doc_head(self):
        (self.root / 'README.md').write_text('# Example\n\nClarification.\n')
        return self.commit()

    def result(self, head, event='pull_request'):
        return documentation_result(self.root, {'event': event, 'head_sha': head, 'id': 2, 'run_attempt': 1},
                                    self.config, self.approval, self.job)

    def test_doc_receipt_has_exact_identity_and_explicit_scope(self):
        head = self.doc_head()
        result = self.result(head)
        self.assertEqual(result['source_sha'], head)
        self.assertEqual(result['identity'], '2/1')
        self.assertEqual(result['baseline_sha'], self.base)
        self.assertFalse(result['full_suite_executed'])
        self.assertEqual(result['changed_paths'], ['README.md'])
        self.assertIsNotNone(self.result(head, 'push'))
        self.assertIsNone(self.result(head, 'workflow_dispatch'))

    def test_no_baseline_or_changed_policy_or_docs_baseline_requires_full(self):
        head = self.doc_head()
        for change in ({'finished': False}, {'status': 'FAIL'}, {'scope': 'documentation'},
                       {'policy_identity': 'old'}, {'source_sha': '0' * 40}):
            with self.subTest(change=change):
                self.receipt.write_text(json.dumps(self.prior | change))
                self.assertIsNone(self.result(head))
        self.receipt.unlink()
        self.assertIsNone(self.result(head))

    def test_code_and_unknown_markdown_require_full(self):
        for name in ('code.rs', 'WORKFLOW.lifecycle.md', 'AGENTS.md', 'docs/unreviewed.md'):
            with self.subTest(name=name):
                self.git('reset', '--hard', self.base)
                path = self.root / name
                path.parent.mkdir(exist_ok=True)
                path.write_text('changed\n')
                self.assertIsNone(self.result(self.commit()))

    def test_deleted_renamed_executable_and_symlink_docs_require_full(self):
        for kind in ('deleted', 'renamed', 'executable', 'symlink'):
            with self.subTest(kind=kind):
                self.git('reset', '--hard', self.base)
                path = self.root / 'README.md'
                if kind == 'deleted': path.unlink()
                elif kind == 'renamed': path.rename(self.root / 'guide.md')
                elif kind == 'executable': path.chmod(0o755)
                else:
                    path.unlink()
                    path.symlink_to('code.rs')
                self.assertIsNone(self.result(self.commit()))

    def test_invalid_docs_reject_instead_of_pass(self):
        for data in (b'<<<<<<< unresolved\n', b'null\x00\n', b'\xff', b'x' * (2 * 1024 * 1024 + 1), b'trailing space \n'):
            with self.subTest(size=len(data)):
                (self.root / 'README.md').write_bytes(data)
                head = self.commit()
                with self.assertRaises((ValueError, subprocess.CalledProcessError)):
                    self.result(head)

    def test_nonancestor_success_cannot_authorize_docs(self):
        self.doc_head()
        other = self.git('rev-parse', 'HEAD')
        self.git('reset', '--hard', self.base)
        (self.root / 'README.md').write_text('different branch\n')
        head = self.commit()
        self.assertIsNone(documentation_changes(self.root, other, head))


class Cancellation(unittest.TestCase):
    def test_stale_attempt_and_completed_actions_are_cancelled(self):
        run = {'id': 1, 'event': 'pull_request', 'run_attempt': 1}
        with patch.object(host, 'installation_token', return_value='fixture'), patch.object(host, 'request') as request:
            for value, expected in (({'run_attempt': 1, 'status': 'in_progress'}, False),
                                    ({'run_attempt': 2, 'status': 'in_progress'}, True),
                                    ({'run_attempt': 1, 'status': 'completed'}, True)):
                request.return_value = value
                self.assertEqual(host.actions_cancelled(run, {'repository': 'owner/repo'}), expected)
            request.reset_mock()
            self.assertFalse(host.actions_cancelled(run | {'event': 'push'}, {}))
            self.assertFalse(host.actions_cancelled(run | {'event': 'workflow_dispatch'}, {}))
            request.assert_not_called()

    def test_cancelled_queue_does_not_clone_or_create_check(self):
        run = {'repository': {'full_name': 'owner/repo'}, 'head_repository': {'full_name': 'owner/repo'},
               'path': '.github/workflows/quality.yml', 'event': 'pull_request', 'head_sha': 'a'*40,
               'id': 1, 'run_attempt': 1}
        with patch.object(host, 'actions_cancelled', return_value=True), patch.object(host, 'evaluate') as evaluate:
            host.process(run, {'repository': 'owner/repo'}, Path('/unused'))
            evaluate.assert_not_called()

    def test_documentation_does_not_prepare_dependencies_or_launch_full_gate(self):
        with patch.object(host, 'prepare', return_value=(Path('/source'), {})), \
             patch.object(host, 'actions_cancelled', return_value=False), \
             patch.object(host, 'documentation_result', return_value={'scope': 'documentation'}) as docs, \
             patch.object(host, 'prepare_dependencies') as deps, \
             patch.object(host, 'run_cancellable') as launch:
            self.assertEqual(host.evaluate({}, {}, Path('/job')), {'scope': 'documentation'})
            docs.assert_called_once()
            deps.assert_not_called()
            launch.assert_not_called()

    def test_midrun_cancellation_publishes_cancelled_not_success(self):
        run = {'repository': {'full_name': 'owner/repo'}, 'head_repository': {'full_name': 'owner/repo'},
               'path': '.github/workflows/quality.yml', 'event': 'pull_request', 'head_sha': 'a'*40,
               'id': 1, 'run_attempt': 1, 'html_url': 'https://example.invalid'}
        with tempfile.TemporaryDirectory() as tmp, \
             patch.object(host, 'actions_cancelled', return_value=False), \
             patch.object(host, 'installation_token', return_value='fixture'), \
             patch.object(host, 'request', return_value={'id': 9}) as request, \
             patch.object(host, 'evaluate', side_effect=SupersededRun('cancelled')):
            host.process(run, {'repository': 'owner/repo'}, Path(tmp))
            self.assertEqual(request.call_args.args[3]['conclusion'], 'cancelled')
            receipt = json.loads((Path(tmp)/'jobs/1-1/receipt.json').read_text())
            self.assertEqual(receipt['status'], 'CANCELLED')
            self.assertTrue(receipt['finished'])

    def test_process_exit_timeout_and_api_error(self):
        command = [sys.executable, '-c', 'import time; time.sleep(30)']
        self.assertEqual(run_cancellable([sys.executable, '-c', 'raise SystemExit(7)'],
                                        subprocess.DEVNULL, subprocess.DEVNULL, lambda: False, interval=.01), 7)
        with self.assertRaises(subprocess.TimeoutExpired):
            run_cancellable(command, subprocess.DEVNULL, subprocess.DEVNULL, lambda: False, timeout=.05, interval=.01)
        with self.assertRaisesRegex(RuntimeError, 'API failed'):
            run_cancellable(command, subprocess.DEVNULL, subprocess.DEVNULL,
                            lambda: (_ for _ in ()).throw(RuntimeError('API failed')))

    def test_cancellation_stops_child_process_group(self):
        with tempfile.TemporaryDirectory() as tmp:
            pidfile = Path(tmp) / 'child'
            code = ('import subprocess,time,pathlib; '
                    'p=subprocess.Popen(["sleep","30"]); '
                    f'pathlib.Path({str(pidfile)!r}).write_text(str(p.pid)); time.sleep(30)')
            with self.assertRaises(SupersededRun):
                run_cancellable([sys.executable, '-c', code], subprocess.DEVNULL, subprocess.DEVNULL,
                                pidfile.exists, interval=.01)
            pid = int(pidfile.read_text())
            for _ in range(100):
                stat = Path(f'/proc/{pid}/stat')
                if not stat.exists() or stat.read_text().split()[2] == 'Z': break
                time.sleep(.01)
            else: self.fail('child process survived cancellation')


if __name__ == '__main__':
    unittest.main()
