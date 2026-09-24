import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import build_cache as cache
import test_receipt as capture


class PrivateCache(unittest.TestCase):
    def test_workspace_outputs_are_excluded_before_copying(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary); repo = root / 'repo'; repo.mkdir()
            (repo / 'Cargo.toml').write_text('[package]\nname="project-code"\n[lib]\nname="custom_lib"\n[[test]]\nname="declared_case"\n')
            (repo / 'tests').mkdir(); (repo / 'tests/automatic.rs').write_text('test source')
            (repo / 'src/bin/tool').mkdir(parents=True); (repo / 'src/bin/tool/main.rs').write_text('fn main() {}')
            with patch.object(cache.subprocess, 'check_output', return_value='Cargo.toml\0tests/automatic.rs\0'):
                packages, targets = cache.workspace_outputs(repo)
            self.assertEqual(packages, {'project-code'})
            self.assertTrue({'custom_lib', 'declared_case', 'automatic', 'tool'} <= targets)
            source = root / 'target'; source.mkdir()
            excluded = ['debug/project-code', 'debug/project-code.d', 'debug/deps/libcustom_lib-hash.rlib',
                        'debug/deps/automatic-hash', 'debug/deps/declared_case-hash.d', 'debug/deps/tool-hash',
                        'debug/.fingerprint/project-code-hash/state', 'debug/build/project-code-hash/out/data',
                        'llvm-cov-target/debug/deps/project_code-hash', 'llvm-cov-target/debug/deps/automatic-hash',
                        'llvm-cov-target/debug/incremental/data', 'llvm-cov-target/old.profraw']
            retained = ['debug/deps/libserde-hash.rlib', 'debug/.fingerprint/serde-hash/state',
                        'debug/build/serde-hash/out/private.rs', 'llvm-cov-target/debug/deps/libserde-hash.rlib']
            for name in excluded + retained:
                p = source / name; p.parent.mkdir(parents=True, exist_ok=True); p.write_text('bytes')
            with patch.object(cache.shutil, 'copy2', wraps=cache.shutil.copy2) as copied:
                cache.copy_dependencies(source, root / 'cache', packages, targets)
                self.assertEqual(copied.call_count, len(retained))
            for name in excluded: self.assertFalse((root / 'cache' / name).exists(), name)
            for name in retained: self.assertEqual((root / 'cache' / name).read_text(), 'bytes')

    def test_main_seed_is_private_and_counters_never_cross_runs(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / 'source'; source.mkdir()
            (source / 'library.rlib').write_bytes(b'compiled')
            (source / 'sccache').mkdir()
            (source / 'sccache/content-key').write_bytes(b'compiler result')
            (source / 'old.profraw').write_bytes(b'old counter')
            (source / 'incremental').mkdir()
            (source / 'incremental/large').write_bytes(b'not retained')
            key = 'a' * 64
            result = cache.publish(root / 'cache', key, source, 'main-sha', max_bytes=4096)
            self.assertTrue(result['published'])
            self.assertTrue(cache.restore(root / 'cache', key, root / 'pr')['hit'])
            self.assertFalse((root / 'pr/old.profraw').exists())
            self.assertFalse((root / 'pr/incremental').exists())
            (root / 'pr/sccache/content-key').write_bytes(b'private compiler mutation')
            self.assertEqual((root / 'cache' / key / 'target/sccache/content-key').read_bytes(),b'compiler result')
            (root / 'pr/library.rlib').write_bytes(b'PR mutation')
            self.assertEqual((root / 'cache' / key / 'target/library.rlib').read_bytes(), b'compiled')
            self.assertNotEqual((root / 'pr/library.rlib').stat().st_ino,
                                (root / 'cache' / key / 'target/library.rlib').stat().st_ino)

    def test_capacity_ttl_and_unsafe_entries(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary); source = root / 'source'; source.mkdir()
            (source / 'binary').write_bytes(b'x' * 100)
            self.assertFalse(cache.publish(root / 'cache', 'a' * 64, source, 'sha', max_bytes=1)['published'])
            self.assertFalse(cache.publish(root / 'cache', 'a' * 64, source, 'sha', max_bytes=100)['published'])
            cache.publish(root / 'cache', 'b' * 64, source, 'sha', max_bytes=4096)
            os.utime(root / 'cache' / ('b' * 64), (0, 0))
            self.assertFalse(cache.restore(root / 'cache', 'b' * 64, root / 'restored', ttl=1)['hit'])
            (source / 'unsafe').symlink_to(root)
            with self.assertRaises(ValueError):
                cache.publish(root / 'cache', 'c' * 64, source, 'sha')
            self.assertEqual(list((root / 'cache').glob('.pending-*')), [])


class CapturedTests(unittest.TestCase):
    def fixture(self, root):
        repo = root / 'repo'; repo.mkdir()
        (repo / 'apps/server').mkdir(parents=True)
        (repo / 'migrations').mkdir()
        (repo / 'Cargo.toml').write_text('[workspace]\nmembers = ["apps/server"]\n')
        (repo / 'Cargo.lock').write_text('locked')
        (repo / 'apps/server/Cargo.toml').write_text('[package]\nname = "fixture"\n')
        run = root / 'run-fixture'; backend = run / 'probes/backend'; backend.mkdir(parents=True)
        (backend / 'capture.stdout').write_text('running 1 test\ntest result: ok. 1 passed; 0 failed;\n')
        pipeline = {'capture': 'cargo-llvm-cov-locked/v1', 'tests': [], 'manifest': 'apps/server/Cargo.toml'}
        request = {'context': {'commit': 'a' * 40}, 'parameters': {
            'receipt': {'pipeline': pipeline, 'inputs': capture.backend_inputs(repo)}}}
        (backend / 'bundle.json').write_text(json.dumps({'request': request}))
        context = {'commit': 'a' * 40, 'run': run.name}
        return repo, run, context

    def test_same_capture_reused_but_source_log_commit_and_partial_selection_rejected(self):
        for change in ('none', 'source', 'log', 'commit', 'partial', 'run', 'exit'):
            with self.subTest(change=change), tempfile.TemporaryDirectory() as temporary:
                repo, run, context = self.fixture(Path(temporary))
                capture.seal(run, repo, context)
                receipt = run / 'test-capture.json'
                value = json.loads(receipt.read_text())
                if change == 'source': (repo / 'apps/server/new.rs').write_text('new code')
                if change == 'log': (run / 'probes/backend/capture.stdout').write_text('forged')
                if change == 'commit': value['context']['commit'] = 'b' * 40
                if change == 'partial': value['pipeline']['tests'] = ['one-test']
                if change == 'run': value['context']['run'] = 'another-run'
                if change == 'exit': value['exit_code'] = 1
                receipt.write_text(json.dumps(value))
                with patch.object(capture.subprocess, 'check_output', return_value='a' * 40 + '\n'):
                    if change == 'none':
                        self.assertIn(b'1 passed', capture.verified_log(receipt, repo))
                    else:
                        with self.assertRaises(ValueError): capture.verified_log(receipt, repo)

    def test_expanded_workspace_falls_back_and_partial_capture_never_gets_sealed(self):
        with tempfile.TemporaryDirectory() as temporary:
            repo, run, context = self.fixture(Path(temporary))
            path = run / 'probes/backend/bundle.json'
            value = json.loads(path.read_text())
            value['request']['parameters']['receipt']['pipeline']['tests'] = ['subset']
            path.write_text(json.dumps(value))
            with self.assertRaises(ValueError): capture.seal(run, repo, context)
            self.assertFalse((run / 'test-capture.json').exists())
            (repo / 'Cargo.toml').write_text('[workspace]\nmembers = ["apps/server", "new-crate"]\n')
            capture.seal(run, repo, context)
            self.assertFalse((run / 'test-capture.json').exists())
