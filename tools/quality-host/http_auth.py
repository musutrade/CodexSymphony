"""Data-only auth adapter: isolated candidate admin command, test secrets via stdin."""
import json
import re
import secrets
import subprocess
from pathlib import Path
from http_scenarios import capture_values
from isolation import command


def load_adapter(repository):
    path = repository/'api/capture-auth.json'
    if not path.exists() and not path.is_symlink():
        return None
    if path.is_symlink() or not path.is_file() or path.stat().st_size > 65536:
        raise ValueError('regular bounded capture auth adapter required')
    data = json.loads(path.read_text())
    if set(data) != {'schema', 'config', 'bootstrap_args', 'bootstrap_input', 'login', 'headers'}:
        raise ValueError('invalid capture auth adapter fields')
    if data['schema'] != 'codexsymphony-http-auth/v1':
        raise ValueError('invalid capture auth schema')
    args = data['bootstrap_args']
    if (not isinstance(args, list) or not 1 <= len(args) <= 12 or args[0] != 'auth'
            or any(not isinstance(x, str) or not re.fullmatch(r'[a-zA-Z0-9_-]{1,64}', x) for x in args)):
        raise ValueError('bounded auth subcommand required; no paths or secrets in argv')
    if not isinstance(data['config'], dict) or not isinstance(data['bootstrap_input'], dict):
        raise ValueError('JSON config and stdin objects required')
    if not isinstance(data['login'], list) or not 1 <= len(data['login']) <= 8:
        raise ValueError('bounded login scenarios required')
    if any(s.get('record') is not False for s in data['login']):
        raise ValueError('credential-bearing login scenarios must use record=false')
    if set(data['headers']) - {'x-codexsymphony-csrf'}:
        raise ValueError('only default CSRF proof is configurable')
    return data


def prepare(adapter, run, origin):
    variables = {'origin': origin, 'username': 'capture-' + secrets.token_hex(8),
                 'password': secrets.token_urlsafe(32)}
    root = run/'tmp'; root.mkdir(exist_ok=True)
    path = root/'capture-auth-config.json'
    path.write_text(json.dumps(capture_values(adapter['config'], variables)))
    path.chmod(0o600)
    return variables, {'AUTH_CONFIG': '/tmp/capture-auth-config.json', 'WEB_ORIGIN': origin}


def bootstrap(adapter, variables, *, binary, run, repository, plugins, environment):
    argv = [binary, *adapter['bootstrap_args']]
    args = command(argv, run=run, repository=repository, plugins=plugins,
                   readonly=[binary], environment=environment)
    payload = json.dumps(capture_values(adapter['bootstrap_input'], variables)).encode()
    # No raw stdout/stderr retention: a broken candidate CLI might echo its input.
    result = subprocess.run(args, input=payload, stdout=subprocess.DEVNULL,
                            stderr=subprocess.DEVNULL, timeout=30)
    if result.returncode:
        raise RuntimeError(f'isolated test-account bootstrap failed (exit {result.returncode})')
