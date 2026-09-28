import json
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch

import capture_handoff as handoff


class HandoffTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(); self.addCleanup(self.tmp.cleanup)
        self.base = Path(self.tmp.name).resolve()
        self.root = self.base / 'source'; self.root.mkdir()
        (self.root / 'source.rs').write_text('source')
        (self.root / 'target').mkdir(); (self.root / 'target/cache').write_text('not evidence')
        self.run = self.base / 'run'; self.run.mkdir()
        bundle = self.run / 'probes/backend/bundle.json'; bundle.parent.mkdir(parents=True); bundle.write_text('{}')
        self.registration = {'root': str(self.run), 'bundle_sha256': handoff.rust_capture.digest(bundle)}
        (self.run / 'capture-registration.json').write_text(json.dumps(self.registration))
        self.inputs = {'source.rs': handoff.fixed_workspace.fingerprint(self.root / 'source.rs')}
        self.pending = self.base / 'pending.json'
        self.record = {'capture': str(self.run), 'source_inputs': {'source.rs': self.inputs['source.rs']['sha256']}}
        self.pending.write_text(json.dumps(self.record))
        measured = self.run / 'measurements.json'; measured.write_text('{}')
        self.result = {'coverage_and_crap': 'PASS', 'measurement': str(measured), 'measurement_sha256': handoff.rust_capture.digest(measured)}
        p = patch.object(handoff.fixed_workspace, 'sources', return_value=self.inputs); p.start(); self.addCleanup(p.stop)

    def test_handoff_keeps_only_manifest_sources_and_then_releases_pending(self):
        handoff.complete(self.run, self.root, self.pending, self.result)
        self.assertFalse(self.pending.exists())
        archive = self.run / 'source.tar.gz'
        with tarfile.open(archive) as source:
            self.assertEqual(source.getnames(), ['source.rs'])
            self.assertEqual(source.extractfile('source.rs').read(), b'source')
        before = archive.stat().st_mtime_ns
        handoff.archive_sources(self.run, self.root, self.inputs)
        self.assertEqual(archive.stat().st_mtime_ns, before)
        archive.write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError, 'archive changed'):
            handoff.archive_sources(self.run, self.root, self.inputs)

    def test_invalid_or_changed_measurement_and_capture_keep_pending(self):
        for changes, message in [({'coverage_and_crap': 'FAIL'}, 'failed measurement'), ({'measurement_sha256': 'wrong'}, 'measurement changed')]:
            with self.assertRaisesRegex(ValueError, message): handoff.complete(self.run, self.root, self.pending, dict(self.result, **changes))
        self.pending.write_text(json.dumps(dict(self.record, capture='/wrong')))
        with self.assertRaisesRegex(ValueError, 'identity changed'): handoff.complete(self.run, self.root, self.pending, self.result)
        self.pending.write_text(json.dumps(dict(self.record, source_inputs={})))
        with self.assertRaisesRegex(ValueError, 'source changed'): handoff.complete(self.run, self.root, self.pending, self.result)
        self.pending.write_text(json.dumps(self.record))
        (self.run / 'capture-registration.json').write_text(json.dumps(dict(self.registration, bundle_sha256='wrong')))
        with self.assertRaisesRegex(ValueError, 'registered capture'): handoff.complete(self.run, self.root, self.pending, self.result)
        self.assertTrue(self.pending.exists())

    def test_source_change_during_archive_does_not_issue_handoff(self):
        with patch.object(handoff.fixed_workspace, 'sources', return_value={}):
            with self.assertRaisesRegex(ValueError, 'source changed while'): handoff.archive_sources(self.run, self.root, self.inputs)
        self.assertFalse((self.run / 'source-archive.json').exists())
        self.assertTrue(self.pending.exists())


if __name__ == '__main__': unittest.main()
