"""Synthetic keys, actual loopback HTTP, SQLite restart/concurrency and PG facts."""
import concurrent.futures
import contextlib
import hashlib
import http.server
import importlib.util
import json
import os
from pathlib import Path
import secrets
import shutil
import sqlite3
import subprocess
import tempfile
import threading
import time
import unittest
import urllib.parse
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT/'apps/notifier/bark.py'
spec = importlib.util.spec_from_file_location('bark', SCRIPT)
bark = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bark)


class Receiver(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        raw = self.rfile.read(int(self.headers['Content-Length']))
        self.server.received.append((self.path, dict(self.headers), json.loads(raw)))
        if self.server.delay:
            time.sleep(self.server.delay)
        self.send_response(self.server.status)
        if self.server.status == 302:
            self.send_header('Location', self.server.redirect)
        self.end_headers()
        with contextlib.suppress(BrokenPipeError, ConnectionResetError):
            self.wfile.write(json.dumps({'code': self.server.status}).encode())


class BarkTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='bark-synthetic-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Receiver)
        self.server.received = []
        self.server.status = 200
        self.server.delay = 0
        self.server.redirect = 'http://127.0.0.1:9/leaked'
        thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        thread.start()
        self.addCleanup(self.server.server_close)
        self.addCleanup(self.server.shutdown)
        self.cfg = dict(psql_program=shutil.which('psql'), enabled=True, application_origin='https://platform.example.invalid',
                        endpoint=f'http://127.0.0.1:{self.server.server_port}/push',
                        device_key='synthetic-fixture-only-' + secrets.token_hex(8),
                        database='postgresql://fixture@127.0.0.1/fixture', state_directory=str(self.root), local_fixture=True)
        self.path = self.root/'bark.json'
        self.save()
        self.item = dict(requirement_id=74, kind='question', action_key=hashlib.sha256(b'question1').hexdigest())

    def save(self):
        self.path.write_text(json.dumps(self.cfg))
        self.path.chmod(0o600)

    def test_lifecycle_actionability_includes_environment_preparation_and_blockers(self):
        for phase, status in [('environment', {'passed': False}), ('preparation', {'todo': True}),
                              ('blocker', {'resolved': False}), ('hook', {'status': 'unknown'})]:
            self.assertEqual(bark.event_kind({'phase': phase, 'facts': {'status': status}}), 'failed')
        self.assertEqual(bark.event_kind({'phase': 'environment', 'facts': {'status': {'passed': True}}}), 'progress')

    def test_lifecycle_ingress_is_durable_and_channel_policy_is_plugin_owned(self):
        self.cfg['event_policy'] = 'actionable'
        del self.cfg['database']
        del self.cfg['psql_program']
        self.save()
        event = dict(protocol_version=1, event_id=1, attempt=1, requirement_id=74,
                     phase='requirement', facts={'status': {'state': 'Done'}})
        self.assertEqual(bark.receive_event(self.cfg, event)['status'], 'ignored')
        self.cfg['event_policy'] = 'all'
        self.save()
        # A received event retains its original disposition after policy changes.
        self.assertEqual(bark.receive_event(self.cfg, dict(event, attempt=2))['status'], 'ignored')
        event['event_id'] = 2
        self.assertEqual(bark.receive_event(self.cfg, event)['status'], 'accepted')
        self.assertEqual(bark.receive_event(self.cfg, dict(event, attempt=2))['status'], 'accepted')
        bark.tick_events(self.path, self.cfg)
        bark.tick_events(self.path, self.cfg)
        self.assertEqual(len(self.server.received), 1)
        self.assertEqual(self.server.received[0][2]['body'], '需求业务验收已完成')
        with self.assertRaises(ValueError):
            bark.receive_event(self.cfg, dict(event, requirement_id=75))
        self.assertNotIn(self.cfg['device_key'].encode(), (self.root/'delivery.sqlite3').read_bytes())

    def test_disabled_and_invalid_configuration(self):
        result = subprocess.run(['python3', '-I', str(SCRIPT)], capture_output=True, text=True, check=True)
        self.assertEqual(result.stdout.strip(), 'disabled')
        for key, value in [('application_origin','https://bad.invalid/path'),
                           ('endpoint','https://api.day.app/secret'),
                           ('endpoint','https://api.day.app/push?key=secret'),
                           ('local_fixture',False), ('device_key','')]:
            cfg = dict(self.cfg)
            self.cfg[key] = value
            self.save()
            with self.assertRaises(ValueError):
                bark.configuration(self.path)
            self.cfg = cfg
        self.save()
        self.path.chmod(0o644)
        with self.assertRaises(ValueError):
            bark.configuration(self.path)

    def test_json_post_and_no_redirect_or_secret_in_result(self):
        self.assertEqual(bark.deliver(self.path,self.item), 'accepted')
        path, headers, body = self.server.received[0]
        self.assertEqual(path, '/push')
        self.assertEqual(headers['Content-Type'], 'application/json')
        self.assertEqual(body['device_key'], self.cfg['device_key'])
        self.assertEqual(body['url'], 'https://platform.example.invalid/requirements/74')
        self.assertNotIn(self.cfg['device_key'], path + json.dumps(headers))
        self.server.status = 302
        self.assertEqual(bark.deliver(self.path,self.item), 'rejected')
        self.assertEqual(len(self.server.received), 2)
        self.server.status = 500
        self.assertEqual(bark.deliver(self.path,self.item), 'retryable_http')

    def test_restart_dedup_new_actions_and_no_progress_push(self):
        with patch.object(bark, 'actions', return_value=[self.item]):
            self.assertEqual(bark.tick(self.path,self.cfg), 'checked')
            for _ in range(3):
                bark.tick(self.path,self.cfg)
        self.assertEqual(len(self.server.received), 1)
        other = dict(self.item, action_key=hashlib.sha256(b'question2').hexdigest())
        with patch.object(bark, 'actions', return_value=[self.item,other]):
            bark.tick(self.path,self.cfg)
        self.assertEqual(len(self.server.received), 2)
        with patch.object(bark, 'actions', return_value=[]):
            bark.tick(self.path,self.cfg)
        self.assertEqual(len(self.server.received), 2)
        self.assertNotIn(self.cfg['device_key'].encode(), (self.root/'delivery.sqlite3').read_bytes())

    def test_concurrency_claims_one_send(self):
        self.server.delay = .2
        with patch.object(bark, 'actions', return_value=[self.item]):
            with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
                results = list(pool.map(lambda _: bark.tick(self.path,self.cfg), range(4)))
        self.assertIn('checked', results)
        self.assertEqual(len(self.server.received), 1)

    def test_finite_retry_backoff_deadline_and_obsolete(self):
        self.server.status = 500
        with patch.object(bark, 'actions', return_value=[self.item]):
            bark.tick(self.path,self.cfg)
            db = bark.ledger(self.root)
            first = dict(db.execute('SELECT * FROM delivery').fetchone())
            self.assertEqual(first['next_attempt']-first['first_seen'],30)
            bark.tick(self.path,self.cfg)
            self.assertEqual(len(self.server.received),1)
            for _ in range(2):
                with db:
                    db.execute('UPDATE delivery SET next_attempt=0')
                bark.tick(self.path,self.cfg)
            self.assertEqual(len(self.server.received),3)
            self.assertEqual(db.execute('SELECT state,attempts FROM delivery').fetchone()[:],('failed',3))
            bark.tick(self.path,self.cfg)
            self.assertEqual(len(self.server.received),3)
            second = dict(self.item,action_key=hashlib.sha256(b'deadline').hexdigest())
            bark.synchronize(db,[second],int(time.time())-700)
            bark.synchronize(db,[second],int(time.time()))
            self.assertEqual(db.execute('SELECT state FROM delivery WHERE action_key=?',(second['action_key'],)).fetchone()[0],'failed')
            third = dict(self.item,action_key=hashlib.sha256(b'obsolete').hexdigest())
            bark.synchronize(db,[third],int(time.time()))
            bark.synchronize(db,[],int(time.time()))
            self.assertEqual(db.execute('SELECT state FROM delivery WHERE action_key=?',(third['action_key'],)).fetchone()[0],'obsolete')
            db.close()

    def test_timeout_is_unknown_and_crash_claim_survives_restart(self):
        self.server.delay = 4
        # Actual receiver accepts body; shorten only the test parent's wall clock.
        with patch.object(bark,'TIMEOUT',2):
            self.assertEqual(bark.deliver(self.path,self.item),'unknown')
        self.assertEqual(len(self.server.received),1)
        db = bark.ledger(self.root)
        now = int(time.time())
        bark.synchronize(db,[self.item],now)
        with db:
            db.execute("UPDATE delivery SET state='sending',attempts=1,next_attempt=?,result='unknown'",(now+30,))
        db.close()
        with patch.object(bark,'actions',return_value=[self.item]):
            bark.tick(self.path,self.cfg)
        self.assertEqual(len(self.server.received),1)


class ProjectionTests(unittest.TestCase):
    setUp = BarkTests.setUp
    save = BarkTests.save

    def test_real_postgres_commits_versions_and_readonly_privileges(self):
        base = os.environ['TEST_DATABASE_URL']
        schema = 'bark_' + secrets.token_hex(8)
        role = schema + '_reader'
        def sql(statement, scoped=True, check=True):
            prefix = f'SET search_path TO {schema}; ' if scoped else ''
            return subprocess.run([shutil.which('psql'),'-XAt','-v','ON_ERROR_STOP=1'], input=prefix+statement,
                text=True,capture_output=True,check=check,env={**bark.database_environment(base),'PGOPTIONS':''})
        sql('CREATE SCHEMA '+schema,False)
        try:
            for migration in sorted((ROOT/'migrations').glob('*.sql')):
                sql(migration.read_text())
            sql("INSERT INTO requirement(version,state,contract,paused) VALUES(1,'Draft','{}',true)")
            sql("INSERT INTO business_event(object_id,kind,version) VALUES('requirement:1','operator_pause',1)")
            parts = urllib.parse.urlsplit(base)
            query = urllib.parse.parse_qsl(parts.query) + [('options','-csearch_path='+schema)]
            self.cfg['database'] = urllib.parse.urlunsplit(parts._replace(query=urllib.parse.urlencode(query)))
            self.save()
            def tick_process():
                return subprocess.run(['python3','-I',str(SCRIPT),'--config',str(self.path)],
                                      capture_output=True,text=True,check=True)
            # Separate OS processes share only the durable ledger and committed DB.
            tick_process()
            tick_process()
            self.assertEqual(len(self.server.received),1)
            with concurrent.futures.ThreadPoolExecutor(max_workers=3) as pool:
                list(pool.map(lambda _:tick_process(),range(3)))
            self.assertEqual(len(self.server.received),1)
            self.assertEqual(len(bark.actions(self.cfg)),1)
            before = sql('SELECT action_key FROM notification_action').stdout.splitlines()[-1]
            sql("INSERT INTO business_event(object_id,kind,version) VALUES('requirement:1','operator_pause',1),('requirement:1','progress',100)")
            self.assertEqual(before,sql('SELECT action_key FROM notification_action').stdout.splitlines()[-1])
            tick_process()
            self.assertEqual(len(self.server.received),1)
            sql("UPDATE requirement SET version=2")
            after = sql('SELECT action_key FROM notification_action').stdout.splitlines()[-1]
            self.assertEqual(before,after)
            sql('BEGIN; UPDATE requirement SET revision=1; ROLLBACK;')
            self.assertEqual(before,sql('SELECT action_key FROM notification_action').stdout.splitlines()[-1])
            sql('UPDATE requirement SET revision=1')
            self.assertNotEqual(before,sql('SELECT action_key FROM notification_action').stdout.splitlines()[-1])
            sql("UPDATE requirement SET paused=false")
            self.assertEqual(sql('SELECT count(*) FROM notification_action').stdout.splitlines()[-1],'0')
            sql("INSERT INTO requirement_revision VALUES(1,1,'{}'); INSERT INTO agent_run(id,requirement_id,revision,incarnation,request_id,workspace,workspace_identity,launch,state) VALUES('run',1,1,'fixture','request','fixture','fixture','{}','Failed'); INSERT INTO runtime_session(run_id,created_at,last_progress) VALUES('run',1,1)")
            sql("INSERT INTO runtime_question(id,requirement_id,revision,run_id,rpc_id,original,created_at) VALUES('q1',1,1,'run','1','{}',1),('q2',1,1,'run','2','{}',2)")
            old = {x['action_key'] for x in bark.actions(self.cfg)}
            self.assertEqual(len(old),2)
            tick_process()
            self.assertEqual(len(self.server.received),3)
            sql("UPDATE runtime_question SET version=2 WHERE id='q1'")
            new = {x['action_key'] for x in bark.actions(self.cfg)}
            self.assertEqual(len(new & old),1)
            tick_process()
            self.assertEqual(len(self.server.received),4)
            sql("UPDATE runtime_question SET resume_state='invalid'")
            self.assertEqual(bark.actions(self.cfg),[])
            tick_process()
            self.assertEqual(len(self.server.received),4)
            self.assertNotIn(self.cfg['device_key'].encode(),(self.root/'delivery.sqlite3').read_bytes())
            sql(f'CREATE ROLE {role}; GRANT USAGE ON SCHEMA {schema} TO {role}; GRANT SELECT ON notification_action TO {role}')
            sql(f'SET ROLE {role}; SELECT * FROM notification_action')
            self.assertNotEqual(sql(f'SET ROLE {role}; SELECT * FROM requirement',check=False).returncode,0)
            self.assertNotEqual(sql(f'SET ROLE {role}; UPDATE requirement SET paused=true',check=False).returncode,0)
        finally:
            sql('DROP SCHEMA '+schema+' CASCADE',False)
            sql('DROP ROLE IF EXISTS '+role,False)


if __name__ == '__main__':
    unittest.main()
