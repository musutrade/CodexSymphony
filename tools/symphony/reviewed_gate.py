"""Select only a host-reviewed Gate binary from the project's version lock."""
from pathlib import Path
import hashlib

PINS = {
    'harness-gate v0.4.5': ('v0.4.5', '70721282c751826ed4d57e14bd7de9516e73e833aa058d758dbd2154c0aa5e10'),
    'harness-gate v0.4.6-rc.1': ('v0.4.6-rc.1', '6d5dcf10b8d6b1248679974664a97b29628fa24778047484f18b0ebbe5d27dd3'),
}

VERSIONS = Path('/home/gem/.local/share/harness-gate/versions')

def gate_bin(workspace):
    lock = (Path(workspace) / 'harness-gate-version.lock').read_text().splitlines()
    if not lock or lock[0] not in PINS:
        raise ValueError('Gate version is not host-reviewed')
    version, digest = PINS[lock[0]]
    directory = VERSIONS / version / 'bin'
    if hashlib.sha256((directory / 'harness-gate').read_bytes()).hexdigest() != digest:
        raise ValueError('Reviewed Gate binary digest mismatch')
    return str(directory)

if __name__ == '__main__':
    print(gate_bin(Path.cwd()))
