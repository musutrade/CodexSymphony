"""B03/B07 real service restarts + HTTPS Chromium with a scripted Runtime peer.

Only an isolated schema of TEST_DATABASE_URL and owned temporary files are used.
The pinned Codex protocol test is separate; this fixture makes no model-quality,
real GitHub, signed Gate or production deployment claim.
"""
from contextlib import ExitStack
import base64
import hashlib
import json
import os
from pathlib import Path
import secrets
import signal
import subprocess
import tempfile
import time
import urllib.parse

from auth_browser_acceptance import BrowserRelay
from auth_contract_acceptance import ROOT, sql, wait_http_address, TLSCapture

OUT = ROOT / 'artifacts/gh73/recovery'


def query(url, statement):
    result = subprocess.run(['psql', url, '-XAt', '-v', 'ON_ERROR_STOP=1', '-c', statement],
                            capture_output=True, text=True, check=True)
    return result.stdout.strip()


def wait(predicate, description, seconds=40):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(.2)
    raise AssertionError('Timed out: ' + description)


def owned(path):
    stat = path.stat()
    return {'path': str(path), 'identity': {'device': stat.st_dev, 'inode': stat.st_ino}}


def configuration(root, baseline):
    execution, cold = root/'execution', root/'cold'
    execution.mkdir(); cold.mkdir()
    workspace = execution/'workspaces'
    workspace.mkdir()
    for name in ['runs', 'archives']:
        (workspace/name).mkdir()
    subprocess.run(['git', 'init', '--bare', '--template=', str(workspace/'canonical.git')], check=True, capture_output=True)
    subprocess.run(['git', '--git-dir', str(workspace/'canonical.git'), 'fetch', str(root/'seed'), 'main'], check=True, capture_output=True)
    storage = {'policy': {'version': 'gh73-recovery-fixture', 'reason': 'Disposable service recovery test',
               'global_bytes': 16<<30, 'control_bytes': 256<<20, 'run_bytes': 1<<30,
               'requirement_bytes': 4<<30, 'entry_bytes': 1<<20, 'entry_count': 100000,
               'categories': {name: {'bytes': 8<<30, 'seconds': 86400, 'reserve_bytes': 16<<20}
                              for name in ['workspace','hot','cold','database','record']}},
               'execution': owned(execution), 'cold': owned(cold), 'database_filesystem': owned(execution), 'database_extras': []}
    (root/'storage.json').write_text(json.dumps(storage))
    runtime = {'settings': {'startup_seconds': 10, 'response_seconds': 10, 'stall_seconds': 120,
               'reservation': {'tokens': 100, 'turns': 1, 'model_seconds': 60}, 'codex_config': ''},
               'preparation_adapter': str(ROOT/'tools/preparation/app_server.py'),
               'preparation': {'launcher': ['/usr/bin/python3', str(ROOT/'tools/fixtures/mobile_runtime.py')],
               'baseline': baseline, 'deployment_identity': 'gh73-owned-fixture', 'uid': os.getuid(),
               'dependencies': [{'command': ['git','--version'], 'expected': 'git version'}],
               'writable_paths': [str(execution)], 'network_urls': []}}
    (root/'runtime.json').write_text(json.dumps(runtime))
    return execution


