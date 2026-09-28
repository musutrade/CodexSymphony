import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import install_bounded_retention as installer


class InstallTests(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory();self.addCleanup(self.tmp.cleanup)
        self.base=Path(self.tmp.name).resolve();self.release=self.base/'release';self.release.mkdir()
        self.entry=self.release/'bounded_retention.py';self.entry.write_text('fixture')
        self.approval={'host_release':str(self.release),'runtime_files':{str(self.entry):installer.rust_capture.digest(self.entry)}}
        (self.base/'approval.json').write_text(json.dumps(self.approval))
        self.timer=self.base/'codexsymphony-archive.timer';self.timer.write_text('[Timer]\nOnBootSec=5min\nOnUnitInactiveSec=1h\nUnit=old.service\n')
        self.deployment=self.base/'deployment.json';self.save_schedule()
        for owner,name,value in [(installer,'HOME',self.base),(installer,'UNITS',self.base),(installer.manual_measure,'DEPLOYMENT',self.deployment)]:
            p=patch.object(owner,name,value);p.start();self.addCleanup(p.stop)

    def save_schedule(self):
        self.deployment.write_text(json.dumps({'files':{str(self.timer):installer.rust_capture.digest(self.timer)}}))

    def test_only_installed_approved_entry_can_be_enabled(self):
        self.assertEqual(installer.accepted_entry(),self.entry)
        self.entry.write_text('drift')
        with self.assertRaises(ValueError):installer.accepted_entry()
        self.approval['runtime_files']={};(self.base/'approval.json').write_text(json.dumps(self.approval))
        with self.assertRaisesRegex(ValueError,'not in'):installer.accepted_entry()

    def test_schedule_uses_approved_intervals_and_rejects_drift_or_unknown_fields(self):
        self.assertIn('OnUnitInactiveSec=1h',installer.approved_schedule())
        self.timer.write_text('[Timer]\nUnexpected=yes\n')
        with self.assertRaisesRegex(ValueError,'changed'):installer.approved_schedule()
        self.save_schedule()
        with self.assertRaisesRegex(ValueError,'unsupported'):installer.approved_schedule()

    def test_install_records_fixed_entry_and_rejects_unit_alias(self):
        with patch.object(installer.layout,'ensure'),patch.object(installer.subprocess,'run') as command:
            result=installer.install()
            self.assertEqual(result['entry'],str(self.entry))
            self.assertEqual(command.call_count,2)
            service=self.base/(installer.NAME+'.service')
            self.assertIn(str(self.entry)+' --apply',service.read_text())
            service.unlink();service.symlink_to(self.entry)
            with self.assertRaisesRegex(ValueError,'aliased'):installer.install()


if __name__=='__main__':unittest.main()
