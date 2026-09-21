"""AAuth real-process HTTPS/restart checks in a disposable test schema only."""
import http.cookiejar
import hashlib
import io
import json
import os
from pathlib import Path
import secrets
import signal
import subprocess
import tempfile
import threading
import urllib.error
import urllib.parse
import urllib.request
from auth_contract_acceptance import ROOT, TLSCapture, sql, wait_http_address


class Client:
    def __init__(self, tls, sentinels):
        self.tls, self.sentinels = tls, sentinels
        self.jar = http.cookiejar.CookieJar()
        self.http = urllib.request.build_opener(urllib.request.ProxyHandler({}),
            urllib.request.HTTPSHandler(context=tls.context), urllib.request.HTTPCookieProcessor(self.jar))
        self.proof = ''

    def call(self, method, path, data=None, extra=None, proof=True):
        headers = {'Content-Type':'application/json'}
        if method != 'GET':
            headers['Origin'] = self.tls.origin
            if proof: headers['x-codexsymphony-csrf'] = self.proof
        headers.update(extra or {})
        request = urllib.request.Request(self.tls.origin + path, method=method, headers=headers,
                                         data=json.dumps(data).encode() if data is not None else None)
        try: response = self.http.open(request, timeout=10)
        except urllib.error.HTTPError as error: response = error
        with response:
            body = response.read()
            value = json.loads(body) if body else None
            if isinstance(value, dict) and 'csrf_token' in value:
                self.proof = value['csrf_token']; self.sentinels.add(self.proof)
            for cookie in self.jar: self.sentinels.add(cookie.value)
            return response.status, value

    def login(self, name, password):
        assert self.call('GET', '/api/auth/csrf')[0] == 200
        return self.call('POST','/api/auth/login', {'username':name,'password':password})[0]


