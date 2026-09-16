"""Real pinned app-server RPC smoke with a local scripted provider, zero model calls.

Run inside the assigned command sandbox. This validates environment capability,
not a product Runtime adapter. Shared CODEX_HOME/auth are never read or copied.
"""
import errno
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import select
import subprocess
import tempfile
import threading
import time
import uuid


class RPC:
    def __init__(self, process):
        self.process = process
        self.pending = b''
        self.messages = []

    def send(self, message):
        self.process.stdin.write((json.dumps(message) + '\n').encode())
        self.process.stdin.flush()

    def wait(self, predicate, timeout=25):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            for index, message in enumerate(self.messages):
                if predicate(message):
                    return self.messages.pop(index)
            if b'\n' in self.pending:
                line, self.pending = self.pending.split(b'\n', 1)
                self.messages.append(json.loads(line))
                if len(self.messages) > 500:
                    raise RuntimeError('RPC notification limit exceeded')
            elif select.select([self.process.stdout], [], [], .2)[0]:
                data = os.read(self.process.stdout.fileno(), 65536)
                if not data:
                    raise RuntimeError('app-server exited before RPC completed')
                self.pending += data
                if len(self.pending) > 2 * 1024 * 1024:
                    raise RuntimeError('RPC frame limit exceeded')
        raise TimeoutError('Runtime smoke RPC deadline')

    def call(self, identity, method, params):
        self.send({'id': identity, 'method': method, 'params': params})
        reply = self.wait(lambda value: value.get('id') == identity)
        if 'error' in reply:
            raise RuntimeError(f'{method}: {reply["error"]}')
        return reply['result']


def isolation(root):
    if os.getuid() != 1000:
        raise ValueError('assigned Agent UID required')
    for name in ('/home/gem/.secrets', '/home/gem/.local/share/codexsymphony/gate-host/approval.json'):
        if Path(name).exists():
            raise ValueError('run this probe inside the assigned Agent sandbox')
    canary = root / '.git' / ('runtime-smoke-' + uuid.uuid4().hex)
    try:
        with canary.open('x') as stream:
            stream.write('probe')
    except OSError as error:
        if error.errno not in (errno.EROFS, errno.EACCES, errno.EPERM):
            raise
    else:
        canary.unlink()
        raise ValueError('Git metadata unexpectedly writable')
    return {'uid': os.getuid(), 'git_readonly': True, 'host_credentials_hidden': True}


