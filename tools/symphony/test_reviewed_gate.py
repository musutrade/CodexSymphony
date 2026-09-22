"""Gate selection rejects unreviewed versions and altered installed binaries."""
import hashlib
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import reviewed_gate


class ReviewedGateTests(unittest.TestCase):
    def test_reviewed_versions_select_their_own_binary_and_reject_tampering(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            versions = root / 'versions'
            pins = {}
            for lock, (version, _) in reviewed_gate.PINS.items():
                binary = versions / version / 'bin/harness-gate'
                binary.parent.mkdir(parents=True)
                binary.write_bytes(version.encode())
                pins[lock] = (version, hashlib.sha256(binary.read_bytes()).hexdigest())
            with patch.object(reviewed_gate, 'VERSIONS', versions), patch.object(reviewed_gate, 'PINS', pins):
                for lock, (version, _) in pins.items():
                    (root / 'harness-gate-version.lock').write_text(lock + '\n')
                    selected = Path(reviewed_gate.gate_bin(root))
                    self.assertEqual(selected, versions / version / 'bin')
                    (selected / 'harness-gate').write_bytes(b'changed')
                    with self.assertRaisesRegex(ValueError, 'digest mismatch'):
                        reviewed_gate.gate_bin(root)

    def test_unknown_empty_and_path_injected_versions_fail_closed(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for lock in ('', 'harness-gate v99', 'harness-gate ../../unreviewed'):
                with self.subTest(lock=lock):
                    (root / 'harness-gate-version.lock').write_text(lock)
                    with self.assertRaisesRegex(ValueError, 'not host-reviewed'):
                        reviewed_gate.gate_bin(root)
