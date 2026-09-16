"""Fixed reviewed Rust integration entry; no caller-supplied command or path."""
import hashlib
import json
from pathlib import Path
import subprocess
import time
import uuid
from preflight import ROOT

BASE = Path(__file__).parent


def installation():
    path = BASE/'reviewed-runtime/manifest.json'
    manifest = json.loads(path.read_text())
    if manifest['workspace'] != str(ROOT):
        raise ValueError('Runtime installation workspace mismatch')
    current = {'Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', 'codex-version.lock'}
    for directory in ('apps', 'migrations', '.cargo'):
        current.update(str(p.relative_to(ROOT)) for p in (ROOT/directory).rglob('*')
                       if p.is_file() and p.suffix in ('.rs', '.toml', '.sql'))
    if current != set(manifest['sources']):
        raise ValueError('Runtime source inventory changed; reviewed rebuild required')
    for name, digest in manifest['sources'].items():
        source = ROOT/name
        if source.resolve() != source.absolute() or not source.is_relative_to(ROOT):
            raise ValueError('Runtime source symlink or path mismatch')
        if hashlib.sha256(source.read_bytes()).hexdigest() != digest:
            raise ValueError('Runtime reviewed source changed: '+name)
    return manifest


def probe():
    before = installation()
    # All untrusted test code runs inside the dedicated outer filesystem and
    # network namespace. The actual app-server still enforces managed policy.
    result = subprocess.run([str(BASE.parent/'codex-sandbox'), '--runtime-product-acceptance'],
                            cwd=ROOT, capture_output=True, text=True, timeout=90)
    after = installation()
    if before != after:
        raise ValueError('Runtime installation changed during acceptance')
    if result.returncode != 0:
        raise RuntimeError('Rust Runtime integration failed: '+(result.stdout+result.stderr)[-6000:])
    if '1 passed; 0 failed' not in result.stdout:
        raise ValueError('Rust Runtime test did not execute')
    proof = dict(ok=True, status='PASS', sample_id=uuid.uuid4().hex, checked_at=time.time(),
                 workspace=str(ROOT), model_calls=0, installation=before,
                 scope='Rust transport, generated protocol, dynamic tool reply and supervisor with pinned app-server',
                 full_runtime_client_acceptance=False, command_exec=dict(exitCode=result.returncode,
                 stdout=result.stdout[-12000:], stderr=result.stderr[-4000:]))
    destination = BASE/'client/runtime-product-acceptance.json'
    pending = destination.with_suffix('.new')
    pending.write_text(json.dumps(proof, indent=2)+'\n');pending.replace(destination)
    return proof


if __name__ == '__main__':
    print(json.dumps(probe(), indent=2))
