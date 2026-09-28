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
EXECUTION_VERSION=3

def runtime_pins():
    policy=contract.load()
    pinned_path=contract.tool_path(policy)
    roots=[RUST,TS,HTTP,TS.parent.parent/'typescript',HTTP.parent.parent/'typescript']
    pins={str(p):sha(p.read_bytes()) for base in roots for p in sorted(base.rglob('*')) if p.is_file() and '__pycache__' not in p.parts}
    for name in ('harness-gate','harness-gate-rust-collector','node','python3','cargo-llvm-cov','sccache','/usr/local/libexec/codexsymphony/bwrap'):
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

def snapshot(repo, run, revision=None):
    import bounded_layout as layout
    import fixed_workspace
    if revision is None:
        root, files = fixed_workspace.synchronize(repo, layout.slot())
    else:
        root, files = fixed_workspace.synchronize_revision(repo, layout.slot(), revision)
    write(run / 'source-inputs.json', files)
    return root, files


def arguments():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repository', type=Path, required=True)
    parser.add_argument('--profile', choices=['ci', 'full'], default='ci')
    parser.add_argument('--bootstrap', action='store_true')
    parser.add_argument('--revision', help='exact remote commit already fetched into the repository')
    parser.add_argument('--approval', type=Path, default=HOME / 'approval.json')
    return parser.parse_args()


def approved(args, repo, source=None):
    source = source or repo
    policy = contract.load(source)
    contract.check_files(source, policy)
    os.environ.update(contract.test_environment(policy))
    os.environ['PATH'] = contract.tool_path(policy)
    if args.bootstrap:
        if args.approval.exists():
            raise ValueError('approval already exists; bootstrap cannot overwrite it')
        return None
    approval = load(args.approval)
    if approval['host_release'] != str(Path(__file__).parent) or approval['execution_version'] != EXECUTION_VERSION:
        raise ValueError('approval does not bind this host release')
    if approval['repository'] != str(repo):
        raise ValueError('approval repository mismatch')
    check_pins(approval['runtime_files'])
    if configuration_files(source) != approval['config_files']:
        raise ValueError('project policy changed; host review required')
    if trusted_files(source) != approval['trusted_files']:
        raise ValueError('capture/host entrypoint changed; host review required')
    return approval


def baseline_for(args, approval, repo, run, revision):
    if args.bootstrap:
        path = run / 'baseline.json'
        shutil.copyfile(repo / 'api/baseline.json', path)
        return {'path': str(path), 'sha256': sha(path.read_bytes()), 'commit': revision}
    baseline = approval['baseline']
    if sha(Path(baseline['path']).read_bytes()) != baseline['sha256']:
        raise ValueError('approved baseline changed')
    return baseline


def reset_generated(root):
    for name in ('.harness-gate/runtime', '.harness-gate/reports'):
        path = root / name
        if path.resolve() != path:
            raise ValueError('symlink generated Gate directory')
        if path.exists():
            shutil.rmtree(path)


def run_candidate(args, approval, repo, run, root, inputs):
    revision = subprocess.check_output(['git', '-C', root, 'rev-parse', 'HEAD'], text=True).strip()
    baseline = baseline_for(args, approval, repo, run, revision)
    context = {'commit': revision, 'base_commit': baseline['commit'], 'run': run.name,
               'target': 'x86_64-unknown-linux-gnu'}
    reset_generated(root)
    with phase(run, 'capture-all'):
        requests, identities = captures(run, repo, root, context, baseline)
        seal(run, root, context)
    groups = configure(root, requests, identities)
    desired = configuration_files(root)
    if approval and desired != approval['config_files']:
        raise ValueError('measurement series or policy changed; host review required')
    write(run / 'groups.json', groups)
    provision(run, root, state(requests, identities, groups, args.profile), requests, desired, HOME / 'keys')
    with phase(run, 'verify'):
        code = verify(run, repo, root, args.profile)
        if code:
            raise RuntimeError('complete Gate failed: ' + str(code))
    verify_source(args, repo, root, inputs)
    shutil.copytree(root / '.harness-gate/reports', run / 'reports')
    if args.bootstrap:
        accept_bootstrap(args, repo, root, run, baseline, desired, identities)
    return desired


def verify_source(args, repo, root, inputs):
    import fixed_workspace
    import bounded_layout as layout
    current = fixed_workspace.sources(root if args.revision else repo)
    if current != fixed_workspace.prior_inputs(layout.slot()):
        raise ValueError('source contents or modes changed during Gate')
    if {name: row['sha256'] for name, row in current.items()} != inputs:
        raise ValueError('source changed during complete Gate')
    if not args.bootstrap and fixed_workspace.sources(root) != current:
        raise ValueError('validation source differs from development source')


