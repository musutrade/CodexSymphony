"""Real mount/process acceptance; fixtures contain only non-sensitive sentinels."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[2]
EXECUTOR = ROOT / 'apps/server/deployment/executor.py'


class BoundaryTests(unittest.TestCase):
    def test_installer_stages_private_examples_and_refuses_overwrite(self):
        # Root exists only inside this disposable user/mount namespace; the
        # host /opt is hidden by tmpfs and the rest of the filesystem read-only.
        code = f'''import pathlib, subprocess
installer = {str(ROOT / 'tools/deployment/install.sh')!r}
subprocess.run(['/bin/sh', installer, 'fixture'], check=True)
release = pathlib.Path('/opt/codexsymphony-m2/releases/fixture')
assert (release/'coding').stat().st_mode & 0o777 == 0o755
assert (release/'examples/auth.json').stat().st_mode & 0o777 == 0o600
assert subprocess.run(['/bin/sh', installer, 'fixture']).returncode != 0
assert subprocess.run([str(release/'coding'), 'app-server']).returncode != 0
'''
        result = subprocess.run(['/usr/bin/bwrap', '--unshare-user', '--uid', '0',
            '--gid', '0', '--ro-bind', '/', '/', '--tmpfs', '/opt', '--',
            '/usr/bin/python3', '-I', '-c', code], capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='m2-boundary-')
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.workspace = self.root/'workspaces/project'
        self.workspace.mkdir(parents=True)
        self.home = self.root/'execution/run/codex-home'
        self.home.mkdir(parents=True)
        self.private = []
        for name in ('control', 'github', 'signing', 'notifier', 'host-home'):
            directory = self.root/name
            directory.mkdir(mode=0o700)
            secret = directory/'sentinel'
            secret.write_text('non-sensitive-' + name)
            secret.chmod(0o600)
            self.private.append(directory)
        host_home = self.root/'host-home'
        (host_home/'.gitconfig').write_text('[credential]\n helper = host-only-helper\n')
        (host_home/'.codex').mkdir()
        (host_home/'.codex'/'config.toml').write_text('[mcp_servers.host]\ncommand = "host-only"\n[features]\napps = true\n')
        (host_home/'.ssh').mkdir()
        (host_home/'.ssh'/'id_ed25519').write_text('non-sensitive-SSH-sentinel')
        (self.workspace/'source.txt').write_text('authorized source')
        self.project = self.root/'project-test'
        self.project.mkdir()
        (self.project/'sentinel').write_text('authorized project test data')
        self.config = dict(version=1, role='coding', workspace_root=str(self.workspace.parent),
                           runtime_home_root=str(self.root/'execution'),
                           mounts=[dict(path='/usr', writable=False),
                                   dict(path=str(self.project), writable=False)],
                           private_paths=list(map(str, self.private)), environment={},
                           program=str(Path('/usr/bin/python3').resolve()))
        self.path = self.root/'boundary.json'

    def save(self):
        self.path.write_text(json.dumps(self.config))
        self.path.chmod(0o600)

    def command(self, code):
        return ['/usr/bin/python3', '-I', str(EXECUTOR), str(self.path), '-c', code]

    def run_child(self, code):
        self.save()
        return subprocess.run(self.command(code), cwd=self.workspace,
            env=dict(os.environ, CODEX_HOME=str(self.home), CONTROL_SENTINEL='must-not-inherit',
                     DATABASE_URL='must-not-inherit', SSH_AUTH_SOCK=str(self.root/'host-home'/'agent.sock')), capture_output=True, text=True, timeout=10)

    def test_coding_and_validation_cannot_read_private_files_or_parent_environment(self):
        for role in ('coding', 'validation'):
            with self.subTest(role=role):
                self.config.update(role=role, runtime_home_root=str(self.root/'execution') if role == 'coding' else None)
                code = f'''import os
from pathlib import Path
assert Path('source.txt').read_text() == 'authorized source'
assert Path({str(self.project/'sentinel')!r}).read_text() == 'authorized project test data'
for name in {list(map(str, self.private))!r}:
    try: Path(name, 'sentinel').read_text()
    except (FileNotFoundError, PermissionError): pass
    else: raise AssertionError('private sentinel readable')
assert 'CONTROL_SENTINEL' not in os.environ and 'DATABASE_URL' not in os.environ
assert 'SSH_AUTH_SOCK' not in os.environ
assert os.environ['HOME'] == '/home/executor'
assert not Path({str(self.root/'host-home'/'.gitconfig')!r}).exists()
assert not Path({str(self.root/'host-home'/'.codex'/'config.toml')!r}).exists()
assert not Path({str(self.root/'host-home'/'.ssh'/'id_ed25519')!r}).exists()
assert not Path(os.environ['HOME'], '.codex').exists()
assert not Path({str(self.path)!r}).exists()
assert not Path('/proc/{os.getpid()}/root').exists()
Path('ordinary-command-output').write_text('working')
print('source/test data available; private files/environment unavailable')
'''
                child = self.run_child(code)
                self.assertEqual(child.returncode, 0, child.stderr)
                self.assertEqual((self.workspace/'ordinary-command-output').read_text(), 'working')

    def test_reject_private_mount_and_incomplete_or_writable_configuration(self):
        self.config['mounts'].append(dict(path=str(self.private[0]), writable=False))
        self.assertNotEqual(self.run_child("print('MUST NOT RUN')").returncode, 0)
        self.config.pop('private_paths')
        self.assertNotEqual(self.run_child("print('MUST NOT RUN')").returncode, 0)
        self.save()
        self.path.chmod(0o644)
        child = subprocess.run(self.command('pass'), cwd=self.workspace, capture_output=True)
        self.assertNotEqual(child.returncode, 0)

    def test_product_supervisor_stops_namespace_descendants_and_rejects_replay(self):
        self.save()
        directory = self.root/'supervisor'
        directory.mkdir()
        key = dict(run_id='boundary-run', request_id='boundary-request', incarnation='boundary-incarnation')
        code = "import os,time; from pathlib import Path; os.fork(); Path('started').touch(); time.sleep(120)"
        command = self.command(code)
        launch = dict(key=key, workspace=str(self.workspace), workspace_identity='fixture',
                      program=command[0], args=command[1:])
        for name, value in [('launch.json', launch), ('start.json', key), ('storage-heartbeat.json', key)]:
            (directory/name).write_text(json.dumps(value))
        binary = ROOT/os.environ.get('CARGO_TARGET_DIR', 'target')/'debug/codexsymphony-server'
        process = subprocess.Popen([str(binary), '--supervise', str(directory)],
            env=dict(os.environ, CODEX_HOME=str(self.home)), stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        try:
            deadline = time.monotonic() + 8
            while not (self.workspace/'started').exists() and time.monotonic() < deadline:
                time.sleep(.02)
            self.assertTrue((self.workspace/'started').exists())
            (directory/'stop.json').write_text('true')
            _, stderr = process.communicate(timeout=8)
            self.assertEqual(process.returncode, 0, stderr)
            receipt = json.loads((directory/'quiescent.json').read_text())
            self.assertEqual(receipt, json.loads((directory/'identity.json').read_text()))
            self.assertEqual(receipt['key'], key)
            replay = subprocess.run([str(binary), '--supervise', str(directory)], capture_output=True, timeout=5)
            self.assertNotEqual(replay.returncode, 0)
        finally:
            if process.poll() is None:
                process.kill()
                process.communicate()


if __name__ == '__main__':
    unittest.main()
