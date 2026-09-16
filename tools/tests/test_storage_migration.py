import importlib.util,json,sys,tempfile,unittest
from pathlib import Path
from unittest.mock import patch,Mock
sys.path.insert(0,str(Path(__file__).parents[1]))
import migrate_symphony_storage as migration

class MigrationTests(unittest.TestCase):
    def test_activation_requires_admin_and_verified_copy(self):
        with patch.object(migration.os,'geteuid',return_value=1000):
            with self.assertRaises(PermissionError):migration.activate()
        with tempfile.TemporaryDirectory() as tmp:
            audit=Path(tmp);(audit/'state.json').write_text(json.dumps({'status':'preparing'}))
            with patch.object(migration,'AUDIT',audit),patch.object(migration,'safe_paths'),patch.object(migration.os,'geteuid',return_value=0):
                with self.assertRaisesRegex(ValueError,'verification'):migration.activate()
    def test_prepare_mismatch_keeps_sources_and_services_stopped(self):
        with tempfile.TemporaryDirectory() as tmp:
            audit=Path(tmp)/'audit';answer=Mock(stdout='inactive\n')
            with patch.object(migration,'AUDIT',audit),patch.object(migration,'safe_paths'),patch.object(migration.os,'geteuid',return_value=1000),patch.object(migration.os.path,'ismount',return_value=False),patch.object(migration,'units',return_value=['test.service','test.timer']),patch.object(migration,'systemctl',return_value=answer) as control,patch.object(migration,'synchronize',side_effect=['copied','checksum changed']):
                with self.assertRaisesRegex(ValueError,'differs'):migration.prepare()
                self.assertEqual(json.loads((audit/'state.json').read_text())['status'],'preparing')
                self.assertTrue(any(c.args[0]=='stop' for c in control.call_args_list))
                self.assertFalse(any(c.args[0]=='start' for c in control.call_args_list))
    def test_missing_ssd_is_rejected(self):
        with patch.object(migration.os.path,'ismount',return_value=False):
            with self.assertRaisesRegex(ValueError,'mount missing'):migration.safe_paths()
    def test_backup_with_nested_mount_or_open_worker_is_retained(self):
        with tempfile.TemporaryDirectory() as tmp:
            base=Path(tmp)/'codexsymphony'
            backup=Path(tmp)/'codexsymphony.pre-ssd-20260916T000000Z'
            backup.mkdir();nested=backup/'nested';nested.mkdir()
            with patch.object(migration,'BASE',base),patch.object(Path,'is_mount',lambda p:p==nested):
                with self.assertRaisesRegex(ValueError,'contains mount'):migration.validate_backup(backup)
            with patch.object(migration,'BASE',base),patch.object(migration,'gate_busy',return_value=True):
                with self.assertRaisesRegex(ValueError,'still in use'):migration.validate_backup(backup)
            self.assertTrue(nested.exists())
if __name__=='__main__':unittest.main()
