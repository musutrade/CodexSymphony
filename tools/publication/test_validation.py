import copy
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import time
import unittest
from unittest.mock import patch

import validation

spec = importlib.util.spec_from_file_location('reuse_gate', Path(__file__).with_name('gate.py'))
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)


class VerificationTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.repo = self.root / 'repo'
        self.repo.mkdir()
        self.git('init', '-q')
        self.git('config', 'user.name', 'Fixture')
        self.git('config', 'user.email', 'fixture@example.invalid')
        (self.repo / 'source').write_text('measured source')
        self.git('add', '.')
        self.git('commit', '-qm', 'first')
        self.first = self.git('rev-parse', 'HEAD')
        self.runtime = self.root / 'runtime.py'
        self.runtime.write_text('approved runtime')
        self.approval = {'runtime_files': {str(self.runtime): validation.file_digest(self.runtime)},
                         'config_files': {}, 'trusted_files': {}, 'baseline': {'commit': 'b' * 40}}
        self.contract = {'schema': validation.SCHEMA, 'approval': validation.digest(self.approval),
                         **self.approval, 'complete_dependency_review': True, 'max_age_seconds': 3600,
                         'required_evidence': sorted(validation.MINIMUM_EVIDENCE),
                         'external_inputs': [str(self.runtime)],
                         'git_reads': [{'reader': str(self.runtime), 'sha256': validation.file_digest(self.runtime),
                                        'command': 'head', 'classification': 'label-only'}]}
        self.contract_path = self.root / 'contract.json'
        self.setting = self.review()
        self.current = {'tree': gate.source_tree(self.repo), 'environment': 'e' * 64,
                        'approval': validation.digest(self.approval)}
        self.frozen = validation.freeze(self.repo, self.current, self.contract)
        self.ledger = self.root / 'ledger'
        self.run = self.root / 'evidence/gate/run-176000000001'
        self.run.mkdir(parents=True)
        self.fake_evidence(self.run, self.first)
        self.report = self.run / 'report'
        self.report.write_text(json.dumps({'passed': True, 'evidence_complete': True,
                                          'source_identity': 'working-tree:' + self.first}))
        (self.run / 'verification-inputs.json').write_text(json.dumps(self.frozen))
        identity = gate.ledger_inputs(self.current, self.first)
        evidence = {name: str(self.run / name) for name in validation.MINIMUM_EVIDENCE}
        self.validation_id = gate.ledger.record_pass(self.ledger, gate.ledger.begin(self.ledger, identity),
                    identity, evidence, {'scope': 'complete-local-isolated-gate', 'run': str(self.run),
                                         'verification_inputs': self.frozen})
        self.record = gate.ledger.verify(self.ledger, self.validation_id, identity)
        self.selected = self.pin_settings(self.root)

    def pin_settings(self, prefix):
        pins = prefix / 'evidence/pins'
        pins.mkdir(parents=True)
        (pins / 'lock').touch()
        storage = prefix / 'storage'
        storage.mkdir()
        policy = storage / 'compact_gate_evidence.py'
        policy.write_text('def maintain(budget=10000000):\n    pass\n')
        deployment = prefix / 'deployment.json'
        deployment.write_text(json.dumps({'release': str(storage),
                                         'files': {str(policy): validation.file_digest(policy)}}))
        remote = prefix / 'remote.json'
        remote.write_text(json.dumps({'repository': 'fixture/repo', 'pins': str(pins),
                                     'storage_deployment': str(deployment), 'pin_ttl_seconds': 3600}))
        return {'remote_config': str(remote)}

    def fake_evidence(self, run, commit):
        for name in validation.MINIMUM_EVIDENCE:
            (run / name).write_text('{}')
        (run / 'report').write_text(json.dumps({'passed': True, 'evidence_complete': True,
                                              'source_identity': 'working-tree:' + commit}))
        (run / 'source.tar.gz').write_bytes(b'fake archive')
        (run / 'source-archive.json').write_text(json.dumps({'sha256': validation.file_digest(run / 'source.tar.gz'),
                                                           'inputs': {'source': {}}}))
        (run / 'probes/backend').mkdir(parents=True)
        bundle = run / 'probes/backend/bundle.json'
        bundle.write_text('{}')
        (run / 'capture-registration.json').write_text(json.dumps({'root': str(run),
                                                  'bundle_sha256': validation.file_digest(bundle)}))
        measurement = run / 'backend-measurements'
        measurement.write_text('{}')
        (run / 'measurement-summary.json').write_text(json.dumps({'coverage_and_crap': 'PASS',
                        'measurement': str(measurement), 'measurement_sha256': validation.file_digest(measurement)}))

    def git(self, *arguments):
        return subprocess.check_output(['git', '-C', str(self.repo), *arguments],
                                       stderr=subprocess.PIPE, text=True).strip()

    def review(self):
        self.contract_path.write_text(json.dumps(self.contract))
        return {'path': str(self.contract_path), 'sha256': validation.file_digest(self.contract_path)}

    def test_only_complete_installed_review_accepts_known_classifications(self):
        self.assertEqual(validation.reviewed(self.setting, self.approval), self.contract)
        for name, change in [('schema', 'old'), ('complete_dependency_review', False),
                             ('approval', 'other'), ('runtime_files', {}), ('max_age_seconds', 0),
                             ('required_evidence', ['report']), ('git_reads', [])]:
            with self.subTest(name=name):
                saved = self.contract[name]
                self.contract[name] = change
                with self.assertRaises(ValueError):
                    validation.reviewed(self.review(), self.approval)
                self.contract[name] = saved
        for changes in [{'classification': 'unknown'}, {'command': 'shell'}, {'sha256': 'forged'},
                        {'classification': 'source-selection'}]:
            saved = self.contract['git_reads']
            self.contract['git_reads'] = [saved[0] | changes]
            with self.assertRaises(ValueError):
                validation.reviewed(self.review(), self.approval)
            self.contract['git_reads'] = saved
        self.setting = self.review()
        self.contract_path.write_text('{}')
        with self.assertRaisesRegex(ValueError, 'changed'):
            validation.reviewed(self.setting, self.approval)
        self.assertIsNone(validation.reviewed(None, self.approval))

    def test_same_all_inputs_different_commit_reuses_without_rewriting_evidence(self):
        original = (self.report.read_bytes(), (self.ledger / 'events.jsonl').read_bytes())
        self.git('commit', '--allow-empty', '-qm', 'metadata only')
        second = self.git('rev-parse', 'HEAD')
        frozen = validation.freeze(self.repo, self.current, self.contract)
        self.assertEqual(frozen, self.frozen)
        validation.reusable(self.record, frozen, self.contract)
        state = self.root / 'state'
        state.mkdir()
        with patch.object(gate, 'LEDGER', self.ledger):
            record, reason = gate.prior_verification(self.current, frozen, self.contract)
        self.assertEqual(record['validation_id'], self.validation_id)
        with patch.object(gate, 'settings', return_value=self.selected):
            result = gate.bind_publication(state, record, second, frozen, self.first)
        bound = validation.pins.active(self.root / 'evidence/pins', result['binding_pin'],
                                       self.validation_id)['publication']['binding']
        self.assertEqual((bound['commit'], bound['validated_commit']), (second, self.first))
        self.assertEqual((bound['expensive_capture_count'], bound['native_added_bytes']), (0, 0))
        self.assertEqual(original, (self.report.read_bytes(), (self.ledger / 'events.jsonl').read_bytes()))
        with patch.object(gate, 'settings', return_value=self.selected):
            self.assertEqual(gate.bind_publication(state, record, second, frozen, self.first), result)

    def test_actual_inputs_dirty_selection_and_commit_sensitive_reads_invalidate(self):
        for name in self.current:
            with self.subTest(name=name), self.assertRaises(ValueError):
                frozen = validation.freeze(self.repo, self.current | {name: 'changed'}, self.contract)
                validation.reusable(self.record, frozen, self.contract)
        (self.repo / 'extra').write_text('untracked')
        with self.assertRaises(ValueError):
            validation.reusable(self.record, validation.freeze(self.repo, self.current, self.contract), self.contract)
        (self.repo / 'extra').unlink()
        self.contract['git_reads'][0]['classification'] = 'input'
        first = validation.freeze(self.repo, self.current, self.contract)
        self.git('commit', '--allow-empty', '-qm', 'sensitive commit')
        self.assertNotEqual(first, validation.freeze(self.repo, self.current, self.contract))

    def test_expiry_incomplete_missing_payload_forged_and_revoked_results_refused(self):
        for now in [self.record['created_at_ms'] / 1000 - 1,
                    self.record['created_at_ms'] / 1000 + 3600]:
            with self.assertRaisesRegex(ValueError, 'expired|future'):
                validation.reusable(self.record, self.frozen, self.contract, now=now)
        for change in [{'scope': 'hook'}, {'verification_inputs': {}}]:
            record = self.record | {'details': self.record['details'] | change}
            with self.assertRaises(ValueError):
                validation.reusable(record, self.frozen, self.contract)
        payload = self.run / 'requests.json'
        payload.unlink()
        with self.assertRaises(ValueError):
            validation.reusable(self.record, self.frozen, self.contract)
        payload.write_text('{}')
        with self.assertRaisesRegex(ValueError, 'old or incompatible'):
            validation.reusable(self.record, self.frozen, self.contract | {'max_age_seconds': 7200})
        with patch.object(gate, 'LEDGER', self.ledger):
            gate.ledger.revoke(self.ledger, self.validation_id, 'review withdrawn')
            with self.assertRaisesRegex(ValueError, 'blocked'):
                gate.prior_verification(self.current, self.frozen, self.contract)

    def test_local_binding_reserves_capacity_and_release_revokes_admission(self):
        config = json.loads(Path(self.selected['remote_config']).read_text())
        with patch.object(gate, 'settings', return_value=self.selected), \
                patch.object(validation.pins, 'records_budget', return_value=1):
            with self.assertRaisesRegex(ValueError, 'record bytes'):
                gate.bind_publication(self.root, self.record, self.first, self.frozen, self.first)
        self.assertEqual(validation.pins.current(config['pins']), {})
        with patch.object(gate, 'settings', return_value=self.selected):
            bound = gate.bind_publication(self.root, self.record, self.first, self.frozen, self.first)
        receipt = {'status': 'PASS', 'scope': 'complete-local-isolated-gate', 'inputs': self.current,
                   'commit': self.first, 'validation_id': self.validation_id, 'report': str(self.report),
                   'report_sha256': validation.file_digest(self.report), **bound}
        with patch.object(gate, 'settings', return_value=self.selected), patch.object(gate, 'LEDGER', self.ledger), \
                patch.object(gate, 'inputs', return_value=self.current), \
                patch.object(gate, 'verification_context', return_value=(self.contract, self.frozen)), \
                patch.object(gate, 'prepublication', return_value=self.first):
            self.assertEqual(gate.admit(self.repo, receipt, self.current['tree'])['commit'], self.first)
            validation.pins.release(config['pins'], {bound['binding_pin']})
            with self.assertRaisesRegex(ValueError, 'expired'):
                gate.admit(self.repo, receipt, self.current['tree'])

    def test_actual_external_dependency_bytes_and_permissions_are_inputs(self):
        dependency = self.root / 'dependencies'
        dependency.mkdir()
        library = dependency / 'library'
        library.write_text('actual installed dependency')
        self.contract['external_inputs'] = [str(dependency)]
        initial = validation.freeze(self.repo, self.current, self.contract)
        library.write_text('changed without lockfile change')
        self.assertNotEqual(initial, validation.freeze(self.repo, self.current, self.contract))
        library.write_text('actual installed dependency')
        library.chmod(0o755)
        self.assertNotEqual(initial, validation.freeze(self.repo, self.current, self.contract))
        library.unlink()
        library.symlink_to(self.runtime)
        with self.assertRaisesRegex(ValueError, 'unsupported external dependency link'):
            validation.freeze(self.repo, self.current, self.contract)

    def test_candidate_git_clean_filters_are_never_executed_by_freezing(self):
        (self.repo / '.gitattributes').write_text('* filter=host-command\n')
        self.git('add', '.')
        self.git('commit', '-qm', 'attributes')
        marker = self.root / 'FILTER_EXECUTED'
        self.git('config', 'filter.host-command.clean', 'touch ' + str(marker) + '; cat')
        current = self.current | {'tree': gate.source_tree(self.repo)}
        validation.freeze(self.repo, current, self.contract)
        self.assertFalse(marker.exists())

    def test_main_fetch_is_bounded_and_never_reads_candidate_git_configuration(self):
        with patch.object(gate.subprocess, 'run') as execute, \
                patch.object(gate.subprocess, 'check_output', return_value=self.first + '\n'):
            self.assertEqual(gate.fetch_main(self.repo, 'fixture/repo'), self.first)
        self.assertEqual(execute.call_count, 2)
        fetch = execute.call_args_list[1]
        self.assertNotIn(str(self.repo), fetch.args[0])
        self.assertTrue(any(value.startswith('--git-dir=') for value in fetch.args[0]))
        self.assertEqual(fetch.kwargs['timeout'], 60)
        self.assertEqual(fetch.kwargs['env']['GIT_CONFIG_GLOBAL'], '/dev/null')

    def test_old_chain_two_captures_new_chain_one_in_same_fixture_environment(self):
        first = self.git('rev-parse', 'HEAD')
        outcomes = []
        for enabled in [False, True]:
            self.git('checkout', '-q', '--detach', first)
            prefix = self.root / ('new' if enabled else 'old')
            prefix.mkdir()
            ledger_root = prefix / 'ledger'
            configured = self.pin_settings(prefix)
            calls = []
            added_bytes = []
            def capture(root, state, before, commit):
                run = prefix / 'evidence/gate' / ('run-' + str(len(calls)).zfill(12))
                run.mkdir(parents=True)
                self.fake_evidence(run, commit)
                report = run / 'report'
                # Fake Runtime performs real writes; count its actual newly written bytes.
                native = run / 'native-fixture'
                native.write_bytes(bytes(4096))
                calls.append(commit)
                added_bytes.append(native.stat().st_size)
                return run, report, {'execution_version': 3}, 'complete-local-isolated-gate'
            selected = configured if enabled else None
            reviewed = self.contract if enabled else None
            frozen = self.frozen if enabled else None
            with patch.object(gate, 'workspace', return_value=self.repo), patch.object(gate, 'STATE', prefix), \
                    patch.object(gate, 'LEDGER', ledger_root), patch.object(gate, 'settings', return_value=selected), \
                    patch.object(gate, 'inputs', return_value=self.current), \
                    patch.object(gate, 'verification_context', return_value=(reviewed, frozen)), \
                    patch.object(gate, 'prepublication', return_value=first), \
                    patch.object(gate, 'run_gate', side_effect=capture):
                started = time.monotonic()
                gate.validate('GH-176')
                self.git('commit', '--allow-empty', '-qm', 'metadata-only second')
                gate.validate('GH-176')
                outcomes.append({'captures': len(calls), 'native_bytes': sum(added_bytes),
                                 'wall_ms': (time.monotonic() - started) * 1000})
            self.assertTrue((prefix / 'GH-176/timings.jsonl').exists())
        self.assertEqual([row['captures'] for row in outcomes], [2, 1])
        self.assertEqual([row['native_bytes'] for row in outcomes], [8192, 4096])
        self.assertTrue(all(row['wall_ms'] > 0 for row in outcomes))

    def test_preflight_and_main_change_do_not_merge_or_repeat_capture(self):
        remote = self.root / 'remote.json'
        approval_path = self.root / 'approval.json'
        baseline = self.root / 'baseline.json'
        baseline.write_text('{}')
        approval = self.approval | {'baseline': {'path': str(baseline), 'sha256': validation.file_digest(baseline)}}
        approval_path.write_text(json.dumps(approval))
        remote.write_text(json.dumps({'mode': 'verify-only', 'candidate_registration': True,
                  'gate_approval': str(approval_path), 'repository': 'fixture/repo', 'protected_files': {}}))
        selected = {'remote_config': str(remote), 'remote_config_sha256': validation.file_digest(remote)}
        self.git('update-ref', 'refs/remotes/origin/main', self.first)
        with patch.object(gate, 'APPROVAL', approval_path):
            self.assertEqual(gate.prepublication(self.repo, selected, fetch=False), self.first)
            self.git('checkout', '-qb', 'new-main')
            (self.repo / 'source').write_text('main changed')
            self.git('commit', '-qam', 'main advanced')
            newer = self.git('rev-parse', 'HEAD')
            self.git('update-ref', 'refs/remotes/origin/main', newer)
            self.git('checkout', '-q', '--detach', self.first)
            with self.assertRaisesRegex(ValueError, 'reconcile explicitly'):
                gate.prepublication(self.repo, selected, fetch=False)
            self.assertEqual(self.git('rev-parse', 'HEAD'), self.first)

    def test_validate_second_commit_has_zero_expensive_calls_and_immutable_original(self):
        self.git('commit', '--allow-empty', '-qm', 'second')
        second = self.git('rev-parse', 'HEAD')
        state = self.root / 'publication'
        original = (self.ledger / 'events.jsonl').read_bytes()
        with patch.object(gate, 'workspace', return_value=self.repo), patch.object(gate, 'STATE', state), \
                patch.object(gate, 'LEDGER', self.ledger), patch.object(gate, 'settings', return_value=self.selected), \
                patch.object(gate, 'inputs', return_value=self.current), \
                patch.object(gate, 'verification_context', return_value=(self.contract, self.frozen)), \
                patch.object(gate, 'prepublication', return_value=self.first), \
                patch.object(gate, 'run_gate', side_effect=AssertionError('duplicate capture')):
            gate.validate('GH-176')
        receipt = json.loads((state / 'GH-176/receipt.json').read_text())
        self.assertEqual((receipt['status'], receipt['commit'], receipt['validation_id']),
                         ('PASS', second, self.validation_id))
        self.assertEqual(original, (self.ledger / 'events.jsonl').read_bytes())
        with patch.object(gate, 'workspace', return_value=self.repo), patch.object(gate, 'STATE', state), \
                patch.object(gate, 'LEDGER', self.ledger), patch.object(gate, 'settings', return_value=self.selected), \
                patch.object(gate, 'inputs', return_value=self.current), \
                patch.object(gate, 'verification_context', return_value=(self.contract, self.frozen)), \
                patch.object(gate, 'prepublication', side_effect=[self.first, 'changed-main']), \
                patch.object(gate, 'run_gate', side_effect=AssertionError('unbounded retry')):
            with self.assertRaisesRegex(ValueError, 'main changed'):
                gate.validate('GH-176')
        self.assertEqual(original, (self.ledger / 'events.jsonl').read_bytes())


if __name__ == '__main__':
    unittest.main()
