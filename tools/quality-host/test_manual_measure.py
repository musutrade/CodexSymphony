from contextlib import nullcontext
import copy
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch
import manual_measure as mm


class ManualMeasureTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(); self.addCleanup(self.tmp.cleanup)
        self.base = Path(self.tmp.name).resolve()
        self.run = self.base / 'evidence/gate/run-fixture'
        self.raw = self.run / 'probes/backend/raw'; self.raw.mkdir(parents=True)
        self.slot = self.base / 'slot'; self.slot.mkdir()
        self.root = self.slot / 'workspace'; self.root.mkdir()
        self.repo = self.base / 'repo'; self.repo.mkdir()
        self.rule = {'required': True, 'on_violation': 'fail', 'operator': 'ge', 'metric': 'coverage.line',
                     'limit': {'covered': 80, 'total': 100}}
        self.function = {'source': 'source.rs', 'name': 'function', 'coverage.line': {'numerator': 1, 'denominator': 1}}
        (self.raw / 'object').write_bytes(b'object')
        self.receipt = {'raw_root': str(self.raw), 'raw': {'object': mm.rust_capture.digest(self.raw / 'object')}}
        self.request = {'workspace_root': str(self.root), 'parameters': {'receipt': self.receipt}}
        (self.run / 'probes/backend/bundle.json').write_text(json.dumps({'request': self.request}))
        policy = self.root / '.harness-gate/packs/backend/policy.json'; policy.parent.mkdir(parents=True)
        policy.write_text(json.dumps({'rules': [self.rule]}))
        self.pending = self.slot / 'pending-capture.json'
        self.pending.write_text(json.dumps({'capture': str(self.run), 'source_inputs': {'source': 'hash'}}))
        for name, value in [('VOLUME', self.base)]:
            p = patch.object(mm.layout, name, value); p.start(); self.addCleanup(p.stop)

    def test_required_exact_ratio_policy_and_missing_measurements(self):
        self.assertFalse(mm.fails({'numerator': 4, 'denominator': 5}, self.rule))
        self.assertTrue(mm.fails({'numerator': 79, 'denominator': 100}, self.rule))
        rule = dict(self.rule, operator='le', limit={'numerator': 10, 'denominator': 1})
        self.assertTrue(mm.fails({'numerator': 11, 'denominator': 1}, rule))
        self.assertFalse(mm.fails({'numerator': 10, 'denominator': 1}, rule))
        for value in (None, {'numerator': 0, 'denominator': 0}): self.assertTrue(mm.fails(value, self.rule))
        for change in ({'required': False}, {'on_violation': 'ignore'}, {'operator': 'unknown'}):
            with self.assertRaises(ValueError): mm.fails(None, dict(self.rule, **change))
        self.assertEqual(mm.violations([self.function], [self.rule]), [])
        self.function['coverage.line'] = None
        self.assertEqual(len(mm.violations([self.function], [self.rule])), 1)
        for functions, rules in (([], [self.rule]), ([self.function], [])):
            with self.assertRaises(ValueError): mm.violations(functions, rules)

    def test_independent_raw_inventory_rejects_missing_modified_or_aliased_data(self):
        mm.raw_inventory(self.run, self.request)
        self.assertTrue((self.run / 'independent-raw-inventory.json').is_file())
        other = copy.deepcopy(self.request); other['parameters']['receipt']['raw_root'] = '/wrong'
        with self.assertRaises(ValueError): mm.raw_inventory(self.run, other)
        (self.raw / 'object').write_bytes(b'changed')
        with self.assertRaises(ValueError): mm.raw_inventory(self.run, self.request)
        (self.raw / 'alias').symlink_to(self.raw / 'object')
        with self.assertRaises(ValueError): mm.raw_inventory(self.run, self.request)

    def test_retention_registration_checks_installed_files_and_identity(self):
        release = self.base / 'release'; release.mkdir()
        program = release / 'capture_cache_retention.py'; program.write_text('fixture')
        deployment = self.base / 'deployment.json'
        record = {'release': str(release), 'files': {str(program): mm.rust_capture.digest(program)}}
        deployment.write_text(json.dumps(record))
        with patch.object(mm, 'DEPLOYMENT', deployment), patch.object(mm.subprocess, 'check_output', return_value=json.dumps({'root': str(self.run)})):
            mm.register(self.run)
            self.assertTrue((self.run / 'capture-registration.json').exists())
        with patch.object(mm, 'DEPLOYMENT', deployment), patch.object(mm.subprocess, 'check_output', return_value=json.dumps({'root': '/wrong'})):
            with self.assertRaises(ValueError): mm.register(self.run)
        program.write_text('drift')
        with patch.object(mm, 'DEPLOYMENT', deployment):
            with self.assertRaisesRegex(ValueError, 'tool drift'): mm.register(self.run)
        record['files'] = {}; deployment.write_text(json.dumps(record))
        with patch.object(mm, 'DEPLOYMENT', deployment):
            with self.assertRaisesRegex(ValueError, 'lacks installed approval'): mm.register(self.run)

    def test_measure_binds_source_and_checks_policy(self):
        def reexport(request, path):
            path.write_text('{}')
            return {'functions': [self.function]}
        collector = SimpleNamespace(reexport=reexport)
        with patch.object(mm.rust_capture, 'load_collector', return_value=collector):
            result = mm.measure(self.run, self.root)
            self.assertEqual(result['coverage_and_crap'], 'PASS')
            self.function['coverage.line'] = None
            self.assertEqual(mm.measure(self.run, self.root)['coverage_and_crap'], 'FAIL')
        with self.assertRaisesRegex(ValueError, 'source identity differs'):
            mm.measure(self.run, self.repo)

    def finish_patches(self):
        for owner, name, value in [(mm.layout, 'lease', nullcontext()), (mm.layout, 'slot', self.slot),
                                   (mm.fixed_workspace, 'sources', {'source': {'sha256': 'hash', 'mode': 0o644}})]:
            p = patch.object(owner, name, return_value=value); p.start(); self.addCleanup(p.stop)

    def test_only_successful_measured_and_registered_capture_clears_pending(self):
        self.finish_patches()
        with patch.object(mm, 'measure', return_value={'coverage_and_crap': 'FAIL'}), patch.object(mm, 'register'):
            mm.finish(self.repo)
        self.assertTrue(self.pending.exists())
        with patch.object(mm, 'measure', return_value={'coverage_and_crap': 'PASS'}), patch.object(mm, 'register', side_effect=RuntimeError('retention failed')):
            with self.assertRaises(RuntimeError): mm.finish(self.repo)
        self.assertTrue(self.pending.exists())
        with patch.object(mm, 'measure', return_value={'coverage_and_crap': 'PASS'}), patch.object(mm, 'register'), patch('capture_handoff.complete', side_effect=lambda *args: self.pending.unlink()) as handoff:
            mm.finish(self.repo)
            self.assertEqual(handoff.call_args.args[:3], (self.run, self.root, self.pending))
        self.assertFalse(self.pending.exists())

    def test_pending_paths_and_source_changes_block_completion(self):
        self.finish_patches()
        with patch.object(mm.fixed_workspace, 'sources', side_effect=[{}, {'different': {}}]):
            with self.assertRaisesRegex(ValueError, 'source differ'): mm.finish(self.repo)
        with patch.object(mm.fixed_workspace, 'sources', return_value={}):
            with self.assertRaisesRegex(ValueError, 'inputs changed'): mm.finish(self.repo)
        self.pending.write_text(json.dumps({'capture': str(self.base / 'wrong')}))
        with self.assertRaisesRegex(ValueError, 'outside bounded evidence'): mm.finish(self.repo)
        self.pending.unlink(); self.pending.symlink_to(self.run / 'probes/backend/bundle.json')
        with self.assertRaisesRegex(ValueError, 'symlink pending'): mm.finish(self.repo)

    def test_temporary_directory_restored_after_failure_and_cli_status(self):
        previous = tempfile.tempdir
        with self.assertRaises(RuntimeError):
            with mm.bounded_temporary():
                self.assertEqual(tempfile.tempdir, str(self.base / 'tmp'))
                raise RuntimeError('fixture')
        self.assertEqual(tempfile.tempdir, previous)
        import sys
        with patch.object(sys, 'argv', ['measure']), patch.object(mm, 'finish', return_value={'coverage_and_crap': 'PASS'}):
            with self.assertRaises(SystemExit) as exit:
                mm.main()
            self.assertEqual(exit.exception.code, False)


if __name__ == '__main__': unittest.main()
