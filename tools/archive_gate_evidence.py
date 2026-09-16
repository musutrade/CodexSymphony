#!/usr/bin/env python3
"""Verified cold storage for inactive raw backend evidence; reports stay online."""
import argparse, fcntl, hashlib, json, os, re, shutil, tarfile, time
from pathlib import Path, PurePosixPath
from storage_maintenance import ROOT, gate_busy

COLD=Path('/data/codexsymphony-archive')

def sha(path):
    with path.open('rb') as f:
        return hashlib.file_digest(f,'sha256').hexdigest()

def inventory(root):
    result={}
    for p in sorted(root.rglob('*')):
        if p.is_symlink():raise ValueError('symlink evidence requires manual review: '+str(p))
        if p.is_file():result[str(p.relative_to(root))]={'sha256':sha(p),'size':p.stat().st_size,'mode':p.stat().st_mode & 0o777}
        elif not p.is_dir():raise ValueError('unsupported evidence file: '+str(p))
    return result

def save(path,value):
    temporary=path.with_suffix('.new');temporary.write_text(json.dumps(value,indent=2)+'\n');temporary.replace(path)

def verify(archive, manifest):
    seen=set()
    with tarfile.open(archive,'r:gz') as tar:
        for member in tar:
            if PurePosixPath(member.name).is_absolute() or '..' in PurePosixPath(member.name).parts:
                raise ValueError('unsafe archive path')
            if member.isdir():continue
            if not (member.isfile() or member.islnk()) or member.name not in manifest:
                raise ValueError('unexpected archive member: '+member.name)
            if member.name in seen:raise ValueError('duplicate archive member')
            seen.add(member.name)
            with tar.extractfile(member) as stream:
                digest=hashlib.file_digest(stream,'sha256').hexdigest()
            if digest!=manifest[member.name]['sha256']:raise ValueError('archive content mismatch')
    if seen!=set(manifest):raise ValueError('incomplete archive')

def archive_run(run, cold=COLD, busy=gate_busy):
    runs=run.parent
    if not re.fullmatch(r'run-[0-9a-f]{12}',run.name) or run.resolve()!=run.absolute():raise ValueError('invalid run')
    if busy(runs):raise RuntimeError('active or unreadable Gate worker; archive deferred')
    marker=run/'archive.json'
    if marker.exists():return json.loads(marker.read_text())
    payload=run/'probes/backend'
    if not payload.is_dir():return None
    if cold.resolve()!=cold.absolute() or not os.path.ismount(cold.parent):raise ValueError('cold-storage disk is not mounted')
    cold.mkdir(mode=0o700,exist_ok=True)
    manifest=inventory(payload)
    destination=cold/(run.name+'-backend.tar.gz')
    temporary=destination.with_suffix('.partial')
    if destination.exists():raise ValueError('archive already exists without receipt; review required')
    with tarfile.open(temporary,'w:gz',compresslevel=1) as tar:
        for name in manifest:tar.add(payload/name,arcname=name,recursive=False)
    verify(temporary,manifest)
    if busy(runs) or inventory(payload)!=manifest:raise RuntimeError('source changed or worker started; originals retained')
    with temporary.open('rb') as stream:os.fsync(stream.fileno())
    temporary.replace(destination)
    directory=os.open(cold,os.O_RDONLY|os.O_DIRECTORY)
    try:os.fsync(directory)
    finally:os.close(directory)
    record={'schema':'gate-evidence-archive/v1','run':run.name,'archive':str(destination),
            'archive_sha256':sha(destination),'created_at':time.time(),'files':manifest,
            'original_bytes':sum(v['size'] for v in manifest.values()),'compressed_bytes':destination.stat().st_size}
    save(cold/(run.name+'-backend.json'),record)
    save(marker,record)
    # Keep diagnostic logs at their original paths; only verified raw payload leaves SSD.
    for child in payload.iterdir():
        if child.name in ('capture.stdout','capture.stderr'):continue
        if child.is_dir():shutil.rmtree(child)
        else:child.unlink()
    return record

def restore(run, destination):
    record=json.loads((run/'archive.json').read_text());archive=Path(record['archive'])
    if sha(archive)!=record['archive_sha256']:raise ValueError('archive checksum mismatch')
    verify(archive,record['files'])
    if destination.exists():raise ValueError('restore destination must be new')
    destination.mkdir(parents=True,mode=0o700)
    with tarfile.open(archive,'r:gz') as tar:tar.extractall(destination,filter='data')
    for name, item in record['files'].items():(destination/name).chmod(item['mode'])
    if inventory(destination)!=record['files']:raise ValueError('restored evidence mismatch')
    return destination

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--apply',action='store_true')
    parser.add_argument('--keep',type=int,default=10)
    parser.add_argument('--days',type=int,default=30)
    parser.add_argument('--restore',type=Path)
    parser.add_argument('--destination',type=Path)
    args=parser.parse_args()
    if args.restore:
        if not args.destination:parser.error('--destination required with --restore')
        print(restore(args.restore,args.destination));return
    if args.keep<1 or args.days<1:parser.error('positive retention required')
    runs=ROOT/'gate-host/runs'
    COLD.mkdir(mode=0o700,exist_ok=True)
    with (COLD/'archive.lock').open('a') as lock:
        fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
        def created(run):
            source=run/'source-inputs.json'
            return source.stat().st_mtime if source.exists() else run.stat().st_mtime
        ordered=sorted((p for p in runs.iterdir() if re.fullmatch(r'run-[0-9a-f]{12}',p.name) and not (p/'archive.json').exists()),key=created,reverse=True)
        for index,run in enumerate(ordered):
            age=time.time()-created(run)
            if (index<args.keep and age<args.days*86400) or age<3600 or (run/'archive.json').exists():continue
            if not (run/'probes/backend').is_dir():continue
            if args.apply:
                if gate_busy(runs):print('Deferred: Gate is active');break
                result=archive_run(run)
                print(json.dumps({k:result[k] for k in ('run','original_bytes','compressed_bytes')}),flush=True)
            else:print(run)

if __name__=='__main__':main()
