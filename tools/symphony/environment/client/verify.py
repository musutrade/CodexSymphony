import errno, hashlib, json, os, subprocess, sys, uuid
from pathlib import Path
from dbctl import request

def sql(role,statement):
    return subprocess.check_output(['psql',os.environ['TEST_DATABASE_URL' if role=='test' else 'DEV_DATABASE_URL'],
                                    '-X','-v','ON_ERROR_STOP=1','-Atc',statement],text=True).strip()

source=Path('/home/gem/arc-admin')
manifest=json.loads((source/'SOURCE.json').read_text())
for name,identity in manifest['files'].items():
    assert hashlib.sha256((source/name).read_bytes()).hexdigest()==identity['sha256']
assert manifest['commit']=='2faa1ca6c1a2b1a45956540e97e68a01532a40f7'
for path in [source/'write-probe']:
    try:
        with path.open('x') as f:f.write('probe')
        path.unlink();raise AssertionError('Protected path is writable: '+str(path))
    except OSError as error: assert error.errno in [errno.EROFS,errno.EACCES,errno.EPERM],error
assert not Path('/home/gem/.local/share/codexsymphony/gate-host/approval.json').exists()
assert not Path('/home/gem/.secrets/my-disposable-bot.2026-09-08.private-key.pem').exists()
proof={'source_sha':manifest['commit'],'source_hashes':'PASS','readonly_reference_source':'PASS',
       'host_gate_and_key_hidden':True,'select_1':{},'recreation':{}}
table='environment_probe_'+uuid.uuid4().hex
for role in ['test','dev']:
    assert sql(role,'SELECT 1')=='1'
    proof['select_1'][role]='PASS'
    sql(role,f'CREATE TABLE {table}(value text); INSERT INTO {table} VALUES (\'gh12-environment\')')
    result=request('recreate',role)
    assert result['before_id']!=result['container_id']
    if role=='dev':
        assert sql(role,f'SELECT value FROM {table}')=='gh12-environment'
        sql(role,f'DROP TABLE {table}')
        result['retained_row']='PASS'
    else:
        assert sql(role,f"SELECT to_regclass('public.{table}') IS NULL")=='t'
        result['discarded_row']='PASS'
    proof['recreation'][role]=result
target=Path('target/gh12-environment-verified.json');target.parent.mkdir(exist_ok=True)
target.write_text(json.dumps(proof,indent=2)+'\n')
print(json.dumps(proof))
