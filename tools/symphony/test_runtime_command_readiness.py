import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

DIRECTORY = Path(__file__).parent / 'environment'
sys.path.insert(0, str(DIRECTORY))
import runtime_command_readiness as readiness
import broker

class RuntimeCommandReadinessTests(unittest.TestCase):
    def test_fixed_action_rejects_command_injection(self):
        with self.assertRaises(ValueError):
            broker.perform({'action': 'runtime-command-readiness', 'command': ['/bin/sh']})

    def test_network_failures_never_publish_receipt(self):
        for key in ('allowed_probe', 'denied_probe', 'direct_connection_rejected'):
            with self.subTest(key=key), tempfile.TemporaryDirectory() as temporary:
                base = Path(temporary)
                (base / 'requirements.toml').write_text('[experimental_network]\ndomains = { "crates.io" = "allow" }\n')
                probes = dict(allowed_probe=True, denied_probe=True, direct_connection_rejected=True)
                probes[key] = False
                response = {'command_exec': {'result': {'exitCode': 0, 'stdout': json.dumps({'network': probes})}},
                            'requirements': {'requirements': {'network': {'enabled': True, 'allowedDomains': ['crates.io']}}}}
                with patch.object(readiness, 'BASE', base), patch.object(readiness.socket, 'getaddrinfo', return_value=[]), patch.object(readiness, 'execute', return_value=response):
                    with self.assertRaisesRegex(ValueError, 'network boundary'):
                        readiness.probe()
                self.assertFalse((base / 'client/runtime-command-readiness.json').exists())

    def test_failed_command_never_becomes_ready(self):
        response = {'command_exec': {'result': {'exitCode': 101, 'stderr': 'denied'}}}
        with patch.object(readiness.socket, 'getaddrinfo', return_value=[]), patch.object(readiness, 'execute', return_value=response):
            with self.assertRaisesRegex(RuntimeError, 'environment failed'):
                readiness.probe()

if __name__ == '__main__':
    unittest.main()
