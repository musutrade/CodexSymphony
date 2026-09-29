import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import bounded_records as records

EMPTY=records.evidence_pins.empty()
NONE=frozenset()


class RecordTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name).resolve()
        self.parent = self.base / 'gate'; self.parent.mkdir()
        self.state = self.base / 'state'; self.state.mkdir()
        (self.state / 'capture-caches.json').write_text(json.dumps({'schema':'capture-cache-registry/v1','captures':[]}))
        changed = patch.object(records.manual_measure, 'DEPLOYMENT', self.state / 'deployment.json')
        changed.start(); self.addCleanup(changed.stop)

    def run_record(self, suffix='aaaaaaaaaaaa', timestamp=100):
        root = self.parent / ('run-' + suffix); root.mkdir()
        (root / 'source-archive.json').write_text(json.dumps({'sha256':'source'}))
        (root / 'source-inputs.json').write_text('{}')
        os.utime(root / 'source-archive.json', (timestamp, timestamp))
        return root

    def test_duplicate_reports_preserve_bytes_and_modes_and_never_follow_links(self):
        run = self.run_record()
        self.assertEqual(records.deduplicate(run, True), 0)
        report = run / 'reports'; report.mkdir()
        (report / 'a').write_bytes(b'content')
        (report / 'b').write_bytes(b'content')
        self.assertEqual(records.deduplicate(run, False), 7)
        self.assertNotEqual((report/'a').stat().st_ino,(report/'b').stat().st_ino)
        self.assertEqual(records.deduplicate(run, True), 7)
        self.assertEqual((report/'a').stat().st_ino,(report/'b').stat().st_ino)
        self.assertEqual(records.deduplicate(run, True), 0)
        (report / 'c').write_bytes(b'content'); (report/'c.deduplicate-new').write_bytes(b'pending')
        with self.assertRaisesRegex(ValueError,'unfinished'): records.deduplicate(run, True)
        (report/'c.deduplicate-new').unlink(); (report/'c.deduplicate-new').symlink_to(report/'a')
        with self.assertRaisesRegex(ValueError,'unfinished'):records.deduplicate(run,True)
        (report/'c.deduplicate-new').unlink();os.link(report/'a',report/'c.deduplicate-new')
        self.assertEqual(records.deduplicate(run,True),7)
        (report/'c').unlink()
        (report/'link').symlink_to(report/'a')
        with self.assertRaisesRegex(ValueError,'aliased'): records.deduplicate(run, True)
        with patch.object(Path,'is_mount',return_value=True):
            with self.assertRaisesRegex(ValueError,'unsafe'): records.safe_directory(run)
        with patch.object(records.shutil.rmtree,'avoids_symlink_attacks',False):
            with self.assertRaises(ValueError): records.safe_directory(run)
        original=Path.is_mount
        with patch.object(Path,'is_mount',lambda p: p==report or original(p)):
            with self.assertRaisesRegex(ValueError,'foreign'): records.safe_directory(run)

    def test_summary_preserves_outcome_identity_and_explicit_artifact_expiry(self):
        run = self.run_record()
        self.assertEqual(records.summary(run)['timings'], [])
        (run/'measurement-summary.json').write_text('{"coverage_and_crap":"PASS"}')
        (run/'timings.jsonl').write_text('{"phase":"capture","status":"FAIL"}\n{"phase":"recovery","status":"PASS"}\n')
        report=run/'reports/test_result.json';report.parent.mkdir()
        report.write_text(json.dumps(dict(passed=True,evidence_complete=True,configuration_digest='policy',source_identity='source')))
        value=records.summary(run)
        self.assertFalse(value['artifacts_available'])
        self.assertEqual(value['timings'][0]['status'],'FAIL')
        self.assertEqual(value['measurement-summary.json']['coverage_and_crap'],'PASS')
        self.assertEqual(value['report']['sha256'],records.rust_capture.digest(report))

    def test_retirement_is_durable_before_delete_and_can_resume_after_interruption(self):
        run=self.run_record(); index=[]
        self.assertTrue(records.retire(run,self.parent,index,NONE,False)['artifacts_available'])
        self.assertTrue(run.exists()); self.assertEqual(index,[])
        original=records.shutil.rmtree
        with patch.object(records.shutil,'rmtree',side_effect=OSError('interrupted')) as remove:
            remove.avoids_symlink_attacks=True
            with self.assertRaises(OSError):records.retire(run,self.parent,index,NONE,True)
        self.assertFalse(run.exists())
        index=records.read_index(self.parent);self.assertEqual(index[0]['run'],run.name)
        records.resume_deletions(self.parent,index,NONE,False)
        self.assertTrue((self.parent/('retiring-'+run.name)).exists())
        records.resume_deletions(self.parent,index,NONE,True)
        records.resume_deletions(self.parent,index,NONE,True)
        self.assertFalse((self.parent/('retiring-'+run.name)).exists())
        rogue=self.parent/'retiring-run-bbbbbbbbbbbb';rogue.mkdir()
        with self.assertRaisesRegex(ValueError,'unregistered'):records.resume_deletions(self.parent,index,NONE,True)
        self.assertTrue(rogue.exists())
        run=self.run_record('cccccccccccc');(self.parent/('retiring-'+run.name)).mkdir()
        with self.assertRaisesRegex(ValueError,'collision'):records.retire(run,self.parent,index,NONE,True)

    def test_index_is_bounded_by_time_bytes_and_rejects_invalid_entries(self):
        self.assertEqual(records.read_index(self.parent),[])
        values=[{'run':'run-aaaaaaaaaaaa','expired_at':100,'artifacts_available':False},
                {'run':'run-bbbbbbbbbbbb','expired_at':200,'artifacts_available':False}]
        with patch.object(records.time,'time',return_value=201):
            records.trim_index(self.parent,values,24,1000,False)
            self.assertFalse((self.parent/'expired-records.json').exists())
            records.trim_index(self.parent,values,.01,1000,True)
            self.assertEqual(records.read_index(self.parent),values[1:])
            records.trim_index(self.parent,values,24,0,True)
            self.assertEqual(records.read_index(self.parent),[])
        for value in [values[0]|{'run':'../outside'},values[0]|{'artifacts_available':True}]:
            (self.parent/'expired-records.json').write_text(json.dumps([value]))
            with self.assertRaises(ValueError):records.read_index(self.parent)

    def test_completed_run_count_age_and_budget_preserve_current_proof(self):
        old=self.run_record(timestamp=100); new=self.run_record('bbbbbbbbbbbb',timestamp=200)
        with patch.object(records.time,'time',return_value=201):
            result=records.maintain(self.parent,[old,new],2,24,10000,False,EMPTY)
            self.assertTrue(all(row['retained'] for row in result))
            result=records.maintain(self.parent,[old,new],1,24,10000,False,EMPTY)
            self.assertEqual(result[1]['action'],'retire-completed-record')
            with self.assertRaisesRegex(ValueError,'current required'):records.maintain(self.parent,[new],1,24,1,False,EMPTY)
            records.maintain(self.parent,[old,new],2,.01,10000,True,EMPTY)
            self.assertTrue(new.exists());self.assertFalse(old.exists())
            records.maintain(self.parent,[new],2,24,10000,True,EMPTY)
            self.assertTrue(new.exists())
        with patch.object(records.time,'time',return_value=200+86401):
            records.maintain(self.parent,[new],2,24,10000,True,EMPTY)
            self.assertFalse(new.exists())
            self.assertFalse(records.read_index(self.parent)[0]['artifacts_available'])

    def test_registry_prunes_only_recorded_absent_captures_under_shared_lock(self):
        present=self.run_record(); absent=self.parent/'run-bbbbbbbbbbbb'
        unrelated=self.base/'other'
        value={'schema':'capture-cache-registry/v1','captures':[{'root':str(p)} for p in [present,absent,unrelated]]}
        path=self.state/'capture-caches.json';path.write_text(json.dumps(value))
        index=[{'run':present.name},{'run':absent.name}]
        records.unregister_expired(self.parent,index,False)
        self.assertEqual(json.loads(path.read_text()),value)
        records.unregister_expired(self.parent,index,True)
        self.assertEqual(json.loads(path.read_text())['captures'],[{'root':str(present)},{'root':str(unrelated)}])
        path.write_text('{"schema":"invalid"}')
        with self.assertRaisesRegex(ValueError,'invalid capture'):records.unregister_expired(self.parent,index,True)

    def pins(self,*runs):
        return EMPTY|{'pins':{str(i):{'run':str(run),'validation_id':'v','expires_at_ms':2**62} for i,run in enumerate(runs)}}

    def test_pinned_records_are_never_retired_and_are_charged_first(self):
        old=self.run_record(timestamp=100);new=self.run_record('bbbbbbbbbbbb',timestamp=200)
        (old/'evidence').write_bytes(b'x'*50);(new/'evidence').write_bytes(b'y'*50)
        pins=self.pins(old)
        with patch.object(records.time,'time',return_value=200+86401):
            result=records.maintain(self.parent,[old,new],2,24,10000,True,pins)
        self.assertTrue(old.exists());self.assertFalse(new.exists())
        self.assertEqual(result[1],{'run':str(old),'deduplicated_bytes':0,'retained':True,'pinned':True})
        middle=self.run_record('cccccccccccc',timestamp=300);newest=self.run_record('dddddddddddd',timestamp=400)
        (middle/'evidence').write_bytes(b'z'*50)
        with patch.object(records.time,'time',return_value=401):
            # The pinned run, the current record and the pin state leave no room for the older unpinned record.
            budget=records.charged(pins,[old,newest,middle])-1
            result=records.maintain(self.parent,[old,middle,newest],3,24,budget,False,pins)
        self.assertEqual([row.get('action') for row in result],[None,'retire-completed-record',None])
        self.assertEqual(result[0]['run'],str(newest))

    def test_pins_and_a_newer_current_record_that_exceed_the_budget_fail_before_any_change(self):
        old=self.run_record(timestamp=100);(old/'evidence').write_bytes(b'x'*500)
        pins=self.pins(old)
        # The pin fitted alone; a newer current record produced afterwards no longer fits beside it.
        budget=records.evidence_pins.reserved(pins,[])
        new=self.run_record('bbbbbbbbbbbb',timestamp=200);(new/'evidence').write_bytes(b'y'*500)
        self.assertGreater(records.evidence_pins.reserved(pins,[new]),budget)
        with patch.object(records.time,'time',return_value=201):
            with self.assertRaisesRegex(ValueError,'pins and current required evidence need'):
                records.maintain(self.parent,[old,new],2,24,budget,True,pins)
            self.assertTrue(old.exists() and new.exists())
            self.assertEqual(records.read_index(self.parent),[])
            records.maintain(self.parent,[old,new],2,24,records.evidence_pins.reserved(pins,[new]),True,pins)
        self.assertTrue(old.exists() and new.exists())

    def test_hard_linked_reports_are_charged_once(self):
        run=self.run_record();(run/'a').write_bytes(b'x'*100);os.link(run/'a',run/'b')
        self.assertEqual(records.charged(EMPTY,[run,run]),records.charged(EMPTY,[run]))
        self.assertEqual(records.charged(EMPTY,[run])-records.evidence_pins.state_bytes(EMPTY),
                         sum(p.stat().st_size for p in run.iterdir() if p.name!='b'))

    def test_pinned_retirement_and_resumed_deletion_are_refused(self):
        run=self.run_record();index=[]
        with self.assertRaisesRegex(ValueError,'pinned record cannot'):records.retire(run,self.parent,index,frozenset({run}),True)
        self.assertTrue(run.exists())
        records.retire(run,self.parent,index,NONE,False)
        retiring=self.parent/('retiring-'+run.name);run.rename(retiring)
        index=[{'run':run.name,'expired_at':1,'artifacts_available':False}]
        with self.assertRaisesRegex(ValueError,'pinned record is being retired'):
            records.resume_deletions(self.parent,index,frozenset({run}),True)
        self.assertTrue(retiring.exists())

    def test_older_records_must_fit_and_the_current_record_expires_only_by_count_and_age(self):
        self.assertTrue(records.kept(0,0,False,1,24))
        self.assertFalse(records.kept(1,0,False,2,24))
        self.assertTrue(records.kept(1,0,True,2,24))
        self.assertFalse(records.kept(0,86400,True,1,24))
        self.assertFalse(records.kept(1,0,True,1,24))


if __name__=='__main__':unittest.main()
