import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('install_remote_gate', ROOT / 'tools/install_remote_gate.py')
installer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installer)
import evidence_admission as admission  # noqa: E402  (path added by the installer)

NAMES = ['.github/workflows/quality.yml', 'WORKFLOW.lifecycle.md', 'web/angular/package.json',
         'web/angular/package-lock.json', 'tools/install_remote_gate.py', 'tools/install_symphony_development.py',
         'tools/install_sccache.py', 'tools/symphony/trusted_environment.py', 'tools/symphony/reviewed_gate.py',
         'tools/symphony/check_deployment.py', 'tools/remote-gate/host.py', 'tools/remote-gate/github.py',
         'tools/evidence_ledger.py', 'tools/quality-host/evidence_pins.py', 'tools/publication/validation.py', 'tools/trusted.py']


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


class InstallerTest(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name).resolve()
        self.root = self.base / 'repo'
        for name in NAMES:
            (self.root / name).parent.mkdir(parents=True, exist_ok=True)
            (self.root / name).write_text(name)
        self.approval = {'execution_version': 3, 'trusted_files': {'tools/trusted.py': digest(self.root / 'tools/trusted.py')},
                         'runtime_files': {}}
        reader = {'source': 'trusted_files', 'name': 'tools/trusted.py', 'digest': digest(self.root / 'tools/trusted.py')}
        self.read = {'path': 'run.py', 'reader': reader, 'read': 'git rev-parse HEAD', 'use': 'label',
                     'verdict': admission.LABEL_ONLY}
        self.selection = self.read | {'path': 'secrets/mod.rs', 'read': 'git ls-files', 'use': 'secret scan',
                                      'verdict': admission.TREE_FILE_SELECTION, 'guards': [reader],
                                      'retained': list(admission.SELECTION_EVIDENCE)}
        self.approval_path = self.base / 'approval.json'
        self.approval_path.write_text(json.dumps(self.approval))
        self.ledger = self.base / 'ledger'
        self.ledger.mkdir()
        (self.ledger / 'lock').touch()
        self.validation = self.passed()
        self.home = self.base / 'home'
        (self.base / 'evidence/gate').mkdir(parents=True)
        self.pins = self.base / 'evidence/pins'
        self.storage = self.deployment(24)
        for owner, target, value in [(installer, 'ROOT', self.root), (installer, 'HOME', self.home),
                                     (installer, 'PINS', self.pins), (installer, 'STORAGE', self.storage),
                                     (installer, 'SERVICE', self.base / 'unit.service'), (installer.os, 'umask', lambda mask: 0)]:
            item = patch.object(owner, target, value)
            item.start()
            self.addCleanup(item.stop)

    def deployment(self, hours):
        release = self.base / 'storage'
        release.mkdir(exist_ok=True)
        entry = release / 'compact_gate_evidence.py'
        entry.write_text(f'GIB=1024**3\ndef maintain(keep=2, hours={hours}, budget=4*GIB):\n    pass\n')
        path = self.base / 'storage-deployment.json'
        path.write_text(json.dumps({'release': str(release), 'files': {str(entry): digest(entry)}}))
        return path

    def args(self, **changes):
        return argparse.Namespace(**{'verify_only': False, 'publication_ledger': self.ledger,
                                     'equivalence_audit': None, 'audit_window_seconds': None,
                                     'pin_ttl_seconds': None} | changes)

    def verify_only(self, **changes):
        return self.args(**{'verify_only': True, 'audit_window_seconds': 3600, 'pin_ttl_seconds': 7200} | changes)

    def passed(self, omitted=()):
        """A ledger PASS retaining the environment and, unless omitted, the source snapshot and archive."""
        inputs = {'approval': admission.approval_identity(self.approval), 'commit': 'd' * 40,
                  'environment': 'e' * 64, 'tree': 'c' * 40}
        environment = self.base / 'environment.json'
        environment.write_text(json.dumps({'fingerprint': inputs['environment']}))
        evidence = {'environment.json': str(environment)}
        for name in admission.SELECTION_EVIDENCE:
            (self.base / name).write_text(json.dumps({'name': name}))
            if name not in omitted:
                evidence[name] = str(self.base / name)
        attempt = admission.ledger.begin(self.ledger, inputs)
        return admission.ledger.record_pass(self.ledger, attempt, inputs, evidence, {})

    def audit(self, **changes):
        path = self.base / 'audit.json'
        path.write_text(json.dumps({'schema': admission.AUDIT_SCHEMA, 'rule': admission.TREE_EQUIVALENCE,
                                    'conditions': list(admission.CONDITIONS),
                                    'approval': admission.approval_identity(self.approval), 'runtime_files': {},
                                    'tree': 'c' * 40, 'validation_id': self.validation, 'environment': 'e' * 64,
                                    'environment_evidence_sha256': digest(self.base / 'environment.json'),
                                    'git_metadata_reads': [self.read]} | changes))
        return path


