"""Actual bounded plugin subprocesses; no external channel credentials."""
import hashlib
import importlib.util
import json
import os
import secrets
import urllib.parse
from pathlib import Path
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[2] / 'apps/notifier/dispatch.py'
spec = importlib.util.spec_from_file_location('notification_dispatch', SCRIPT)
dispatch = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dispatch)


class DispatchTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.plugin = self.root / 'plugin.py'
        self.python = str(Path(sys.executable).resolve())
        self.event = dict(protocol_version=1, event_id=7, attempt=1, requirement_id=2)

    def config(self, script):
        self.plugin.write_text(script)
        return dict(enabled=True, plugin_id='fixture', database='postgres://synthetic@localhost/test',
                    psql_program='/usr/bin/psql', argv=[self.python, '-I', str(self.plugin)],
                    implementation={str(p): hashlib.sha256(p.read_bytes()).hexdigest()
                                    for p in [Path(self.python), self.plugin]})

    def test_actual_plugin_receives_event_without_inherited_credentials(self):
        cfg = self.config("import json,os,sys\ne=json.load(sys.stdin)\nassert 'PGPASSWORD' not in os.environ\nassert 'TOKEN' not in os.environ\nprint(json.dumps(dict(protocol_version=1,event_id=e['event_id'],status='accepted')))\n")
        previous = os.environ.get('TOKEN')
        os.environ['TOKEN'] = 'synthetic-must-not-cross-boundary'
        try:
            self.assertEqual(dispatch.invoke(cfg, self.event), 'accepted')
        finally:
            if previous is None:
                del os.environ['TOKEN']
            else:
                os.environ['TOKEN'] = previous
        path = self.root/'dispatch.json'
        path.write_text(json.dumps(cfg))
        path.chmod(0o600)
        self.assertEqual(dispatch.config(path), cfg)
        self.plugin.write_text('raise SystemExit(1)')
        with self.assertRaises(ValueError):
            dispatch.invoke(cfg, self.event)
        path.chmod(0o644)
        with self.assertRaises(ValueError):
            dispatch.config(path)

    def test_invalid_ack_exit_and_output_limits_remain_unknown(self):
        for script in ["print('{\"protocol_version\":true,\"event_id\":7,\"status\":\"accepted\"}')", "print('not json')", "raise SystemExit(1)", "print('x'*70000)",
                       "print('{\"protocol_version\":1,\"event_id\":8,\"status\":\"accepted\"}')",
                       "print('{\"protocol_version\":1,\"event_id\":7,\"status\":\"pass\"}')",
                       "print('{\"protocol_version\":2,\"event_id\":7,\"status\":\"accepted\"}')"]:
            with self.subTest(script=script):
                self.assertEqual(dispatch.invoke(self.config(script), self.event), 'unknown')
        for status in ['ignored', 'failed']:
            self.assertEqual(dispatch.invoke(self.config(f"print('{{\"protocol_version\":1,\"event_id\":7,\"status\":\"{status}\"}}')"), self.event), status)

    def test_database_options_and_disabled_registration(self):
        password = secrets.token_hex(12) + '!'
        env = dispatch.database_environment('postgres://name:' + urllib.parse.quote(password, safe='') + '@localhost/test?sslmode=require')
        self.assertEqual(env['PGPASSWORD'], password)
        self.assertEqual(env['PGSSLMODE'], 'require')
        for uri in ['http://localhost/test', 'postgres://localhost/test?invalid=x']:
            with self.assertRaises(ValueError):
                dispatch.database_environment(uri)
        path = self.root/'disabled.json'
        path.write_text('{"enabled":false}')
        path.chmod(0o600)
        self.assertEqual(dispatch.config(path), {'enabled': False})


if __name__ == '__main__':
    unittest.main()
