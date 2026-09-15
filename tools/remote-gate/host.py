#!/usr/bin/env python3
"""Installed host polls Actions; validates exact commits outside GitHub runners."""
import argparse
from datetime import datetime,timezone
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time
from github import installation_token,request

CHECK='Trusted Harness-Gate'


def load(path): return json.loads(Path(path).read_text())
def write(path,value):
    path=Path(path);pending=path.with_suffix('.new')
    pending.write_text(json.dumps(value,indent=2)+'\n');pending.replace(path)
def sha(path): return hashlib.sha256(Path(path).read_bytes()).hexdigest()
def timestamp(): return datetime.now(timezone.utc).isoformat().replace('+00:00','Z')


def validate_run(run,config):
    if run['repository']['full_name']!=config['repository']: raise ValueError('repository mismatch')
    if run['head_repository']['full_name']!=config['repository']: raise ValueError('fork unsupported')
    if run['path']!='.github/workflows/quality.yml': raise ValueError('workflow mismatch')
    if run['event'] not in ('pull_request','push','workflow_dispatch'): raise ValueError('event unsupported')
    if run['event']=='push' and run['head_branch']!='main': raise ValueError('push branch unsupported')
    if not re.fullmatch('[0-9a-f]{40}',run['head_sha']): raise ValueError('invalid commit')
    if not isinstance(run['run_attempt'],int) or run['run_attempt']<1: raise ValueError('invalid attempt')
    return str(run['id'])+'/'+str(run['run_attempt'])


def check_files(root,pins):
    for name,digest in pins.items():
        path=root/name
        if path.is_symlink() or sha(path)!=digest: raise ValueError('unapproved input: '+name)


def prepare(run,config,job):
    root=job/'source'
    subprocess.run(['git','clone','--quiet','--no-checkout','https://github.com/'+config['repository']+'.git',root],check=True)
    subprocess.run(['git','-C',root,'fetch','--quiet','origin',run['head_sha']],check=True)
    subprocess.run(['git','-C',root,'checkout','--quiet','--detach',run['head_sha']],check=True)
    check_files(root,config['protected_files'])
    approval=load(config['gate_approval'])
    check_files(root,approval['trusted_files']);check_files(root,approval['config_files'])
    # Dependencies are prepared by the operator, never installed by a credentialed PR job.
    # Require exact reviewed lockfiles before exposing the installed dependency tree.
    deps=Path(config['dependency_source'])
    for name in ('package.json','package-lock.json'):
        if sha(root/'web/angular'/name)!=sha(deps.parent/name): raise ValueError('frontend dependencies need host review')
    shutil.copytree(deps,root/'web/angular/node_modules',symlinks=True,copy_function=os.link)
    approved=approval|{'repository':str(root)}
    write(job/'gate-approval.json',approved)
    return root,approved


def evaluate(run,config,job):
    root,approval=prepare(run,config,job)
    launcher=Path(approval['host_release'])/'run.py'
    with (job/'gate.stdout').open('w') as out,(job/'gate.stderr').open('w') as err:
        result=subprocess.run(['/usr/bin/python3',launcher,'--repository',root,'--approval',job/'gate-approval.json'],stdout=out,stderr=err,timeout=1500)
    if result.returncode: raise RuntimeError('complete gate failed; inspect retained gate.stderr and run reports')
    lines=(job/'gate.stdout').read_text().splitlines()
    accepted=json.loads(lines[-1])
    if accepted.get('status')!='PASS': raise ValueError('missing complete acceptance')
    retained=Path(accepted['run'])
    report=retained/'workspace/.harness-gate/reports/test_result.json';value=load(report)
    if not value['passed'] or not value['evidence_complete']: raise ValueError('incomplete evidence')
    if value['source_identity'] not in ('commit:'+run['head_sha'],'working-tree:'+run['head_sha']):
        raise ValueError('report source identity mismatch')
    evidence=value['quality']['evidence']
    if any(row['context']['commit']!=run['head_sha'] for row in evidence): raise ValueError('evidence commit mismatch')
    return {'run':str(retained),'report_sha256':sha(report),'source_sha':run['head_sha'],
            'records':len(evidence),'producers':len(value['quality']['producers']),'status':'PASS'}


def process(run,config,home):
    identity=validate_run(run,config)
    job=home/'jobs'/identity.replace('/','-');job.mkdir(parents=True,exist_ok=True)
    receipt=job/'receipt.json'
    token=installation_token(config);prefix='/repos/'+config['repository']
    if receipt.exists():
        prior=load(receipt)
        if prior.get('finished'): return
        # A process restart never turns an interrupted run into success.
        request(prefix+'/check-runs/'+str(prior['check_id']),token,'PATCH',
                {'status':'completed','conclusion':'failure','completed_at':timestamp(),
                 'output':{'title':'Host interrupted','summary':'Retained run requires a fresh Actions attempt.'}})
        write(receipt,prior|{'finished':True,'status':'interrupted'});return
    check=request(prefix+'/check-runs',token,'POST',{'name':CHECK,'head_sha':run['head_sha'],
        'external_id':identity,'status':'in_progress','started_at':timestamp(),'details_url':run['html_url']})
    state={'identity':identity,'check_id':check['id'],'source_sha':run['head_sha'],'finished':False}
    write(receipt,state)
    try:
        result=evaluate(run,config,job)
        conclusion='success';title='Complete isolated gate passed'
        summary=(f"Commit: `{run['head_sha']}`\n\nActions attempt: `{identity}`\n\n"
                 f"Evidence: {result['records']} records, {result['producers']} producers. CRAP ≤10; coverage ≥80%.\n\n"
                 f"Report SHA-256: `{result['report_sha256']}`\n\nHost retention: `{result['run']}`")
    except Exception as error:
        result={'status':'FAIL','error':str(error)};conclusion='failure';title='Trusted host rejected validation'
        summary=f"Commit: `{run['head_sha']}`\n\nActions attempt: `{identity}`\n\n{type(error).__name__}: {error}\n\nLocal receipt: `{job}`"
    # Refresh the installation token after potentially long native compilation.
    token=installation_token(config)
    request(prefix+'/check-runs/'+str(check['id']),token,'PATCH',
            {'status':'completed','conclusion':conclusion,'completed_at':timestamp(),
             'output':{'title':title,'summary':summary}})
    write(receipt,state|result|{'finished':True})
    print(json.dumps({'identity':identity,'conclusion':conclusion,'check_id':check['id']}),flush=True)


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--config',type=Path,required=True)
    parser.add_argument('--once',action='store_true');args=parser.parse_args()
    config=load(args.config);home=Path(config['state_root']);home.mkdir(parents=True,exist_ok=True)
    with (home/'service.lock').open('w') as lock:
        fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
        while True:
            try:
                token=installation_token(config)
                # New PR workflows need not exist on the default branch yet.
                runs=request('/repos/'+config['repository']+'/actions/runs?status=in_progress&per_page=30',token)
                for run in reversed(runs['workflow_runs']):
                    if run['path']=='.github/workflows/quality.yml': process(run,config,home)
            except Exception as error:
                print(json.dumps({'error':type(error).__name__,'detail':str(error)}),flush=True)
                if args.once: raise
            if args.once: return
            time.sleep(15)

if __name__=='__main__': main()
