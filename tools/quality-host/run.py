#!/usr/bin/env python3
"""Run installed, approved local host capture and the complete isolated CI gate."""
import argparse
import json
import os
from pathlib import Path

import sys
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import environment_contract as contract
import shutil
import subprocess
import sys
import uuid
from capture import captures, sha, load, write, TS, HTTP, RUST, PLUGIN_ROOT
from configure import configure, state
from signing import CORE, configuration_files, provision
from verify import verify
from timing import phase
from preflight import preflight
from test_receipt import seal
import build_cache

HOME=Path('/home/gem/.local/share/codexsymphony/gate-host')
EXECUTION_VERSION=2

def runtime_pins():
    policy=contract.load()
    pinned_path=contract.tool_path(policy)
    roots=[RUST,TS,HTTP,TS.parent.parent/'typescript',HTTP.parent.parent/'typescript']
    pins={str(p):sha(p.read_bytes()) for base in roots for p in sorted(base.rglob('*')) if p.is_file() and '__pycache__' not in p.parts}
    for name in ('harness-gate','harness-gate-rust-collector','node','python3','cargo-llvm-cov','/usr/local/libexec/codexsymphony/bwrap'):
        p=(CORE if name=='harness-gate' else
           Path.home()/'.local/share/harness-gate/versions'/('rust-collector-v'+policy['tools']['rust_collector'])/'bin/harness-gate-rust-collector' if name=='harness-gate-rust-collector' else
           Path(shutil.which(name,path=pinned_path)).resolve())
        pins[str(p)]=sha(p.read_bytes())
    codex=(contract.codex_bin(contract.load())/'codex').resolve()
    pins[str(codex)]=sha(codex.read_bytes())
    pins.update({str(p):sha(p.read_bytes()) for p in Path(__file__).parent.glob('*.py')})
    manifest=Path(__file__).with_name(contract.NAME)
    if manifest.exists(): pins[str(manifest)]=sha(manifest.read_bytes())
    return pins

def check_pins(pins):
    for name,digest in pins.items():
        if sha(Path(name).read_bytes())!=digest: raise ValueError('approved runtime changed: '+name)

def trusted_files(repo):
    names=['environment.lock.json','tools/environment_contract.py','tools/gate.py','tools/gate_selftest.py','harness-gate-version.lock','web/angular/tools/probe-typescript-risk.cjs','tools/install_gate_plugins.py','.harness-gate/collector-candidates.json']
    names += [str(p.relative_to(repo)) for p in sorted((repo/'tools/quality-host').glob('*.py'))]
    return {name:sha((repo/name).read_bytes()) for name in names}

def snapshot(repo,run):
    root=run/'workspace'
    subprocess.run(['git','clone','--quiet','--no-hardlinks','--local',repo,root],check=True)
    names=subprocess.check_output(['git','-C',repo,'ls-files','-z','--cached','--others','--exclude-standard']).decode().split('\0')
    files={}
    for name in filter(None,names):
        p=repo/name
        if p.is_symlink(): raise ValueError('symlink source snapshot: '+name)
        if not p.is_file():
            # Preserve deletions relative to HEAD instead of silently restoring code.
            if (root/name).is_file(): (root/name).unlink()
            continue
        target=root/name;target.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(p,target);shutil.copymode(p,target)
        files[name]=sha(p.read_bytes())
    write(run/'source-inputs.json',files)
    return root,files

