"""Upgrade regressions; collect source-bound Python measurements before replay."""
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import runpy
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]


def module(name, relative):
    spec = importlib.util.spec_from_file_location(name, ROOT / relative)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


class CodexUpgradeTests(unittest.TestCase):
    def test_generator_preserves_optional_capabilities_and_schema_identity(self):
        generator = module('upgrade_protocol', 'tools/generate_runtime_protocol.py')
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            schema = {'properties': {'requiredName': {'type': 'string'},
                                     'capabilities': {'type': 'object'}},
                      'required': ['requiredName']}
            for name in generator.NAMES:
                (directory / (name + '.json')).write_text(json.dumps(schema))
            source, before = generator.generate(directory)
            self.assertIn('Codex 0.157.1.', source)
            self.assertIn('pub required_name: String,', source)
            self.assertIn('pub capabilities: Option<serde_json::Value>,', source)
            changed = directory / 'InitializeParams.json'
            schema['properties']['capabilities']['properties'] = {
                'explicitGatewayOauth': {'type': 'boolean'}}
            changed.write_text(json.dumps(schema))
            after_source, after = generator.generate(directory)
            self.assertEqual(source, after_source)
            old_hashes, new_hashes = json.loads(before), json.loads(after)
            self.assertNotEqual(old_hashes.pop('InitializeParams'),
                                new_hashes.pop('InitializeParams'))
            self.assertEqual(old_hashes, new_hashes)

    def test_preparation_accepts_new_runtime_and_rejects_old_runtime(self):
        probe = module('upgrade_probe', 'tools/preparation/environment_probe.py')
        with patch.object(probe, 'run', return_value='codex-cli 0.157.1'):
            self.assertEqual(probe.tools(project_verified=True), [])
        with patch.object(probe, 'run', return_value='codex-cli 0.156.1'):
            with self.assertRaisesRegex(ValueError, 'Codex version'):
                probe.tools(project_verified=True)
        with tempfile.TemporaryDirectory() as temporary:
            executable = Path(temporary) / 'harness-gate'
            executable.write_bytes(b'reviewed core fixture')
            import hashlib
            digest = hashlib.sha256(executable.read_bytes()).hexdigest()
            lock = {'core_version': 'core fixture', 'core_sha256': digest,
                    'codex_version': 'codex-cli 0.157.1'}
            with patch.object(probe.shutil, 'which', return_value=str(executable)):
                with patch.object(probe, 'run', side_effect=[
                        'core fixture', 'core fixture', 'codex-cli 0.157.1']):
                    self.assertEqual(len(probe.tools(lock)), 2)
                with patch.object(probe, 'run', return_value='wrong core'):
                    with self.assertRaisesRegex(ValueError, 'Core version'):
                        probe.tools(lock)
            with patch.object(probe.shutil, 'which', return_value=None):
                with self.assertRaises(FileNotFoundError):
                    probe.tools(lock)

    def test_mobile_fixture_handshake_and_resume_remain_consistent(self):
        messages = [
            {'id': 1, 'method': 'initialize'},
            {'id': 2, 'method': 'thread/start'},
            {'id': 3, 'method': 'turn/start', 'params': {'input': [
                {'text': json.dumps({'confirmed_answers': []})}]}},
            {'id': 4, 'method': 'turn/start', 'params': {'input': [
                {'text': json.dumps({'confirmed_answers': ['continue']})}]}},
            {'id': 5, 'method': 'turn/interrupt'},
            {'method': 'unknown-notification'},
        ]
        output = io.StringIO()
        with tempfile.TemporaryDirectory() as temporary, contextlib.chdir(temporary):
            messages.append({'id': 6, 'method': 'command/exec', 'params': {
                'cwd': os.getcwd(), 'command': ['/bin/sh', '-c', "printf probe"]}})
            data = '\n'.join(json.dumps(value) for value in messages) + '\n'
            with patch('sys.stdin', io.StringIO(data)), contextlib.redirect_stdout(output):
                namespace = runpy.run_path(str(ROOT / 'tools/fixtures/mobile_runtime.py'), run_name='__main__')
                namespace['tool']('fixture-tool', 'fixture', {})
            self.assertEqual(Path('paid-work.txt').read_text(), 'original unfinished work\n')
            self.assertEqual(json.loads(Path('resumed-proof.json').read_text()), ['continue'])
        frames = [json.loads(line) for line in output.getvalue().splitlines()]
        replies = {frame['id']: frame for frame in frames if 'result' in frame}
        self.assertEqual(replies[1]['result']['userAgent'],
                         'scripted-mobile-fixture/0.157.1 (test)')
        self.assertEqual(replies[6]['result']['stdout'], 'probe')
        self.assertEqual(sum(frame.get('method') == 'item/tool/requestUserInput'
                             for frame in frames), 1)


if __name__ == '__main__':
    unittest.main()
