"""Isolated AAuth05 fixture. This is not public-domain or production acceptance."""
import io
import http.client
from contextlib import closing
import json
import os
from pathlib import Path
import secrets
import signal
import subprocess
import tempfile
import threading
import urllib.parse

from auth_contract_acceptance import ROOT, sql, wait_http_address
from auth_session_acceptance import Client
from deployment.check_ingress import origin_listener, remote
from deployment.nginx_fixture import NginxFixture


def main():
    output = ROOT/'artifacts/gh72/https.json'
    output.parent.mkdir(parents=True, exist_ok=True)
    evidence = {'scope': 'isolated HTTPS fixture', 'production_acceptance': False, 'checks': {}}
    base = os.environ['TEST_DATABASE_URL']
    schema = 'm2_https_' + secrets.token_hex(8)
    sql(base, 'CREATE SCHEMA ' + schema)
    parts = urllib.parse.urlsplit(base)
    query = urllib.parse.parse_qsl(parts.query) + [('options', '-csearch_path=' + schema)]
    url = urllib.parse.urlunsplit(parts._replace(query=urllib.parse.urlencode(query)))
    binary = ROOT/os.environ.get('CARGO_TARGET_DIR', 'target')/'debug/codexsymphony-server'
    try:
        with tempfile.TemporaryDirectory(prefix='m2-https-') as directory, NginxFixture() as tls:
            root = Path(directory)
            config = root/'auth.json'
            config.write_text(json.dumps({'public_origin': tls.origin, 'trusted_proxies': ['127.0.0.1']}))
            config.chmod(0o600)
            env = dict(os.environ, DATABASE_URL=url, AUTH_CONFIG=str(config), WEB_ORIGIN=tls.origin,
                       BIND_ADDRESS='127.0.0.1:0', EXECUTION_DIRECTORY=str(root/'execution'))
            for key in ('RUNTIME_CONFIG', 'GITHUB_CONFIG', 'STORAGE_CONFIG', 'DRAFT_GENERATION_CONFIG'):
                env.pop(key, None)
            process = subprocess.Popen([str(binary)], env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
            logs = []
            try:
                startup = io.BytesIO()
                address = wait_http_address(process, startup)
                logs.append(startup.getvalue())
                reader = threading.Thread(target=lambda: logs.append(process.stdout.read()), daemon=True)
                reader.start()
                tls.start(address)
                checks = evidence['checks']
                checks['shipped_nginx_template_syntax_and_start'] = True
                checks.update(remote(tls.origin, str(Path(tls.directory.name)/'cert.pem')))
                checks.update(origin_listener(process.pid, address))
                host, port = address.split(':')
                with closing(http.client.HTTPConnection(host, int(port),
                             source_address=('127.0.0.2', 0), timeout=5)) as connection:
                    connection.request('GET', '/api/drafts', headers={
                        'X-Forwarded-Host': urllib.parse.urlsplit(tls.origin).netloc,
                        'X-Forwarded-Proto': 'https', 'X-Forwarded-For': '127.0.0.1'})
                    response = connection.getresponse()
                    assert response.status == 401
                    response.read()
                checks['untrusted_socket_peer_headers_do_not_authenticate'] = True
                password = secrets.token_urlsafe(32)
                result = subprocess.run([str(binary), 'auth', 'init', '--stdin-json'], env=env,
                    input=json.dumps({'username': 'operator', 'password': password}).encode(), capture_output=True)
                assert result.returncode == 0, 'isolated account bootstrap failed'
                logs.extend([result.stdout, result.stderr])
                sentinels = {password}
                client = Client(tls, sentinels)
                assert client.login('operator', password) == 200
                assert client.call('GET', '/api/drafts')[0] == 200
                checks['platform_login_without_access'] = True
                cookie = next(iter(client.jar))
                assert cookie.name == '__Host-codexsession' and cookie.secure and cookie.path == '/'
                assert cookie.has_nonstandard_attr('HttpOnly') and cookie.get_nonstandard_attr('SameSite') == 'Lax'
                assert not cookie.domain_specified
                checks['secure_host_cookie'] = True
                assert client.call('GET', '/api/drafts', extra={'Host': 'attacker.invalid'})[0] == 403
                assert client.call('GET', '/api/drafts', extra={'Origin': 'https://attacker.invalid'})[0] == 403
                anonymous = Client(tls, sentinels)
                assert anonymous.call('GET', '/api/drafts', extra={
                    'X-Forwarded-For': '198.51.100.1', 'X-Forwarded-Proto': 'https',
                    'X-Forwarded-Host': 'attacker.invalid', 'Forwarded': 'for=198.51.100.1'})[0] == 401
                checks['host_origin_and_proxy_spoof_rejected'] = True
                assert client.call('POST', '/api/auth/logout', {})[0] == 204
                assert client.call('GET', '/api/drafts')[0] == 401
                checks['logout_rejected'] = True
                process.send_signal(signal.SIGINT)
                process.wait(timeout=10)
                reader.join(timeout=10)
                retained = b''.join(logs) + tls.log_bytes()
                assert all(value.encode() not in retained for value in sentinels)
                checks['credential_free_logs'] = True
                evidence['status'] = 'passed'
            finally:
                if process.poll() is None:
                    process.send_signal(signal.SIGINT)
                    try:
                        process.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait()
    except Exception as error:
        evidence.update(status='failed', failure_type=type(error).__name__)
        raise
    finally:
        output.write_text(json.dumps(evidence, indent=2) + '\n')
        sql(base, 'DROP SCHEMA ' + schema + ' CASCADE')
    print(json.dumps(evidence))


if __name__ == '__main__':
    main()