def main():
    base = os.environ['TEST_DATABASE_URL']
    schema = 'auth_process_' + secrets.token_hex(8)
    sql(base, 'CREATE SCHEMA ' + schema)
    parts = urllib.parse.urlsplit(base)
    url = urllib.parse.urlunsplit(parts._replace(query=urllib.parse.urlencode(
        urllib.parse.parse_qsl(parts.query) + [('options','-csearch_path='+schema)])))
    binary = (ROOT / os.environ.get('CARGO_TARGET_DIR','target') / 'debug/codexsymphony-server').resolve()
    logs, sentinels, identities = [], set(), []
    server = None
    try:
        with tempfile.TemporaryDirectory(prefix='auth-process-') as directory, TLSCapture() as tls:
            root = Path(directory)
            config = root/'auth.json'
            config.write_text(json.dumps({'public_origin':tls.origin,'trusted_proxies':['127.0.0.1']})); config.chmod(0o600)
            env = dict(os.environ, DATABASE_URL=url, AUTH_CONFIG=str(config), WEB_ORIGIN=tls.origin,
                       BIND_ADDRESS='127.0.0.1:0', EXECUTION_DIRECTORY=str(root/'execution'))
            for key in ['RUNTIME_CONFIG','GITHUB_CONFIG','STORAGE_CONFIG']: env.pop(key,None)
            def start():
                process = subprocess.Popen([str(binary)], env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
                startup = io.BytesIO()
                address = wait_http_address(process, startup)
                identities.append({'pid': process.pid, 'address': address, 'schema': schema})
                logs.append(startup.getvalue())
                thread = threading.Thread(target=lambda: logs.append(process.stdout.read()), daemon=True); thread.start()
                tls.server.backend = address
                return process, thread
            def stop(process, thread):
                process.send_signal(signal.SIGINT); process.wait(timeout=10); thread.join(timeout=10)
            def admin(command, password):
                sentinels.add(password)
                result = subprocess.run([str(binary),'auth',command,'--stdin-json'], env=env,
                    input=json.dumps({'username':'operator','password':password}).encode(), capture_output=True)
                logs.extend([result.stdout,result.stderr]); assert result.returncode == 0
            server, thread = start(); tls.start(tls.server.backend)
            password = secrets.token_urlsafe(32); admin('init', password)
            desktop, mobile = Client(tls,sentinels), Client(tls,sentinels)
            assert desktop.login('operator',password) == mobile.login('operator',password) == 200
            expired = Client(tls,sentinels)
            assert expired.login('operator',password) == 200
            expired_cookie = next(iter(expired.jar)).value
            expired_digest = hashlib.sha256(expired_cookie.encode()).hexdigest()
            # Move only this disposable session's deadline, never the host clock.
            sql(url, "UPDATE platform_session SET expires_at=0 WHERE digest='" + expired_digest + "'")
            assert expired.call('GET','/api/drafts',extra={'Cookie':'__Host-codexsession='+expired_cookie})[0] == 401
            desktop_cookie = next(iter(desktop.jar)).value
            assert desktop.call('POST','/api/auth/logout',{})[0] == 204
            stop(server,thread); server,thread = start()
            assert expired.call('GET','/api/drafts',extra={'Cookie':'__Host-codexsession='+expired_cookie})[0] == 401
            assert expired.login('operator',password) == 200
            assert expired.call('GET','/api/drafts')[0] == 200
            replay = Client(tls,sentinels)
            assert replay.call('GET','/api/drafts',extra={'Cookie':'__Host-codexsession='+desktop_cookie})[0] == 401
            assert mobile.call('GET','/api/drafts')[0] == 200
            old_mobile = next(iter(mobile.jar)).value
            replacement = secrets.token_urlsafe(32); admin('change',replacement)
            assert mobile.call('GET','/api/drafts')[0] == 401
            assert expired.call('GET','/api/drafts')[0] == 401
            stop(server,thread); server,thread = start()
            assert replay.call('GET','/api/drafts',extra={'Cookie':'__Host-codexsession='+old_mobile})[0] == 401
            assert desktop.login('operator',password) == 401
            assert desktop.login('operator',replacement) == 200
            newest = secrets.token_urlsafe(32); admin('reset',newest)
            assert desktop.call('GET','/api/drafts')[0] == 401
            assert mobile.login('operator',replacement) == 401
            assert mobile.login('operator',newest) == 200
            anonymous = Client(tls,sentinels)
            for i in range(10): assert anonymous.login('limited', 'incorrect-password') == 401
            stop(server,thread); server,thread = start()
            assert anonymous.login('limited','incorrect-password') == 429
            # Source dimension survives process restart and cannot be changed by client headers.
            for i in range(55):
                anonymous.call('GET','/api/auth/csrf')
                status,_ = anonymous.call('POST','/api/auth/login',{'username':f'unknown-{i}','password':'incorrect-password'},
                    extra={'X-Forwarded-For':f'198.51.100.{i}','CF-Access-Authenticated-User-Email':'operator'})
            assert status == 429
            stop(server,thread); server,thread = start()
            assert anonymous.login('another-unknown','incorrect-password') == 429
            assert anonymous.call('GET','/api/drafts',extra={'CF-Access-Jwt-Assertion':'forged'})[0] == 401
            stop(server,thread); server = None
        output = b'\n'.join(logs)
        assert all(value.encode() not in output for value in sentinels if value)
        out = ROOT/'artifacts/gh71-resume'
        out.mkdir(parents=True, exist_ok=True)
        (out/'auth-process-server.log').write_bytes(output)
        (out/'auth-process.json').write_text(json.dumps({'status':'passed','verified_https':True,
            'real_process_restarts':len(identities)-1,'process_instances':identities,
            'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),
            'expired_session_rejected_across_restart':True,
            'expired_client_relogin':True,
            'logout_replay_rejected':True,'change_and_reset_revoke_all':True,
            'persistent_account_and_source_limits':True,'identity_headers_do_not_authenticate':True,
            'credential_sentinel_matches':0}, indent=2)+'\n')
        print('PASS: verified HTTPS, real process restarts, revocation, dual limits and zero credential log matches')
    finally:
        if server is not None and server.poll() is None: server.kill(); server.wait()
        sql(base, 'DROP SCHEMA ' + schema + ' CASCADE')


if __name__ == '__main__': main()
