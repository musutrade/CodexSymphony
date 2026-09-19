"""Run existing local acceptance checks; never create a real Requirement or PR.

Supply TEST_DATABASE_URL for a disposable PostgreSQL database. Ordinary tools,
installed npm dependencies and Playwright browsers are required. Formal exact-
commit Gates and the separately authorized a01-smoke.mjs remain independent.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import subprocess
import time
import urllib.request
import urllib.parse
import uuid

ROOT = Path(__file__).resolve().parents[1]
WEB = ROOT / 'web/angular'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('evidence', type=Path, help='new directory; never overwrite an earlier run')
    parser.add_argument('--api-port', type=int, default=3082)
    args = parser.parse_args()
    if not 1024 <= args.api_port <= 65535 or args.api_port in (3081, 4200, 4300):
        parser.error('use a dedicated API port (default 3082), not a product/frontend port')
    output = args.evidence.resolve()
    output.mkdir(parents=True, exist_ok=False)
    env = os.environ.copy()
    report = {'mode': 'local-deterministic', 'status': 'incomplete', 'checks': [],
              'real_a01': 'not run', 'formal_gates': 'not run'}

    def save():
        (output / 'result.json').write_text(json.dumps(report, indent=2) + '\n')

    def check(name, command, cwd=ROOT):
        print(name, flush=True)
        with (output / (name + '.log')).open('w') as log:
            result = subprocess.run(command, cwd=cwd, env=env, stdout=log, stderr=subprocess.STDOUT)
        report['checks'].append({'name': name, 'command': ['<TEST_DATABASE_URL>' if value == env.get('TEST_DATABASE_URL') else value for value in command], 'cwd': str(cwd), 'exit_code': result.returncode})
        save()
        if result.returncode:
            raise RuntimeError(f'{name} failed; see retained log')

    server = None
    server_log = None
    try:
        save()
        for tool in ('cargo', 'rustc', 'git', 'node', 'npm', 'psql', 'codex'):
            if not shutil.which(tool):
                raise RuntimeError(f'preflight: missing {tool}')
        if not env.get('TEST_DATABASE_URL'):
            raise RuntimeError('preflight: TEST_DATABASE_URL required (disposable fixture)')
        env['DATABASE_URL'] = env['TEST_DATABASE_URL']
        # Product worker configuration belongs to the separately managed A01 service.
        for key in ('RUNTIME_CONFIG', 'GITHUB_APP_CONFIG', 'STORAGE_CONFIG'):
            env.pop(key, None)
        for port in (args.api_port, 4300):
            with socket.socket() as probe:
                probe.bind(('127.0.0.1', port))
        for directory in (ROOT / 'target', output):
            directory.mkdir(exist_ok=True)
            probe = directory / f'.preflight-{os.getpid()}'
            with probe.open('x') as stream:
                stream.write('ordinary writable build/evidence directory')
            probe.unlink()
        check('database-preflight', ['psql', '-X', '-d', env['TEST_DATABASE_URL'], '-v', 'ON_ERROR_STOP=1', '-c', 'SELECT 1'], cwd=ROOT)
        check('browser-preflight', ['node', '-e',
              "const {chromium}=require('@playwright/test'); (async()=>{const b=await chromium.launch();await b.close()})().catch(e=>{console.error(e);process.exit(1)})"], WEB)
        check('fmt', ['cargo', 'fmt', '--all', '--', '--check'])
        check('workspace-tests', ['cargo', 'test', '--workspace', '--locked'])
        check('clippy', ['cargo', 'clippy', '--workspace', '--all-targets', '--locked', '--', '-D', 'warnings'])
        check('observer-tests', ['node', '--test', 'tools/tests/a01-observe.test.mjs'])
        check('frontend-lint', ['npm', 'run', 'lint'], WEB)
        check('frontend-unit', ['npm', 'test', '--', '--watch=false'], WEB)
        check('frontend-build', ['npm', 'run', 'build'], WEB)
        check('server-build', ['cargo', 'build', '--locked', '-p', 'codexsymphony-server'])
        schema = 'gh24_browser_' + uuid.uuid4().hex
        check('browser-schema', ['psql', '-X', '-d', env['TEST_DATABASE_URL'], '-v',
                                'ON_ERROR_STOP=1', '-c', f'CREATE SCHEMA {schema}'])
        url = urllib.parse.urlsplit(env['TEST_DATABASE_URL'])
        query = urllib.parse.parse_qs(url.query)
        query['options'] = [f'-csearch_path={schema}']
        env['TEST_DATABASE_URL'] = urllib.parse.urlunsplit(url._replace(query=urllib.parse.urlencode(query, doseq=True)))
        env['DATABASE_URL'] = env['TEST_DATABASE_URL']
        report['browser_schema'] = schema
        env['BIND_ADDRESS'] = f'127.0.0.1:{args.api_port}'
        env['WEB_ORIGIN'] = 'http://127.0.0.1:4300'
        env['E2E_API_PORT'] = str(args.api_port)
        env['EXECUTION_DIRECTORY'] = str(output / 'execution')
        server_log = (output / 'api.log').open('w')
        server = subprocess.Popen([str(ROOT / 'target/debug/codexsymphony-server')],
                                  cwd=ROOT, env=env, stdout=server_log, stderr=subprocess.STDOUT)
        for _ in range(100):
            if server.poll() is not None:
                raise RuntimeError('fixture API exited; see api.log')
            try:
                with urllib.request.urlopen(f'http://127.0.0.1:{args.api_port}/api/health', timeout=1) as reply:
                    if reply.status == 200:
                        break
            except OSError:
                time.sleep(.1)
        else:
            raise RuntimeError('fixture API readiness timeout')
        check('browser-tests', ['npm', 'run', 'test:e2e', '--', '--output', str(output / 'browser')], WEB)
        check('gate-config', ['python3', 'tools/gate.py', 'config', 'check'])
        check('gate-secrets', ['python3', 'tools/gate.py', 'secrets', '--json'])
        check('gate-audit', ['python3', 'tools/gate.py', 'audit', '--json'])
        report['status'] = 'passed'
    except Exception as error:
        report['error'] = str(error)
        raise
    finally:
        if server is not None and server.poll() is None:
            server.send_signal(signal.SIGINT)
            try:
                server.wait(timeout=5)
            except subprocess.TimeoutExpired:
                server.kill()
                server.wait()
        if server_log:
            server_log.close()
        save()


if __name__ == '__main__':
    main()
