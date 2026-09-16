import importlib.util
import json
import os
from pathlib import Path
import tempfile
import subprocess
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location('storage', Path(__file__).parents[1] / 'storage_maintenance.py')
STORAGE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(STORAGE)


class StorageMaintenance(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.run = self.root / 'gate-host/runs/run-0123456789ab'
        self.target = self.run / 'target'
        self.target.mkdir(parents=True)
        (self.target / 'object').write_bytes(b'cache')
        os.utime(self.target, (0, 0))

    def test_collects_old_cache_without_success_receipt_and_preserves_evidence(self):
        evidence = self.run / 'probes/raw'
        evidence.mkdir(parents=True)
        (evidence / 'object').write_bytes(b'evidence')
        (self.run / 'signature').write_text('signed')
        result = STORAGE.collect(self.root, busy=lambda _: False, now=1000)
        self.assertEqual(result['removed'], [str(self.target)])
        self.assertEqual((evidence / 'object').read_bytes(), b'evidence')
        self.assertEqual((self.run / 'signature').read_text(), 'signed')
        self.assertFalse(self.target.exists())

    def test_active_capture_defers_cleanup(self):
        self.assertTrue(STORAGE.collect(self.root, busy=lambda _: True, now=1000)['deferred'])
        self.assertTrue(self.target.exists())

    def test_recent_target_is_kept(self):
        self.assertEqual(STORAGE.collect(self.root, busy=lambda _: False, now=100)['removed'], [])

    def test_target_symlink_is_not_followed(self):
        other = self.root / 'outside'
        self.target.rename(other)
        self.target.symlink_to(other)
        STORAGE.collect(self.root, busy=lambda _: False, now=1000)
        self.assertTrue((other / 'object').exists())

    def test_nested_symlink_does_not_delete_destination(self):
        other = self.root / 'evidence'
        other.mkdir()
        (other / 'keep').write_text('keep')
        (self.target / 'link').symlink_to(other)
        os.utime(self.target, (0, 0))
        STORAGE.collect(self.root, busy=lambda _: False, now=1000)
        self.assertTrue((other / 'keep').exists())

    def test_symlinked_runs_root_rejected(self):
        runs = self.run.parent
        outside = self.root / 'outside'
        runs.rename(outside)
        runs.symlink_to(outside)
        with self.assertRaises(ValueError):
            STORAGE.collect(self.root, busy=lambda _: False, now=1000)

    def process(self, command):
        proc = self.root / 'proc'
        process = proc / '99999999'
        process.mkdir(parents=True)
        (process / 'cmdline').write_bytes(command)
        (process / 'fd').mkdir()
        (process / 'cwd').symlink_to(self.root)
        return proc, process

    def test_launcher_without_run_argument_is_busy(self):
        proc, _ = self.process(b'python3\0/home/gem/.local/share/codexsymphony/gate-host/releases/abc/run.py\0')
        self.assertTrue(STORAGE.gate_busy(self.run.parent, proc))

    def test_orphan_worker_open_descriptor_is_busy(self):
        proc, process = self.process(b'rustc\0')
        (process / 'fd/3').symlink_to(self.target / 'object')
        self.assertTrue(STORAGE.gate_busy(self.run.parent, proc))

    def test_unrelated_worker_does_not_block_collection(self):
        proc, _ = self.process(b'python3\0unrelated.py\0')
        self.assertFalse(STORAGE.gate_busy(self.run.parent, proc))

    def test_unreadable_process_defers_collection(self):
        proc, _ = self.process(b'worker\0')
        with patch.object(Path, 'read_bytes', side_effect=PermissionError):
            self.assertTrue(STORAGE.gate_busy(self.run.parent, proc))

    def test_session_manager_does_not_require_protected_descriptors(self):
        proc, process = self.process(b'/usr/lib/systemd/systemd\0--user\0--deserialize=12\0')
        (process / 'fd').rmdir()
        self.assertFalse(STORAGE.gate_busy(self.run.parent, proc))

    def test_fixture_broker_spool_does_not_block_but_children_are_scanned(self):
        command = b'/usr/bin/python3\0' + str(STORAGE.ROOT / 'symphony/gh18-environment/broker.py').encode() + b'\0'
        proc, process = self.process(command)
        (process / 'fd/3').symlink_to(self.run.parent / 'requests')
        self.assertFalse(STORAGE.gate_busy(self.run.parent, proc))
        child = proc / '99999998'; child.mkdir(); (child / 'fd').mkdir()
        (child / 'cmdline').write_bytes(b'rustc\0')
        (child / 'cwd').symlink_to(self.target)
        self.assertTrue(STORAGE.gate_busy(self.run.parent, proc))

    def test_real_orphan_worker_blocks_until_it_exits(self):
        worker = subprocess.Popen(['sleep', '60'], cwd=self.target)
        try:
            self.assertTrue(STORAGE.collect(self.root, now=1000)['deferred'])
            self.assertTrue(self.target.exists())
        finally:
            worker.terminate()
            worker.wait(timeout=5)
        result = STORAGE.collect(self.root, now=1000)
        self.assertFalse(result['deferred'])
        self.assertEqual(result['removed'], [str(self.target)])

    def test_start_condition_rejects_low_disk(self):
        with patch('sys.argv', ['storage', '--check-start']), patch.object(STORAGE.shutil, 'disk_usage') as usage:
            usage.return_value.free = 19 * STORAGE.GIB
            with self.assertRaises(SystemExit) as error:
                STORAGE.main()
            self.assertEqual(error.exception.code, 1)
            usage.return_value.free = 20 * STORAGE.GIB
            STORAGE.main()

    def test_pressure_pauses_only_active_services_and_restart_survives_new_invocation(self):
        calls = []
        def control(action, service):
            calls.append((action, service))
            return service == STORAGE.SERVICES[0] if action == 'is-active' else None
        no_gc = lambda _: {'removed': [], 'deferred': True}
        state = STORAGE.maintain(self.root, control, lambda: 10 * STORAGE.GIB, no_gc)
        self.assertEqual(state['paused_services'], [STORAGE.SERVICES[0]])
        self.assertIn(('stop', STORAGE.SERVICES[0]), calls)
        self.assertNotIn(('stop', STORAGE.SERVICES[1]), calls)
        calls.clear()
        STORAGE.maintain(self.root, control, lambda: 16 * STORAGE.GIB, no_gc)
        self.assertEqual(calls, [])  # Hysteresis prevents start/stop loops.
        state = STORAGE.maintain(self.root, control, lambda: 25 * STORAGE.GIB, no_gc)
        self.assertEqual(calls, [('start', STORAGE.SERVICES[0])])
        self.assertEqual(state['paused_services'], [])

    def test_failed_stop_retains_recovery_state_and_never_collects(self):
        def control(action, service):
            if action == 'is-active':
                return True
            raise RuntimeError('stop failed')
        with self.assertRaises(RuntimeError):
            STORAGE.maintain(self.root, control, lambda: 0,
                             lambda _: self.fail('must not collect after failed stop'))
        state = json.loads((self.root / 'storage-maintenance/state.json').read_text())
        self.assertEqual(state['paused_services'], [STORAGE.SERVICES[0]])


if __name__ == '__main__':
    unittest.main()

class WorkspaceCaches(unittest.TestCase):
    def test_done_and_stopped_waiting_only_and_preserves_coverage(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp); ledger=root/'symphony/WORKFLOW.lifecycle.md.handoffs.json'
            ledger.parent.mkdir(); entries={}
            for issue,phase in [('1','done'),('2','waiting'),('3','implementing')]:
                entries[issue]={'identifier':'GH-'+issue,'phase':phase}
                target=root/('workspaces/GH-'+issue)/'target'
                (target/'debug').mkdir(parents=True);(target/'debug/object').write_text('cache')
                os.utime(target/'debug',(0,0))
                (target/'coverage.json').write_text('evidence')
                (target/'llvm-cov-target').mkdir();(target/'llvm-cov-target/raw.profraw').write_text('raw')
            ledger.write_text(json.dumps({'entries':entries}))
            result=STORAGE.collect_workspace_caches(root,busy=lambda _:False,scheduler_stopped=False)
            self.assertEqual(len(result['removed']),1)
            self.assertTrue((root/'workspaces/GH-2/target/debug').exists())
            result=STORAGE.collect_workspace_caches(root,busy=lambda _:False,scheduler_stopped=True)
            self.assertEqual(len(result['removed']),1)
            self.assertTrue((root/'workspaces/GH-3/target/debug').exists())
            for issue in entries:
                self.assertTrue((root/('workspaces/GH-'+issue)/'target/coverage.json').exists())
                self.assertTrue((root/('workspaces/GH-'+issue)/'target/llvm-cov-target/raw.profraw').exists())

    def test_busy_and_symlink_workspaces_are_kept(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);p=root/'symphony/WORKFLOW.lifecycle.md.handoffs.json';p.parent.mkdir()
            p.write_text(json.dumps({'entries':{'1':{'identifier':'GH-1','phase':'done'}}}))
            w=root/'workspaces/GH-1';(w/'target/debug').mkdir(parents=True);os.utime(w/'target/debug',(0,0))
            self.assertEqual(STORAGE.collect_workspace_caches(root,busy=lambda _:True)['removed'],[])
            w.rename(root/'saved');w.symlink_to(root/'saved')
            self.assertEqual(STORAGE.collect_workspace_caches(root,busy=lambda _:False)['removed'],[])