def smoke(root):
    root = Path(root).resolve()
    proof = isolation(root)
    version = subprocess.check_output(['codex', '--version'], text=True).strip()
    expected = (root / 'codex-version.lock').read_text().strip()
    if version != expected or version != 'codex-cli 0.154.0':
        raise ValueError('pinned Codex version mismatch')
    target = root / 'target'
    if target.is_symlink():
        raise ValueError('target must not be a symlink')
    target.mkdir(exist_ok=True)
    requests = []
    followup = threading.Event()
    finish = threading.Event()

    class Provider(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_POST(self):
            size = int(self.headers.get('Content-Length', '0'))
            if not 0 < size <= 2 * 1024 * 1024 or self.path != '/responses':
                self.send_error(400)
                return
            requests.append(json.loads(self.rfile.read(size)))
            if len(requests) > 1:
                followup.set()
                finish.wait(30)
                return
            events = [
                {'type': 'response.created', 'response': {'id': 'fixture-1'}},
                {'type': 'response.output_item.done', 'item': {'type': 'function_call',
                 'call_id': 'runtime-call', 'name': 'runtime_probe', 'arguments': '{"value":"probe"}'}},
                {'type': 'response.completed', 'response': {'id': 'fixture-1', 'usage':
                 {'input_tokens': 0, 'output_tokens': 0, 'total_tokens': 0}}},
            ]
            body = ''.join('data: ' + json.dumps(event) + '\n\n' for event in events).encode()
            self.send_response(200)
            self.send_header('Content-Type', 'text/event-stream')
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)

    server = ThreadingHTTPServer(('127.0.0.1', 0), Provider)
    server.daemon_threads = True
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        with tempfile.TemporaryDirectory(prefix='runtime-smoke-', dir=target) as temporary:
            runtime_home = Path(temporary)
            (runtime_home / 'config.toml').write_text(f'''
model = "gpt-6-astra"
model_provider = "runtime_fixture"
approval_policy = "never"
sandbox_mode = "workspace-write"
[model_providers.runtime_fixture]
name = "Local deterministic Runtime fixture"
base_url = "http://127.0.0.1:{server.server_port}"
wire_api = "responses"
requires_openai_auth = false
supports_websockets = false
request_max_retries = 0
stream_max_retries = 0
''')
            env = os.environ.copy()
            # Per-process Codex runtime state, no authentication copied or linked.
            env['CODEX_HOME'] = str(runtime_home)
            # Only the fixture in this command's own network namespace is local.
            env['NO_PROXY'] = env['no_proxy'] = '127.0.0.1,localhost,::1'
            with (runtime_home / 'stderr').open('wb') as err:
                process = subprocess.Popen(['codex', 'app-server'], cwd=root, env=env,
                                           stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=err)
                try:
                    rpc = RPC(process)
                    initialized = rpc.call(1, 'initialize', {'clientInfo': {'name': 'runtime-smoke', 'version': '1'},
                                                            'capabilities': {'experimentalApi': True}})
                    rpc.send({'method': 'initialized'})
                    started = rpc.call(2, 'thread/start', {'cwd': str(root), 'approvalPolicy': 'never',
                        'sandbox': 'workspace-write', 'ephemeral': True, 'dynamicTools': [{
                        'type': 'function', 'name': 'runtime_probe', 'description': 'Fixed environment probe',
                        'inputSchema': {'type': 'object', 'properties': {'value': {'type': 'string'}},
                                        'required': ['value'], 'additionalProperties': False}}]})
                    if started['cwd'] != str(root) or started['thread']['cwd'] != str(root):
                        raise ValueError('thread cwd mismatch')
                    thread = started['thread']['id']
                    turn = rpc.call(3, 'turn/start', {'threadId': thread, 'input': [
                        {'type': 'text', 'text': 'Runtime protocol fixture.', 'text_elements': []}]})['turn']['id']
                    tool = rpc.wait(lambda value: value.get('method') == 'item/tool/call')
                    params = tool['params']
                    if (params['threadId'], params['turnId'], params['tool'], params['arguments']) != (
                            thread, turn, 'runtime_probe', {'value': 'probe'}):
                        raise ValueError('dynamic tool identity mismatch')
                    rpc.send({'id': tool['id'], 'result': {'success': True, 'contentItems': [
                        {'type': 'inputText', 'text': 'runtime-probe-ok'}]}})
                    if not followup.wait(10):
                        raise TimeoutError('dynamic tool reply not delivered to provider fixture')
                    outputs = [item for item in requests[-1]['input'] if item.get('type') == 'function_call_output']
                    if not any(item.get('call_id') == 'runtime-call' and 'runtime-probe-ok' in json.dumps(item) for item in outputs):
                        raise ValueError('dynamic tool reply missing from provider request')
                    rpc.call(4, 'turn/interrupt', {'threadId': thread, 'turnId': turn})
                    completed = rpc.wait(lambda value: value.get('method') == 'turn/completed')
                    if completed['params']['turn']['status'] != 'interrupted':
                        raise ValueError('turn did not interrupt')
                    proof.update(status='PASS', codex=version, workspace=str(root), process_cwd=str(root),
                                 thread_cwd=started['cwd'], initialize=bool(initialized), dynamic_tool_roundtrip=True,
                                 turn_interrupt=True, model_calls=0, provider='local scripted fixture',
                                 product_runtime_acceptance=False)
                except Exception:
                    err.flush()
                    # Bounded diagnostics only; the fixture home contains no auth.
                    print((runtime_home / 'stderr').read_text(errors='replace')[-4000:], file=__import__('sys').stderr)
                    raise
                finally:
                    finish.set()
                    process.terminate()
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait()
                    process.stdin.close()
                    process.stdout.close()
    finally:
        finish.set()
        server.shutdown()
        server.server_close()
    proof.update(source_sha=subprocess.check_output(['git', '-C', root, 'rev-parse', 'HEAD'], text=True).strip(),
                 sampler_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(), checked_at=time.time())
    return proof


if __name__ == '__main__':
    print(json.dumps(smoke(Path.cwd()), ensure_ascii=False))
