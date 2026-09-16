#!/usr/bin/env python3
"""Retain review records, expire rebuildable Gate payloads with bounded hot/cold storage."""
import argparse, fcntl, json, os, re, shutil, tarfile, time
from pathlib import Path
from archive_gate_evidence import COLD, ROOT, inventory, sha, verify
from storage_maintenance import gate_busy

GIB=1024**3
RUN=re.compile(r'run-[0-9a-f]{12}')


def save(path, value):
    temporary=path.with_suffix('.new')
    with temporary.open('w') as stream:
        json.dump(value,stream,indent=2);stream.write('\n');stream.flush();os.fsync(stream.fileno())
    temporary.replace(path)
    fd=os.open(path.parent,os.O_RDONLY|os.O_DIRECTORY)
    try:os.fsync(fd)
    finally:os.close(fd)


def safe_tree(path):
    if path.resolve()!=path.absolute():raise ValueError('symlink path: '+str(path))
    device=path.stat().st_dev
    for directory,dirs,files in os.walk(path,followlinks=False):
        for item in [Path(directory),*(Path(directory)/n for n in dirs+files)]:
            if item.is_symlink() or item.is_mount() or item.stat().st_dev!=device:
                raise ValueError('symlink or mount in payload: '+str(item))


def payloads(run):
    return [p for p in (run/'probes/backend/raw',run/'http-server') if p.exists()]


def bytes_used(paths):
    return sum(p.stat().st_size for root in paths for p in ([root] if root.is_file() else root.rglob('*')) if p.is_file())


def review_files(run):
    selected=set()
    for parent in (run,run/'probes/backend',run/'probes/frontend'):
        if parent.exists():
            selected.update(p for p in parent.iterdir() if p.is_file() and p.suffix in ('.json','.md','.stdout','.stderr'))
    for parent in (run/'signed',run/'workspace/.harness-gate/reports'):
        if parent.exists():
            safe_tree(parent);selected.update(p for p in parent.rglob('*') if p.is_file())
    return sorted(p for p in selected if p.name not in ('compact-retention.json',))


def compact(run, busy=None):
    busy=busy or (lambda p:gate_busy(p,include_launchers=False))
    if not RUN.fullmatch(run.name) or run.resolve()!=run.absolute():raise ValueError('invalid run')
    if busy(run):return {'run':run.name,'deferred':True}
    marker=run/'compact-retention.json'
    if marker.exists():
        record=json.loads(marker.read_text())
        if record.get('complete'):return {'run':run.name,'already_compact':True}
        package=run/'review-record.tar.gz'
        if sha(package)!=record['archive_sha256']:raise ValueError('retained review archive changed')
        verify(package,record['files'])
        return finish_compaction(run,record)
    if not (run/'source-inputs.json').is_file():return {'run':run.name,'deferred':'missing source identity'}
    removable=payloads(run)
    report=run/'workspace/.harness-gate/reports'
    if report.exists():removable.extend(p for p in report.iterdir() if p.is_dir())
    for p in removable:safe_tree(p)
    # Preserve original reports, signatures, source/lockfile hashes and command logs
    # before discarding only the explicitly named reproducible binary/raw payloads.
    files=review_files(run)
    if any(p.resolve()!=p.absolute() for p in files):raise ValueError('symlink review record')
    manifest={str(p.relative_to(run)):{'sha256':sha(p),'size':p.stat().st_size,'mode':p.stat().st_mode & 0o777} for p in files}
    package=run/'review-record.tar.gz';temporary=run/'review-record.tar.partial'
    with tarfile.open(temporary,'w:gz',compresslevel=6) as tar:
        for p in files:tar.add(p,arcname=str(p.relative_to(run)),recursive=False)
    verify(temporary,manifest)
    if busy(run):return {'run':run.name,'deferred':True}
    if any(sha(run/name)!=item['sha256'] for name,item in manifest.items()):raise ValueError('review record changed')
    with temporary.open('rb') as stream:os.fsync(stream.fileno())
    temporary.replace(package)
    removed=[{'path':str(p.relative_to(run)),'bytes':bytes_used([p])} for p in removable]
    record={'schema':'gate-compact-retention/v1','run':run.name,'created_at':time.time(),
            'complete':False,'archive':package.name,'archive_sha256':sha(package),'files':manifest,
            'removed':removed,'replay':'Fetch the recorded Git commit and lockfiles, rebuild with recorded tools. Historical raw coverage/binaries are intentionally unavailable.'}
    save(marker,record)
    return finish_compaction(run,record)


