"""GH-12 operator fixtures only; never accepts a user-selected database/container.

Run with: python3 /opt/gh12-env/run.py python3 docs/quality/gh12/probe-fixtures.py
The broker recreates its two fixed synthetic containers. Do not run concurrently
with database tests. Compose rendering needs its CLI, not access to Docker.
"""
import hashlib
import json
import os
from pathlib import Path
import subprocess
from urllib.parse import urlparse
import uuid


def sql(role, statement):
    url = os.environ['TEST_DATABASE_URL' if role == 'test' else 'DEV_DATABASE_URL']
    parsed = urlparse(url)
    assert parsed.hostname == '127.0.0.1'
    assert parsed.port == {'test': 54329, 'dev': 54330}[role]
    assert parsed.username == 'codexsymphony_' + role
    assert parsed.path == '/codexsymphony_' + role
    return subprocess.check_output(
        ['psql', url, '-X', '-v', 'ON_ERROR_STOP=1', '-Atc', statement], text=True
    ).strip()


def control(operation, role):
    return json.loads(subprocess.check_output(
        ['python3', '/opt/gh12-env/dbctl.py', operation, role], text=True
    ))


def compose(path):
    config = json.loads(subprocess.check_output(
        ['docker', 'compose', '--env-file', '.env.example', '-f', path,
         'config', '--format', 'json'], text=True
    ))
    return config['services']['postgres']


def main():
    dev = compose('docker-compose.dev.yml')
    test = compose('docker-compose.yml')
    assert dev['image'] == test['image'] == 'postgres:16-alpine'
    assert dev['volumes'][0]['type'] == 'volume'
    assert dev['volumes'][0]['target'] == '/var/lib/postgresql/data'
    assert not dev.get('tmpfs')
    assert test['tmpfs'] == ['/var/lib/postgresql/data']
    assert not test.get('volumes')
    for role, config, port in [('dev', dev, '54330'), ('test', test, '54329')]:
        assert config['environment']['POSTGRES_DB'] == 'codexsymphony_' + role
        assert config['environment']['POSTGRES_USER'] == 'codexsymphony_' + role
        assert config['ports'][0]['host_ip'] == '127.0.0.1'
        assert config['ports'][0]['published'] == port
        assert sql(role, 'SELECT current_database()') == 'codexsymphony_' + role
    proof = {'compose_structure': 'PASS', 'files': {}, 'fixtures': {}}
    for name in ['docker-compose.yml', 'docker-compose.dev.yml', '.env.example']:
        proof['files'][name] = hashlib.sha256(Path(name).read_bytes()).hexdigest()
    table = 'gh12_acceptance_' + uuid.uuid4().hex
    for role, storage in [('dev', 'named-volume'), ('test', 'tmpfs')]:
        assert control('status', role)['storage'] == storage
        sql(role, f"CREATE TABLE {table}(value text); INSERT INTO {table} VALUES ('gh12-synthetic')")
        other = 'test' if role == 'dev' else 'dev'
        try:
            assert sql(other, f"SELECT to_regclass('public.{table}') IS NULL") == 't'
            result = control('recreate', role)
            assert result['before_id'] != result['container_id']
            assert result['storage'] == storage
            if role == 'dev':
                assert sql(role, f'SELECT value FROM {table}') == 'gh12-synthetic'
                result['retained_row'] = 'PASS'
            else:
                assert sql(role, f"SELECT to_regclass('public.{table}') IS NULL") == 't'
                result['discarded_row'] = 'PASS'
            result['cross_database_isolation'] = 'PASS'
            proof['fixtures'][role] = result
        finally:
            sql(role, f'DROP TABLE IF EXISTS {table}')
    proof['limit'] = ('Compose is rendered and checked against the fixed fixture storage, database roles and ports. '
                      'The host broker owns container names/network/passwords; workspace Docker did not create these containers. '
                      'No real user database or disaster recovery was tested.')
    Path('target/gh12-persistence.json').write_text(json.dumps(proof, indent=2) + '\n')
    print(json.dumps(proof))


if __name__ == '__main__':
    main()
