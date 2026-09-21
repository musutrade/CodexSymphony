#!/usr/bin/env python3
"""Administrator-only cold backup and isolated restore. Disabled until configured."""
import argparse
import contextlib
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import select
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile
import time
import uuid

from common import Refused, digest, environment, facts, identity, private, quoted, read_json, require, run, sql, sync_directory, write_json
from crypto import decrypt, encrypt, key
from offsite import request, transfer


@contextlib.contextmanager
def locked(path):
    with private(path).open('r+b') as stream:
        fcntl.flock(stream, fcntl.LOCK_EX | fcntl.LOCK_NB)
        yield


def stopped(cfg):
    if cfg['fixture']:
        supplied = environment(os.environ.get('TEST_DATABASE_URL', ''))
        source = environment(cfg['database'])
        require(identity(supplied)[:2] == identity(source)[:2] and source['PGDATABASE'].startswith('symphony_backup_'), 'fixture must use a dedicated database on supplied test endpoint')
        return
    require(cfg['controller_lock'] == '/tmp/codexsymphony-controller.lock', 'production controller lock is fixed')
    require(len(cfg['units']) >= 3, 'controller, notifier timer/service and executor units required')
    for unit in cfg['units']:
        require(re.fullmatch(r'[A-Za-z0-9_.@-]+\.(service|timer|slice)', unit), 'invalid unit')
        state = run(['systemctl', 'show', unit, '--property=ActiveState,LoadState,UnitFileState,ControlGroup']).decode()
        properties = dict(line.split('=', 1) for line in state.splitlines())
        require(properties.get('ActiveState') == 'inactive' and properties.get('UnitFileState') in ('masked', 'masked-runtime'),
                'all writers must be stopped and masked by administrator')
        group = properties.get('ControlGroup')
        if group:
            root = Path('/sys/fs/cgroup') / group.lstrip('/')
            require(not root.exists() or all(not p.read_text().strip() for p in root.rglob('cgroup.procs')),
                    'writer descendants still alive')