def finish_compaction(run,record):
    allowed={'probes/backend/raw','http-server'}
    for entry in record['removed']:
        name=entry['path'];relative=Path(name)
        if name not in allowed and not (relative.parent==Path('workspace/.harness-gate/reports') and relative.name not in ('.','..')):
            raise ValueError('unexpected compact payload')
        p=run/relative
        if not p.exists():continue
        safe_tree(p)
        if p.is_dir():shutil.rmtree(p)
        else:p.unlink()
    record['complete']=True;save(run/'compact-retention.json',record)
    return {'run':run.name,'released_bytes':sum(p['bytes'] for p in record['removed']),'review_archive_bytes':(run/'review-record.tar.gz').stat().st_size}


def expire_cold(cold, now, apply=False, days=7, budget=4*GIB):
    if cold.resolve()!=cold.absolute() or not os.path.ismount(cold.parent):raise ValueError('cold disk missing')
    entries=[]
    for p in cold.iterdir():
        if re.fullmatch(r'run-[0-9a-f]{12}-backend\.tar\.(gz|partial)',p.name):
            if p.is_symlink() or not p.is_file():raise ValueError('unsafe cold payload')
            entries.append(p)
    entries.sort(key=lambda p:p.stat().st_mtime,reverse=True);kept=0;out=[]
    for p in entries:
        size=p.stat().st_size;age=now-p.stat().st_mtime
        # Incomplete archives have no audit value; originals were never deleted.
        remove=(p.suffix=='.partial' and age>3600) or age>days*86400 or kept+size>budget
        if not remove:kept+=size;continue
        record={'archive':p.name,'sha256':sha(p),'bytes':size,'expired_at':now,'reason':'cold retention limit','raw_available':False}
        if apply:save(cold/(p.name+'.expired.json'),record);p.unlink()
        out.append(record)
    return out


def maintain(root=ROOT,cold=COLD,apply=False,now=None,keep=2,hours=24,budget=4*GIB):
    now=now or time.time();runs=root/'gate-host/runs';chosen=[];used=0;output=[]
    if runs.resolve()!=runs.absolute():raise ValueError('symlink runs root')
    for run in runs.iterdir():
        if RUN.fullmatch(run.name) and not run.is_symlink() and (run/'source-inputs.json').is_file():
            chosen.append(run)
    chosen.sort(key=lambda p:(p/'source-inputs.json').stat().st_mtime,reverse=True)
    for index,run in enumerate(chosen):
        age=now-(run/'source-inputs.json').stat().st_mtime
        size=bytes_used(payloads(run))
        result=run/'verify-result.json'
        recent=now-result.stat().st_mtime<300 if result.exists() else age<3600
        if recent:continue
        if index<keep and age<hours*3600 and used+size<=budget:used+=size;continue
        if (run/'compact-retention.json').exists() and json.loads((run/'compact-retention.json').read_text()).get('complete'):continue
        output.append(compact(run) if apply else {'run':run.name,'rebuildable_bytes':size})
    output.extend(expire_cold(cold,now,apply=apply))
    return output


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--apply',action='store_true');args=parser.parse_args()
    if not shutil.rmtree.avoids_symlink_attacks:raise RuntimeError('fd-safe deletion required')
    if not os.path.ismount(ROOT) or not os.path.ismount(COLD.parent):raise ValueError('runtime/cold mount missing')
    with (COLD/'archive.lock').open('a') as lock:
        fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
        for result in maintain(apply=args.apply):print(json.dumps(result),flush=True)

if __name__=='__main__':main()