class Service:
    def __init__(self, binary, env, tls):
        self.binary, self.env, self.tls = binary, env, tls
        self.child = None
        self.identities = []
        self.logs = []
    def start(self):
        log = (OUT/f'service-{len(self.identities)+1}.log').open('wb')
        self.logs.append(log)
        self.child = subprocess.Popen([str(self.binary)], env=self.env, stdout=subprocess.PIPE, stderr=log)
        address = wait_http_address(self.child, log)
        self.identities.append({'pid': self.child.pid, 'address': address})
        if self.tls.thread:
            self.tls.server.backend = address
        else:
            self.tls.server.RequestHandlerClass = BrowserRelay
            self.tls.start(address)
        # Drain logs so no pipe can stall the actual service.
        import threading
        child = self.child
        def drain():
            for line in child.stdout:
                log.write(line)
                log.flush()
        threading.Thread(target=drain, daemon=True).start()
    def stop(self):
        if self.child and self.child.poll() is None:
            self.child.send_signal(signal.SIGINT)
            try: self.child.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.child.kill(); self.child.wait()
        self.child = None


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    base = os.environ['TEST_DATABASE_URL']
    schema = 'mobile_recovery_' + secrets.token_hex(8)
    sql(base, 'CREATE SCHEMA ' + schema)
    parts = urllib.parse.urlsplit(base)
    url = urllib.parse.urlunsplit(parts._replace(query=urllib.parse.urlencode(
        urllib.parse.parse_qsl(parts.query)+[('options','-csearch_path='+schema)])))
    service = None
    try:
        with tempfile.TemporaryDirectory(prefix='gh73-recovery-') as directory, TLSCapture() as tls, ExitStack() as cleanup:
            root = Path(directory)
            seed = root/'seed'; seed.mkdir()
            subprocess.run(['git','init','--template=','-b','main',str(seed)], check=True, capture_output=True)
            (seed/'baseline.txt').write_text('baseline\n')
            subprocess.run(['git','-C',str(seed),'add','.'], check=True)
            subprocess.run(['git','-C',str(seed),'-c','user.name=Fixture','-c','user.email=fixture@example.invalid','commit','-m','baseline'], check=True, capture_output=True)
            baseline = subprocess.check_output(['git','-C',str(seed),'rev-parse','HEAD'], text=True).strip()
            execution = configuration(root, baseline)
            (root/'auth.json').write_text(json.dumps({'public_origin':tls.origin,'trusted_proxies':['127.0.0.1']}))
            (root/'auth.json').chmod(0o600)
            binary = ROOT/'target/debug/codexsymphony-server'
            env = dict(os.environ, DATABASE_URL=url, AUTH_CONFIG=str(root/'auth.json'), WEB_ORIGIN=tls.origin,
                       BIND_ADDRESS='127.0.0.1:0', EXECUTION_DIRECTORY=str(execution),
                       STORAGE_CONFIG=str(root/'storage.json'), RUNTIME_CONFIG=str(root/'runtime.json'))
            for key in ['GITHUB_CONFIG','GITHUB_APP_CONFIG','DRAFT_GENERATION_CONFIG']:
                env.pop(key, None)
            service = Service(binary, env, tls)
            cleanup.callback(service.stop)
            service.start()
            account = {'username':'mobile-'+secrets.token_hex(8),'password':secrets.token_hex(32)}
            subprocess.run([str(binary),'auth','init','--stdin-json'], env=env, input=json.dumps(account), text=True, check=True, capture_output=True)
            cert = Path(tls.directory.name)/'cert.pem'
            public = subprocess.check_output(['openssl','x509','-in',str(cert),'-pubkey','-noout'])
            der = subprocess.check_output(['openssl','pkey','-pubin','-outform','DER'], input=public)
            common = {'origin':tls.origin,'spki':base64.b64encode(hashlib.sha256(der).digest()).decode(),
                      'account':account,'output':str(OUT)}
            browser_env = dict(os.environ, NODE_EXTRA_CA_CERTS=str(cert))
            def browser(step, **extra):
                result = subprocess.run(['node',str(ROOT/'tools/mobile_recovery_browser.mjs')],
                    env=browser_env, input=json.dumps(dict(common,step=step,**extra)), text=True, capture_output=True)
                (OUT/f'{step}-browser.log').write_text(result.stderr)
                if result.returncode:
                    raise AssertionError(f'Browser {step} failed; see retained log')
                return json.loads(result.stdout)
            identity = browser('create')['id']
            # Explicit synthetic GitHub observation; no remote operation is configured or tested.
            query(url, "INSERT INTO github_repository(repository_id,repository_version,policy,probe_pr,capability,checked_at,stale) VALUES(123,1,'{}',1,'{\"blockers\":[],\"policy\":{}}',extract(epoch FROM now())::bigint,false)")
            def state():
                return json.loads(query(url, "SELECT json_build_object('runs',(SELECT json_agg(row_to_json(a) ORDER BY run_sequence) FROM agent_run a),'questions',(SELECT json_agg(row_to_json(q)) FROM runtime_question q),'calls',(SELECT json_agg(row_to_json(m)) FROM model_call m),'authorizations',(SELECT json_agg(row_to_json(b)) FROM budget_authorization b),'budget',(SELECT row_to_json(b) FROM requirement_budget b LIMIT 1),'sessions',(SELECT json_agg(row_to_json(t)) FROM runtime_session t),'operations',(SELECT json_agg(row_to_json(o)) FROM workspace_operation o),'storage',(SELECT row_to_json(g) FROM storage_guard g),'preparation',(SELECT json_agg(row_to_json(p)) FROM preparation_record p),'workspaces',(SELECT json_agg(row_to_json(w)) FROM run_workspace w),'snapshots',(SELECT json_agg(row_to_json(s)) FROM workspace_snapshot s),'control',(SELECT row_to_json(c) FROM execution_control c),'requirement',(SELECT row_to_json(r) FROM requirement r LIMIT 1))"))
            cleanup.callback(lambda: (OUT/'last-state.json').write_text(json.dumps(state(),indent=2)))
            question = wait(lambda: (state()['questions'] or [None])[0], 'original business question persisted')
            old = question['run_id']
            before = state()
            (OUT/'before-timeout.json').write_text(json.dumps(before,indent=2))
            # Advance only persisted fixture clocks. No host/system or production clock is changed.
            query(url, "UPDATE runtime_session SET created_at=created_at-90000, waiting_since=waiting_since-90000; UPDATE agent_run SET created_at=created_at-interval '25 hours'")
            wait(lambda: all(r['quiescent'] for r in state()['runs']) and state()['snapshots'], 'timeout stop receipt and preserved work')
            stopped = state()
            service.stop(); service.start()
            browser('answer', id=identity, question=question['id'])
            def executed(run):
                try:
                    proof = json.loads((Path(run['workspace'])/'executed-by.json').read_text())
                    return proof['cwd'] == run['workspace'] and (Path(run['workspace'])/'resumed-proof.json').exists()
                except (FileNotFoundError, json.JSONDecodeError):
                    return False
            def resumed():
                for run in state()['runs']:
                    if run['id'] != old and executed(run): return run
            new = wait(resumed, 'new Run consumes answer and original work')
            assert (Path(new['workspace'])/'paid-work.txt').read_text() == 'original unfinished work\n'
            browser('pause', id=identity)
            wait(lambda: all(r['quiescent'] for r in state()['runs']) and len(state()['snapshots']) == 2, 'pause stops and preserves second Run')
            paused = state()
            service.stop(); service.start()
            time.sleep(3)
            after_restart = state()
            assert len(after_restart['runs']) == 2 and after_restart['requirement']['paused']
            browser('inspect-paused', id=identity)
            browser('resume', id=identity)
            third = wait(lambda: next((r for r in state()['runs'] if r['id'] not in [old,new['id']] and executed(r)),None), 'resume saved phase')
            answers = json.loads((Path(third['workspace'])/'received-input.json').read_text())['confirmed_answers']
            assert answers[0]['question_id'] == question['id'] and answers[0]['version'] == question['version']
            browser('cancel', id=identity)
            wait(lambda: state()['requirement']['cleanup_complete'], 'cancel reconciliation')
            final = state()
            assert final['requirement']['state'] == 'Cancelled'
            assert final['budget'] == before['budget']
            assert final['authorizations'] == before['authorizations']
            assert len(final['calls']) == 3
            assert final['calls'][0]['reserved'] == before['calls'][0]['reserved']
            assert final['questions'][0]['run_id'] == old
            assert final['questions'][0]['version'] == question['version']
            assert final['control']['requirement_id'] is None
            assert all(run['quiescent'] for run in final['runs'])
            links = {work['run_id']:work for work in final['workspaces']}
            assert links[new['id']]['restored_from'] == old
            assert links[third['id']]['restored_from'] == new['id']
            assert stopped['snapshots'][0] in final['snapshots']
            evidence = {'scenario':'B03/B07','transport':'scripted app-server; real service, subprocesses, PostgreSQL, Git and Chromium',
                        'clock_advance_seconds':90000,'baseline':baseline,'service_instances':service.identities,
                        'question_id':question['id'],'question_version':question['version'], 'old_run':old,'new_run':new['id'],'pause_resume_run':third['id'],
                        'before':before,'stopped':stopped,'paused':paused,'after_restart':after_restart,'final':final}
            (OUT/'result.json').write_text(json.dumps(evidence,indent=2))
            print(json.dumps({'passed':True,'evidence':'artifacts/gh73/recovery/result.json'}))
    finally:
        if service: service.stop()
        sql(base, 'DROP SCHEMA ' + schema + ' CASCADE')


if __name__ == '__main__':
    main()
