#!/usr/bin/env python3
"""Consolidate finished PR retries after a later successful trusted Gate attempt."""
import json, os, re, shutil, subprocess, time
from pathlib import Path
from compact_gate_evidence import save, sha
from storage_maintenance import gate_busy

REPOSITORY='musutrade/CodexSymphony'
IDENTITY=re.compile(r'\d+/\d+')
RUN=re.compile(r'run-[0-9a-f]{12}')


def api(path):
    return json.loads(subprocess.check_output(['/usr/bin/gh','api',path],text=True,timeout=30))


def context(job,receipt,lookup=api):
    cached=job/'attempt-context.json'
    if cached.exists():
        value=json.loads(cached.read_text())
        if value['identity']!=receipt['identity'] or value['source_sha']!=receipt['source_sha'] or value['repository']!=REPOSITORY:
            raise ValueError('attempt context mismatch')
        return value
    identity=receipt['identity']
    if not IDENTITY.fullmatch(identity):raise ValueError('invalid attempt identity')
    run=lookup(f'repos/{REPOSITORY}/actions/runs/{identity.split("/")[0]}/attempts/{identity.split("/")[1]}')
    if (run['head_sha']!=receipt['source_sha'] or run['repository']['full_name']!=REPOSITORY
            or run['event']!='pull_request' or run['path']!='.github/workflows/quality.yml'):
        return None
    prs=run.get('pull_requests') or lookup(f'repos/{REPOSITORY}/commits/{receipt["source_sha"]}/pulls')
    # A commit can also belong to a later stacked PR. Resolve that only using
    # the exact Actions head ref among GitHub-confirmed commit associations.
    if len(prs)>1 and run.get('head_branch'):
        prs=[p for p in prs if p.get('head',{}).get('ref')==run['head_branch']]
    numbers={p['number'] for p in prs if p.get('base',{}).get('repo',{}).get('full_name')==REPOSITORY}
    if len(numbers)!=1:return None  # Never guess from a branch name or ambiguous commit.
    value={'repository':REPOSITORY,'pr':numbers.pop(),'identity':identity,'source_sha':receipt['source_sha']}
    save(cached,value);return value


def disposable(path):
    if path.resolve()!=path.absolute() or path.is_mount():raise ValueError('unsafe disposable root')
    device=path.stat().st_dev
    for directory,dirs,files in os.walk(path,followlinks=False):
        for p in [Path(directory),*(Path(directory)/n for n in dirs+files)]:
            # Dependency .bin links are legitimate. rmtree must unlink, not follow them.
            if not p.is_symlink() and (p.is_mount() or p.lstat().st_dev!=device):raise ValueError('nested mount')
    if not shutil.rmtree.avoids_symlink_attacks:raise ValueError('fd-safe removal required')


def linked_run(root,job,receipt):
    name=receipt.get('run')
    if not name and (job/'gate.stdout').is_file():
        matches=re.findall(r'^Retaining complete gate run: (.+)$',(job/'gate.stdout').read_text(),re.M)
        if len(matches)==1:name=matches[0]
    if not name:return None
    run=Path(name)
    if run.parent!=root/'gate-host/runs' or not RUN.fullmatch(run.name) or run.resolve()!=run.absolute():raise ValueError('invalid linked Gate run')
    if not run.exists():return None
    marker=run/'superseded-attempt.json'
    if marker.exists():
        prior=json.loads(marker.read_text())
        if prior['identity']!=receipt['identity'] or prior['source_sha']!=receipt['source_sha']:raise ValueError('retired run identity mismatch')
    else:
        result=subprocess.run(['git','-C',str(run/'workspace'),'rev-parse','HEAD'],capture_output=True,text=True,timeout=10)
        if result.returncode or result.stdout.strip()!=receipt['source_sha']:raise ValueError('linked run commit mismatch')
    return run


def log_record(path):
    if path.is_symlink():raise ValueError('symlink log')
    size=path.stat().st_size
    with path.open('rb') as stream:
        head=stream.read(32768)
        if size>65536:stream.seek(-32768,os.SEEK_END);body=head+b'\n[... truncated; original hash retained ...]\n'+stream.read()
        else:body=head+stream.read()
    return {'sha256':sha(path),'bytes':size,'truncated':size>65536,'text':body.decode(errors='replace')}


