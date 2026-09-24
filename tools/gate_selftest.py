#!/usr/bin/env python3
"""Run gate regression suites in separate processes, without module-name collisions."""
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
SUITES = [('tools/remote-gate', 'test_*.py'), ('tools/quality-host', 'test_*.py'), ('tools/publication','test_*.py')]
SUITES += [('tools/tests','test_environment_contract.py'), ('tools/symphony','test_environment_installation.py')]
SUITES += [('tools/tests', name) for name in (
    'test_artifact_packaging.py', 'test_gate_install.py', 'test_http_readiness.py', 'test_sccache.py',
)]


def main():
    for directory, pattern in SUITES:
        path = ROOT / directory
        if not list(path.glob(pattern)):
            raise ValueError('missing gate regression suite: ' + directory + '/' + pattern)
        subprocess.run([sys.executable, '-B', '-m', 'unittest', 'discover',
                        '-s', str(path), '-p', pattern, '-v'], cwd=ROOT, check=True)


if __name__ == '__main__':
    main()
