import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import operator_bridge as bridge
import product_identity_preflight as preflight


class OperatorBridgeTest(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.entry = {'issue_identifier': 'GH-42', 'recovery': {'revision': 1, 'external': {
            'operation': 'product.preflight', 'request_id': 'a01', 'evidence': 'artifact'}}}
        self.grant = {'issue': 'GH-42', 'revision': 1, 'operation': 'product.preflight',
                      'request_id': 'a01', 'request_sha256': bridge.digest(self.entry['recovery']['external'])}

    def test_grant_binds_whole_request_issue_and_revision(self):
        self.assertEqual(bridge.matching_grant(self.entry, [self.grant]), self.grant)
        for field, value in [('issue', 'GH-43'), ('revision', 2), ('request_sha256', 'bad'),
                             ('operation', 'deploy'), ('request_id', 'a02')]:
            self.assertIsNone(bridge.matching_grant(self.entry, [{**self.grant, field: value}]))

    def test_preapproved_profile_is_read_only_and_binds_the_resume_condition(self):
        request = self.entry['recovery']['external']
        request.update(operation='product.identity_preflight', repo='musutrade/CodexSymphony', resume_condition='identity only')
        profile = {'operation': request['operation'], 'repo': request['repo'], 'resume_condition': request['resume_condition']}
        grant = bridge.matching_grant(self.entry, [], [profile])
        self.assertEqual(grant['request_sha256'], bridge.digest(request))
        request['resume_condition'] = 'Runtime acceptance'
        self.assertIsNone(bridge.matching_grant(self.entry, [], [profile]))
        profile['operation'] = request['operation'] = 'product.deploy'
        self.assertIsNone(bridge.matching_grant(self.entry, [], [profile]))

    def test_no_grant_no_execution_and_failed_operation_never_resumes(self):
        def unexpected(*args):
            self.fail('unauthorized operation')
        self.assertEqual(bridge.process(self.entry, None, self.root, unexpected, unexpected), 'awaiting_authorization')
        self.assertEqual(bridge.process(self.entry, self.grant, self.root, unexpected, lambda _: 9), 'failed')
        self.assertEqual(bridge.process(self.entry, self.grant, self.root, unexpected, unexpected), 'failed')

    def test_lost_api_response_retries_completion_but_never_repeats_mutation(self):
        calls = []
        def complete(issue, body):
            calls.append(body)
            if len(calls) == 1:
                raise OSError('response lost')
            return {'revision': 2, 'status': 'implementing'}
        executions = []
        def execute(grant):
            executions.append(grant)
            return 0
        with self.assertRaises(OSError):
            bridge.process(self.entry, self.grant, self.root, complete, execute)
        self.assertEqual(bridge.process(self.entry, self.grant, self.root, complete, execute), 'acknowledged')
        self.assertEqual(len(executions), 1)
        self.assertEqual(calls[0], calls[1])
        self.assertEqual(bridge.process(self.entry, self.grant, self.root, complete, execute), 'acknowledged')

    def test_ambiguous_started_receipt_does_not_replay(self):
        def crash(_):
            raise KeyboardInterrupt()
        with self.assertRaises(KeyboardInterrupt):
            bridge.process(self.entry, self.grant, self.root, lambda *_: self.fail(), crash)
        self.assertEqual(bridge.process(self.entry, self.grant, self.root, lambda *_: self.fail(), lambda _: self.fail()), 'started')

    def test_preflight_waits_for_observation_refresh_but_has_a_hard_attempt_bound(self):
        with patch.object(preflight, 'check', side_effect=[RuntimeError('stale'), None]) as check:
            with patch.object(preflight.time, 'sleep') as pause:
                preflight.main()
                self.assertEqual(check.call_count, 2)
                pause.assert_called_once_with(5)
        with patch.object(preflight, 'check', side_effect=RuntimeError('unavailable')) as check:
            with patch.object(preflight.time, 'sleep'):
                with self.assertRaises(RuntimeError):
                    preflight.main()
                self.assertEqual(check.call_count, 4)

    def test_pinned_host_executable_and_timeout(self):
        script = self.root / 'check'
        script.write_text('#!/bin/sh\nexit 0\n')
        script.chmod(0o700)
        import hashlib
        grant = {'argv': [str(script)], 'executable_sha256': hashlib.sha256(script.read_bytes()).hexdigest()}
        self.assertEqual(bridge.run_operation(grant), 0)
        for bad in [{**grant, 'executable_sha256': 'changed'}, {**grant, 'argv': ['relative']},
                    {**grant, 'timeout_seconds': 999}, {**grant, 'argv': []}]:
            with self.assertRaises(ValueError):
                bridge.run_operation(bad)
        with patch.object(bridge.subprocess, 'run', side_effect=bridge.subprocess.TimeoutExpired('test', 1)):
            self.assertEqual(bridge.process(self.entry, grant, self.root / 'timeout', lambda *_: self.fail()), 'failed')


if __name__ == '__main__':
    unittest.main()
