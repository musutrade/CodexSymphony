#!/usr/bin/env python3
"""Host-owned Bark adapter. Install outside all Agent/validation mounts."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import sqlite3
import stat
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

KINDS = {
    'cancel_cleanup': '取消收尾需要处理', 'paused': '任务已暂停，请确认后续操作',
    'failed': '任务需要处理', 'question': '任务有问题等待回答或恢复',
    'run_blocker': '任务被阻塞', 'preparation': '执行准备需要处理',
    'delivery': '交付需要处理', 'validation': '验证需要处理',
    'storage_guard': '存储需要处理', 'storage_material': '保存或清理需要处理',
    'evidence_cleanup': '证据清理需要处理',
}
ATTEMPTS = 3
BACKOFF = (30, 120)
DEADLINE = 600
TIMEOUT = 10
QUERY = "SELECT COALESCE(json_agg(n),'[]'::json) FROM notification_action n"


def protected(path):
    path = Path(path)
    info = path.lstat()
    if not path.is_absolute() or not stat.S_ISREG(info.st_mode) or info.st_mode & 0o077:
        raise ValueError('private regular configuration required')
    if info.st_uid != os.geteuid():
        raise ValueError('configuration must belong to notifier user')
    return json.loads(path.read_text())


def origin(value):
    url = urllib.parse.urlsplit(value)
    if url.scheme != 'https' or not url.hostname or url.username or url.password:
        raise ValueError('HTTPS application origin required')
    if url.path or url.query or url.fragment or url.netloc != url.hostname + (f':{url.port}' if url.port else ''):
        raise ValueError('canonical application origin required')
    return value


def configuration(path):
    cfg = protected(path)
    if cfg == {'enabled': False}:
        return cfg
    if set(cfg) != {'enabled', 'application_origin', 'endpoint', 'device_key', 'database', 'state_directory', 'local_fixture', 'psql_program'}:
        raise ValueError('incomplete configuration')
    if cfg['enabled'] is not True or type(cfg['local_fixture']) is not bool:
        raise ValueError('invalid enabled configuration')
    origin(cfg['application_origin'])
    url = urllib.parse.urlsplit(cfg['endpoint'])
    allowed_scheme = url.scheme == 'https'
    if cfg['local_fixture']:
        allowed_scheme = url.scheme == 'http' and url.hostname == '127.0.0.1'
    if not allowed_scheme or not url.hostname or url.username or url.password:
        raise ValueError('endpoint rejected')
    if url.path != '/push' or url.query or url.fragment:
        raise ValueError('credential-free /push endpoint required')
    if not isinstance(cfg['device_key'], str) or not 1 <= len(cfg['device_key']) <= 256:
        raise ValueError('device key required')
    if not isinstance(cfg['database'], str) or not cfg['database']:
        raise ValueError('database required')
    database_environment(cfg['database'])
    program = Path(cfg['psql_program'])
    if not program.is_absolute() or not program.is_file() or not os.access(program, os.X_OK):
        raise ValueError('administrator-installed psql required')
    directory = Path(cfg['state_directory'])
    info = directory.lstat()
    if not directory.is_absolute() or not stat.S_ISDIR(info.st_mode) or info.st_mode & 0o077 or info.st_uid != os.geteuid():
        raise ValueError('private state directory required')
    return cfg


def database_environment(value):
    url = urllib.parse.urlsplit(value)
    if url.scheme not in ('postgres', 'postgresql') or not url.hostname or not url.path:
        raise ValueError('PostgreSQL connection URI required')
    env = {'PATH': '/usr/bin:/bin', 'PGHOST': url.hostname,
           'PGPORT': str(url.port or 5432), 'PGDATABASE': urllib.parse.unquote(url.path[1:]),
           'PGUSER': urllib.parse.unquote(url.username or ''),
           'PGPASSWORD': urllib.parse.unquote(url.password or ''), 'PGCONNECT_TIMEOUT': '5'}
    allowed = {'sslmode': 'PGSSLMODE', 'sslrootcert': 'PGSSLROOTCERT', 'options': 'PGOPTIONS'}
    for key, item in urllib.parse.parse_qsl(url.query):
        if key not in allowed:
            raise ValueError('unsupported database option')
        env[allowed[key]] = item
    env['PGOPTIONS'] = env.get('PGOPTIONS', '') + ' -c default_transaction_read_only=on -c statement_timeout=5000 -c jit=off'
    return env


def actions(cfg):
    # The deployment role has only SELECT on notification_action. The explicit
    # read-only transaction remains defense in depth, not a replacement for ACLs.
    env = database_environment(cfg['database'])
    result = subprocess.run([cfg['psql_program'], '-XAt', '-v', 'ON_ERROR_STOP=1', '-c', QUERY],
                            env=env, capture_output=True, timeout=10, check=True)
    items = json.loads(result.stdout)
    for item in items:
        if set(item) != {'requirement_id', 'kind', 'action_key'}:
            raise ValueError('unexpected action projection')
        if type(item['requirement_id']) is not int or item['requirement_id'] <= 0 or item['kind'] not in KINDS:
            raise ValueError('invalid action')
        if not isinstance(item['action_key'], str) or len(item['action_key']) != 64 or any(c not in '0123456789abcdef' for c in item['action_key']):
            raise ValueError('invalid action identity')
    return items


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def send(cfg, item):
    payload = {'device_key': cfg['device_key'], 'title': 'CodexSymphony 需要行动',
               'body': KINDS[item['kind']], 'group': 'CodexSymphony',
               'url': cfg['application_origin'] + '/requirements/' + str(item['requirement_id'])}
    request = urllib.request.Request(cfg['endpoint'], data=json.dumps(payload).encode(),
                                    headers={'Content-Type': 'application/json'}, method='POST')
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())
    try:
        with opener.open(request, timeout=TIMEOUT) as response:
            body = response.read(4097)
            accepted = response.status == 200 and len(body) <= 4096 and json.loads(body).get('code') == 200
            return 'accepted' if accepted else 'rejected'
    except urllib.error.HTTPError as error:
        # Never retain response body, reason, URL or headers (they can echo keys).
        return 'retryable_http' if error.code == 429 or error.code >= 500 else 'rejected'
    except (OSError, ValueError, TypeError, AttributeError):
        return 'unknown'


def deliver(path, item):
    # Hard wall-clock bound includes TLS, response streaming and local failures.
    # Kill after timeout; an accepted remote request can still have unknown result.
    try:
        result = subprocess.run([sys.executable, '-I', str(Path(__file__).resolve()),
                                 '--config', str(path), '--send'],
                                input=json.dumps(item), capture_output=True, text=True, timeout=TIMEOUT)
        code = result.stdout.strip()
        return code if result.returncode == 0 and code in ('accepted', 'rejected', 'retryable_http', 'unknown') else 'unknown'
    except (OSError, subprocess.TimeoutExpired):
        return 'unknown'


def ledger(directory):
    connection = sqlite3.connect(directory / 'delivery.sqlite3', timeout=10)
    connection.row_factory = sqlite3.Row
    connection.execute('PRAGMA synchronous=FULL')
    connection.execute('''CREATE TABLE IF NOT EXISTS delivery (
        action_key TEXT PRIMARY KEY, requirement_id INTEGER NOT NULL, kind TEXT NOT NULL,
        first_seen INTEGER NOT NULL, deadline INTEGER NOT NULL, attempts INTEGER NOT NULL DEFAULT 0,
        next_attempt INTEGER NOT NULL, state TEXT NOT NULL DEFAULT 'pending', result TEXT)''')
    connection.execute('''CREATE TABLE IF NOT EXISTS attempt_result (
        action_key TEXT NOT NULL, ordinal INTEGER NOT NULL, started_at INTEGER NOT NULL,
        result TEXT NOT NULL, PRIMARY KEY(action_key,ordinal))''')
    connection.commit()
    return connection


def synchronize(db, items, now):
    active = {item['action_key'] for item in items}
    with db:
        for item in items:
            db.execute('INSERT OR IGNORE INTO delivery(action_key,requirement_id,kind,first_seen,deadline,next_attempt) VALUES(?,?,?,?,?,?)',
                       (item['action_key'], item['requirement_id'], item['kind'], now, now + DEADLINE, now))
        for row in db.execute("SELECT action_key FROM delivery WHERE state IN ('pending','sending')"):
            if row['action_key'] not in active:
                db.execute("UPDATE delivery SET state='obsolete',result='resolved_or_changed' WHERE action_key=?", (row['action_key'],))
        db.execute("UPDATE delivery SET state='failed',result='deadline_or_attempt_limit' WHERE state IN ('pending','sending') AND (deadline<=? OR attempts>=?)", (now, ATTEMPTS))


def attempt(db, path, item, now):
    # A whole-process lock serializes snapshot, claims and HTTP across instances.
    # Commit attempt BEFORE external I/O; a crash consumes an attempt, never
    # erases it. Restart waits for persisted backoff and respects the deadline.
    ordinal = item['attempts'] + 1
    delay = BACKOFF[min(ordinal - 1, len(BACKOFF) - 1)]
    with db:
        db.execute("UPDATE delivery SET state='sending',attempts=?,next_attempt=?,result='unknown' WHERE action_key=?",
                   (ordinal, now + delay, item['action_key']))
        db.execute('INSERT INTO attempt_result VALUES(?,?,?,?)',
                   (item['action_key'], ordinal, now, 'unknown'))
    result = deliver(path, dict(item))
    state = 'pending'
    if result == 'accepted':
        state = 'accepted'
    elif result == 'rejected' or ordinal == ATTEMPTS:
        state = 'failed'
    with db:
        db.execute('UPDATE delivery SET state=?,result=? WHERE action_key=?', (state, result, item['action_key']))
        db.execute('UPDATE attempt_result SET result=? WHERE action_key=? AND ordinal=?',
                   (result, item['action_key'], ordinal))


def tick(path, cfg):
    directory = Path(cfg['state_directory'])
    with (directory / 'worker.lock').open('a') as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            return 'busy'
        # Failure to read business facts must not cancel pending records.
        items = actions(cfg)
        db = ledger(directory)
        try:
            now = int(time.time())
            synchronize(db, items, now)
            rows = db.execute("SELECT * FROM delivery WHERE state IN ('pending','sending') AND next_attempt<=? AND deadline>? AND attempts<? ORDER BY first_seen,action_key LIMIT 100",
                              (now, now + TIMEOUT, ATTEMPTS)).fetchall()
            for row in rows:
                # Refresh before each send; obsolete notification never executes
                # an action anyway, and endpoint writes require current versions.
                current = {item['action_key'] for item in actions(cfg)}
                if row['action_key'] in current and int(time.time()) + TIMEOUT < row['deadline']:
                    attempt(db, path, row, int(time.time()))
            return 'checked'
        finally:
            db.close()


def main():
    os.umask(0o077)
    parser = argparse.ArgumentParser()
    parser.add_argument('--config', type=Path)
    parser.add_argument('--send', action='store_true', help=argparse.SUPPRESS)
    parser.add_argument('--status', action='store_true')
    args = parser.parse_args()
    if args.config is None:
        print('disabled')
        return
    cfg = configuration(args.config)
    if not cfg['enabled']:
        print('disabled')
    elif args.send:
        print(send(cfg, json.load(sys.stdin)))
    elif args.status:
        db = ledger(Path(cfg['state_directory']))
        try:
            print(json.dumps([dict(row) for row in db.execute('SELECT * FROM delivery ORDER BY first_seen,action_key')], ensure_ascii=False))
        finally:
            db.close()
    else:
        print(tick(args.config, cfg))


if __name__ == '__main__':
    try:
        main()
    except Exception:
        # Deliberately omit exception text: HTTP/database/config errors may echo
        # credentials. Nonzero status is visible in the host service journal.
        sys.exit('notification unavailable; check private configuration, database and state directory')