@contextlib.contextmanager
def database_snapshot(env):
    require(sql(env, "SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname NOT IN ('public','pg_catalog','information_schema') AND n.nspname NOT LIKE 'pg_toast%' AND n.nspname NOT LIKE 'pg_temp%' AND c.relkind IN ('r','S','v','m','f')") == '0', 'additional database schemas require reviewed backup scope')
    tables = json.loads(sql(env, "SELECT json_agg(tablename ORDER BY tablename) FROM pg_tables WHERE schemaname='public'"))
    require(tables and 'platform_session' in tables, 'M2 schema required')
    process = subprocess.Popen(['psql', '-XqAt', '-v', 'ON_ERROR_STOP=1'], env=env,
                               stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    try:
        statement = 'BEGIN; LOCK TABLE ' + ','.join('public.' + quoted(t) for t in tables) + ' IN SHARE MODE; SELECT pg_export_snapshot();\n'
        process.stdin.write(statement.encode()); process.stdin.flush()
        require(select.select([process.stdout], [], [], 15)[0], 'database snapshot timeout')
        snapshot = process.stdout.readline().decode().strip()
        require(re.fullmatch(r'[0-9A-Fa-f]+-[0-9A-Fa-f]+-[0-9]+', snapshot), 'database snapshot lock failed')
        yield snapshot
        require(process.poll() is None, 'database snapshot connection lost')
    finally:
        process.stdin.close()
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill(); process.wait()
        process.stdout.close()


def roots(cfg):
    require(set(cfg['roots']) == {'execution', 'cold', 'configuration', 'notifier'}, 'all four material roots required')
    result = {}
    for label, value in cfg['roots'].items():
        path = private(value['path'], True)
        info = path.stat()
        require([info.st_dev, info.st_ino] == value['identity'], 'material mount identity changed')
        result[label] = path
    paths = list(result.values()) + [private(cfg['destination'], True)]
    require(all(a != b and a not in b.parents and b not in a.parents for i, a in enumerate(paths) for b in paths[i+1:]), 'overlapping material/destination roots')
    return result


def inventory(materials):
    result = {}
    for label, root in materials.items():
        for path in [root, *sorted(root.rglob('*'))]:
            info = path.lstat()
            name = label + '/' + str(path.relative_to(root))
            if stat.S_ISLNK(info.st_mode):
                # Store the link itself, never follow it into credential/host paths.
                result[name] = {'link': os.readlink(path)}
            elif stat.S_ISREG(info.st_mode):
                require(info.st_mode & 0o444 and os.access(path, os.R_OK), 'unreadable material')
                result[name] = {'sha256': digest(path), 'size': info.st_size, 'mode': stat.S_IMODE(info.st_mode)}
            else:
                require(stat.S_ISDIR(info.st_mode), 'special material file rejected')
                require(info.st_mode & 0o400 and info.st_mode & 0o100 and os.access(path, os.R_OK | os.X_OK), 'unreadable material directory')
                result[name] = {'directory': True, 'mode': stat.S_IMODE(info.st_mode)}
    return result


def check_references(env, materials):
    # Full roots include Git common dirs/worktrees, preservation archives, runtime
    # evidence, notifier SQLite+WAL and deployment configuration recovery references.
    paths = json.loads(sql(env, "SELECT coalesce(json_agg(path),'[]') FROM (SELECT path FROM storage_material WHERE status IN ('available','archiving','deleting') AND category<>'database' UNION SELECT archive->>'path' FROM storage_material WHERE status='archived') required"))
    for value in paths:
        require(isinstance(value, str), 'archived material recovery reference missing')
        path = Path(value)
        require(path.is_absolute() and any(path == root or root in path.parents for root in materials.values()), 'database material outside backup roots')
        require(path.exists() and not path.is_symlink(), 'referenced material missing')


def configured(path):
    cfg = read_json(path)
    if cfg == {'enabled': False}:
        return cfg
    required = {'enabled', 'fixture', 'database', 'controller_lock', 'notifier_lock', 'units', 'roots',
                'destination', 'key_file', 'recovery_references', 'source_sha', 'binary', 'binary_sha256',
                'retain_count', 'max_bytes', 'offsite'}
    require(set(cfg) == required and cfg['enabled'] is True and type(cfg['fixture']) is bool, 'incomplete backup configuration')
    require(re.fullmatch('[0-9a-f]{40}', cfg['source_sha']), 'exact verified source SHA required')
    require(digest(private(cfg['binary'])) == cfg['binary_sha256'], 'verified binary identity mismatch')
    require(type(cfg['retain_count']) is int and 1 <= cfg['retain_count'] <= 100 and cfg['max_bytes'] > 0, 'finite retention required')
    refs = read_json(cfg['recovery_references'])
    require(set(refs) == {'database', 'auth', 'github', 'signing', 'bark', 'backup_key'}, 'all key/config recovery references required')
    require(all(isinstance(v, str) and v for v in refs.values()), 'non-secret custody references required')
    key(cfg['key_file'])
    material_roots = roots(cfg)
    require(Path(cfg['notifier_lock']) == material_roots['notifier']/'worker.lock', 'actual notifier worker lock required')
    require(all(Path(cfg['key_file']) != root and root not in Path(cfg['key_file']).parents for root in material_roots.values()), 'backup key must have separate custody')
    if cfg['offsite']:
        require(set(cfg['offsite']) == {'url','ca','certificate','private_key','fixture','max_object_bytes'}, 'incomplete offsite configuration')
        require(cfg['offsite']['fixture'] is cfg['fixture'], 'fixture transport cannot claim production offsite')
    return cfg


def pack(cfg, directory, env, snapshot):
    materials = roots(cfg)
    check_references(env, materials)
    before = inventory(materials)
    require(sum(entry.get('size', 0) for entry in before.values()) <= cfg['max_bytes'], 'materials exceed backup budget')
    db = directory/'database.dump'
    run(['pg_dump', '--format=custom', '--no-owner', '--no-acl', '--schema=public', '--snapshot=' + snapshot, '--file=' + str(db)], env)
    manifest = {'schema': 'symphony-backup/v1', 'created_at': int(time.time()), 'consistency': 'stopped-writers+share-locks+exported-snapshot',
                'fixture': cfg['fixture'], 'source_sha': cfg['source_sha'], 'binary_sha256': cfg['binary_sha256'],
                'database_sha256': digest(db), 'facts': facts(env), 'materials': before,
                'original_roots': {label: str(path) for label, path in materials.items()},
                'recovery_references': read_json(cfg['recovery_references'])}
    write_json(directory/'manifest.json', manifest)
    archive = directory/'payload.tar'
    with tarfile.open(archive, 'w', dereference=False) as tar:
        tar.add(db, arcname='database.dump')
        tar.add(directory/'manifest.json', arcname='manifest.json')
        for label, path in materials.items():
            tar.add(path, arcname='materials/' + label)
    require(before == inventory(materials) and manifest['facts'] == facts(env), 'data changed during cold backup')
    stopped(cfg)
    return archive


def target_identity(cfg):
    return hashlib.sha256(cfg['url'].encode()).hexdigest() if cfg else None


def retention(cfg, archive):
    destination = archive.parent
    # Only files paired with this tool's validated receipt may be reclaimed.
    backups = sorted(destination.glob('backup-*.receipt.json'), key=lambda p: p.name, reverse=True)
    for receipt in backups[cfg['retain_count']:]:
        record = read_json(receipt)
        name = receipt.name.removesuffix('.receipt.json') + '.enc'
        old = private(destination/name)
        require(digest(old) == record['sha256'], 'retention refuses unverified old archive')
        if record['offsite'] != 'not_configured':
            require(cfg['offsite'] and record['target'] == target_identity(cfg['offsite']), 'offsite target changed; administrator retention reconciliation required')
            request(cfg['offsite'], 'DELETE', old.name)
        old.unlink(); receipt.unlink()


def backup(cfg):
    env = environment(cfg['database'])
    stopped(cfg)
    destination = private(cfg['destination'], True)
    with locked(cfg['controller_lock']), locked(cfg['notifier_lock']), database_snapshot(env) as snapshot:
        with tempfile.TemporaryDirectory(prefix='.backup-', dir=destination) as temp:
            directory = Path(temp)
            payload = pack(cfg, directory, env, snapshot)
            require(payload.stat().st_size <= cfg['max_bytes'], 'backup exceeds storage budget')
            encrypted = directory/'encrypted'
            encrypt(payload, encrypted, key(cfg['key_file']))
            used = sum(p.stat().st_size for p in destination.glob('backup-*.enc'))
            require(used + encrypted.stat().st_size <= cfg['max_bytes'], 'backup destination capacity exhausted; previous copies retained')
            name = 'backup-' + str(time.time_ns()) + '-' + uuid.uuid4().hex
            archive = destination/(name + '.enc')
            encrypted.rename(archive)
            sync_directory(destination)
    checksum = digest(archive)
    status = 'not_configured'
    # Keep local encrypted copy if HTTPS fails; never claim offsite success.
    try:
        if cfg['offsite']:
            status = transfer(cfg['offsite'], archive, checksum)
    except Exception:
        write_json(destination/(name + '.receipt.json'), {'sha256': checksum, 'offsite': 'failed', 'target': target_identity(cfg['offsite'])})
        raise Refused('local encrypted copy retained; offsite transfer/verification failed') from None
    write_json(destination/(name + '.receipt.json'), {'sha256': checksum, 'offsite': status, 'target': target_identity(cfg['offsite'])})
    retention(cfg, archive)
    return {'archive': archive.name, 'sha256': checksum, 'offsite': status}


def unpack(archive, directory, secret):
    payload = directory/'payload.tar'
    decrypt(archive, payload, secret)
    extracted = directory/'verified'
    extracted.mkdir(mode=0o700)
    with tarfile.open(payload) as tar:
        members = tar.getmembers()
        names = [m.name for m in members]
        require(len(names) == len(set(names)), 'duplicate archive entries')
        for item in members:
            path = Path(item.name)
            require(not path.is_absolute() and '..' not in path.parts and path.parts[0] in ('manifest.json', 'database.dump', 'materials'), 'unsafe archive path')
            require(item.isfile() or item.isdir() or item.issym() or item.islnk(), 'special archive entry')
        # Never use extractall: validate every parent, create regular entries first,
        # then links. Hardlinks are materialized as regular files, retaining bytes.
        for item in members:
            if item.issym():
                continue
            path = extracted/item.name
            require(path.parent.resolve().is_relative_to(extracted), 'unsafe archive parent')
            if item.isdir():
                path.mkdir(parents=True, exist_ok=True)
            else:
                path.parent.mkdir(parents=True, exist_ok=True)
                content = tar.extractfile(item)
                require(content is not None, 'missing archive content')
                with path.open('xb') as output:
                    shutil.copyfileobj(content, output)
            require(not item.mode & 0o7000, 'privileged mode rejected')
            path.chmod(item.mode)
        for link in [m for m in members if m.issym()]:
            path = extracted/link.name
            require(not path.exists() and path.parent.resolve().is_relative_to(extracted), 'unsafe link parent')
            path.symlink_to(link.linkname)
    manifest = read_json(extracted/'manifest.json')
    require(manifest['schema'] == 'symphony-backup/v1', 'unsupported backup manifest')
    require(digest(extracted/'database.dump') == manifest['database_sha256'], 'database checksum mismatch')
    material = {label: extracted/'materials'/label for label in manifest['original_roots']}
    require(inventory(material) == manifest['materials'], 'missing or corrupt materials')
    payload.unlink()
    return extracted, manifest


def verify(cfg, archive):
    with tempfile.TemporaryDirectory(prefix='.verify-', dir=private(cfg['destination'], True)) as temp:
        _, manifest = unpack(archive, Path(temp), key(cfg['key_file']))
        return {'status': 'verified', 'source_sha': manifest['source_sha'], 'binary_sha256': manifest['binary_sha256']}


def restore(cfg, archive, target_path):
    target = read_json(target_path)
    require(set(target) == {'database', 'directory'}, 'isolated target configuration required')
    env = environment(target['database'])
    require(identity(env) != identity(environment(cfg['database'])), 'source database cannot be restored over')
    require(re.fullmatch(r'symphony_restore_[a-z0-9_]+', env['PGDATABASE']), 'dedicated symphony_restore_ database required')
    directory = private(target['directory'], True)
    require(not list(directory.iterdir()), 'restore directory must be empty; repeated restore refused')
    require(sql(env, "SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND n.nspname NOT LIKE 'pg_toast%' AND n.nspname NOT LIKE 'pg_temp%' AND c.relkind IN ('r','S','v','m','f')") == '0', 'restore database must be empty; repeated restore refused')
    # Durable guard BEFORE executing any archived SQL, retained on any failure.
    sql(env, 'CREATE TABLE public.symphony_recovery_guard (id integer PRIMARY KEY CHECK(id=1)); INSERT INTO public.symphony_recovery_guard VALUES(1);')
    with tempfile.TemporaryDirectory(prefix='.restore-', dir=directory) as temp:
        extracted, manifest = unpack(archive, Path(temp), key(cfg['key_file']))
        # The empty target already owns public (and its durable guard). Preserve
        # that schema while restoring every archived object, including sequences.
        listing = run(['pg_restore', '--list', str(extracted/'database.dump')], env).decode()
        entries = listing.splitlines()
        selected = [line for line in entries if not re.match(r'^\d+; \d+ \d+ SCHEMA - public ', line)]
        require(len(entries) - len(selected) == 1, 'expected public schema archive entry missing')
        toc = Path(temp)/'restore.list'
        toc.write_text('\n'.join(selected) + '\n')
        run(['pg_restore', '--exit-on-error', '--single-transaction', '--no-owner', '--no-acl',
             '--use-list=' + str(toc), '--dbname=' + env['PGDATABASE'], str(extracted/'database.dump')], env)
        require(facts(env) == manifest['facts'], 'restored database fact mismatch; guard retained')
        shutil.move(str(extracted/'materials'), directory/'materials')
        write_json(directory/'manifest.json', manifest)
    return {'status': 'isolated_restore_verified', 'source_sha': manifest['source_sha'],
            'external_actions': 'disabled; recovery guard retained', 'production_activation': 'administrator recovery procedure required'}


def main():
    os.umask(0o077)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config', required=True)
    parser.add_argument('action', choices=['status', 'backup', 'verify', 'restore'])
    parser.add_argument('--archive')
    parser.add_argument('--target')
    args = parser.parse_args()
    try:
        cfg = configured(args.config)
        if not cfg['enabled']:
            require(args.action == 'status', 'backup disabled; configure administrator inputs')
            result = {'status': 'disabled', 'offsite': 'not_configured'}
        elif args.action == 'status':
            result = {'status': 'configured', 'offsite': 'configured_unverified' if cfg['offsite'] else 'not_configured'}
        elif args.action == 'backup':
            result = backup(cfg)
        elif args.action == 'verify':
            result = verify(cfg, private(args.archive))
        else:
            result = restore(cfg, private(args.archive), args.target)
        print(json.dumps(result))
    except Refused as error:
        print(json.dumps({'status': 'refused', 'reason': str(error)})); return 1
    except Exception:
        print(json.dumps({'status': 'failed', 'reason': 'private input, integrity, permission or controlled service failure; inspect administrator configuration'})); return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
