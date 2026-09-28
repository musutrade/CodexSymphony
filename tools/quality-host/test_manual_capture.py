from contextlib import nullcontext
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import manual_capture as manual


class ManualCaptureTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(); self.addCleanup(self.tmp.cleanup)
        self.base = Path(self.tmp.name).resolve()
        self.root = self.base / 'source'; self.root.mkdir()
        self.output = self.base / 'run'; self.output.mkdir()
        self.slot = self.base / 'slot'; self.slot.mkdir()
        self.target = self.base / 'target'; self.target.mkdir()

    def test_backend_releases_database_on_success_and_capture_failure(self):
        for fail in (False, True):
            output = self.output / str(fail); output.mkdir()
            with patch.object(manual.layout, 'target', return_value=self.target), patch.object(manual.contract, 'load', return_value={'collectors': {'rust_source': 'fixture'}}), patch.object(manual.database_pool, 'acquire', return_value=('owned', 'fixture-url')), patch.object(manual.database_pool, 'release') as release, patch.object(manual, 'command', return_value=['fixture']) as command, patch.object(manual.host_capture, 'run_logged', side_effect=RuntimeError('capture failed') if fail else None):
                if fail:
                    with self.assertRaises(RuntimeError): manual.backend(output, self.root)
                else: manual.backend(output, self.root)
                release.assert_called_once_with('owned')
                self.assertEqual(command.call_args.kwargs['compiler_target'], self.target)

    def run_patches(self):
        values = [(manual.layout, 'lease', nullcontext()), (manual.layout, 'new_run', self.output),
                  (manual.layout, 'slot', self.slot), (manual.fixed_workspace, 'synchronize', (self.root, {'input': 'hash'})),
                  (manual.contract, 'load', {}), (manual.contract, 'test_environment', {}),
                  (manual.contract, 'tool_path', '/fixture'), (manual.contract, 'fingerprint', {'fingerprint': 'fixture'})]
        for owner, name, value in values:
            p = patch.object(owner, name, return_value=value); p.start(); self.addCleanup(p.stop)

    def test_capture_keeps_pending_marker_until_independent_measurement(self):
        self.run_patches()
        with patch.object(manual, 'backend'), patch.object(manual.fixed_workspace, 'sources', return_value={}):
            self.assertEqual(manual.run(self.root), self.output)
        self.assertEqual(json.loads((self.slot / 'pending-capture.json').read_text())['capture'], str(self.output))
        with self.assertRaisesRegex(ValueError, 'previous capture'):
            manual.run(self.root)

    def test_source_change_retains_pending_marker_for_recovery(self):
        self.run_patches()
        with patch.object(manual, 'backend'), patch.object(manual.fixed_workspace, 'sources', side_effect=[{}, {'changed': {}}]):
            with self.assertRaisesRegex(ValueError, 'source changed'):
                manual.run(self.root)
        self.assertTrue((self.slot / 'pending-capture.json').exists())

    def test_cli_uses_fixed_repository(self):
        with patch.object(manual.sys, 'argv', ['manual-capture']), patch.object(manual, 'run') as run:
            manual.main()
        run.assert_called_once_with(manual.layout.REPOSITORY)


if __name__ == '__main__': unittest.main()
