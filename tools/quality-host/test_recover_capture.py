from contextlib import nullcontext
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import recover_capture as recovery


class RecoveryTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(); self.addCleanup(self.tmp.cleanup)
        self.volume = Path(self.tmp.name).resolve()
        self.slot = self.volume / 'validation/gate'; self.slot.mkdir(parents=True)
        self.root = self.slot / 'workspace'; self.root.mkdir()
        self.run = self.volume / 'evidence/gate/run-fixture'; self.run.mkdir(parents=True)
        self.output = self.run / 'probes/backend'; self.output.mkdir(parents=True)
        self.target = self.volume / 'target'; self.target.mkdir()
        self.raw = self.output / 'raw'; self.raw.mkdir()
        self.inputs = {'Cargo.toml': {'sha256': 'fixture', 'mode': 0o644}}
        self.pending = self.slot / 'pending-capture.json'
        self.pending.write_text(json.dumps({'capture': str(self.run), 'source_inputs': {'Cargo.toml': 'fixture'}}))
        for owner, name, value in [(recovery.layout, 'VOLUME', self.volume),
                                   (recovery.layout, 'slot', lambda: self.slot),
                                   (recovery.layout, 'target', lambda kind: self.target)]:
            p = patch.object(owner, name, value); p.start(); self.addCleanup(p.stop)

    def native(self):
        (self.target / 'object').write_bytes(b'native')
        (self.target / 'one.profraw').write_bytes(b'counters')
        (self.raw / 'objects-0-object').write_bytes(b'native')
        (self.raw / 'profiles-0-one.profraw').write_bytes(b'counters')
        (self.output / 'capture.stderr').write_text('Running `/llvm-cov export -object ' + str(self.target / 'object') + ' -instr-profile=' + str(self.target / 'merged.profdata') + '`\n')

    def test_pending_checks_frozen_source_and_refuses_existing_bundle(self):
        with patch.object(recovery.workspace, 'sources', return_value=self.inputs):
            self.assertEqual(recovery.pending_capture(), (self.run, self.root, self.inputs))
            (self.output / 'bundle.json').write_text('{}')
            with self.assertRaisesRegex(ValueError, 'already exists'): recovery.pending_capture()
            (self.output / 'bundle.json').unlink()
            self.pending.write_text(json.dumps({'capture': str(self.run), 'source_inputs': {}}))
            with self.assertRaisesRegex(ValueError, 'source changed'): recovery.pending_capture()
            self.pending.write_text(json.dumps({'capture': str(self.volume), 'source_inputs': {}}))
            with self.assertRaisesRegex(ValueError, 'outside'): recovery.pending_capture()
        self.pending.unlink(); self.pending.symlink_to(self.run / 'absent')
        with self.assertRaisesRegex(ValueError, 'symlink'): recovery.pending_capture()

    def test_retained_artifacts_match_independent_native_inputs(self):
        self.native()
        _, raw, hashes, names = recovery.retained_artifacts(self.output, self.target)
        self.assertEqual(raw, self.raw)
        self.assertEqual(set(hashes), {'objects-0-object', 'profiles-0-one.profraw'})
        self.assertEqual(names['objects'], ['objects-0-object'])
        (self.raw / 'profiles-0-one.profraw').write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError, 'artifact changed'):
            recovery.retained_artifacts(self.output, self.target)
        (self.raw / 'profiles-0-one.profraw').write_bytes(b'counters')
        (self.raw / 'extra').write_bytes(b'extra')
        with self.assertRaisesRegex(ValueError, 'inventory differs'):
            recovery.retained_artifacts(self.output, self.target)
        with patch.object(recovery.capture, 'artifact_groups', return_value={'objects': []}):
            with self.assertRaisesRegex(ValueError, 'empty'): recovery.retained_artifacts(self.output, self.target)
        with patch.object(recovery.capture, 'artifact_groups', return_value={'objects': [self.pending]}):
            with self.assertRaisesRegex(ValueError, 'outside'): recovery.retained_artifacts(self.output, self.target)

    def test_environment_drift_rejects_recovery(self):
        (self.run / 'environment.json').write_text('{}')
        contract = recovery.manual_capture.contract
        with patch.object(contract, 'load', return_value={}), patch.object(contract, 'test_environment', return_value={}), patch.object(contract, 'tool_path', return_value='/fixture'), patch.object(contract, 'fingerprint', return_value={}):
            self.assertEqual(recovery.environment(self.run, self.root), {})
            (self.run / 'environment.json').write_text('{"changed":true}')
            with self.assertRaisesRegex(ValueError, 'environment changed'): recovery.environment(self.run, self.root)

    def test_packaging_records_old_producer_and_recovery_without_repeating_native_tests(self):
        self.native()
        host = self.root / 'tools/quality-host'; host.mkdir(parents=True)
        (host / 'rust_capture.py').write_text('original producer')
        (host / 'fixed_workspace.py').write_text('original workspace helper')
        collector = SimpleNamespace(inventory=lambda q: {}, discover=lambda q: [], series=lambda q: {'id': 'fixture'}, canonical=json.dumps)
        with patch.object(recovery.capture, 'load_collector', return_value=collector), patch.object(recovery.capture, 'request_base', return_value={'context': {}, 'parameters': {}}), patch.object(recovery.capture, 'tool_identity', return_value={}), patch.object(recovery.capture, 'execute') as execute:
            bundle = recovery.package(self.run, self.root, self.inputs, {'collectors': {'rust_source': 'fixture'}})
            execute.assert_not_called()
            receipt = bundle['request']['parameters']['receipt']
            self.assertFalse(receipt['recovery']['native_tests_repeated'])
            self.assertEqual(receipt['pipeline']['tools']['capture_host']['sha256'], recovery.capture.digest(host / 'rust_capture.py'))
            with self.assertRaises(FileExistsError):
                recovery.package(self.run, self.root, self.inputs, {'collectors': {'rust_source': 'fixture'}})

    def test_recovery_registers_complete_measurement_and_retains_pending(self):
        with patch.object(recovery.layout, 'lease', return_value=nullcontext()), patch.object(recovery, 'pending_capture', return_value=(self.run, self.root, self.inputs)), patch.object(recovery, 'environment', return_value={}), patch.object(recovery, 'package') as package, patch.object(recovery.manual_measure, 'measure', return_value={'coverage_and_crap': 'PASS'}), patch.object(recovery.manual_measure, 'register') as register, patch('capture_handoff.complete') as handoff:
            result = recovery.recover()
            package.assert_called_once_with(self.run, self.root, self.inputs, {})
            register.assert_called_once_with(self.run)
            self.assertEqual(result['coverage_and_crap'], 'PASS')
            self.assertIn('not current', result['scope'])
            handoff.assert_called_once_with(self.run, self.root, self.pending, result)
        self.assertTrue(self.pending.exists())


if __name__ == '__main__': unittest.main()