def accept_bootstrap(args, repo, root, run, baseline, desired, identities):
    for name in desired:
        shutil.copyfile(root / name, repo / name)
    for collector in identities:
        name = f'.harness-gate/packs/{collector}/capabilities.json'
        shutil.copyfile(root / name, repo / name)
    approved_base = HOME / 'baselines' / baseline['sha256']
    approved_base.parent.mkdir(exist_ok=True)
    shutil.copyfile(baseline['path'], approved_base)
    baseline['path'] = str(approved_base)
    approval = {'schema': 'codexsymphony-local-host-approval/v1', 'repository': str(repo),
                'config_files': desired, 'trusted_files': trusted_files(repo), 'runtime_files': runtime_pins(),
                'baseline': baseline, 'series': identities, 'execution_version': EXECUTION_VERSION,
                'host_release': str(Path(__file__).parent)}
    write(args.approval, approval)
    write(run / 'bootstrap-source-change.json', {'scope': 'policy binding bootstrap; final exact-tree Gate still required',
                                               'configuration': desired})


def finish_capture(args, run, root, pending):
    import fixed_workspace
    import capture_handoff
    import bounded_layout as layout
    result = load(run / 'measurement-summary.json')
    if args.bootstrap:
        # Bootstrap explicitly changes only the reviewed generated configuration;
        # record the resulting snapshot rather than relabel the original capture.
        inputs = fixed_workspace.sources(root)
        write(run / 'bootstrap-final-inputs.json', inputs)
        write(pending, {'capture': str(run), 'source_inputs': {n: r['sha256'] for n, r in inputs.items()}})
        fixed_workspace.write_json(layout.slot() / 'source-inputs.json', inputs)
    capture_handoff.complete(run, root, pending, result)
    write(run / 'sccache-stats.json', [load(p) for p in sorted((run / 'tmp').glob('sccache-*.json'))])
    write(run / 'complete-gate.json', {'status': 'PASS', 'scope': 'bootstrap-diagnostic' if args.bootstrap else 'complete-local-isolated-gate',
                                    'approval': str(args.approval), 'source_root': str(root)})


def reset_remote_targets():
    """A PR never receives writable artifacts from a previous trust context."""
    import bounded_layout as layout
    if layout.CACHE_DOMAIN != 'remote':
        raise ValueError('refuse to reset local compiler cache')
    for kind in ('normal', 'instrumented'):
        target = layout.target(kind)
        shutil.rmtree(target)
        target.mkdir()


def active_approval(args):
    return args.approval.resolve() == (HOME / 'approval.json').resolve()


def prepare_run(args, repo):
    import bounded_layout as layout
    import bounded_retention
    if args.revision and args.bootstrap:
        raise ValueError('remote sources cannot bootstrap policy')
    layout.CACHE_DOMAIN = 'remote' if args.revision else 'local'
    approval = None
    if not args.revision:
        approval = approved(args, repo)
    if approval is not None and active_approval(args):
        bounded_retention.maintain_locked(apply=True)
    pending = layout.slot() / 'pending-gate.json'
    if pending.exists() or pending.is_symlink() or (layout.slot() / 'pending-capture.json').exists():
        raise ValueError('unfinished capture requires recovery before another Gate')
    return approval, pending


def main():
    import bounded_layout as layout
    import bounded_retention
    args = arguments()
    repo = args.repository.resolve(strict=True)
    with layout.lease(repo):
        approval, pending = prepare_run(args, repo)
        run = layout.new_run()
        HOME.mkdir(parents=True, exist_ok=True)
        (HOME / 'latest-run').write_text(str(run))
        print('Retaining complete Gate run: ' + str(run), flush=True)
        with phase(run, 'snapshot'):
            root, inputs = snapshot(repo, run, args.revision)
        if args.revision:
            approval = approved(args, repo, source=root)
            reset_remote_targets()
        write(run / 'environment.json', contract.fingerprint(root))
        with pending.open('x') as stream:
            json.dump({'capture': str(run), 'source_inputs': inputs}, stream)
        run_candidate(args, approval, repo, run, root, inputs)
        finish_capture(args, run, root, pending)
        if approval is not None and active_approval(args):
            bounded_retention.maintain_locked(apply=True)
        print(json.dumps({'status': 'PASS', 'scope': 'bootstrap-diagnostic' if args.bootstrap else 'complete-local-isolated-gate', 'run': str(run), 'approval': str(args.approval)}))


if __name__ == '__main__': main()