def prune_build_cache(run):
    target=run/'target'
    if target.is_symlink(): raise ValueError('build cache must not be a symlink')
    # Native objects/counters are already retained under probes, and the HTTP
    # binary is retained separately. No signed input lives in this build cache.
    shutil.rmtree(target)

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repository',type=Path,required=True)
    parser.add_argument('--profile',choices=['ci','full'],default='ci')
    parser.add_argument('--bootstrap',action='store_true',help='explicit one-time local policy bootstrap; never run in PR CI')
    parser.add_argument('--approval',type=Path,default=HOME/'approval.json')
    parser.add_argument('--publish-cache',action='store_true',help='trusted main only; publish after full PASS')
    parser.add_argument('--cache-max-bytes',type=int,default=build_cache.DEFAULT_BYTES)
    parser.add_argument('--cache-ttl-seconds',type=int,default=build_cache.DEFAULT_TTL)
    args=parser.parse_args();repo=args.repository.resolve(strict=True)
    if args.cache_max_bytes < 0 or args.cache_ttl_seconds <= 0:
        parser.error('cache capacity must be nonnegative and TTL positive')
    policy=contract.load(repo)
    contract.check_files(repo,policy)
    os.environ.update(contract.test_environment(policy))
    os.environ['PATH']=contract.tool_path(policy)
    HOME.mkdir(parents=True,exist_ok=True)
    if args.bootstrap and args.approval.exists(): raise ValueError('approval already exists; bootstrap cannot overwrite it')
    approval=None
    if not args.bootstrap:
        approval=load(args.approval)
        if approval['repository']!=str(repo): raise ValueError('approval repository mismatch')
        check_pins(approval['runtime_files'])
        if configuration_files(repo)!=approval['config_files']: raise ValueError('project policy changed; host review required')
        if trusted_files(repo)!=approval['trusted_files']: raise ValueError('capture/host entrypoint changed; host review required')
    run=HOME/'runs'/('run-'+uuid.uuid4().hex[:12]);run.mkdir(parents=True)
    (HOME/'latest-run').write_text(str(run))
    print('Retaining complete gate run: '+str(run),flush=True)
    with phase(run,'snapshot'):
        root,inputs=snapshot(repo,run)
    environment = contract.fingerprint(repo)
    write(run/'environment.json', environment)
    print(json.dumps({'environment_fingerprint': environment['fingerprint']}), flush=True)
    preflight(run,repo)
    cache_key=None
    if approval and args.cache_max_bytes:
        with phase(run,'cache-restore'):
            cache_key=build_cache.cache_key(repo,approval)
            restored=build_cache.restore(HOME/'build-cache',cache_key,run/'target',args.cache_max_bytes,args.cache_ttl_seconds)
            write(run/'cache-restore.json',restored)
    revision=subprocess.check_output(['git','-C',repo,'rev-parse','HEAD'],text=True).strip()
    if args.bootstrap:
        base=run/'baseline.json';shutil.copyfile(repo/'api/baseline.json',base)
        baseline={'path':str(base),'sha256':sha(base.read_bytes()),'commit':revision}
    else:
        baseline=approval['baseline']
        if sha(Path(baseline['path']).read_bytes())!=baseline['sha256']: raise ValueError('approved baseline changed')
    context={'commit':revision,'base_commit':baseline['commit'],'run':run.name,'target':'x86_64-unknown-linux-gnu'}
    with phase(run,'capture-all'):
        requests,identities=captures(run,repo,root,context,baseline)
        seal(run,repo,context)
    # Only the trusted host generates the fixed policy template. A normal run
    # requires byte-for-byte equality with the separately approved configuration.
    groups=configure(root,requests,identities)
    desired=configuration_files(root)
    if approval and desired!=approval['config_files']: raise ValueError('measurement series or policy changed; host review required')
    for name,digest in inputs.items():
        if name in desired and args.bootstrap: continue
        if sha((repo/name).read_bytes())!=digest: raise ValueError('source changed during capture: '+name)
    write(run/'groups.json',groups)
    provision(run,root,state(requests,identities,groups,args.profile),requests,desired,HOME/'keys')
    with phase(run,'verify'):
        code=verify(run,repo,root,args.profile)
        if code: raise SystemExit(code)
    current=subprocess.check_output(['git','-C',repo,'ls-files','-z','--cached','--others','--exclude-standard']).decode().split('\0')
    current={name:sha((repo/name).read_bytes()) for name in current if name and (repo/name).is_file()}
    if current!=inputs: raise ValueError('source inventory changed during verification')
    if args.bootstrap:
        for name in desired: shutil.copyfile(root/name,repo/name)
        for collector in identities:
            name=f'.harness-gate/packs/{collector}/capabilities.json';shutil.copyfile(root/name,repo/name)
        approved_base=HOME/'baselines'/baseline['sha256'];approved_base.parent.mkdir(exist_ok=True);shutil.copyfile(baseline['path'],approved_base)
        baseline['path']=str(approved_base)
        approval={'schema':'codexsymphony-local-host-approval/v1','repository':str(repo),'config_files':desired,'trusted_files':trusted_files(repo),'runtime_files':runtime_pins(),'baseline':baseline,'series':identities,'execution_version':EXECUTION_VERSION,'host_release':str(Path(__file__).parent)}
        write(args.approval,approval)
    if args.publish_cache and cache_key:
        with phase(run,'cache-publish'):
            published=build_cache.publish(HOME/'build-cache',cache_key,run/'target',revision,args.cache_max_bytes,args.cache_ttl_seconds,repository=repo)
            write(run/'cache-publish.json',published)
    with phase(run,'cache-cleanup'):
        prune_build_cache(run)
    print(json.dumps({'status':'PASS','scope':'complete-local-isolated-gate','run':str(run),'approval':str(args.approval)}))

if __name__=='__main__': main()
