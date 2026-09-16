import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock, patch

spec=importlib.util.spec_from_file_location('runtime_smoke',Path(__file__).parent/'environment/client/runtime_smoke.py')
smoke=importlib.util.module_from_spec(spec);spec.loader.exec_module(smoke)


class RuntimePreparation(unittest.TestCase):
    def test_host_execution_is_rejected_before_launch(self):
        with patch.object(smoke.os,'getuid',return_value=1000), patch.object(smoke.Path,'exists',return_value=True):
            with self.assertRaisesRegex(ValueError,'assigned Agent sandbox'):
                smoke.isolation(Path('/fixture'))

    def test_writable_git_is_rejected_and_canary_removed(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);(root/'.git').mkdir()
            with patch.object(smoke.os,'getuid',return_value=1000), patch.object(smoke.Path,'exists',return_value=False):
                with self.assertRaisesRegex(ValueError,'Git metadata unexpectedly writable'):
                    smoke.isolation(root)
            self.assertEqual(list((root/'.git').iterdir()),[])

    def test_wrong_version_rejected_before_creating_state(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);(root/'codex-version.lock').write_text('codex-cli 0.154.0')
            with patch.object(smoke,'isolation',return_value={}), patch.object(smoke.subprocess,'check_output',return_value='codex-cli 0.1'):
                with self.assertRaisesRegex(ValueError,'version mismatch'):
                    smoke.smoke(root)
            self.assertFalse((root/'target').exists())

    def test_rpc_preserves_interleaved_notifications_and_errors(self):
        process=Mock(stdin=io.BytesIO());rpc=smoke.RPC(process)
        rpc.pending=b'{"method":"notice"}\n{"id":1,"result":{"ok":true}}\n'
        self.assertEqual(rpc.call(1,'initialize',{}),{'ok':True})
        self.assertEqual(rpc.wait(lambda row:row.get('method')=='notice'),{'method':'notice'})
        rpc.pending=b'{"id":2,"error":{"code":-1}}\n'
        with self.assertRaisesRegex(RuntimeError,'thread/start'):
            rpc.call(2,'thread/start',{})

    def test_rpc_noise_is_bounded(self):
        rpc=smoke.RPC(Mock());rpc.pending=b'{"method":"notice"}\n'*502
        with self.assertRaisesRegex(RuntimeError,'notification limit'):
            rpc.wait(lambda _:False)

    def test_installer_includes_smoke_for_future_environments(self):
        p=Path(__file__).parents[1]/'install_symphony_development.py'
        spec=importlib.util.spec_from_file_location('runtime_installer',p)
        installer=importlib.util.module_from_spec(spec);spec.loader.exec_module(installer)
        content=installer.environment_sources()[Path('client/runtime_smoke.py')]
        self.assertEqual(content,Path(smoke.__file__).read_bytes())


if __name__=='__main__':unittest.main()