def summarize(root,job,receipt,winner,run):
    marker=job/'superseded-attempt.json'
    if marker.exists():return json.loads(marker.read_text())
    logs={}
    for p in sorted(job.glob('gate.*')):
        if p.suffix in ('.stdout','.stderr'):logs['job/'+p.name]=log_record(p)
    if run:
        for pattern in ('*.stdout','*.stderr','probes/*/capture.stdout','probes/*/capture.stderr'):
            for p in sorted(run.glob(pattern)):logs['run/'+str(p.relative_to(run))]=log_record(p)
    value={'schema':'pr-attempt-retention/v1','repository':REPOSITORY,'pr':winner['pr'],
           'identity':receipt['identity'],'source_sha':receipt['source_sha'],'status':receipt['status'],
           'error':str(receipt.get('error',''))[:12000],'superseded_by':winner['identity'],
           'successful_sha':winner['source_sha'],'run':str(run) if run else None,
           'logs':logs,'created_at':time.time(),'complete':False}
    if run and (run/'source-inputs.json').is_file():value['input_hashes']=json.loads((run/'source-inputs.json').read_text())
    if run and (run/'workspace/.harness-gate/reports/test_result.json').is_file():
        value['original_report_sha256']=sha(run/'workspace/.harness-gate/reports/test_result.json')
    if (job/'gate-approval.json').is_file():value['tool_approval']=json.loads((job/'gate-approval.json').read_text())
    save(marker,value);return value


def retire(root,cold,job,receipt,winner,run,busy):
    if busy(job) or (run and busy(run)):return {'job':job.name,'deferred':True}
    summary=summarize(root,job,receipt,winner,run)
    if summary.get('complete'):return {'job':job.name,'already_retired':True}
    if busy(job) or (run and busy(run)):return {'job':job.name,'deferred':True}
    if run:
        disposable(run)
        save(run/'superseded-attempt.json',summary)
        for child in run.iterdir():
            if child.name in ('superseded-attempt.json','source-inputs.json'):continue
            if child.is_dir() and not child.is_symlink():shutil.rmtree(child)
            else:child.unlink()
        for suffix in ('.tar.gz','.tar.partial'):
            p=cold/(run.name+'-backend'+suffix)
            if p.exists():
                if p.is_symlink():raise ValueError('symlink cold archive')
                save(cold/(p.name+'.expired.json'),{'reason':'superseded PR attempt','pr':winner['pr'],'identity':receipt['identity'],'sha256':sha(p),'expired_at':time.time()})
                p.unlink()
    source=job/'source'
    if source.exists():disposable(source);shutil.rmtree(source)
    summary['complete']=True;save(job/'superseded-attempt.json',summary)
    if run:save(run/'superseded-attempt.json',summary)
    return {'job':job.name,'pr':winner['pr'],'superseded_by':winner['identity'],'retired':True}


def maintain_attempts(root,cold,apply=False,now=None,resolve=context,busy=None):
    now=now or time.time();busy=busy or (lambda p:gate_busy(p,include_launchers=False))
    groups={};output=[]
    for path in sorted((root/'remote-gate/jobs').glob('*/receipt.json')):
        if path.is_symlink() or path.parent.resolve()!=path.parent.absolute():raise ValueError('symlink job')
        receipt=json.loads(path.read_text());identity=receipt.get('identity','')
        if not IDENTITY.fullmatch(identity) or path.parent.name!=identity.replace('/','-'):raise ValueError('invalid job identity')
        if not receipt.get('finished') or now-path.stat().st_mtime<300:continue
        if receipt.get('status') not in ('PASS','FAIL','interrupted'):continue
        try:ctx=resolve(path.parent,receipt)
        except (subprocess.SubprocessError,ValueError,KeyError) as error:
            output.append({'job':path.parent.name,'deferred':type(error).__name__});continue
        if ctx:groups.setdefault(ctx['pr'],[]).append((path,receipt,ctx))
    for pr,rows in groups.items():
        successes=[row for row in rows if row[1]['status']=='PASS']
        if not successes:continue
        winner=max(successes,key=lambda row:row[0].stat().st_mtime)
        # A passing host receipt must still point at its exact retained report.
        success_run=linked_run(root,winner[0].parent,winner[1])
        report=success_run/'workspace/.harness-gate/reports/test_result.json' if success_run else None
        if not report or not report.is_file() or sha(report)!=winner[1].get('report_sha256'):continue
        for path,receipt,ctx in rows:
            if path.stat().st_mtime>winner[0].stat().st_mtime:continue
            job=path.parent;run=linked_run(root,job,receipt)
            if path==winner[0]:
                source=job/'source'
                if apply and source.exists() and not busy(job) and not busy(success_run):
                    disposable(source)
                    save(job/'source-retention.json',{'identity':receipt['identity'],'source_sha':receipt['source_sha'],'reason':'successful run retained separately; source rebuildable from Git'})
                    shutil.rmtree(source)
                continue
            if run==success_run:raise ValueError('attempts share a Gate run')
            if apply:output.append(retire(root,cold,job,receipt,winner[2],run,busy))
            else:output.append({'job':job.name,'pr':pr,'superseded_by':winner[1]['identity']})
    return output
