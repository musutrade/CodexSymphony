import json
from pathlib import Path
import unittest

import evidence_admission as admission
import test_evidence_admission as fixtures


class RemoteVerificationTest(unittest.TestCase):
    def setUp(self):
        self.fixture = fixtures.AdmissionTest(methodName='runTest')
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.verifier = admission.load_verification()
        self.config = self.fixture.config | {'candidate_registration': True}
        self.dependency = self.fixture.base / 'installed-dependency'
        self.dependency.write_text('actual dependency v1')
        approval = self.fixture.approval
        self.contract = {'schema': self.verifier.SCHEMA, 'approval': admission.approval_identity(approval),
                    'runtime_files': approval['runtime_files'], 'config_files': approval['config_files'],
                    'trusted_files': approval['trusted_files'], 'baseline': approval['baseline'],
                    'git_reads': [{'reader': self.fixture.runtime, 'sha256': approval['runtime_files'][self.fixture.runtime],
                                   'command': 'head', 'classification': 'label-only'}],
                    'external_inputs': [str(self.dependency)],
                    'required_evidence': sorted(self.verifier.MINIMUM_EVIDENCE),
                    'max_age_seconds': 3600, 'complete_dependency_review': True}
        self.contract_path = self.fixture.base / 'contract.json'
        self.configure()
        inputs = self.fixture.inputs()
        self.frozen = {'schema': self.verifier.INPUT_SCHEMA, 'contract': self.verifier.digest(self.contract),
                    **{key: inputs[key] for key in ('tree', 'environment', 'approval')},
                    'baseline': approval['baseline'], 'source_inventory': '1' * 64, 'tracked': '2' * 64,
                    'workspace_status': '3' * 64, 'git_inputs': {},
                    'external_inputs': {str(self.dependency): self.verifier.external_input(self.dependency)}}
        self.evidence = self.fixture.evidence()
        self.run = Path(self.evidence['report']).parent
        for name in self.verifier.MINIMUM_EVIDENCE - set(self.evidence):
            path = self.run / name
            path.write_text('{}')
            self.evidence[name] = str(path)
        (self.run / 'source.tar.gz').write_bytes(b'fake archive')
        (self.run / 'source-archive.json').write_text(json.dumps({
                    'sha256': self.verifier.file_digest(self.run / 'source.tar.gz'), 'inputs': {'source': {}}}))
        bundle = self.run / 'probes/backend/bundle.json'
        bundle.parent.mkdir(parents=True)
        bundle.write_text('{}')
        (self.run / 'capture-registration.json').write_text(json.dumps({
                    'root': str(self.run), 'bundle_sha256': self.verifier.file_digest(bundle)}))
        measurement = self.run / 'backend-measurements'
        measurement.write_text('{}')
        (self.run / 'measurement-summary.json').write_text(json.dumps({
                    'coverage_and_crap': 'PASS', 'measurement': str(measurement),
                    'measurement_sha256': self.verifier.file_digest(measurement)}))
        self.record = self.record_pass()

    def configure(self):
        self.contract_path.write_text(json.dumps(self.contract))
        self.config['verification_contract'] = {'path': str(self.contract_path),
                                                 'sha256': self.verifier.file_digest(self.contract_path)}

    def record_pass(self):
        fixture = self.fixture
        inputs = fixture.inputs()
        (self.run / 'verification-inputs.json').write_text(json.dumps(self.frozen))
        validation = admission.ledger.record_pass(fixture.ledger, admission.ledger.begin(fixture.ledger, inputs),
                         inputs, self.evidence, {'scope': 'complete-local-isolated-gate', 'run': str(self.run),
                                                'verification_inputs': self.frozen})
        return admission.ledger.verify(fixture.ledger, validation, inputs)

    def bind(self):
        value = self.verifier.binding(self.record, fixtures.SHA, self.frozen, fixtures.BASE)
        subject = admission.subject(self.config['repository'], 7, fixtures.SHA, fixtures.BASE, fixtures.TREE)
        pin = admission.pins.pin(self.config['pins'], 10 ** 9, self.record, subject, 'local/fixture', 3600,
                                 {'kind': 'local-publication-binding', 'binding': value})
        with admission.pins.holding(self.config['pins'], pin['pin_id'], self.record['validation_id']) as root:
            admission.pins.confirm(root, pin['pin_id'], self.record['validation_id'])
        return pin

    def check(self, sha=fixtures.SHA, base=fixtures.BASE):
        admission.check_verification_inputs(self.config, self.fixture.approval, self.record, sha, base, 7,
                                           [fixtures.PARENT])

    def test_exact_head_base_needs_live_independent_binding_and_original_payloads(self):
        with self.assertRaisesRegex(ValueError, 'live independent'):
            self.check()
        bound = self.bind()
        self.check()
        with self.assertRaisesRegex(ValueError, 'live independent'):
            self.check(base='d' * 40)
        admission.pins.release(self.config['pins'], {bound['pin_id']})
        with self.assertRaisesRegex(ValueError, 'live independent'):
            self.check()
        Path(self.evidence['requests.json']).unlink()
        with self.assertRaisesRegex(ValueError, 'regular verification input'):
            self.check()

    def test_dependency_and_contract_changes_are_rejected_before_success(self):
        self.bind()
        self.dependency.write_text('changed without lockfile change')
        with self.assertRaisesRegex(ValueError, 'dependency inputs changed'):
            self.check()
        self.dependency.write_text('actual dependency v1')
        self.contract['max_age_seconds'] = 7200
        self.configure()
        with self.assertRaisesRegex(ValueError, 'old or incompatible'):
            self.check()

    def test_sensitive_reads_rerun_and_unknown_remote_metadata_fails_closed(self):
        self.contract['git_reads'][0]['classification'] = 'input'
        self.configure()
        self.frozen['contract'] = self.verifier.digest(self.contract)
        self.frozen['git_inputs'] = {self.fixture.runtime + ':head': (fixtures.SHA + '\n').encode().hex()}
        self.record = self.record_pass()
        self.bind()
        self.check()
        with self.assertRaisesRegex(ValueError, 'commit-sensitive'):
            self.check(sha='d' * 40)
        self.frozen['git_inputs'] = {self.fixture.runtime + ':tracked': b'source\0'.hex(),
                                   self.fixture.runtime + ':status': b''.hex()}
        admission.check_git_inputs(self.record, self.frozen, 'd' * 40, [])
        self.frozen['git_inputs'] = {'unreviewed:future-read': '00'}
        with self.assertRaisesRegex(ValueError, 'commit-sensitive'):
            admission.check_git_inputs(self.record, self.frozen, fixtures.SHA, [])


if __name__ == '__main__':
    unittest.main()
