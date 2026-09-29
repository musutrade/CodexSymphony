"""Signed execution budgets and private-key cleanup at the provisioning boundary."""
import base64
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import signing


class ProvisionTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name)
        self.root = self.base / 'repository'
        self.run = self.base / 'run'
        self.keys = self.base / 'private'
        self.run.mkdir()
        self.runtime = self.root / '.harness-gate/runtime'
        self.runtime.mkdir(parents=True)
        names = ['.harness-gate/flow.toml', '.harness-gate/quality.toml']
        names += [f'.harness-gate/packs/{name}/policy.json'
                  for name in ('backend', 'frontend', 'frontend-api')]
        for name in names:
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text('{}')
        self.config = signing.configuration_files(self.root)
        self.state = {'profile': 'ci', 'expected': {'run': 'run'}, 'series': {}}
        self.requests = {}
        for name in ('backend', 'frontend', 'frontend-api'):
            self.state['series'][name] = {'id': name + '-series'}
            self.requests[name] = {
                'collector': {'name': name, 'version': 'fixture'},
                'project': 'fixture-project', 'context': {'commit': 'fixture-source'},
                'output_root': str(self.run / name),
                'parameters': {'subjects': [{'id': 'second'}, {'id': 'first'}]},
                'requested_capabilities': ['risk.crap', 'coverage.line'],
            }
        self.compiled = {name: {} for name in
                         ('project', 'policy', 'expected', 'selection', 'mappings', 'exceptions')}
        for name, value in [('run_logged', self.compile), ('runtime_launcher', self.launcher)]:
            replacement = patch.object(signing, name, value)
            replacement.start()
            self.addCleanup(replacement.stop)

    def compile(self, run, label, argv):
        self.assertEqual(label, 'compile')
        Path(argv[-1]).write_text(json.dumps(self.compiled))

    def launcher(self, directory, collector):
        path = directory / (collector + '-collector')
        path.write_text('#!/bin/false\n')
        return path

    def provision(self):
        return signing.provision(self.run, self.root, self.state, self.requests,
                                 self.config, self.keys)

    def test_each_budget_is_signed_with_fresh_nonce_and_final_state_pins(self):
        state_path = self.provision()
        keys = json.loads((self.runtime / 'trusted-keys.json').read_text())
        public = self.base / 'public.der'
        # RFC 8410 SubjectPublicKeyInfo prefix for Ed25519.
        public.write_bytes(bytes.fromhex('302a300506032b6570032100') +
                           base64.b64decode(keys[0]['public_key']))
        nonces = set()
        expected_budgets = {'backend': 300000, 'frontend': 120000, 'frontend-api': 120000}
        final = json.loads(state_path.read_text())
        for name, timeout in expected_budgets.items():
            request_path = self.runtime / (name + '-request.json')
            request = json.loads(request_path.read_text())
            payload = self.run / 'signed' / (name + '-sign-input.json')
            signed = json.loads(payload.read_text())
            self.assertEqual(request['timeout_ms'], timeout)
            self.assertEqual(signed['timeout_ms'], timeout)
            self.assertEqual(request['expires_at_ms'] - request['issued_at_ms'], 900000)
            self.assertEqual(request['input']['context'], {'commit': 'fixture-source'})
            self.assertEqual(request['invocation_id'], 'run')
            nonces.add(request['nonce'])
            relative = '.harness-gate/runtime/' + request_path.name
            self.assertEqual(final['config_files'][relative], signing.sha(request_path.read_bytes()))
            signature = self.base / (name + '.sig')
            signature.write_bytes(base64.b64decode(request['adapter']['signature']['value']))
            argv = ['openssl', 'pkeyutl', '-verify', '-rawin', '-pubin', '-keyform', 'DER',
                    '-inkey', str(public), '-sigfile', str(signature), '-in', str(payload)]
            subprocess.run(argv, check=True, capture_output=True)
            signed['timeout_ms'] += 1
            payload.write_text(json.dumps(signed, separators=(',', ':'), ensure_ascii=False))
            self.assertNotEqual(subprocess.run(argv, capture_output=True).returncode, 0)
        self.assertEqual(len(nonces), 3)
        self.assertFalse((self.keys / 'run.pem').exists())

    def test_policy_drift_fails_before_key_creation(self):
        (self.root / '.harness-gate/quality.toml').write_text('changed')
        with self.assertRaisesRegex(ValueError, 'host-approved policy'):
            self.provision()
        self.assertFalse(self.keys.exists())

    def test_signing_error_removes_private_key(self):
        actual = subprocess.check_output

        def fail_signature(argv, **kwargs):
            if argv[1] == 'pkeyutl':
                raise subprocess.CalledProcessError(1, argv)
            return actual(argv, **kwargs)

        with patch.object(signing.subprocess, 'check_output', side_effect=fail_signature):
            with self.assertRaises(subprocess.CalledProcessError):
                self.provision()
        self.assertFalse((self.keys / 'run.pem').exists())

    def test_key_generation_error_restores_umask(self):
        previous = os.umask(0o027)
        try:
            with patch.object(signing.subprocess, 'run', side_effect=OSError('key generation failed')):
                with self.assertRaisesRegex(OSError, 'key generation failed'):
                    self.provision()
            current = os.umask(0o027)
            self.assertEqual(current, 0o027)
        finally:
            os.umask(previous)
