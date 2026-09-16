#!/usr/bin/env python3
"""Prepare/activate a verified fixed-path Symphony migration; activation requires root."""
import argparse, datetime, hashlib, json, os, re, shutil, subprocess, time
from pathlib import Path
from storage_maintenance import gate_busy

BASE=Path('/home/gem/.local/share/codexsymphony')
DEST=Path('/mnt/dev-ssd/codexsymphony-state')
AUDIT=Path('/home/gem/.local/share/codexsymphony-migration')
FSTAB=Path('/etc/fstab')
DISK_UUID='e70b231b-95dd-49b8-85fb-f91b04dd5114'


def systemctl(*args, check=True):
    command=['systemctl','--user',*args]
    if os.geteuid()==0:command=['runuser','-u','gem','--','env','XDG_RUNTIME_DIR=/run/user/1000',*command]
    return subprocess.run(command,check=check,capture_output=True,text=True)


def safe_paths():
    for path in (BASE,DEST,AUDIT):
        if path.resolve()!=path.absolute():raise ValueError('symlink migration path: '+str(path))
    if not os.path.ismount('/mnt/dev-ssd'):raise ValueError('SSD mount missing')
    actual=subprocess.check_output(['findmnt','-n','-o','UUID','--target',str(DEST)],text=True).strip()
    if actual!=DISK_UUID:raise ValueError('unexpected SSD device')
    if BASE.stat().st_uid!=1000 or DEST.stat().st_uid!=1000:raise ValueError('unexpected directory owner')


def units():
    output=systemctl('list-unit-files','codexsymphony*','symphony-codexsymphony*','--no-legend','--no-pager').stdout
    return [line.split()[0] for line in output.splitlines() if line.split() and line.split()[0].endswith(('.service','.timer'))]


def synchronize(dry=False):
    args=['rsync','-aHAXS','--delete','--checksum','--itemize-changes']
    if dry:args.append('--dry-run')
    return subprocess.check_output([*args,str(BASE)+'/',str(DEST)+'/'],text=True)


def prepare():
    if os.geteuid()!=1000:raise ValueError('prepare as gem')
    safe_paths();AUDIT.mkdir(mode=0o700,exist_ok=True)
    if os.path.ismount(BASE):raise ValueError('already mounted; inspect status')
    all_units=units()
    active=[name for name in all_units if systemctl('is-active',name,check=False).stdout.strip()=='active']
    # Symphony was paused explicitly while waiting for the CI/migration window.
    active=sorted(set(active+['symphony-codexsymphony.service','codexsymphony-remote-gate.service',
                             'codexsymphony-storage.timer','codexsymphony-diagnostics.timer','codexsymphony-archive.timer']))
    state={'source':str(BASE),'destination':str(DEST),'resume_units':active,'all_units':all_units,
           'status':'preparing','created_at':datetime.datetime.now(datetime.timezone.utc).isoformat()}
    (AUDIT/'state.json').write_text(json.dumps(state,indent=2)+'\n')
    systemctl('stop',*[u for u in all_units if u.endswith('.timer')])
    systemctl('stop',*[u for u in all_units if u.endswith('.service')])
    # Stop must complete before the final copy; never resume automatically on error.
    for name in all_units:
        if systemctl('is-active',name,check=False).stdout.strip() in ('active','activating','deactivating'):
            raise RuntimeError('unit still running: '+name)
    changes=synchronize();(AUDIT/'final-sync.log').write_text(changes)
    differences=synchronize(dry=True);(AUDIT/'checksum-verify.log').write_text(differences)
    if differences.strip():raise ValueError('source/destination checksum comparison differs')
    state['status']='verified';(AUDIT/'state.json').write_text(json.dumps(state,indent=2)+'\n')
    print('Verified copy; originals retained. Administrator activation is ready.')