class ArgumentTest(InstallerTest):
    def parse(self, *argv):
        with patch('sys.argv', ['install_remote_gate.py', *argv]):
            return installer.arguments()

    def test_defaults_execute_and_explicit_verify_only(self):
        self.assertFalse(self.parse().verify_only)
        args = self.parse('--verify-only', '--publication-ledger', str(self.ledger),
                          '--audit-window-seconds', '3600', '--pin-ttl-seconds', '7200')
        self.assertEqual((args.verify_only, args.publication_ledger, args.audit_window_seconds, args.pin_ttl_seconds),
                         (True, self.ledger, 3600, 7200))

    def test_invalid_arguments_are_refused(self):
        for argv in (['--gate-timeout-seconds', '0'], ['--cache-max-bytes', '-1'], ['--cache-ttl-seconds', '0'],
                     ['--equivalence-audit', 'audit.json'],
                     ['--verify-only', '--publication-ledger', str(self.base / 'absent')]):
            with self.subTest(argv=argv), patch('sys.stderr'), self.assertRaises(SystemExit):
                self.parse(*argv)

    def test_verification_error_requires_a_canonical_locked_ledger(self):
        self.assertIsNone(installer.verification_error(self.args()))
        self.assertIsNone(installer.verification_error(self.verify_only()))
        self.assertRegex(installer.verification_error(self.args(equivalence_audit=Path('a'))), 'requires --verify-only')
        link = self.base / 'ledger-link'
        link.symlink_to(self.ledger)
        for ledger in (Path('ledger'), link, self.base):
            with self.subTest(ledger=ledger):
                self.assertRegex(installer.verification_error(self.verify_only(publication_ledger=ledger)),
                                 'canonical installed ledger')

    def test_retention_windows_are_explicit_bounded_and_ordered(self):
        self.assertRegex(installer.verification_error(self.args(pin_ttl_seconds=1)), 'require --verify-only')
        for changes in ({'audit_window_seconds': None}, {'pin_ttl_seconds': None}):
            with self.subTest(changes=changes):
                self.assertRegex(installer.verification_error(self.verify_only(**changes)), 'requires explicit')
        limit = 24 * 3600
        self.assertEqual(installer.retention_seconds(), limit)
        for window, ttl in ((0, 10), (20, 10), (10, limit - 19), (limit // 3 + 1, limit // 3 + 1)):
            with self.subTest(window=window, ttl=ttl):
                self.assertRegex(installer.verification_error(self.verify_only(audit_window_seconds=window, pin_ttl_seconds=ttl)),
                                 f'pin TTL \\+ 2 \\* audit window <= {limit} installed retention seconds')
        self.assertIsNone(installer.verification_error(self.verify_only(audit_window_seconds=10, pin_ttl_seconds=limit - 20)))
        self.assertIsNone(installer.verification_error(self.verify_only(audit_window_seconds=10, pin_ttl_seconds=10)))

    def test_retention_bound_follows_the_installed_policy(self):
        self.deployment(1)
        self.assertRegex(installer.verification_error(self.verify_only()), '3600 installed retention seconds')
        self.storage.write_text('{"release": "relative", "files": {}}')
        with self.assertRaisesRegex(ValueError, 'canonical'):
            installer.verification_error(self.verify_only())


class VerificationTest(InstallerTest):
    def test_mode_is_always_explicit(self):
        self.assertEqual(installer.verification(self.args(), self.approval), {'mode': 'execute'})
        self.assertEqual(installer.verification(self.verify_only(), self.approval),
                         {'mode': 'verify-only', 'publication_ledger': str(self.ledger), 'pins': str(self.pins),
                          'storage_deployment': str(self.storage), 'audit_window_seconds': 3600,
                          'pin_ttl_seconds': 7200})

    def test_audit_is_installed_only_when_admission_accepts_it_for_its_ledger_pass(self):
        path = self.audit()
        value = installer.verification(self.verify_only(equivalence_audit=path), self.approval)
        self.assertEqual(value['equivalence'], {'rule': admission.TREE_EQUIVALENCE, 'audit': str(path),
                                                'audit_sha256': digest(path)})
        for changes, pattern in [({'conditions': ['same-tree']}, 'enforced conditions'),
                                 ({'approval': 'f' * 64}, 'not reviewed'), ({'runtime_files': {'a': 'b'}}, 'runtime tools'),
                                 ({'git_metadata_reads': []}, 'no reviewed Git metadata reads'),
                                 ({'git_metadata_reads': [self.read | {'path': 'build.rs', 'verdict': 'affects-result'}]},
                                  'unknown Git metadata read classification: build.rs'),
                                 ({'git_metadata_reads': [self.read | {'reader': self.read['reader'] | {'digest': 'f' * 64}}]},
                                  'binding does not match: run.py'),
                                 ({'git_metadata_reads': [self.selection | {'retained': ['source-inputs.json']}]},
                                  'must retain source-inputs.json and source-archive.json'),
                                 ({'tree': 'f' * 40}, 'binds another tree'), ({'tree': ['c' * 40]}, 'binds another tree'),
                                 ({'environment': 'f' * 64}, 'binds another environment'),
                                 ({'environment_evidence_sha256': 'f' * 64}, 'binds another environment_evidence_sha256')]:
            with self.subTest(changes=changes), self.assertRaisesRegex(ValueError, pattern):
                installer.equivalence(self.audit(**changes), self.approval, self.ledger)
        for validation_id, pattern in (('f' * 64, 'unknown validation_id'), ([self.validation], 'invalid validation_id')):
            with self.subTest(validation_id=validation_id), self.assertRaisesRegex(ValueError, pattern):
                installer.equivalence(self.audit(validation_id=validation_id), self.approval, self.ledger)
        with self.assertRaises(FileNotFoundError):
            installer.equivalence(self.base / 'absent.json', self.approval, self.ledger)

    def test_file_selection_audit_needs_the_retained_source_evidence_of_its_pass(self):
        selection = [self.read, self.selection]
        path = self.audit(git_metadata_reads=selection)
        self.assertEqual(installer.equivalence(path, self.approval, self.ledger)['audit_sha256'], digest(path))
        # Installation checks retained evidence against the audited record, as admission does.
        for name in admission.SELECTION_EVIDENCE:
            with self.subTest(name=name), self.assertRaisesRegex(ValueError, 'record lacks: ' + name):
                installer.equivalence(self.audit(git_metadata_reads=selection, validation_id=self.passed((name,))),
                                      self.approval, self.ledger)


class DeploymentTest(InstallerTest):
    def test_previous_deployments_are_deduplicated_and_bound_to_the_repository(self):
        prior = {'repository': 'musutrade/CodexSymphony', 'protected_files': {'a': 'b'},
                 'gate_approval': str(self.approval_path), 'other': 1}
        path = self.base / 'prior.json'
        path.write_text(json.dumps(prior))
        self.assertEqual(installer.previous_deployments([path, path]),
                         [{'protected_files': {'a': 'b'}, 'gate_approval': str(self.approval_path)}])
        path.write_text(json.dumps(prior | {'repository': 'other/repo'}))
        with self.assertRaisesRegex(ValueError, 'repository differs'):
            installer.previous_deployments([path])

    def test_protected_files_include_the_ledger_and_every_bridge_module(self):
        protected = installer.protected_files(self.approval)
        self.assertEqual(set(protected), set(NAMES) - {'tools/trusted.py'})
        self.assertEqual(protected['tools/evidence_ledger.py'], digest(self.root / 'tools/evidence_ledger.py'))
        (self.root / 'tools/trusted.py').write_text('changed')
        with self.assertRaisesRegex(ValueError, 'host input changed'):
            installer.protected_files(self.approval)

    def test_release_carries_the_ledger_and_detects_drift(self):
        (self.root / 'tools/remote-gate/__pycache__').mkdir()
        release = installer.install_release('v1')
        self.assertEqual(release, self.home / 'releases/v1')
        self.assertEqual(sorted(p.name for p in release.iterdir()),
                         ['evidence_ledger.py', 'evidence_pins.py', 'github.py', 'host.py', 'validation.py'])
        self.assertEqual(installer.install_release('v1'), release)
        (release / 'evidence_ledger.py').write_text('drift')
        with self.assertRaisesRegex(ValueError, 'bridge changed'):
            installer.install_release('v1')

    def test_pin_root_is_installed_beside_bounded_runs_only_for_verify_only(self):
        installer.install_pins({'mode': 'execute'})
        self.assertFalse(self.pins.exists())
        installer.install_pins({'mode': 'verify-only', 'pins': str(self.pins)})
        installer.install_pins({'mode': 'verify-only', 'pins': str(self.pins)})
        self.assertTrue((self.pins / 'lock').is_file())
        (self.pins / 'lock').unlink()
        (self.pins / 'lock').symlink_to(self.base / 'elsewhere')
        with self.assertRaisesRegex(ValueError, 'regular file'):
            installer.install_pins({'mode': 'verify-only', 'pins': str(self.pins)})
        with self.assertRaisesRegex(ValueError, 'beside the bounded Gate runs'):
            installer.install_pins({'mode': 'verify-only', 'pins': str(self.base / 'other/pins')})
        (self.base / 'alias').symlink_to(self.pins)
        (self.base / 'gate').mkdir()
        with self.assertRaisesRegex(ValueError, 'canonical'):
            installer.install_pins({'mode': 'verify-only', 'pins': str(self.base / 'alias')})

    def test_service_is_written_and_its_effective_command_checked(self):
        with patch.object(installer.subprocess, 'run') as run, patch.object(installer, 'check_commands') as check:
            service = installer.install_service(Path('/release'), Path('/config.json'))
        text = service.read_text()
        self.assertIn('ExecStart=/usr/bin/python3 /release/host.py --config /config.json', text)
        run.assert_called_once_with(['systemctl', '--user', 'daemon-reload'], check=True)
        check.assert_called_once_with({'codexsymphony-remote-gate.service':
                                       '/usr/bin/python3 /release/host.py --config /config.json'})

    def test_main_installs_an_immutable_config_with_an_explicit_mode(self):
        argv = ['install_remote_gate.py', '--gate-approval', str(self.approval_path), '--verify-only',
                '--publication-ledger', str(self.ledger), '--equivalence-audit', str(self.audit()),
                '--audit-window-seconds', '3600', '--pin-ttl-seconds', '7200']
        with patch('sys.argv', argv), patch.object(installer, 'install_service', return_value=Path('/unit')) as service, \
                patch('builtins.print') as printed:
            installer.main()
        result = json.loads(printed.call_args.args[0])
        config = json.loads(Path(result['config']).read_text())
        self.assertEqual((config['mode'], config['publication_ledger'], result['started']), ('verify-only', str(self.ledger), False))
        self.assertEqual(config['equivalence']['rule'], admission.TREE_EQUIVALENCE)
        self.assertEqual(config['gate_approval'], str(self.approval_path))
        self.assertEqual((config['audit_window_seconds'], config['pin_ttl_seconds'], config['pins']), (3600, 7200, str(self.pins)))
        self.assertTrue((self.pins / 'lock').is_file())
        service.assert_called_once_with(Path(result['release']), Path(result['config']))
        Path(result['config']).write_text('{}')
        with patch('sys.argv', argv), patch.object(installer, 'install_service'), patch('builtins.print'), \
                self.assertRaisesRegex(ValueError, 'immutable config collision'):
            installer.main()


if __name__ == '__main__':
    unittest.main()
