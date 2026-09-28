"""Two fixed disposable PostgreSQL slots, exclusively leased by the Gate host.

A process crash releases flock. The next owner checks the installed database
contract and restarts the tmpfs-backed cluster before use. Names and Docker log growth are bounded.
"""
import atexit
import fcntl
import json
import os
from pathlib import Path
import subprocess
import time

import environment_contract as contract

VOLUME = Path('/mnt/dev-ssd/codexsymphony-bounded/data')
DIRECTORY = VOLUME / 'cache/database'
OWNER = 'codexsymphony-fixed-validation/v1'
LEASES = {}


def docker(*args):
    return subprocess.check_output(['docker', *map(str, args)], stderr=subprocess.PIPE, text=True).strip()


def inspection(name):
    # Listing by exact name distinguishes absence from Docker daemon failure.
    rows = docker('ps', '-aq', '--no-trunc', '--filter', 'name=^/' + name + '$').split()
    if not rows:
        return None
    if len(rows) != 1:
        raise ValueError('ambiguous fixed database identity')
    return json.loads(docker('inspect', rows[0]))[0]


def lock_slot(role):
    if not VOLUME.is_mount() or DIRECTORY.resolve() != DIRECTORY:
        raise ValueError('bounded database storage unavailable')
    DIRECTORY.mkdir(mode=0o700, parents=True, exist_ok=True)
    descriptor = os.open(DIRECTORY / (role + '.lock'), os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
    try:
        fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BaseException:
        os.close(descriptor)
        raise
    return descriptor


def tmpfs_options(policy):
    return 'rw,size=' + str(policy['postgres']['test']['memory'])


def validate(info, name, policy):
    if info['Name'] != '/' + name or info['Config'].get('Labels', {}).get('codexsymphony.owner') != OWNER:
        raise ValueError('refuse to adopt unrelated database')
    if info['HostConfig']['LogConfig'] != {'Type': 'local', 'Config': {'max-size': '1m', 'max-file': '2'}}:
        raise ValueError('database log bound changed')
    if info['HostConfig'].get('Tmpfs', {}).get('/var/lib/postgresql/data') != tmpfs_options(policy):
        raise ValueError('database temporary storage bound changed')
    if any(mount['Type'] != 'tmpfs' for mount in info['Mounts']):
        raise ValueError('persistent test database mount is forbidden')
    return contract.check_database(info, policy)


def create(name, policy):
    docker('create', '--name', name, '--label', 'codexsymphony.owner=' + OWNER,
           '--restart', 'no', '--log-driver', 'local', '--log-opt', 'max-size=1m', '--log-opt', 'max-file=2',
           '--publish', '127.0.0.1::5432', '--env', 'POSTGRES_DB=gate_test', '--env', 'POSTGRES_USER=gate_test',
           '--env', 'POSTGRES_PASSWORD=gate_test', '--tmpfs', '/var/lib/postgresql/data:' + tmpfs_options(policy),
           *contract.database_args(policy), policy['postgres']['image'])


def ready(name):
    for _ in range(60):
        result = subprocess.run(['docker', 'exec', name, 'pg_isready', '--host', '127.0.0.1',
                                 '-U', 'gate_test', '-d', 'postgres'], capture_output=True)
        if result.returncode == 0:
            return
        time.sleep(.5)
    raise RuntimeError('fixed test database did not become ready')


def psql(name, query):
    return docker('exec', '--user', 'postgres', name, 'psql', '-X', '-v', 'ON_ERROR_STOP=1',
                  '-U', 'gate_test', '-d', 'postgres', '-Atc', query)


def stop_idle(name):
    count = psql(name, "SELECT count(*) FROM pg_stat_activity WHERE backend_type='client backend' AND pid<>pg_backend_pid()")
    if count != '0':
        raise ValueError('test database still has clients; refuse stop')
    docker('stop', '--time', '5', name)


def provision(name, policy):
    info = inspection(name)
    if info is None:
        create(name, policy)
        info = inspection(name)
    validate(info, name, policy)
    if info['State']['Running']:
        ready(name)
        stop_idle(name)
    # Restarting the fixed container remounts an empty, capacity-bounded tmpfs.
    # This resets all databases, roles and cluster settings, not just gate_test.
    docker('start', name)
    ready(name)
    info = inspection(name)
    proof = validate(info, name, policy)
    ports = info['NetworkSettings']['Ports']['5432/tcp']
    if len(ports) != 1 or ports[0]['HostIp'] != '127.0.0.1':
        raise ValueError('test database publication changed')
    return proof, 'postgres://gate_test:gate_test@127.0.0.1:' + ports[0]['HostPort'] + '/gate_test'


def acquire(run, purpose='', repository=None):
    role = 'http' if purpose == '-http' else 'primary'
    name = 'codexsymphony-gate-fixed-' + role
    if name in LEASES:
        raise ValueError('fixed database slot is already leased by this process')
    descriptor = lock_slot(role)
    try:
        proof, url = provision(name, contract.load(repository))
        (run / ('database' + purpose + '.json')).write_text(json.dumps(proof, indent=2) + '\n')
        LEASES[name] = descriptor
        return name, url
    except BaseException:
        os.close(descriptor)
        raise


def release(name):
    descriptor = LEASES.pop(name)
    try:
        info = inspection(name)
        if info is not None and info['State']['Running']:
            stop_idle(name)
    finally:
        os.close(descriptor)


def release_on_exit():
    # Best effort on ordinary exit. Crash recovery always checks and restarts the temporary cluster again.
    for name in list(LEASES):
        try:
            release(name)
        except Exception:
            pass


atexit.register(release_on_exit)
