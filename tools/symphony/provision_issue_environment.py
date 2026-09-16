"""Provision an independent development fixture for any assigned GH workspace."""
import ipaddress, json, os, re, shutil, subprocess, sys, time
from pathlib import Path

BASE=Path(__file__).parent
WORKSPACES=BASE.parent/'workspaces'
TEMPLATE=BASE/'environment-template'
IMAGE='sha256:57c72fd2a128e416c7fcc499958864df5301e940bca0a56f58fddf30ffc07777'

def run(*args):
    result=subprocess.run(args,check=True,capture_output=True,text=True,timeout=90)
    return result.stdout.strip()

def available_subnet(networks):
    used=[ipaddress.ip_network(config['Subnet']) for network in networks
          for config in (network.get('IPAM',{}).get('Config') or []) if config.get('Subnet')]
    for candidate in ipaddress.ip_network('172.30.0.0/16').subnets(new_prefix=24):
        if not any(other.version==4 and candidate.overlaps(other) for other in used):
            return str(candidate.network_address).rsplit('.',1)[0]
    raise RuntimeError('No free fixture subnet in 172.30.0.0/16; host provisioning required')

def wait_ready(name,user):
    deadline=time.monotonic()+60
    while time.monotonic()<deadline:
        result=subprocess.run(['docker','exec',name,'pg_isready','-h','127.0.0.1','-U',user],
                              capture_output=True,text=True,timeout=5)
        if result.returncode==0:return
        time.sleep(.5)
    raise RuntimeError('Fixture PostgreSQL did not become ready: '+name)

