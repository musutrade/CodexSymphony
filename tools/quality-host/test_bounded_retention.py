from contextlib import nullcontext
import hashlib
import json
import os
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import MagicMock,patch

import bounded_retention as retention


class RetentionTests(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory();self.addCleanup(self.tmp.cleanup)
        self.base=Path(self.tmp.name).resolve();self.slot=self.base/'slot';self.slot.mkdir()
        self.run=self.base/'evidence/gate/run-aaaaaaaaaaaa';self.run.mkdir(parents=True)
        for owner,name,value in [(retention.layout,'VOLUME',self.base),(retention.layout,'slot',lambda:self.slot)]:
            p=patch.object(owner,name,value);p.start();self.addCleanup(p.stop)

    def complete(self):
        archive=self.run/'source.tar.gz';archive.write_bytes(b'archive')
        (self.run/'source-archive.json').write_text(json.dumps({'sha256':retention.rust_capture.digest(archive)}))
        bundle=self.run/'probes/backend/bundle.json';bundle.parent.mkdir(parents=True);bundle.write_text('{}')
        (self.run/'capture-registration.json').write_text(json.dumps({'root':str(self.run),'bundle_sha256':retention.rust_capture.digest(bundle)}))
        measurement=self.run/'measurements.json';measurement.write_text('{}')
        result={'coverage_and_crap':'PASS','measurement':str(measurement),'measurement_sha256':retention.rust_capture.digest(measurement)}
        (self.run/'measurement-summary.json').write_text(json.dumps(result))
        return result

    def test_installed_policy_requires_deployment_hashes(self):
        release=self.base/'release';release.mkdir()
        entry=release/'compact_gate_evidence.py';entry.write_text('def maintain(keep=2,hours=24,budget=123):pass\n')
        deployment=self.base/'deployment.json'
        value={'release':str(release),'files':{str(entry):retention.rust_capture.digest(entry)}}
        deployment.write_text(json.dumps(value))
        with patch.object(retention.manual_measure,'DEPLOYMENT',deployment):
            self.assertEqual(retention.installed_policy().maintain.__defaults__,(2,24,123))
            entry.write_text('changed')
            with self.assertRaisesRegex(ValueError,'changed'):retention.installed_policy()
            value['files']={};deployment.write_text(json.dumps(value))
            with self.assertRaisesRegex(ValueError,'not approved'):retention.installed_policy()

    def test_pending_capture_and_gate_are_both_protected(self):
        self.assertEqual(retention.pending_runs(),set())
        for name in ('pending-capture.json','pending-gate.json'):
            (self.slot/name).write_text(json.dumps({'capture':str(self.run)}))
        self.assertEqual(retention.pending_runs(),{str(self.run)})
        (self.slot/'pending-gate.json').unlink();(self.slot/'pending-gate.json').symlink_to(self.run/'absent')
        with self.assertRaises(ValueError):retention.pending_runs()

    def test_completed_capture_requires_source_registration_and_measurement_identity(self):
        self.assertFalse(retention.completed(self.run))
        result=self.complete();self.assertTrue(retention.completed(self.run))
        (self.run/'measurement-summary.json').unlink();self.assertFalse(retention.completed(self.run))
        (self.run/'recovery-measurement.json').write_text(json.dumps(result));self.assertTrue(retention.completed(self.run))
        result['measurement_sha256']='wrong';(self.run/'recovery-measurement.json').write_text(json.dumps(result))
        with self.assertRaisesRegex(ValueError,'measurement'):retention.completed(self.run)
        (self.run/'capture-registration.json').write_text('{"root":"wrong"}')
        with self.assertRaisesRegex(ValueError,'registration'):retention.completed(self.run)
        (self.run/'source.tar.gz').write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError,'archive'):retention.completed(self.run)

    def test_retention_uses_installed_bounds_and_skips_pending_or_incomplete_runs(self):
        def maintain(keep=2,hours=24,budget=123):pass
        policy=SimpleNamespace(maintain=maintain)
        self.complete()
        with patch.object(retention.layout,'lease',return_value=nullcontext()),patch.object(retention,'installed_policy',return_value=policy),patch.object(retention,'expire',return_value=[]) as expire,patch.object(retention.bounded_records,'maintain',return_value=[]) as record_cleanup:
            retention.maintain();self.assertEqual(expire.call_args.args[1],[self.run])
            self.assertEqual(record_cleanup.call_args.args[1],[self.run])
            self.assertEqual(expire.call_args.args[2:5],(2,24,123))
            (self.slot/'pending-gate.json').write_text(json.dumps({'capture':str(self.run)}))
            retention.maintain();self.assertEqual(expire.call_args.args[1],[])
            (self.run.parent/'run-unsafe').mkdir()
            with self.assertRaisesRegex(ValueError,'unsafe'):retention.maintain()

    def test_raw_byte_age_and_count_limits_delegate_only_named_payloads(self):
        self.complete();os.utime(self.run/'source-archive.json',(100,100))
        policy=SimpleNamespace(bytes_used=lambda paths:100,payloads=lambda run:[run/'probes/backend/raw'],compact=MagicMock(return_value={'released':100}))
        with patch.object(retention.time,'time',return_value=101):
            self.assertEqual(retention.expire(policy,[self.run],2,24,200,False),[])
            for keep,hours,budget in ((0,24,200),(2,0,200),(2,24,99)):
                self.assertEqual(retention.expire(policy,[self.run],keep,hours,budget,False)[0]['rebuildable_bytes'],100)
            self.assertEqual(retention.expire(policy,[self.run],0,24,200,True),[{'released':100}])
            policy.compact.assert_called_once_with(self.run)

    def test_scratch_cleanup_is_scoped_and_does_not_follow_dependency_links(self):
        outside=self.base/'dependencies';outside.mkdir();(outside/'keep').write_text('keep')
        scratch=self.run/'tmp';scratch.mkdir();(scratch/'node_modules').symlink_to(outside,target_is_directory=True)
        retention.cleanup_scratch(self.run)
        self.assertEqual((outside/'keep').read_text(),'keep');self.assertFalse(scratch.exists())
        with patch.object(retention.shutil.rmtree,'avoids_symlink_attacks',False):
            with self.assertRaises(ValueError):retention.cleanup_scratch(self.run)
        scratch.mkdir()
        with patch.object(Path,'is_mount',return_value=True):
            with self.assertRaisesRegex(ValueError,'foreign mount'):retention.cleanup_scratch(self.run)
        self.assertTrue(scratch.exists())

    def test_cli_defaults_to_dry_run(self):
        import sys
        with patch.object(sys,'argv',['retention']),patch.object(retention,'maintain',return_value=[]) as maintain:
            retention.main();maintain.assert_called_once_with(False)


if __name__=='__main__':unittest.main()
