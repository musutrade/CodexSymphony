#!/usr/bin/env python3
"""System notification dispatcher: durable claim -> reviewed plugin -> exact ack.

Run outside Agent/validation mounts with a notification-only PostgreSQL login.
Channel selection and credentials belong to the invoked plugin.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import stat
import subprocess
import tempfile
import time
import urllib.parse


def config(path):
    info = path.lstat()
    if not path.is_absolute() or not stat.S_ISREG(info.st_mode) or info.st_mode & 0o077 or info.st_uid != os.geteuid():
        raise ValueError('private dispatcher configuration required')
    value = json.loads(path.read_text())
    if value == {'enabled': False}:
        return value
    if set(value) != {'enabled', 'plugin_id', 'database', 'psql_program', 'argv', 'implementation'}:
        raise ValueError('invalid dispatcher configuration')
    if value['enabled'] is not True or not re.fullmatch(r'[a-zA-Z0-9_-]{1,80}', value['plugin_id']):
        raise ValueError('invalid plugin registration')
    argv = value['argv']
    if not isinstance(argv, list) or not argv or len(argv) > 32 or not all(isinstance(a, str) for a in argv):
        raise ValueError('fixed argument vector required')
    if not Path(argv[0]).is_absolute() or argv[0] not in value['implementation']:
        raise ValueError('pinned executable required')
    verify_implementation(value)
    return value


def verify_implementation(cfg):
    for name, expected in cfg['implementation'].items():
        path = Path(name)
        if not path.is_absolute() or path.is_symlink() or not path.is_file():
            raise ValueError('invalid plugin implementation path')
        if hashlib.sha256(path.read_bytes()).hexdigest() != expected:
            raise ValueError('plugin implementation changed')


def database_environment(uri):
    url = urllib.parse.urlsplit(uri)
    if url.scheme not in ('postgres', 'postgresql') or not url.hostname or not url.path:
        raise ValueError('invalid notification database')
    env = {'PATH': '/usr/bin:/bin', 'PGHOST': url.hostname, 'PGPORT': str(url.port or 5432),
           'PGDATABASE': urllib.parse.unquote(url.path[1:]), 'PGUSER': urllib.parse.unquote(url.username or ''),
           'PGPASSWORD': urllib.parse.unquote(url.password or ''), 'PGCONNECT_TIMEOUT': '5'}
    options = {'sslmode': 'PGSSLMODE', 'sslrootcert': 'PGSSLROOTCERT', 'options': 'PGOPTIONS'}
    for key, value in urllib.parse.parse_qsl(url.query):
        if key not in options:
            raise ValueError('invalid database option')
        env[options[key]] = value
    env['PGOPTIONS'] = env.get('PGOPTIONS', '') + ' -c statement_timeout=5000 -c lock_timeout=2000'
    return env


def query(cfg, sql):
    program = Path(cfg['psql_program'])
    if not program.is_absolute():
        raise ValueError('absolute psql path required')
    result = subprocess.run([str(program), '-XAt', '-v', 'ON_ERROR_STOP=1'], input=sql,
                            env=database_environment(cfg['database']), capture_output=True, text=True,
                            timeout=10, check=True)
    return result.stdout.strip()


def invoke(cfg, event):
    verify_implementation(cfg)
    # No database or inherited host credentials reach a plugin subprocess.
    with tempfile.TemporaryFile() as output, tempfile.TemporaryFile() as diagnostic:
        process = subprocess.Popen(cfg['argv'], stdin=subprocess.PIPE, stdout=output, stderr=diagnostic,
                                   env={'PATH': '/usr/bin:/bin', 'LANG': 'C.UTF-8'}, start_new_session=True)
        try:
            process.stdin.write(json.dumps(event).encode())
            process.stdin.close()
            deadline = time.monotonic() + 10
            while process.poll() is None:
                if time.monotonic() >= deadline or output.tell() > 65536 or diagnostic.tell() > 1048576:
                    return 'unknown'
                time.sleep(0.02)
            output.seek(0)
            raw = output.read(65537)
            if process.returncode != 0 or len(raw) > 65536:
                return 'unknown'
            reply = json.loads(raw)
            if set(reply) != {'protocol_version', 'event_id', 'status'}:
                return 'unknown'
            if type(reply['protocol_version']) is not int or reply['protocol_version'] != 1 or type(reply['event_id']) is not int or reply['event_id'] != event['event_id']:
                return 'unknown'
            return reply['status'] if reply['status'] in ('accepted', 'ignored', 'failed') else 'unknown'
        except (OSError, ValueError, TypeError):
            return 'unknown'
        finally:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait()


def tick(cfg):
    plugin = cfg['plugin_id']  # restricted identifier, never arbitrary SQL
    value = query(cfg, f"SELECT notification_claim('{plugin}');")
    if not value:
        return 'idle'
    event = json.loads(value)
    if type(event.get('event_id')) is not int or type(event.get('attempt')) is not int:
        raise ValueError('invalid claim identity')
    result = invoke(cfg, event)
    query(cfg, f"SELECT notification_ack('{plugin}',{event['event_id']},{event['attempt']},'{result}');")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config', type=Path, required=True)
    args = parser.parse_args()
    cfg = config(args.config)
    if not cfg['enabled']:
        print('disabled')
        return
    for _ in range(100):
        if tick(cfg) == 'idle':
            break
    print('checked')


if __name__ == '__main__':
    try:
        main()
    except Exception:
        raise SystemExit('notification dispatch unavailable; inspect private configuration and ledger')
