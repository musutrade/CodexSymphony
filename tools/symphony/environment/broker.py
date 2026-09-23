"""Host-owned, fixed-policy lifecycle for the two GH-12 test fixtures only."""
import json, os, re, subprocess, time
from pathlib import Path

BASE=Path(__file__).parent
SPOOL=Path('/home/gem/.local/share/codexsymphony/workspaces/GH-12/.agent-env/requests')
IMAGE='sha256:57c72fd2a128e416c7fcc499958864df5301e940bca0a56f58fddf30ffc07777'
NETWORK='codexsymphony-gh12-env'

def fixture_policy():
    return json.loads((BASE/'fixture-policy.json').read_text())

def memory_bytes(role):
    setting=fixture_policy()[role+'_memory']
    return int(setting[:-1])*(1024**3 if setting.endswith('g') else 1024**2)

def memory_events(name):
    lines=docker('exec',name,'cat','/sys/fs/cgroup/memory.events').splitlines()
    return {key:int(value) for key,value in (line.split() for line in lines)}

def docker(*args):
    p=subprocess.run(['docker',*args],capture_output=True,text=True,timeout=60)
    if p.returncode: raise RuntimeError(p.stderr[-2000:])
    return p.stdout.strip()

def info(role):
    value=json.loads(docker('inspect','codexsymphony-gh12-'+role))[0]
    expected=json.loads((BASE/'initial-containers.json').read_text())
    assert value['Image']==IMAGE
    assert NETWORK in value['NetworkSettings']['Networks']
    assert value['Id']==expected[role] or value['Config']['Labels'].get('codexsymphony.fixture')=='GH-12-'+role
    return value

def perform(request):
    if set(request)!={'action','role'} or request['role'] not in ['test','dev'] or request['action'] not in ['status','recreate']:
        raise ValueError('Only status/recreate of test/dev fixtures is allowed')
    role=request['role'];before=info(role);name='codexsymphony-gh12-'+role
    if request['action']=='recreate':
        docker('stop','--time','10',name);docker('rm',name)
        user='codexsymphony_'+role
        policy=fixture_policy()
        args=['run','-d','--restart','unless-stopped','--name',name,'--label','codexsymphony.fixture=GH-12-'+role,
              '--network',NETWORK,'--ip','172.30.212.'+('2' if role=='test' else '3'),
              '--user','postgres','--cap-drop','ALL','--security-opt','no-new-privileges',
              '--memory',policy[role+'_memory'],'--memory-swap',policy[role+'_memory_swap'],
              '--cpus',policy['cpus'],'-e','POSTGRES_USER='+user,
              '-e','POSTGRES_PASSWORD='+user,'-e','POSTGRES_DB='+user]
        if role=='test': args+=['--tmpfs','/var/lib/postgresql/data:uid=70,gid=70,mode=0700']
        else: args+=['--mount','type=volume,source=codexsymphony-gh12-dev-data,target=/var/lib/postgresql/data']
        docker(*args,IMAGE)
        for _ in range(60):
            try:
                docker('exec',name,'psql','-U',user,'-d',user,'-Atc','SELECT 1')
                # Entrypoint may briefly start a temporary server during init.
                state=info(role)
                if state['State']['Running']:
                    time.sleep(1)
                    docker('exec',name,'pg_isready','-h','127.0.0.1','-U',user)
                    break
            except RuntimeError: pass
            time.sleep(.5)
        else: raise TimeoutError('Database did not become ready')
    after=info(role)
    if after['HostConfig']['Memory']!=memory_bytes(role):
        raise RuntimeError('Fixture memory policy mismatch: '+name)
    return {'ok':True,'role':role,'before_id':before['Id'],'container_id':after['Id'],
        'running':after['State']['Running'],'image':after['Image'],
        'storage':'tmpfs' if role=='test' else 'named-volume',
        'memory_limit_bytes':after['HostConfig']['Memory'],
        'memory_events':memory_events(name)}

def main():
    flags=os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW
    root=os.open(SPOOL.parent.parent,flags)
    try:
        environment=os.open('.agent-env',flags,dir_fd=root)
        try: directory=os.open('requests',flags,dir_fd=environment)
        finally: os.close(environment)
    finally: os.close(root)
    while True:
        for name in os.listdir(directory):
            if not re.fullmatch(r'[0-9a-f]{32}\.request\.json',name): continue
            result_name=name.replace('.request.','.result.')
            if result_name in os.listdir(directory): continue
            try:
                fd=os.open(name,os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK,dir_fd=directory)
                with os.fdopen(fd,'rb') as source:
                    import stat
                    if not stat.S_ISREG(os.fstat(source.fileno()).st_mode): raise ValueError('regular request required')
                    raw=source.read(4097)
                if len(raw)>4096: raise ValueError('request too large')
                request=json.loads(raw);result=perform(request)
            except Exception as error: result={'ok':False,'error':str(error)}
            with (BASE/'operations.jsonl').open('a') as log:
                log.write(json.dumps({'time':time.time(),'request_id':name,'result':result})+'\n')
            try:
                fd=os.open(result_name,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600,dir_fd=directory)
                with os.fdopen(fd,'w') as output: json.dump(result,output)
            except FileExistsError: pass
        time.sleep(.2)

if __name__=='__main__': main()
