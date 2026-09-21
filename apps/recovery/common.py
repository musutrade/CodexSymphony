"""Private files and PostgreSQL subprocesses; never print subprocess diagnostics."""
import hashlib
import json
import os
from pathlib import Path
import stat
import subprocess
import urllib.parse


class Refused(Exception):
    pass


def require(value, message):
    if not value:
        raise Refused(message)


def private(path, directory=False):
    path = Path(path)
    require(path.is_absolute(), 'absolute path required')
    info = path.lstat()
    kind = stat.S_ISDIR if directory else stat.S_ISREG
    require(kind(info.st_mode) and info.st_uid == os.geteuid() and not info.st_mode & 0o077,
            'owner-only regular file/directory required')
    # Reject symlink ancestors too; do not traverse redirected credential paths.
    require(all(not parent.is_symlink() for parent in path.parents), 'symlink ancestor rejected')
    return path


def read_json(path):
    return json.loads(private(path).read_text())


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def write_json(path, value):
    with open(path, 'x', opener=lambda p, f: os.open(p, f, 0o600)) as stream:
        json.dump(value, stream, sort_keys=True, indent=2)
        stream.flush()
        os.fsync(stream.fileno())
    sync_directory(Path(path).parent)


def sync_directory(path):
    descriptor = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def environment(uri):
    url = urllib.parse.urlsplit(uri)
    require(url.scheme in ('postgres', 'postgresql') and url.hostname and url.path,
            'PostgreSQL URI required')
    options = urllib.parse.parse_qs(url.query, strict_parsing=True)
    require(set(options) <= {'sslmode', 'sslrootcert', 'sslcert', 'sslkey'}, 'unsupported database URL option')
    tls = {name.upper().replace('SSL', 'PGSSL', 1): values[0] for name, values in options.items()}
    return {**tls, 'PATH': os.environ.get('PATH', '/usr/bin:/bin'),
            'LD_LIBRARY_PATH': os.environ.get('LD_LIBRARY_PATH', ''),
            'PGHOST': url.hostname, 'PGPORT': str(url.port or 5432),
            'PGDATABASE': urllib.parse.unquote(url.path[1:]),
            'PGUSER': urllib.parse.unquote(url.username or ''),
            'PGPASSWORD': urllib.parse.unquote(url.password or ''),
            'PGCONNECT_TIMEOUT': '5', 'PGAPPNAME': 'symphony-recovery',
            'PGOPTIONS': '-c statement_timeout=60000 -c lock_timeout=5000'}


def run(command, env=None, data=None, output=None):
    result = subprocess.run(command, env=env, input=data, stdout=output or subprocess.PIPE,
                            stderr=subprocess.PIPE, timeout=600)
    require(result.returncode == 0, 'controlled subprocess failed (diagnostics withheld)')
    return result.stdout


def sql(env, statement):
    return run(['psql', '-XAt', '-v', 'ON_ERROR_STOP=1'], env, statement.encode()).decode().strip()


def identity(env):
    return tuple(env[k] for k in ('PGHOST', 'PGPORT', 'PGDATABASE'))


def quoted(name):
    return '"' + name.replace('"', '""') + '"'


def facts(env):
    """All public table rows, sequences and notification projection; no raw facts in logs."""
    tables = json.loads(sql(env, "SELECT coalesce(json_agg(tablename ORDER BY tablename),'[]') FROM pg_tables WHERE schemaname='public' AND tablename<>'symphony_recovery_guard'"))
    result = {}
    for table in tables:
        relation = 'public.' + quoted(table)
        # Hash each row inside PostgreSQL: password/session values never leave in plaintext.
        value = sql(env, f"SELECT count(*) || ':' || encode(sha256(convert_to(coalesce(string_agg(h,'' ORDER BY h),''),'UTF8')),'hex') FROM (SELECT encode(sha256(convert_to(row_to_json(t)::text,'UTF8')),'hex') h FROM {relation} t) hashes")
        result[table] = value
    sequences = json.loads(sql(env, "SELECT coalesce(json_agg(sequencename ORDER BY sequencename),'[]') FROM pg_sequences WHERE schemaname='public'"))
    for sequence in sequences:
        result['sequence:' + sequence] = sql(env, f'SELECT last_value::text || \':\' || is_called::text FROM public.{quoted(sequence)}')
    result['view:notification_action'] = sql(env, "SELECT coalesce(string_agg(action_key,',' ORDER BY action_key),'') FROM notification_action")
    return result