def main(workspace):
    workspace=Path(workspace).resolve()
    if workspace.parent!=WORKSPACES or not re.fullmatch(r'GH-\d+',workspace.name):
        raise ValueError('Assigned GH workspace required')
    label=workspace.name;short=label.lower().replace('-','')
    provision=BASE/(short+'-environment')
    provision.mkdir(mode=0o700,exist_ok=True)
    network='codexsymphony-'+short+'-env'
    identifiers=run('docker','network','ls','--format','{{.ID}}').splitlines()
    networks=json.loads(run('docker','network','inspect',*identifiers))
    existing=next((item for item in networks if item['Name']==network),None)
    if existing:
        assert existing['Internal'] and (existing['Labels'].get('codexsymphony.fixture')==label
                                        or (provision/'initial-containers.json').is_file())
        subnet=existing['IPAM']['Config'][0]['Subnet'].split('/')[0].rsplit('.',1)[0]
    else:
        subnet=available_subnet(networks)
        run('docker','network','create','--internal','--label','codexsymphony.fixture='+label,'--subnet',subnet+'.0/24',network)
    guide=workspace/'.agent-env'
    if guide.is_symlink(): raise ValueError('Environment directory must not be a symlink')
    guide.mkdir(exist_ok=True)
    for name in ['requests','npm-cache','README.md']:
        if (guide/name).is_symlink(): raise ValueError('Environment paths must not be symlinks')
    (guide/'requests').mkdir(exist_ok=True)
    exclude=workspace/'.git/info/exclude'
    if '.agent-env/' not in exclude.read_text():
        with exclude.open('a') as f:f.write('\n.agent-env/\n')
    def adapt(text):
        return text.replace('GH-12',label).replace('gh12',short).replace('172.30.212',subnet)
    if not (provision/'client').exists():
        shutil.copytree(TEMPLATE/'client',provision/'client',ignore=shutil.ignore_patterns('__pycache__'))
        for path in (provision/'client').rglob('*'):
            if path.is_file() and 'reviewed-preparation' not in path.parts and (path.suffix=='.py' or path.parent.name=='bin'):
                path.write_text(adapt(path.read_text()))
        p=provision/'client/e2e.py'
        p.write_text(p.read_text().replace("env['BIND_ADDRESS']='127.0.0.1:3081'", "env['BIND_ADDRESS']='127.0.0.1:3081';env['WEB_ORIGIN']='http://127.0.0.1:4300'"))
        shutil.copytree(TEMPLATE/'arc-admin',provision/'arc-admin')
        for name in ['broker.py','preflight.py']:
            (provision/name).write_text(adapt((TEMPLATE/name).read_text()))
        source=Path('/etc/codex/requirements.toml').read_text()
        source=source.replace('domains = {',f'domains = {{ "{subnet}.2" = "allow", "{subnet}.3" = "allow",')
        (provision/'requirements.toml').write_text(source)
    # Refresh the launcher on every provision, including existing workspaces.
    launcher=provision/'client/run.py'
    pending=launcher.with_suffix('.new')
    pending.write_text(adapt((TEMPLATE/'client/run.py').read_text()))
    pending.replace(launcher)
    broker_changed=False
    for name in ['broker.py','preflight.py','execution_readiness.py','client/execution_readiness.py',
                 'runtime_command_readiness.py','client/runtime_command_readiness.py',
                 'runtime_product_acceptance.py','client/runtime_product_acceptance.py',
                 'product_preparation_acceptance.py','client/product_preparation_acceptance.py','client/runtime_smoke.py',
                 'client/reviewed-preparation/app_server.py','client/reviewed-preparation/sandbox_probe.py',
                 'client/reviewed-preparation/manifest.json']:
        destination=provision/name
        destination.parent.mkdir(parents=True,exist_ok=True)
        content=(TEMPLATE/name).read_text()
        if 'reviewed-preparation/' not in name:content=adapt(content)
        if not destination.exists() or destination.read_text()!=content:
            pending=destination.with_suffix('.new');pending.write_text(content);pending.replace(destination)
            broker_changed=True
    known={}
    names=run('docker','ps','-a','--format','{{.Names}}').splitlines()
    for role,last in [('test','2'),('dev','3')]:
        name='codexsymphony-'+short+'-'+role;user='codexsymphony_'+role
        if name not in names:
            args=['docker','run','-d','--restart','unless-stopped','--name',name,'--label','codexsymphony.fixture='+label+'-'+role,
                  '--network',network,'--ip',subnet+'.'+last,'--user','postgres','--cap-drop','ALL',
                  '--security-opt','no-new-privileges','--memory','512m','--cpus','1',
                  '-e','POSTGRES_USER='+user,'-e','POSTGRES_PASSWORD='+user,'-e','POSTGRES_DB='+user]
            if role=='test': args+=['--tmpfs','/var/lib/postgresql/data:uid=70,gid=70,mode=0700']
            else:
                volume='codexsymphony-'+short+'-dev-data'
                run('docker','volume','create','--label','codexsymphony.fixture='+label,volume)
                assert json.loads(run('docker','volume','inspect',volume))[0]['Labels'].get('codexsymphony.fixture')==label
                args+=['--mount','type=volume,source='+volume+',target=/var/lib/postgresql/data']
            run(*args,IMAGE)
        state=json.loads(run('docker','inspect',name))[0]
        assert state['Image']==IMAGE and state['Config']['Labels'].get('codexsymphony.fixture')==label+'-'+role
        assert state['NetworkSettings']['Networks'][network]['IPAMConfig']['IPv4Address']==subnet+'.'+last
        known[role]=state['Id']
        if not state['State']['Running']:run('docker','start',name)
        wait_ready(name,user)
    if not (provision/'initial-containers.json').exists():
        (provision/'initial-containers.json').write_text(json.dumps(known))
    cache=guide/'npm-cache'
    if not cache.exists():
        run('cp','-a','--reflink=auto',str(TEMPLATE/'npm-cache-seed'),str(cache))
    (guide/'README.md').write_text(adapt((TEMPLATE/'environment-guide-template.md').read_text()))
    unit=Path('/home/gem/.config/systemd/user')/('codexsymphony-'+short+'-db.service')
    if not unit.exists():
        unit.write_text('[Unit]\nDescription='+label+' fixed-policy test database lifecycle\n[Service]\nExecStart=/usr/bin/python3 '+str(provision/'broker.py')+'\nRestart=on-failure\nRestartSec=3\n[Install]\nWantedBy=default.target\n')
        run('systemctl','--user','daemon-reload')
        run('systemctl','--user','enable','--now',unit.name)
    else:run('systemctl','--user','restart' if broker_changed else 'start',unit.name)
    print(label+' isolated environment ready; read .agent-env/README.md')

if __name__=='__main__': main(sys.argv[1] if len(sys.argv)>1 else Path.cwd())
