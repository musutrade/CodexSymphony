"""Real PG16 encrypted cold backup, mTLS object transport and guarded service restart.
Uses only newly created databases in TEST_DATABASE_URL and temporary materials.
"""
import contextlib
import copy
import hashlib
import http.server
import importlib.util
import json
import os
from pathlib import Path
import select
import shutil
import signal
import sqlite3
import ssl
import subprocess
import sys
import tempfile
import threading
import time
import unittest
import urllib.error
import urllib.parse
import urllib.request
import uuid

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT/'apps/recovery'))
import backup
from common import Refused, digest, environment, facts, private, run, sql, write_json
from crypto import decrypt, encrypt, key

spec = importlib.util.spec_from_file_location('bark_fixture', ROOT/'apps/notifier/bark.py')
bark = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bark)


class ObjectStore(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_PUT(self):
        data = self.rfile.read(int(self.headers['Content-Length']))
        self.server.objects[self.path] = data
        self.send_response(self.server.put_status); self.end_headers()

    def do_GET(self):
        data = self.server.objects[self.path]
        self.send_response(200); self.end_headers()
        self.wfile.write(data + (b'corrupt' if self.server.corrupt else b''))

    def do_DELETE(self):
        del self.server.objects[self.path]
        self.send_response(204); self.end_headers()


@contextlib.contextmanager
def service(url, root, drill=False):
    env = {'PATH': os.environ['PATH'], 'DATABASE_URL': url, 'WEB_ORIGIN': 'https://localhost:4200',
           'AUTH_CONFIG': str(root/'auth.json'), 'BIND_ADDRESS': '127.0.0.1:0',
           'EXECUTION_DIRECTORY': str(root/'service-execution'), 'RUST_LOG': 'info'}
    # Poisoned integration configs prove the drill never initializes these adapters.
    if drill:
        env.update({k: '/intentionally-unavailable-recovery-config' for k in ('RUNTIME_CONFIG','STORAGE_CONFIG','GITHUB_CONFIG')})
    args = [str(ROOT/'target/debug/codexsymphony-server')]
    if drill:
        args.append('--recovery-drill')
    child = subprocess.Popen(args, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, bufsize=0)
    try:
        deadline = time.monotonic()+30
        address = None
        while time.monotonic()<deadline:
            if select.select([child.stdout], [], [], .2)[0]:
                line = child.stdout.readline().decode()
                if 'listening at ' in line:
                    address = line.split('listening at ')[1].strip().removeprefix('http://')
                    break
            if child.poll() is not None:
                break
        if not address:
            raise AssertionError('isolated service did not start (diagnostics withheld)')
        yield address
    finally:
        if child.poll() is None:
            child.send_signal(signal.SIGINT)
        try:
            child.wait(timeout=10)
        except subprocess.TimeoutExpired:
            child.kill(); child.wait()
        child.stdout.close(); child.stderr.close()


class RecoveryTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        os.umask(0o077)
        cls.temp = tempfile.TemporaryDirectory(prefix='gh75-controlled-')
        cls.root = Path(cls.temp.name)
        cls.admin_uri = os.environ['TEST_DATABASE_URL']
        cls.admin = environment(cls.admin_uri)
        cls.names = []
        cls.addClassCleanup(cls.cleanup_resources)
        cls.source = cls.database('symphony_backup_')
        cls.env = environment(cls.source)
        write_json(cls.root/'auth.json', {'public_origin':'https://localhost:4200','trusted_proxies':[]})
        with service(cls.source, cls.root):
            pass
        sql(cls.env, (ROOT/'api/capture-fixture.sql').read_text())
        sql(cls.env, """
        UPDATE execution_control SET paused=true,requirement_id=900001;
        UPDATE agent_run SET phase='validation' WHERE id='capture-run';
        UPDATE requirement SET cancel_requested=true WHERE id=900001;
        INSERT INTO requirement(version,state,contract,paused,cancel_requested,cleanup_complete) VALUES(1,'Cancelled','{}',true,true,true);
        INSERT INTO imported_draft VALUES('draft-recovery',1,
          '{"schema":"codexsymphony-draft/v1","parent":{"id":"P1","acceptance_criteria":[{"id":"P-AC1","description":"whole flow"}]},"children":[{"id":"C1","parent_id":"P1","depends_on":[],"acceptance_criteria":[{"id":"AC1","description":"first"}]},{"id":"C2","parent_id":"P1","depends_on":["C1"],"acceptance_criteria":[{"id":"AC2","description":"second"}]}]}',
          '{"format":"json","label":"synthetic recovery fixture"}','fixture',now(),now());
        INSERT INTO imported_draft_revision SELECT id,version,document,source,source_sha256,now() FROM imported_draft;
        INSERT INTO group_review VALUES('draft-recovery',1,1,'{"coverage":[{"parent_ac":"P-AC1","child_id":"C2","child_ac":"AC2"}]}');
        INSERT INTO group_review_revision SELECT draft_id,version,draft_revision,document,now() FROM group_review;
        INSERT INTO group_authorization(draft_id,review_version,request_id,input,snapshot) VALUES('draft-recovery',1,'recovery-reviewed','{"version":1}','{"authorization":"exact synthetic revision"}');
        INSERT INTO group_queue SELECT 'draft-recovery',id,'waiting_scheduler',1,now() FROM group_authorization;
        INSERT INTO group_budget VALUES('draft-recovery','','{"tokens":500,"turns":10,"model_seconds":120}','{"tokens":71,"turns":1,"model_seconds":9}','{"tokens":29,"turns":1,"model_seconds":4}');
        INSERT INTO requirement_budget(requirement_id,limits) VALUES(900001,'{"tokens":200,"turns":5,"model_seconds":60}');
        INSERT INTO model_call(run_id,turn_id,requirement_id,intent,reserved,usage) VALUES('capture-run','turn-1',900001,'{}','{"tokens":29,"turns":1,"model_seconds":4}','{"tokens":71,"turns":1,"model_seconds":9}');
        INSERT INTO runtime_resume VALUES('capture-run','{"stage":"validation","source":"capture-run"}','restoring');
        INSERT INTO workspace_operation VALUES('capture-run','save-fixture','{"operation":"archive"}','partial',null,'synthetic partial save');
        INSERT INTO workspace_snapshot VALUES('capture-run','{"entries":["tracked","untracked","index","evidence"]}',false);
        INSERT INTO platform_account VALUES('fixture-user','synthetic-noncredential-password-hash');
        INSERT INTO platform_session VALUES('synthetic-revoked-digest','fixture-user',4102444800,true),('synthetic-expired-digest','fixture-user',1,false),('synthetic-live-digest','fixture-user',4102444800,false);
        INSERT INTO platform_login_limit VALUES('account','synthetic-account-digest',1789948800,8),('source','synthetic-source-digest',1789948800,40);
        """)
        cls.materials = {}
        for label in ('execution','cold','configuration','notifier'):
            path = cls.root/label; path.mkdir(mode=0o700)
            (path/'sample').write_text('synthetic '+label+' material\n')
            cls.materials[label] = {'path': str(path), 'identity': [path.stat().st_dev,path.stat().st_ino]}
        execution = cls.root/'execution'
        run(['git','init','--template=',str(execution/'worktree')])
        work = execution/'worktree'
        (work/'tracked').write_text('committed\n')
        run(['git','-C',str(work),'add','.'])
        run(['git','-C',str(work),'-c','user.name=fixture','-c','user.email=fixture@example.invalid','commit','-m','synthetic saved work'])
        (work/'tracked').write_text('staged\n'); run(['git','-C',str(work),'add','.'])
        (work/'tracked').write_text('unstaged\n'); (work/'untracked').write_text('unique work\n')
        (execution/'link').symlink_to('worktree/tracked')
        os.link(work/'untracked', execution/'hardlink')
        (cls.root/'notifier/worker.lock').touch(mode=0o600)
        actions = json.loads(sql(cls.env, "SELECT json_agg(n) FROM notification_action n"))
        db = bark.ledger(cls.root/'notifier')
        bark.synchronize(db, actions, int(time.time()))
        db.execute("UPDATE delivery SET attempts=1,result='unknown',next_attempt=next_attempt+30")
        db.commit(); db.close()
        (cls.root/'controller.lock').touch(mode=0o600)
        (cls.root/'key').write_bytes(os.urandom(32))
        refs = {name:'administrator-custody:'+name for name in ('database','auth','github','signing','bark','backup_key')}
        write_json(cls.root/'references.json', refs)
        (cls.root/'destination').mkdir(mode=0o700)
        # Separate installed executable identity; no real host binary or secrets read.
        cls.binary = ROOT/'target/debug/codexsymphony-server'
        cls.binary.chmod(0o700)
        cls.cfg = {'enabled':True,'fixture':True,'database':cls.source,
          'controller_lock':str(cls.root/'controller.lock'),'notifier_lock':str(cls.root/'notifier/worker.lock'),
          'units':[], 'roots':cls.materials,'destination':str(cls.root/'destination'),
          'key_file':str(cls.root/'key'),'recovery_references':str(cls.root/'references.json'),
          'source_sha':run(['git','rev-parse','HEAD']).decode().strip(), 'binary':str(cls.binary),
          'binary_sha256':digest(cls.binary),'retain_count':2,'max_bytes':64<<20,'offsite':None}
        cls.setup_tls()

    @classmethod
    def database(cls, prefix):
        name = prefix+uuid.uuid4().hex
        sql(cls.admin, 'CREATE DATABASE '+name)
        cls.names.append(name)
        url = urllib.parse.urlsplit(cls.admin_uri)
        return urllib.parse.urlunsplit(url._replace(path='/'+name))

    @classmethod
    def setup_tls(cls):
        from cryptography import x509
        from cryptography.hazmat.primitives import hashes, serialization
        from cryptography.hazmat.primitives.asymmetric import rsa
        from cryptography.x509.oid import NameOID, ExtendedKeyUsageOID
        import datetime, ipaddress
        secret = rsa.generate_private_key(public_exponent=65537,key_size=2048)
        subject = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME,'GH75 disposable fixture')])
        now = datetime.datetime.now(datetime.timezone.utc)
        cert = (x509.CertificateBuilder().subject_name(subject).issuer_name(subject).public_key(secret.public_key())
                .serial_number(x509.random_serial_number()).not_valid_before(now-datetime.timedelta(minutes=1))
                .not_valid_after(now+datetime.timedelta(days=1)).add_extension(x509.BasicConstraints(ca=True,path_length=None),True)
                .add_extension(x509.SubjectAlternativeName([x509.IPAddress(ipaddress.ip_address('127.0.0.1'))]),False)
                .add_extension(x509.ExtendedKeyUsage([ExtendedKeyUsageOID.SERVER_AUTH,ExtendedKeyUsageOID.CLIENT_AUTH]),False)
                .sign(secret,hashes.SHA256()))
        (cls.root/'tls.pem').write_bytes(cert.public_bytes(serialization.Encoding.PEM))
        (cls.root/'tls.key').write_bytes(secret.private_bytes(serialization.Encoding.PEM,serialization.PrivateFormat.PKCS8,serialization.NoEncryption()))
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain(cls.root/'tls.pem', cls.root/'tls.key')
        context.load_verify_locations(cls.root/'tls.pem'); context.verify_mode=ssl.CERT_REQUIRED
        cls.http = http.server.ThreadingHTTPServer(('127.0.0.1',0),ObjectStore)
        cls.http.socket = context.wrap_socket(cls.http.socket,server_side=True)
        cls.http.objects={}; cls.http.corrupt=False; cls.http.put_status=201
        cls.thread=threading.Thread(target=cls.http.serve_forever,daemon=True); cls.thread.start()
        cls.offsite={'url':f'https://127.0.0.1:{cls.http.server_port}/restricted',
          'ca':str(cls.root/'tls.pem'),'certificate':str(cls.root/'tls.pem'),'private_key':str(cls.root/'tls.key'),
          'fixture':True,'max_object_bytes':64<<20}

    @classmethod
    def cleanup_resources(cls):
        if hasattr(cls, 'http'):
            cls.http.shutdown(); cls.thread.join(); cls.http.server_close()
        for name in cls.names:
            sql(cls.admin, 'DROP DATABASE '+name+' WITH (FORCE)')
        cls.temp.cleanup()

    def test_01_backup_transport_restore_restart(self):
        cfg = copy.deepcopy(self.cfg); cfg['offsite']=self.offsite
        config = self.root/'backup.json'; write_json(config,cfg)
        self.assertEqual(backup.configured(config),cfg)
        original = facts(self.env)
        result = backup.backup(cfg)
        self.assertEqual(result['offsite'],'fixture_https_verified')
        archive = self.root/'destination'/result['archive']
        self.assertEqual(backup.verify(cfg,archive)['status'],'verified')
        self.assertNotIn(b'synthetic-revoked-digest',archive.read_bytes())
        restored_uri = self.database('symphony_restore_')
        target = self.root/'restored'; target.mkdir(mode=0o700)
        target_cfg = self.root/'target.json'; write_json(target_cfg,{'database':restored_uri,'directory':str(target)})
        self.assertEqual(backup.restore(cfg,archive,target_cfg)['status'],'isolated_restore_verified')
        restored = environment(restored_uri)
        self.assertEqual(facts(restored),original)
        self.assertEqual(sql(restored,"SELECT count(*) FROM platform_session WHERE digest IN ('synthetic-revoked-digest','synthetic-expired-digest') AND NOT revoked AND expires_at>extract(epoch FROM now())"),'0')
        source_git = run(['git','-C',str(self.root/'execution/worktree'),'status','--porcelain'])
        restored_git = run(['git','-C',str(target/'materials/execution/worktree'),'status','--porcelain'])
        self.assertEqual(source_git,restored_git)
        db = sqlite3.connect(target/'materials/notifier/delivery.sqlite3')
        before=db.execute('SELECT * FROM delivery ORDER BY action_key').fetchall(); db.close()
        self.assertGreater(len(before),0)
        restored_actions=json.loads(sql(restored, 'SELECT json_agg(n) FROM notification_action n'))
        ledger=bark.ledger(target/'materials/notifier')
        bark.synchronize(ledger,restored_actions,min(row[3] for row in before)+1)
        after=[tuple(row) for row in ledger.execute('SELECT * FROM delivery ORDER BY action_key')]
        ledger.close()
        self.assertEqual(before,after)
        for _ in range(2):
            with service(restored_uri,self.root,True) as address:
                report=json.load(urllib.request.urlopen('http://'+address+'/api/recovery'))
                self.assertFalse(report['external_actions'])
                self.assertEqual(report['facts']['revoked_sessions'],1)
                for path in ('/api/requirements','/api/auth/login','/api/recovery'):
                    request=urllib.request.Request('http://'+address+path,data=b'{}',method='POST')
                    with self.assertRaises(urllib.error.HTTPError) as error:
                        urllib.request.urlopen(request)
                    self.assertIn(error.exception.code,(404,405))
                    error.exception.close()
            self.assertEqual(facts(restored),original)
        env={**os.environ,'DATABASE_URL':restored_uri,'WEB_ORIGIN':'https://localhost:4200'}
        rejected=subprocess.run([str(self.binary)],env=env,capture_output=True,timeout=15)
        self.assertNotEqual(rejected.returncode,0)
        self.assertIn(b'normal startup forbidden',rejected.stderr)
        with self.assertRaises(Refused): backup.restore(cfg,archive,target_cfg)
        self.assertEqual(facts(restored),original)
        record={'schema':'gh75-controlled-recovery/v1','source_baseline':cfg['source_sha'],
                'binary_sha256':cfg['binary_sha256'],'database_tables_and_sequences':len(original),
                'facts_preserved':list(original),'transport':'real mTLS HTTPS PUT + checksum GET',
                'data':'synthetic only','restarts':2,'write_routes':'404/405','normal_startup':'refused before migration',
                'encrypted_archive_sha256':result['sha256'],'production_offsite':'not configured or exercised'}
        out=ROOT/'artifacts/gh75/recovery-acceptance.json'
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(json.dumps(record,indent=2)+'\n')

    def test_02_failure_paths_and_retention(self):
        cfg=copy.deepcopy(self.cfg)
        result=backup.backup(cfg)
        self.assertEqual(result['offsite'],'not_configured')
        archive=self.root/'destination'/result['archive']
        corrupted=self.root/'corrupted.enc'; data=bytearray(archive.read_bytes()); data[len(data)//2]^=1; corrupted.write_bytes(data)
        with self.assertRaises(Exception): backup.verify(cfg,corrupted)
        missing=self.root/'missing.enc'; missing.write_bytes(archive.read_bytes()[:10])
        with self.assertRaises(Refused): backup.verify(cfg,missing)
        # Authenticated but incomplete material archive must fail too.
        with tempfile.TemporaryDirectory(dir=self.root) as temporary:
            directory=Path(temporary)
            extracted,manifest=backup.unpack(archive,directory,key(cfg['key_file']))
            (extracted/'materials/execution/sample').unlink()
            import tarfile
            payload=directory/'incomplete.tar'
            with tarfile.open(payload,'w') as tar:
                for p in extracted.iterdir(): tar.add(p,arcname=p.name)
            bad=directory/'incomplete.enc'; encrypt(payload,bad,key(cfg['key_file']))
            with self.assertRaises(Refused): backup.verify(cfg,bad)
        original=digest(archive)
        old_key=(self.root/'key').read_bytes()
        (self.root/'key').write_bytes(os.urandom(32))
        try:
            with self.assertRaises(Exception): backup.verify(cfg,archive)
        finally: (self.root/'key').write_bytes(old_key)
        unreadable=self.root/'execution/sample'
        unreadable.chmod(0)
        try:
            with self.assertRaises(Refused): backup.backup(cfg)
        finally: unreadable.chmod(0o600)
        unavailable=copy.deepcopy(cfg)
        url=urllib.parse.urlsplit(cfg['database'])
        unavailable['database']=urllib.parse.urlunsplit(url._replace(path='/symphony_backup_missing_'+uuid.uuid4().hex))
        with self.assertRaises(Refused): backup.backup(unavailable)
        with backup.database_snapshot(self.env):
            with self.assertRaises(Refused): sql(self.env, 'UPDATE requirement SET paused=false WHERE id=900001')
        with backup.locked(cfg['controller_lock']):
            with self.assertRaises(BlockingIOError): backup.backup(cfg)
        with backup.locked(cfg['notifier_lock']):
            with self.assertRaises(BlockingIOError): backup.backup(cfg)
        self.assertEqual(digest(archive),original)
        for mutate in ('permission','missing','identity','budget','binary','enabled'):
            changed=copy.deepcopy(cfg)
            path=self.root/'case.json'
            if path.exists(): path.unlink()
            if mutate=='permission':
                (self.root/'key').chmod(0o644)
            if mutate=='missing': changed['roots']['cold']['path']=str(self.root/'absent')
            if mutate=='identity': changed['roots']['cold']['identity']=[0,0]
            if mutate=='budget': changed['max_bytes']=1
            if mutate=='binary': changed['binary_sha256']='0'*64
            if mutate=='enabled': changed['enabled']=False
            write_json(path,changed)
            try:
                with self.assertRaises((Refused,FileNotFoundError)):
                    backup.backup(backup.configured(path))
            finally: (self.root/'key').chmod(0o600)
        cfg['offsite']=self.offsite
        self.http.corrupt=True
        try:
            with self.assertRaises(Refused): backup.backup(cfg)
        finally: self.http.corrupt=False
        self.http.put_status=503
        try:
            with self.assertRaises(Refused): backup.backup(cfg)
        finally: self.http.put_status=201
        # Failed uploads retain encrypted local material and precise failed receipt.
        receipts=[json.loads(p.read_text()) for p in (self.root/'destination').glob('*.receipt.json')]
        self.assertGreaterEqual(sum(r['offsite']=='failed' for r in receipts),2)
        backup.backup(cfg)
        self.assertEqual(len(list((self.root/'destination').glob('*.enc'))),2)
        self.assertEqual(len(list((self.root/'destination').glob('*.receipt.json'))),2)
        self.assertLessEqual(len(self.http.objects),2)


    def test_03_cli_disabled_and_private_failures(self):
        config=self.root/'disabled.json'
        write_json(config,{'enabled':False})
        command=[sys.executable,str(ROOT/'apps/recovery/backup.py'),'--config',str(config)]
        result=subprocess.run(command+['status'],capture_output=True,text=True,timeout=10)
        self.assertEqual(result.returncode,0)
        self.assertEqual(json.loads(result.stdout),{'status':'disabled','offsite':'not_configured'})
        result=subprocess.run(command+['backup'],capture_output=True,text=True,timeout=10)
        self.assertEqual(result.returncode,1)
        config.write_text('{"private_input":"synthetic-never-log-this-value", invalid')
        result=subprocess.run(command+['status'],capture_output=True,text=True,timeout=10)
        self.assertEqual(result.returncode,1)
        self.assertNotIn('synthetic-never-log-this-value',result.stdout+result.stderr)


if __name__=='__main__':
    unittest.main(verbosity=2)
