#!/usr/bin/env python3
"""Install reviewed local collector packages by exact archive digest."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile

ROOT=Path(__file__).resolve().parents[1]

def install(record, archive, root, bin_dir):
    digest=hashlib.sha256(archive.read_bytes()).hexdigest()
    if digest!=record['sha256']: raise ValueError('collector package digest mismatch: '+archive.name)
    target=root/record['directory']/record['version']
    target.parent.mkdir(parents=True,exist_ok=True)
    if not target.exists():
        if record['npm_package']:
            subprocess.run(['npm','install','--prefix',target,'--ignore-scripts','--omit=dev',archive],check=True)
        else:
            with tempfile.TemporaryDirectory(dir=target.parent) as temporary:
                with tarfile.open(archive) as tar: tar.extractall(temporary,filter='data')
                shutil.move(str(Path(temporary)/'rust-source-risk'),target)
    package=target/'node_modules'/record['npm_package'] if record['npm_package'] else target
    prefix='package/' if record['npm_package'] else 'rust-source-risk/'
    # Existing version directories are immutable. Never silently replace a tool.
    with tarfile.open(archive) as tar:
        for member in tar:
            if not member.isfile(): continue
            if not member.name.startswith(prefix): raise ValueError('unexpected package root')
            path=package/member.name[len(prefix):]
            if path.is_symlink() or path.read_bytes()!=tar.extractfile(member).read():
                raise ValueError('installed collector differs from approved package: '+str(path))
    entry=package/('cli.cjs' if record['npm_package'] else 'plugin.py')
    entry.chmod(entry.stat().st_mode|0o111)
    bin_dir.mkdir(parents=True,exist_ok=True)
    link=bin_dir/record['command']
    if link.exists() and not link.is_symlink(): raise ValueError('refusing to replace a non-symlink command: '+str(link))
    pending=bin_dir/(record['command']+'.new')
    pending.symlink_to(entry);pending.replace(link)
    return {'command':str(link),'package':str(package),'sha256':digest}

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--install-root',type=Path,default=Path.home()/'.local/share/harness-gate')
    parser.add_argument('--bin-dir',type=Path,default=Path.home()/'.local/bin')
    args=parser.parse_args()
    manifest=json.loads((ROOT/'.harness-gate/collector-candidates.json').read_text())
    results=[install(record,ROOT/'tools/gate-plugins/packages'/record['package'],args.install_root,args.bin_dir) for record in manifest['plugins'].values()]
    print(json.dumps(results,indent=2))

if __name__=='__main__': main()