def activate():
    if os.geteuid()!=0:raise PermissionError('activate requires administrator privileges')
    safe_paths();state=json.loads((AUDIT/'state.json').read_text())
    if state['status']!='verified':raise ValueError('prepare and checksum verification required')
    if os.path.ismount(BASE):raise ValueError('already mounted')
    for name in state['all_units']:
        if systemctl('is-active',name,check=False).stdout.strip() in ('active','activating','deactivating'):
            raise RuntimeError('unit started since preparation: '+name)
    if synchronize(dry=True).strip():raise ValueError('source changed since preparation')
    stamp=datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%SZ')
    backup=BASE.with_name(BASE.name+'.pre-ssd-'+stamp)
    fstab=FSTAB;original=fstab.read_bytes();(AUDIT/('fstab-'+stamp)).write_bytes(original)
    if str(BASE) in original.decode():raise ValueError('existing fstab entry needs manual review')
    BASE.rename(backup);BASE.mkdir(mode=0o700);os.chown(BASE,1000,1000)
    created_dropins=[]
    try:
        subprocess.run(['mount','--bind',str(DEST),str(BASE)],check=True)
        if BASE.stat().st_dev!=DEST.stat().st_dev:raise ValueError('bind mount device mismatch')
        entry=f'\n# CodexSymphony runtime on SSD; original path retained\n{DEST} {BASE} none bind,nofail,x-systemd.requires-mounts-for=/mnt/dev-ssd 0 0\n'
        with fstab.open('ab') as stream:stream.write(entry.encode());stream.flush();os.fsync(stream.fileno())
        subprocess.run(['systemctl','daemon-reload'],check=True)
        unit_root=Path('/home/gem/.config/systemd/user')
        for unit in state['all_units']:
            if not unit.endswith('.service'):continue
            directory=unit_root/(unit+'.d');directory.mkdir(exist_ok=True)
            path=directory/'ssd-mount.conf'
            if path.exists():raise ValueError('existing mount guard needs review: '+str(path))
            created_dropins.append(path)
            path.write_text('[Unit]\nConditionPathIsMountPoint='+str(BASE)+'\n\n[Service]\nExecCondition=/usr/bin/mountpoint -q '+str(BASE)+'\n')
            os.chown(directory,1000,1000);os.chown(path,1000,1000)
        systemctl('daemon-reload')
    except BaseException:
        for path in created_dropins:
            if path.exists():path.unlink()
        if os.path.ismount(BASE):subprocess.run(['umount',str(BASE)],check=True)
        BASE.rmdir();backup.rename(BASE);fstab.write_bytes(original)
        subprocess.run(['systemctl','daemon-reload'],check=False)
        systemctl('daemon-reload',check=False)
        raise
    state.update(status='mounted-awaiting-validation',rollback_source=str(backup),mounted_at=time.time())
    (AUDIT/'state.json').write_text(json.dumps(state,indent=2)+'\n');os.chown(AUDIT/'state.json',1000,1000)
    print('Mounted successfully. Services remain stopped for validation; rollback copy retained at '+str(backup))


def validate_backup(backup):
    if backup.parent!=BASE.parent or not re.fullmatch(r'codexsymphony\.pre-ssd-\d{8}T\d{6}Z',backup.name):
        raise ValueError('unexpected rollback directory')
    if backup.resolve()!=backup.absolute() or backup.is_mount():raise ValueError('unsafe rollback directory')
    device=backup.stat().st_dev
    for directory,children,_ in os.walk(backup,followlinks=False):
        for name in children:
            path=Path(directory)/name
            if not path.is_symlink() and (path.is_mount() or path.stat().st_dev!=device):
                raise ValueError('rollback directory contains mount: '+str(path))
    if gate_busy(backup,include_launchers=False):raise ValueError('rollback directory still in use')
    if not shutil.rmtree.avoids_symlink_attacks:raise ValueError('fd-safe removal required')


def finalize():
    if os.geteuid()!=1000:raise ValueError('finalize as gem after fresh CI passes')
    safe_paths();state=json.loads((AUDIT/'state.json').read_text())
    if state['status']!='mounted-awaiting-validation' or not os.path.ismount(BASE):
        raise ValueError('validated SSD mount required')
    if BASE.stat().st_dev!=DEST.stat().st_dev:raise ValueError('wrong mounted device')
    accepted=[]
    for receipt in (BASE/'remote-gate/jobs').glob('*/receipt.json'):
        value=json.loads(receipt.read_text())
        if value.get('finished') and value.get('status')=='PASS' and receipt.stat().st_mtime>state['mounted_at']:
            accepted.append(str(receipt))
    if not accepted:raise ValueError('fresh post-migration full CI PASS required before removing rollback copy')
    backup=Path(state['rollback_source'])
    validate_backup(backup)
    # This is the independently checksum-verified old copy, never the mounted data.
    shutil.rmtree(backup)
    state.update(status='complete',validated_receipts=accepted,finalized_at=time.time())
    (AUDIT/'state.json').write_text(json.dumps(state,indent=2)+'\n')
    print('Migration complete; verified old system-disk copy released.')


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('action',choices=['prepare','activate','finalize','status']);args=parser.parse_args()
    if args.action=='prepare':prepare()
    elif args.action=='activate':activate()
    elif args.action=='finalize':finalize()
    else:print((AUDIT/'state.json').read_text() if (AUDIT/'state.json').exists() else 'Not prepared')

if __name__=='__main__':main()
