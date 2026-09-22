"""Cheap checks before coverage compilation; their failures retain separate logs."""
from capture import PLUGIN_ROOT, run_logged
from isolation import command


def preflight(run, repository):
    checks = [('preflight-selftest', ['python3', '-B', repository / 'tools/gate_selftest.py']),
              ('preflight-format', ['cargo', 'fmt', '--all', '--', '--check']),
              ('preflight-frontend-lint', ['npm', '--prefix', repository / 'web/angular', 'run', 'lint'])]
    for name, args in checks:
        run_logged(run, name, command(args, run=run, repository=repository, plugins=PLUGIN_ROOT,
                                      environment={'PYTHONDONTWRITEBYTECODE': '1'}), timeout=180)
