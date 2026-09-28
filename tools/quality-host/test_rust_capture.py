import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import fixed_workspace as fw
import rust_capture as rc


class RustCaptureTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.base = Path(self.tmp.name).resolve()
        self.repo = self.base / 'repo'
        self.repo.mkdir()
        (self.repo / 'Cargo.toml').write_text('[workspace]\n')
        fw.git(self.repo, 'init', '--quiet')
        fw.git(self.repo, 'config', 'user.name', 'Fixture')
        fw.git(self.repo, 'config', 'user.email', 'fixture@example.invalid')
        fw.git(self.repo, 'add', '.')
        fw.git(self.repo, 'commit', '--quiet', '-m', 'fixture')
        self.output = self.base / 'attempt'
        self.target = self.base / 'target'
        self.collector = self.base / 'collector'
        self.collector.mkdir()
        (self.collector / 'plugin.py').write_text('import json\nCOLLECTOR={"name":"fixture"}\nTYPES={"coverage.line"}\ncanonical=json.dumps\ndef inventory(q):return {}\ndef discover(q):return []\ndef series(q):return {"id":"fixture"}\n')
        self.args = SimpleNamespace(repository=self.repo, output=self.output, target_dir=self.target,
                                    collector=self.collector, manifest='Cargo.toml', source_root=['src'], test=[])
        self.tools = self.base / 'bin'
        self.tools.mkdir()
        for name in ('llvm-cov', 'llvm-profdata'):
            (self.tools / name).write_text('fixture')

    def native_output(self):
        self.target.mkdir(exist_ok=True)
        (self.target / 'subject').write_bytes(b'native object')
        (self.target / 'default.profraw').write_bytes(b'fresh counters')
        export = [str(self.tools / 'llvm-cov'), 'export', '-object', str(self.target / 'subject'),
                  '-instr-profile=' + str(self.target / 'merged.profdata')]
        (self.output / 'capture.stderr').write_text('Running `' + ' '.join(export) + '`\n')
        return export

    def test_capture_uses_existing_source_fresh_receipt_and_independent_artifacts(self):
        import sys
        fake_measure = SimpleNamespace(source_inventories=lambda *args: {})
        def execute(*args):
            self.native_output()
        with patch.dict(sys.modules, {'measure': fake_measure}), patch.object(rc, 'execute', side_effect=execute), patch.object(rc, 'tool_identity', return_value={}):
            bundle = rc.capture(self.args)
        receipt = bundle['request']['parameters']['receipt']
        self.assertEqual(receipt['coverage_root'], str(self.repo))
        self.assertEqual(receipt['pipeline']['capture'], 'cargo-llvm-cov-fixed-source/v1')
        self.assertEqual(receipt['inputs']['Cargo.toml'], rc.digest(self.repo / 'Cargo.toml'))
        self.assertFalse((self.output / 'workspace').exists())
        self.assertTrue((self.output / 'bundle.json').is_file())
        original = (self.output / 'raw/profiles-0-default.profraw').read_bytes()
        (self.target / 'default.profraw').write_bytes(b'next attempt')
        self.assertEqual((self.output / 'raw/profiles-0-default.profraw').read_bytes(), original)

    def test_source_mutation_never_emits_bundle(self):
        import sys
        fake_measure = SimpleNamespace(source_inventories=lambda *args: {})
        def execute(*args):
            self.native_output()
            (self.repo / 'Cargo.toml').write_text('changed')
        with patch.dict(sys.modules, {'measure': fake_measure}), patch.object(rc, 'execute', side_effect=execute):
            with self.assertRaisesRegex(ValueError, 'source changed'):
                rc.capture(self.args)
        self.assertFalse((self.output / 'bundle.json').exists())

    def test_arguments_and_command_reexecute_tests_without_reusing_profiles(self):
        values = rc.arguments(['--repository', str(self.repo), '--output', str(self.output),
                               '--target-dir', str(self.target), '--collector', str(self.collector),
                               '--source-root', 'src', '--test', 'one'])
        self.assertEqual(values.test, ['one'])
        self.output.mkdir()
        with patch.object(rc.subprocess, 'run') as run:
            rc.execute(values, self.repo, self.output, self.target)
        argv = run.call_args.args[0]
        self.assertIn('--locked', argv)
        self.assertEqual(argv[-2:], ['--test', 'one'])
        self.assertNotIn('--no-clean', argv)
        self.assertNotIn('--no-run', argv)
        self.assertEqual(run.call_args.kwargs['env']['CARGO_TARGET_DIR'], str(self.target))
        self.assertEqual(run.call_args.kwargs['cwd'], self.repo)

    def test_manifest_and_output_paths_are_checked(self):
        self.args.manifest = '../missing'
        with self.assertRaisesRegex(ValueError, 'manifest'):
            rc.prepare(self.args)
        self.args.manifest = 'Cargo.toml'
        self.args.output = self.repo / 'output'
        with self.assertRaisesRegex(ValueError, 'outside'):
            rc.prepare(self.args)
        self.assertFalse((self.repo / 'output').exists())

    def test_ambiguous_or_absent_export_is_rejected(self):
        self.output.mkdir()
        log = self.output / 'capture.stderr'
        for text in ('ordinary output\n', 'Running `/x/llvm-cov export a`\nRunning `/x/llvm-cov export b`\n'):
            log.write_text(text)
            with self.assertRaises(ValueError):
                rc.export_command(self.output)

    def test_native_artifacts_are_confined_and_required(self):
        self.output.mkdir()
        export = self.native_output()
        with self.assertRaises(ValueError):
            rc.artifact_groups([], self.target)
        with self.assertRaisesRegex(ValueError, 'profile escapes'):
            rc.artifact_groups(export[:-1] + ['-instr-profile=/outside.profdata'], self.target)
        with self.assertRaisesRegex(ValueError, 'empty'):
            rc.retain_artifacts({'objects': []}, self.output, self.target)
        (self.output / 'raw').rmdir()
        with self.assertRaisesRegex(ValueError, 'artifact escapes'):
            rc.retain_artifacts({'objects': [self.repo / 'Cargo.toml']}, self.output, self.target)

    def test_tool_identity_binds_native_tool_bytes_and_versions(self):
        with patch.object(rc.subprocess, 'check_output', return_value='version\n'):
            tools = rc.tool_identity([str(self.tools / 'llvm-cov')])
        self.assertEqual(tools['llvm-cov']['sha256'], rc.digest(self.tools / 'llvm-cov'))
        self.assertEqual(tools['rustc'], 'version')

    def test_native_inputs_keep_code_and_build_inputs_without_unrelated_document_names(self):
        names = ['Cargo.toml', 'Cargo.lock', 'apps/server/src/lib.rs', 'apps/server/tests/example.rs',
                 'migrations/one.sql', '.cargo/config.toml', 'src/lib.rs', 'spikes/s3/Cargo.toml', 'spikes/s3/Cargo.lock', 'docs/bad\\\\name.md']
        inputs = {name: {'sha256': 'fixture', 'mode': 0o644} for name in names}
        receipt = rc.make_receipt(self.args, self.repo, {'context': {}}, inputs, {},
                                  self.output, {}, {}, {})
        self.assertEqual(set(receipt['inputs']), set(names[:-3]))
        self.assertEqual(set(receipt['source_inputs']), set(names))


if __name__ == '__main__':
    unittest.main()
