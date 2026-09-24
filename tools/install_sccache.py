#!/usr/bin/env python3
"""Install the manifest-pinned host sccache; no Cargo globals or scheduling changes."""
import hashlib
import io
import json
from pathlib import Path
import subprocess
import tarfile
import urllib.request

ROOT = Path(__file__).resolve().parents[1]


def install():
    policy = json.loads((ROOT/'environment.lock.json').read_text())
    target = Path.home()/'.cargo/bin/sccache'
    expected = policy['tool_sha256']['sccache']
    if not target.is_file() or hashlib.sha256(target.read_bytes()).hexdigest() != expected:
        release = policy['sccache_distribution']
        with urllib.request.urlopen(release['url'], timeout=120) as response:
            archive = response.read()
        if hashlib.sha256(archive).hexdigest() != release['sha256']:
            raise ValueError('sccache release archive checksum mismatch')
        with tarfile.open(fileobj=io.BytesIO(archive), mode='r:gz') as bundle:
            member = bundle.getmember('sccache-v'+policy['tools']['sccache']+'-x86_64-unknown-linux-musl/sccache')
            if not member.isfile():
                raise ValueError('sccache release entry is not a regular file')
            binary = bundle.extractfile(member).read()
        if hashlib.sha256(binary).hexdigest() != expected:
            raise ValueError('sccache executable checksum mismatch')
        target.parent.mkdir(parents=True, exist_ok=True)
        pending = target.with_suffix('.new')
        pending.write_bytes(binary)
        pending.chmod(0o755)
        pending.replace(target)
    actual = subprocess.check_output([target, '--version'], text=True).strip()
    if actual != 'sccache '+policy['tools']['sccache']:
        raise ValueError('sccache executable version mismatch')
    print(json.dumps({'binary': str(target), 'version': actual, 'sha256': expected}))


if __name__ == '__main__':
    install()
