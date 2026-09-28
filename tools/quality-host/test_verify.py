from contextlib import nullcontext
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import verify


class VerifyTests(unittest.TestCase):
    def test_fixed_target_and_database_cleanup_cover_success_failure_and_exception(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory).resolve()
            source = base / 'source'; source.mkdir()
            root = base / 'validation'; (root / 'web/angular').mkdir(parents=True)
            target = base / 'target'; target.mkdir()
            for outcome in (0, 1, RuntimeError('verification failed')):
                run = base / str(outcome); run.mkdir()
                (run / 'requests.json').write_text(json.dumps({'frontend-api': {'parameters': {'receipt': {'baseline': {'path': '/fixture-baseline'}}}}}))
                if outcome == 0: (run / 'test-capture.json').write_text('{}')
                with patch.object(verify.layout, 'ensure') as ensure, patch.object(verify.layout, 'target', return_value=target), patch.object(verify.shutil, 'which', return_value='/fixture-bin/collector'), patch.object(verify.database_pool, 'acquire', return_value=('owned-slot', 'fixture-url')), patch.object(verify.database_pool, 'release') as release, patch.object(verify, 'command', return_value=['fixture-command']) as command, patch.object(verify, 'broker', return_value=nullcontext()), patch.object(verify.subprocess, 'run', side_effect=outcome if isinstance(outcome, Exception) else None, return_value=SimpleNamespace(returncode=outcome)):
                    if isinstance(outcome, Exception):
                        with self.assertRaises(RuntimeError): verify.verify(run, source, root)
                    else:
                        self.assertEqual(verify.verify(run, source, root), outcome)
                        self.assertEqual(json.loads((run / 'verify-result.json').read_text())['exit'], outcome)
                    ensure.assert_called_once_with(source)
                    release.assert_called_once_with('owned-slot')
                    kwargs = command.call_args.kwargs
                    self.assertEqual(kwargs['compiler_target'], target)
                    self.assertIn(target, kwargs['writable'])
                    self.assertNotIn(run / 'target', kwargs['writable'])
                    self.assertEqual(kwargs['environment']['TEST_DATABASE_URL'], 'fixture-url')
                    self.assertEqual('HARNESS_GATE_TEST_RECEIPT' in kwargs['environment'], outcome == 0)

    def test_unmounted_or_other_repository_fails_before_database_acquisition(self):
        with patch.object(verify.layout, 'ensure', side_effect=ValueError('invalid repository')), patch.object(verify.database_pool, 'acquire') as acquire:
            with self.assertRaises(ValueError): verify.verify(Path('/run'), Path('/other'), Path('/root'))
            acquire.assert_not_called()


if __name__ == '__main__': unittest.main()
