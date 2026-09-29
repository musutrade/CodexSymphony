import hashlib
import json
from pathlib import Path
import tempfile
import time
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from ci_policy import SupersededRun
import evidence_admission as admission
import host

ledger = admission.ledger
SHA, BASE, TREE, PARENT, MERGE = 'a' * 40, 'b' * 40, 'c' * 40, '9' * 40, '8' * 40
pins = admission.pins
RUNTIME = b"reviewed\ncommand = ['verify', '--all']\n"


class GitHub:
    """Route table standing in for the REST API; records every write."""

    def __init__(self, sha=SHA, tree=TREE, base=BASE):
        self.head = sha
        self.routes = {
            '/git/commits/' + sha: {'sha': sha, 'tree': {'sha': tree}, 'parents': [{'sha': PARENT}]},
            '/git/ref/heads/main': {'object': {'type': 'commit', 'sha': base}},
            '/compare/' + base + '...' + sha: {'status': 'ahead', 'behind_by': 0},
            '/compare/' + sha + '...' + base: {'status': 'ahead', 'behind_by': 0},
            '/pulls/7': {'state': 'open', 'number': 7, 'head': {'sha': sha}, 'base': {'ref': 'main'}},
            '/commits/' + sha + '/pulls': [{'number': 7, 'state': 'open', 'head': {'sha': sha}}],
        }
        self.writes = []
        self.external = '12/1'

    def __call__(self, path, token, method='GET', body=None):
        suffix = path.removeprefix('/repos/owner/repo')
        if method != 'GET':
            self.writes.append((method, suffix, body))
            if (method, suffix) == ('POST', '/check-runs'):
                self.external, self.head = body['external_id'], body['head_sha']
            return {'id': 9}
        if suffix.startswith('/check-runs/') and suffix not in self.routes:
            # A check run reads back as its last PATCH.
            body = [body for method, path, body in self.writes if path == suffix][-1]
            return {'head_sha': body.get('head_sha', self.head), 'external_id': self.external,
                    'status': body['status'], 'conclusion': body['conclusion']}
        if suffix not in self.routes:
            raise AssertionError('unexpected GitHub read: ' + suffix)
        value = self.routes[suffix]
        if isinstance(value, Exception):
            raise value
        # A callable route reads back state that changes between reads.
        return value() if callable(value) else value


class AdmissionTest(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name).resolve()
        self.home = self.base / 'remote'
        (self.home / 'jobs/12-1').mkdir(parents=True)
        self.ledger = self.base / 'ledger'
        runtime = self.base / 'runtime.py'
        runtime.write_bytes(RUNTIME)
        self.runtime = str(runtime)
        self.approval = {'execution_version': 3, 'runtime_files': {self.runtime: hashlib.sha256(RUNTIME).hexdigest()},
                         'trusted_files': {'tools/gate.py': 't' * 64},
                         'config_files': {'.harness-gate/quality.toml': 'q' * 64},
                         'baseline': {'path': '/baselines/b', 'sha256': 's' * 64, 'commit': BASE}}
        self.read = {'path': 'run.py', 'reader': self.reference(), 'read': 'git rev-parse HEAD',
                     'use': 'evidence label', 'verdict': admission.LABEL_ONLY}
        self.approval_path = self.base / 'approval.json'
        self.approval_path.write_text(json.dumps(self.approval))
        self.gate = self.base / 'evidence/gate'
        self.gate.mkdir(parents=True)
        self.pins = self.base / 'evidence/pins'
        self.pins.mkdir()
        (self.pins / 'lock').touch()
        self.config = {'repository': 'owner/repo', 'gate_approval': str(self.approval_path),
                       'publication_ledger': str(self.ledger), 'mode': 'verify-only', 'pins': str(self.pins),
                       'storage_deployment': str(self.deployment(10 ** 9)),
                       'audit_window_seconds': 3600, 'pin_ttl_seconds': 7200}
        self.run = {'id': 12, 'run_attempt': 1, 'event': 'pull_request', 'head_sha': SHA,
                    'pull_requests': [{'number': 7}]}
        self.github = GitHub()
        for target, value in [('request', self.github), ('installation_token', lambda config: 'token')]:
            item = patch.object(admission, target, value)
            item.start()
            self.addCleanup(item.stop)

    def reference(self, source='runtime_files', name=None):
        """An approved file as the audit cites it."""
        name = name or self.runtime
        return {'source': source, 'name': name, 'digest': self.approval[source][name]}

    def deployment(self, budget):
        release = self.base / 'storage'
        release.mkdir(exist_ok=True)
        entry = release / 'compact_gate_evidence.py'
        entry.write_text(f'def maintain(budget={budget}):\n    pass\n')
        path = self.base / 'storage-deployment.json'
        path.write_text(json.dumps({'release': str(release), 'files': {str(entry): pins.file_digest(entry)}}))
        return path

    def inputs(self, commit=SHA, tree=TREE, environment='e' * 64, approval=None):
        return {'approval': approval or admission.approval_identity(self.approval), 'commit': commit,
                'environment': environment, 'tree': tree}

    def evidence(self, commit=SHA, environment='e' * 64, report=None, omitted=()):
        """The retained run evidence, as the v3 host keeps it; omitted names are left out."""
        run = self.gate / ('run-' + commit[:6] + environment[:6])
        run.mkdir(exist_ok=True)
        (run / 'test_result.json').write_text(json.dumps(report or {
            'passed': True, 'evidence_complete': True, 'source_identity': 'working-tree:' + commit}))
        (run / 'environment.json').write_text(json.dumps({'fingerprint': environment}))
        (run / 'source-inputs.json').write_text(json.dumps({'files': {'README.md': 'f' * 64}}))
        (run / 'source-archive.json').write_text(json.dumps({'sha256': 'a' * 64}))
        names = ('environment.json', *admission.SELECTION_EVIDENCE)
        return {'report': str(run / 'test_result.json')} | {name: str(run / name) for name in names if name not in omitted}

    def passed(self, details=None, audited=True, omitted=(), **changes):
        """A ledger PASS; unless audited is False, its candidate's reviewed audit is installed."""
        value = self.inputs(**changes)
        attempt = ledger.begin(self.ledger, value)
        evidence = self.evidence(value['commit'], value['environment'], omitted=omitted)
        run = str(Path(evidence['report']).parent)
        identity = ledger.record_pass(self.ledger, attempt, value, evidence,
                                      {'parents': [PARENT], 'run': run} if details is None else details | {'run': run})
        if audited:
            self.install_audit(identity)
        return identity

    def install_audit(self, reviewed, **changes):
        """The reviewed audit of exactly this ledger PASS; changes mutate any field."""
        record = ledger.load_record(self.ledger, reviewed)
        audit = {'schema': admission.AUDIT_SCHEMA, 'rule': admission.TREE_EQUIVALENCE,
                 'conditions': list(admission.CONDITIONS), 'approval': admission.approval_identity(self.approval),
                 'runtime_files': self.approval['runtime_files'], 'tree': record['inputs']['tree'],
                 'validation_id': reviewed, 'environment': record['inputs']['environment'],
                 'environment_evidence_sha256': record['evidence']['environment.json']['sha256'],
                 'git_metadata_reads': [self.read]} | changes
        path = self.base / 'equivalence-audit.json'
        path.write_text(json.dumps(audit))
        self.config['equivalence'] = {'rule': admission.TREE_EQUIVALENCE, 'audit': str(path),
                                      'audit_sha256': hashlib.sha256(path.read_bytes()).hexdigest()}
        return path

    def publish(self, superseded=lambda: False, identity='12/1'):
        self.github.external, self.github.head = identity, self.run['head_sha']
        (self.home / 'jobs' / identity.replace('/', '-')).mkdir(parents=True, exist_ok=True)
        return admission.publish(self.run, self.config, self.home, identity, 9, superseded)

    def conclusion(self):
        method, path, body = self.github.writes[-1]
        self.assertEqual((method, path), ('PATCH', '/check-runs/9'))
        return body['conclusion'], body['output']['summary']

    def title(self):
        return self.github.writes[-1][2]['output']['title']

    def rejected(self, pattern):
        result = self.publish()
        self.assertEqual(result['status'], 'REJECTED')
        self.assertRegex(result['error'], pattern)
        conclusion, text = self.conclusion()
        self.assertEqual(conclusion, 'failure')
        self.assertIn('No tests or Gate were run, and none starts automatically', text)
        failure = json.loads((self.home / 'jobs/12-1/failure.json').read_text())
        self.assertEqual((failure['status'], failure['category']), ('REJECTED', result['category']))
        return result

    def expired(self, pattern, identity='12/1'):
        """Expired retention is its own category: no quality verdict, no Gate, and a keep-or-revalidate recovery."""
        before = (self.ledger / 'events.jsonl').read_bytes()
        result = self.publish(identity=identity)
        self.assertRegex(result['error'], pattern)
        self.assertEqual((result['status'], result['category']), ('REJECTED', admission.RETENTION_EXPIRED))
        conclusion, text = self.conclusion()
        self.assertEqual((conclusion, self.title()), ('failure', 'Evidence retention expired; no Gate started'))
        self.assertIn('none starts automatically', text)
        self.assertIn('not a quality failure', text)
        self.assertIn('first check whether the local PASS still stands', text)
        failure = json.loads((self.home / 'jobs' / identity.replace('/', '-') / 'failure.json').read_text())
        self.assertEqual(failure['category'], admission.RETENTION_EXPIRED)
        self.assertEqual((self.ledger / 'events.jsonl').read_bytes(), before)
        return result


class SuccessTest(AdmissionTest):
    def test_exact_pass_is_attested_without_rewriting_the_original_record(self):
        identity = self.passed()
        record = self.ledger / 'records' / (identity + '.json')
        before = (record.read_bytes(), (self.ledger / 'events.jsonl').read_bytes())
        result = self.publish()
        self.assertEqual((result['status'], result['rule'], result['validation_id']), ('PASS', admission.EXACT, identity))
        self.assertEqual((result['base'], result['pull_request'], result['full_suite_executed']), (BASE, 7, False))
        self.assertEqual(result['audit_sha256'], self.config['equivalence']['audit_sha256'])
        conclusion, text = self.conclusion()
        self.assertEqual(conclusion, 'success')
        self.assertIn('did not run tests or a Gate', text)
        self.assertEqual((record.read_bytes(), (self.ledger / 'events.jsonl').read_bytes()), before)
        pin = pins.current(self.pins)[result['pin_id']]
        self.assertEqual(pins.digest(pins.canonical(pin['publication'])), result['publication_id'])
        self.assertEqual((pin['publication']['actions_attempt'], pin['publication']['status'], pin['attempt']),
                         ('12/1', 'confirmed', '12/1'))
        self.assertEqual(pin['subject'], admission.subject('owner/repo', 7, SHA, BASE, TREE))
        self.assertEqual(pin['expires_at_ms'] - pin['created_at_ms'], 7200 * 1000)
        self.assertFalse(any(path.name != 'ledger-anchor.json' and path.name != 'jobs' for path in self.home.iterdir()))

    def test_each_attempt_gets_a_new_attestation_of_the_same_validation(self):
        self.passed()
        first = self.publish()
        self.run['run_attempt'] = 2
        second = self.publish(identity='12/2')
        self.assertEqual(first['validation_id'], second['validation_id'])
        self.assertNotEqual(first['publication_id'], second['publication_id'])

    def test_main_push_and_workflow_dispatch_only_verify(self):
        self.passed()
        for event in ['push', 'workflow_dispatch']:
            with self.subTest(event=event):
                self.run = {'id': 12, 'run_attempt': 1, 'event': event, 'head_sha': SHA, 'pull_requests': []}
                result = self.publish()
                self.assertEqual((result['status'], result['pull_request']), ('PASS', None))
                pin = pins.current(self.pins)[result['pin_id']]
                self.assertEqual(pin['expires_at_ms'] - pin['created_at_ms'], 3600 * 1000)
                self.assertIsNone(pin['subject']['pull_request'])

    def test_publication_happens_under_the_ledger_shared_lock(self):
        self.passed()
        observed = []

        def probe(*args):
            # A writer (FAIL or revocation) must not be able to land while the check is published.
            descriptor = ledger.os.open(self.ledger / 'lock', ledger.os.O_RDWR)
            try:
                ledger.fcntl.flock(descriptor, ledger.fcntl.LOCK_EX | ledger.fcntl.LOCK_NB)
                observed.append('free')
            except BlockingIOError:
                observed.append('held')
            finally:
                ledger.os.close(descriptor)
            original(*args)
        original = admission.complete
        with patch.object(admission, 'complete', side_effect=probe):
            self.assertEqual(self.publish()['status'], 'PASS')
        self.assertEqual(observed, ['held'])

class RejectionTest(AdmissionTest):
    def test_missing_evidence_is_rejected_without_a_ledger_write(self):
        self.rejected('ledger has no records')
        self.passed(tree='d' * 40)
        before = (self.ledger / 'events.jsonl').read_bytes()
        result = self.rejected('no complete local PASS')
        self.assertEqual(result['category'], 'missing')
        self.assertEqual((self.ledger / 'events.jsonl').read_bytes(), before)

    def test_newest_blocked_pass_is_not_skipped_for_an_older_one(self):
        self.passed()
        ledger.record_fail(self.ledger, ledger.begin(self.ledger, self.inputs()), self.inputs(), {}, {})
        self.assertEqual(self.rejected('supersedes')['category'], 'blocked')
        newer = self.passed()
        ledger.revoke(self.ledger, newer, 'withdrawn')
        self.rejected('supersedes')
        ledger.begin(self.ledger, self.inputs())
        self.passed()
        self.rejected('has not concluded')

    def test_other_approval_or_changed_runtime_is_rejected(self):
        self.passed(approval='f' * 64)
        self.rejected('no complete local PASS')
        self.passed()
        Path(next(iter(self.approval['runtime_files']))).write_text('changed')
        self.rejected('approved runtime changed')

    def test_non_v3_host_is_rejected(self):
        self.passed()
        self.approval_path.write_text(json.dumps(self.approval | {'execution_version': 2}))
        self.rejected('requires the v3 host')

    def test_retained_environment_or_report_must_back_the_pass(self):
        identity = self.passed()
        record = ledger.verify(self.ledger, identity, self.inputs())
        for name, value, pattern in [
                ('environment.json', {'fingerprint': 'other'}, 'retained environment'),
                ('report', {'passed': True, 'evidence_complete': False}, 'incomplete'),
                ('report', {'passed': True, 'evidence_complete': True, 'source_identity': 'working-tree:' + 'd' * 40},
                 'another commit')]:
            with self.subTest(pattern=pattern):
                path = self.base / 'swap.json'
                path.write_text(json.dumps(value))
                swapped = json.loads(json.dumps(record))
                swapped['evidence'][name]['path'] = str(path)
                with patch.object(admission.ledger, 'verify', return_value=swapped):
                    self.rejected(pattern)

    def test_tampered_evidence_is_rejected(self):
        identity = self.passed()
        Path(ledger.load_record(self.ledger, identity)['evidence']['report']['path']).write_text('{}')
        self.assertEqual(self.rejected('evidence changed')['category'], 'tampered')

    def test_other_commit_needs_the_installed_equivalence_rule(self):
        identity = self.passed(commit='d' * 40, audited=False)
        self.rejected('no reviewed equivalence rule')
        self.config['equivalence'] = {'rule': 'anything'}
        self.rejected('no reviewed equivalence rule')
        self.install_audit(identity)
        result = self.publish()
        self.assertEqual((result['status'], result['rule'], result['validated_commit'], result['parents']),
                         ('PASS', admission.TREE_EQUIVALENCE, 'd' * 40, [PARENT]))
        self.assertEqual(result['audit_sha256'], self.config['equivalence']['audit_sha256'])

    def test_equivalence_audit_must_be_the_reviewed_document_for_this_approval(self):
        identity = self.passed(commit='d' * 40)
        path = self.install_audit(identity)
        path.write_text(path.read_text() + ' ')
        self.rejected('audit changed')
        link = self.base / 'audit-link.json'
        link.symlink_to(self.install_audit(identity))
        self.config['equivalence']['audit'] = str(link)
        self.rejected('audit changed')
        for changes, pattern in [({'schema': 'codexsymphony-equivalence-audit/v1'}, 'enforced conditions'),
                                 ({'rule': admission.EXACT}, 'enforced conditions'),
                                 ({'conditions': list(admission.CONDITIONS[:-1])}, 'enforced conditions'),
                                 ({'approval': 'f' * 64}, 'not reviewed for this approval'),
                                 ({'runtime_files': {}}, 'its runtime tools'),
                                 ({'extra': 1}, 'fields differ'),
                                 ({'git_metadata_reads': []}, 'no reviewed Git metadata reads'),
                                 ({'git_metadata_reads': {'run.py': 'label'}}, 'no reviewed Git metadata reads'),
                                 ({'git_metadata_reads': ['run.py']}, 'malformed Git metadata read'),
                                 ({'git_metadata_reads': [self.read | {'verdict': ''}]},
                                  'unknown Git metadata read classification: run.py'),
                                 ({'git_metadata_reads': [{'path': 'run.py', 'read': 'git rev-parse HEAD'}]},
                                  'malformed Git metadata read'),
                                 ({'git_metadata_reads': [self.read, self.read | {'path': 'build.rs', 'verdict': 'affects-result'}]},
                                  'unknown Git metadata read classification: build.rs')]:
            with self.subTest(changes=changes):
                self.install_audit(identity, **changes)
                self.rejected(pattern)
        self.install_audit(identity)
        audit = json.loads(Path(self.config['equivalence']['audit']).read_text())
        del audit['environment']
        Path(self.config['equivalence']['audit']).write_text(json.dumps(audit))
        self.config['equivalence']['audit_sha256'] = ledger.file_digest(Path(self.config['equivalence']['audit']))
        self.rejected('fields differ')

    def test_equivalence_audit_binds_exactly_one_candidate(self):
        identity = self.passed(commit='d' * 40)
        for changes, name in [({'tree': 'f' * 40}, 'tree'), ({'tree': [TREE]}, 'tree'), ({'tree': '*'}, 'tree'),
                              ({'validation_id': 'f' * 64}, 'validation_id'), ({'validation_id': [identity]}, 'validation_id'),
                              ({'environment': 'f' * 64}, 'environment'),
                              ({'environment_evidence_sha256': 'f' * 64}, 'environment_evidence_sha256')]:
            with self.subTest(changes=changes):
                self.install_audit(identity, **changes)
                self.assertEqual(self.rejected('binds another ' + name + '$')['category'], 'rejected')
        # A later PASS of the same tree is a new candidate; the earlier audit does not cover it.
        self.install_audit(identity)
        self.passed(commit='d' * 40, audited=False)
        self.rejected('binds another validation_id')

    def test_parents_are_only_an_audit_label(self):
        self.install_audit(self.passed(commit='d' * 40, details={'parents': ['e' * 40]}))
        result = self.publish()
        self.assertEqual((result['status'], result['rule'], result['parents']), ('PASS', admission.TREE_EQUIVALENCE, [PARENT]))
        self.assertNotIn('same-parents', admission.CONDITIONS)

    def test_invalid_remote_parent_is_rejected(self):
        self.passed()
        self.github.routes['/git/commits/' + SHA]['parents'] = [{'sha': 'bad'}]
        self.rejected('invalid parent')

    def test_published_head_is_anchored_and_a_rolled_back_ledger_is_tampered(self):
        self.passed()
        anchor = self.home / 'ledger-anchor.json'
        self.assertFalse(anchor.exists())
        result = self.publish()
        self.assertEqual(json.loads(anchor.read_text()), result['ledger_head'])
        self.assertEqual(result['ledger_head']['sequence'], 2)
        self.passed()
        self.assertEqual(self.publish()['ledger_head']['sequence'], 4)
        events = self.ledger / 'events.jsonl'
        kept = events.read_bytes().splitlines(keepends=True)
        events.write_bytes(b''.join(kept[:2]))
        self.assertEqual(self.rejected('shorter than the anchored head')['category'], 'tampered')
        events.write_bytes(b''.join(kept))
        anchor.write_text(json.dumps({'sequence': 4, 'digest': '0' * 64}))
        self.assertEqual(self.rejected('diverges')['category'], 'tampered')

    def test_rejection_does_not_advance_the_anchor(self):
        self.passed()
        self.github.routes['/pulls/7'] = {'state': 'closed', 'head': {'sha': SHA}, 'base': {'ref': 'main'}}
        self.rejected('head or base changed')
        self.assertFalse((self.home / 'ledger-anchor.json').exists())

    def test_head_base_and_pull_request_binding(self):
        self.passed()
        routes = self.github.routes
        cases = [
            ('/pulls/7', {'state': 'closed', 'head': {'sha': SHA}, 'base': {'ref': 'main'}}, 'head or base changed'),
            ('/pulls/7', {'state': 'open', 'head': {'sha': 'd' * 40}, 'base': {'ref': 'main'}}, 'head or base changed'),
            ('/pulls/7', {'state': 'open', 'head': {'sha': SHA}, 'base': {'ref': 'dev'}}, 'head or base changed'),
            ('/compare/' + BASE + '...' + SHA, {'status': 'diverged', 'behind_by': 2}, 'latest main'),
            ('/git/ref/heads/main', {'object': {'type': 'tag', 'sha': BASE}}, 'does not name a commit'),
            ('/git/ref/heads/main', {'object': {'type': 'commit', 'sha': 'main'}}, 'invalid main commit'),
            ('/git/commits/' + SHA, {'sha': 'd' * 40, 'tree': {'sha': TREE}}, 'another commit'),
            ('/git/commits/' + SHA, {'sha': SHA, 'tree': {}}, 'invalid tree')]
        for path, value, pattern in cases:
            with self.subTest(path=path, pattern=pattern):
                original = routes[path]
                routes[path] = value
                self.rejected(pattern)
                routes[path] = original
        for listed in ([], [{'number': 7}, {'number': 8}]):
            with self.subTest(listed=listed):
                self.run['pull_requests'] = listed
                self.rejected('exactly one pull request')
        self.run.update(event='push', pull_requests=[])
        routes['/compare/' + SHA + '...' + BASE] = {'status': 'behind', 'behind_by': 1}
        self.rejected('not on main')

    def test_base_moving_during_verification_or_superseded_attempt_is_rejected(self):
        self.passed()
        routes = self.github.routes
        reads = []
        original = self.github

        def moving(path, token, method='GET', body=None):
            if path.endswith('/git/ref/heads/main') and method == 'GET':
                reads.append(path)
                if len(reads) == 2:
                    return {'object': {'type': 'commit', 'sha': 'e' * 40}}
            return original(path, token, method, body)
        routes['/compare/' + 'e' * 40 + '...' + SHA] = {'status': 'ahead', 'behind_by': 0}
        with patch.object(admission, 'request', moving):
            self.rejected('changed during verification')
        self.rejected_superseded()

    def rejected_superseded(self):
        result = self.publish(superseded=lambda: True)
        self.assertRegex(result['error'], 'superseded')
        self.assertEqual(self.conclusion()[0], 'failure')

    def test_transport_failure_is_retryable_and_never_recorded(self):
        identity = self.passed()
        before = (self.ledger / 'events.jsonl').read_bytes()
        self.github.routes['/git/ref/heads/main'] = OSError('network down')
        self.assertEqual(self.rejected('OSError: network down')['category'], admission.TRANSPORT)
        self.assertEqual(self.title(), 'GitHub transport failed; evidence not judged, no Gate started')
        self.assertIn('never requires a new local validation', self.conclusion()[1])
        self.assertEqual((self.ledger / 'events.jsonl').read_bytes(), before)
        self.github.routes['/git/ref/heads/main'] = admission.http.client.RemoteDisconnected('closed')
        self.assertEqual(self.rejected('RemoteDisconnected: closed')['category'], admission.TRANSPORT)
        with patch.object(admission, 'installation_token', side_effect=OSError('token endpoint down')), \
                self.assertRaisesRegex(admission.Rejected, 'OSError: token endpoint down') as caught:
            admission.token_of(self.config)
        self.assertEqual(caught.exception.category, admission.TRANSPORT)
        self.github.routes['/git/ref/heads/main'] = {'object': {'type': 'commit', 'sha': BASE}}
        self.assertEqual(self.publish()['validation_id'], identity)


class HostTest(AdmissionTest):
    def host_run(self, **changes):
        return self.run | {'repository': {'full_name': 'owner/repo'}, 'head_repository': {'full_name': 'owner/repo'},
                           'path': '.github/workflows/quality.yml', 'html_url': 'https://example.invalid/run'} | changes

    def attempt(self, run, **changes):
        """The Actions run as GitHub reads it back now: by default, this exact attempt, unfinished."""
        return {'id': run['id'], 'head_sha': run['head_sha'], 'event': run['event'], 'run_attempt': run['run_attempt'],
                'status': 'in_progress'} | changes

    def process(self, run, evaluate=None):
        evaluate = evaluate or {'side_effect': AssertionError('Gate must not run')}
        self.github.routes.setdefault('/actions/runs/' + str(run['id']), self.attempt(run))
        with patch.object(host, 'request', self.github), patch.object(host, 'installation_token', lambda c: 'token'), \
                patch.object(host, 'actions_cancelled', return_value=False), \
                patch.object(host, 'evaluate', **evaluate) as called, patch('builtins.print'):
            host.process(run, self.config, self.home)
        return called

    def main_run(self, event):
        """A push, or a dispatch of a main commit, of the validated commit."""
        self.run.update(event=event, pull_requests=[])
        return self.host_run(head_branch='main')

    def test_verify_only_checks_every_event_attempt_before_verifying(self):
        self.passed()
        for event in ['push', 'workflow_dispatch']:
            for changes in [{'status': 'completed'}, {'status': 'completed', 'conclusion': 'cancelled'},
                            {'status': None}, {'run_attempt': 2}, {'head_sha': 'f' * 40}, {'event': 'pull_request'},
                            {'id': 99}]:
                with self.subTest(event=event, changes=changes):
                    run = self.main_run(event)
                    self.github.routes['/actions/runs/12'] = self.attempt(run, **changes)
                    before = len(self.github.writes)
                    self.process(run).assert_not_called()
                    # A superseded attempt gets no check run and no receipt, and is never verified.
                    self.assertEqual(len(self.github.writes), before)
                    self.assertFalse((self.home / 'jobs/12-1/receipt.json').exists())
        self.assertEqual(pins.current(self.pins), {})

    def test_verify_only_checks_the_attempt_again_immediately_before_publication(self):
        self.passed()
        for event in ['push', 'workflow_dispatch']:
            for changes in [{'status': 'completed'}, {'run_attempt': 2}]:
                with self.subTest(event=event, changes=changes):
                    run = self.main_run(event)
                    reads = []

                    def current(run=run, changes=changes):
                        # Current when the host picks it up; superseded by the time it would publish.
                        reads.append(1)
                        return self.attempt(run, **(changes if len(reads) > 1 else {}))
                    self.github.routes['/actions/runs/12'] = current
                    self.process(run).assert_not_called()
                    self.assertEqual(len(reads), 2)
                    self.assertEqual((self.receipt()['status'], self.receipt()['category']), ('REJECTED', 'rejected'))
                    self.assertRegex(json.loads((self.home / 'jobs/12-1/failure.json').read_text())['error'], 'superseded')
                    self.assertEqual(self.conclusion()[0], 'failure')
                    self.assertEqual(pins.current(self.pins), {})
                    (self.home / 'jobs/12-1/receipt.json').unlink()

    def test_verify_only_current_attempt_of_every_event_is_published(self):
        self.passed()
        for event, status in [('push', 'in_progress'), ('workflow_dispatch', 'queued')]:
            with self.subTest(event=event):
                run = self.main_run(event) | {'id': {'push': 13, 'workflow_dispatch': 14}[event]}
                self.github.routes['/actions/runs/' + str(run['id'])] = self.attempt(run, status=status)
                self.process(run).assert_not_called()
                receipt = self.receipt(str(run['id']) + '-1')
                self.assertEqual((receipt['status'], receipt['rule'], receipt['finished']), ('PASS', admission.EXACT, True))

    def test_execute_mode_keeps_its_pull_request_only_cancellation(self):
        self.config['mode'] = 'execute'
        run = self.main_run('push')
        with patch.object(host, 'attempt_superseded', side_effect=AssertionError('verify-only check')), \
                patch.object(host, 'actions_cancelled', return_value=True) as cancelled, \
                patch.object(host, 'evaluate') as evaluate:
            host.process(run, self.config, self.home)
        cancelled.assert_called_once_with(run, self.config)
        evaluate.assert_not_called()

    def receipt(self, identity='12-1'):
        return json.loads((self.home / 'jobs' / identity / 'receipt.json').read_text())

    def test_verify_only_deployment_never_evaluates_a_gate(self):
        self.passed()
        for event in ['pull_request', 'push', 'workflow_dispatch']:
            with self.subTest(event=event):
                current = self.host_run(event=event, head_branch='main',
                                        id={'pull_request': 12, 'push': 13, 'workflow_dispatch': 14}[event])
                if event != 'pull_request':
                    current['pull_requests'] = []
                identity = str(current['id']) + '-1'
                (self.home / 'jobs' / identity).mkdir(parents=True, exist_ok=True)
                self.process(current).assert_not_called()
                self.assertEqual((self.receipt(identity)['status'], self.receipt(identity)['finished']), ('PASS', True))

    def test_verify_only_rejection_finishes_the_receipt(self):
        self.process(self.host_run()).assert_not_called()
        self.assertEqual((self.receipt()['status'], self.receipt()['finished']), ('REJECTED', True))
        self.assertEqual(self.conclusion()[0], 'failure')

    def test_missing_or_unknown_mode_never_creates_a_check_or_runs_a_gate(self):
        for mode in [None, 'verify_only', 'gate']:
            with self.subTest(mode=mode):
                self.config.pop('mode', None)
                if mode:
                    self.config['mode'] = mode
                with self.assertRaisesRegex(ValueError, 'mode missing or unknown'):
                    self.process(self.host_run())
                self.assertEqual(self.github.writes, [])
                self.assertFalse((self.home / 'jobs/12-1/receipt.json').exists())

    def test_existing_receipt_is_never_reverified_or_turned_into_success(self):
        receipt = self.home / 'jobs/12-1/receipt.json'
        receipt.write_text(json.dumps({'identity': '12/1', 'check_id': 5, 'finished': True, 'status': 'PASS'}))
        self.process(self.host_run()).assert_not_called()
        self.assertEqual(self.github.writes, [])
        receipt.write_text(json.dumps({'identity': '12/1', 'check_id': 5, 'finished': False}))
        self.process(self.host_run()).assert_not_called()
        method, path, body = self.github.writes[-1]
        self.assertEqual((method, path, body['conclusion'], len(self.github.writes)), ('PATCH', '/check-runs/5', 'failure', 1))
        self.assertEqual((self.receipt()['status'], self.receipt()['finished']), ('interrupted', True))

    def test_pins_are_settled_only_by_verify_only_deployments(self):
        with patch.object(admission, 'sweep') as sweep:
            host.settle_pins(self.config | {'mode': 'execute'}, 'token')
            sweep.assert_not_called()
            host.settle_pins(self.config, 'token')
            sweep.assert_called_once_with(self.config, 'token')

    def test_execute_mode_publishes_each_evaluated_result(self):
        self.config['mode'] = 'execute'
        results = [
            ({'return_value': {'scope': 'full', 'records': 3, 'producers': 2, 'report_sha256': 'r' * 64,
                               'run': '/retained', 'status': 'PASS'}}, 'success', 'Complete isolated gate passed', 'PASS'),
            ({'return_value': {'scope': 'documentation', 'baseline_sha': BASE, 'baseline_identity': '1/1',
                               'changed_paths': ['README.md'], 'report_sha256': 'r' * 64, 'status': 'PASS'}},
             'success', 'Documentation checks passed', 'PASS'),
            ({'return_value': {'scope': 'identical-tree', 'tree': TREE, 'baseline_sha': BASE, 'baseline_identity': '1/1',
                               'baseline_report_sha256': 'b' * 64, 'report_sha256': 'r' * 64, 'status': 'PASS'}},
             'success', 'Post-merge tree', 'PASS'),
            ({'side_effect': SupersededRun('stopped')}, 'cancelled', 'Superseded', 'CANCELLED'),
            ({'side_effect': RuntimeError('gate broke')}, 'failure', 'Trusted host rejected', 'FAIL')]
        for attempt, (evaluate, conclusion, title, status) in enumerate(results, 1):
            with self.subTest(status=status, title=title):
                self.run['run_attempt'] = attempt
                self.process(self.host_run(run_attempt=attempt), evaluate).assert_called_once()
                method, path, body = self.github.writes[-1]
                self.assertEqual((method, path, body['conclusion']), ('PATCH', '/check-runs/9', conclusion))
                self.assertIn(title, body['output']['title'])
                receipt = self.receipt('12-' + str(attempt))
                self.assertEqual((receipt['status'], receipt['finished']), (status, True))
        failure = json.loads((self.home / 'jobs/12-5/failure.json').read_text())
        self.assertEqual(failure, {'status': 'FAIL', 'error': 'gate broke'})


class ExactPullRequestTest(AdmissionTest):
    """An exact PR head lands on main as another commit: its proof needs the candidate's audit too."""

    def test_exact_pull_request_without_its_audit_is_rejected(self):
        self.passed(audited=False)
        self.assertEqual(self.rejected('no reviewed equivalence rule')['category'], 'rejected')
        self.assertEqual(pins.current(self.pins), {})
        self.github.routes['/compare/' + SHA + '...' + BASE] = {'status': 'diverged', 'behind_by': 1}
        self.run.update(event='workflow_dispatch', pull_requests=[])
        self.rejected('no reviewed equivalence rule')

    def test_exact_pull_request_with_an_invalid_audit_is_rejected(self):
        identity = self.passed()
        for changes, pattern in [({'tree': 'f' * 40}, 'binds another tree'),
                                 ({'validation_id': 'f' * 64}, 'binds another validation_id'),
                                 ({'schema': 'codexsymphony-equivalence-audit/v2'}, 'enforced conditions'),
                                 ({'git_metadata_reads': [self.read | {'verdict': 'affects-result'}]},
                                  'unknown Git metadata read classification')]:
            with self.subTest(changes=changes):
                self.install_audit(identity, **changes)
                self.rejected(pattern)
        path = self.install_audit(identity)
        path.write_text(path.read_text() + ' ')
        self.rejected('audit changed')

    def test_exact_pull_request_with_its_valid_audit_passes(self):
        self.passed()
        result = self.publish()
        self.assertEqual((result['status'], result['rule'], result['pull_request']), ('PASS', admission.EXACT, 7))
        self.assertEqual(result['audit_sha256'], self.config['equivalence']['audit_sha256'])

    def test_exact_main_commit_stays_exact_without_an_audit(self):
        self.passed(audited=False)
        self.run.update(event='push', pull_requests=[])
        result = self.publish()
        self.assertEqual((result['status'], result['rule'], result['pull_request']), ('PASS', admission.EXACT, None))
        self.assertNotIn('audit_sha256', result)

    def test_rejected_exact_pull_request_leaves_its_merge_without_a_confirmed_proof(self):
        identity = self.passed(audited=False)
        self.rejected('no reviewed equivalence rule')
        MergeSourceTest.merged(self)
        self.install_audit(identity)
        self.assertEqual(self.rejected('no live confirmed pull request publication')['category'], 'missing')


class ClassificationTest(AdmissionTest):
    """v3 read classifications: every reference must name an approved value; semantics stay the reviewer's."""

    def setUp(self):
        super().setUp()
        self.selection = self.read | {'path': 'secrets/mod.rs', 'verdict': admission.TREE_FILE_SELECTION,
                                      'guards': [self.reference(), self.reference('trusted_files', 'tools/gate.py')],
                                      'retained': [*admission.SELECTION_EVIDENCE, 'environment.json']}
        self.fixed = self.read | {'path': 'run.py:120', 'verdict': admission.FIXED_APPROVED_INPUT,
                                  'inputs': [{'source': 'baseline', 'name': 'commit', 'value': BASE},
                                             {'source': 'baseline', 'name': 'sha256', 'value': 's' * 64}]}
        self.skipped = self.read | {'path': 'scope/detection.rs', 'verdict': admission.NOT_EXECUTED,
                                    'chain': [self.link(), self.reference('config_files', '.harness-gate/quality.toml')
                                              | {'line': 9, 'text': 'none'}]}

    def link(self, **changes):
        return self.reference() | {'line': 2, 'text': "'--all'"} | changes

    def check(self, entry, pattern, approval=None):
        with self.assertRaisesRegex(admission.Rejected, pattern):
            admission.check_read(entry, approval or self.approval)

    def test_every_classification_with_its_references_is_admitted(self):
        identity = self.passed(audited=False)
        self.install_audit(identity, git_metadata_reads=[self.read, self.selection, self.fixed, self.skipped])
        self.assertEqual(self.publish()['status'], 'PASS')

    def test_retained_evidence_must_be_in_the_admitted_record(self):
        for name in admission.SELECTION_EVIDENCE:
            with self.subTest(name=name):
                identity = self.passed(audited=False, omitted=(name,))
                self.install_audit(identity, git_metadata_reads=[self.selection])
                self.rejected('relies on evidence this record lacks: ' + name)
        identity = self.passed(audited=False)
        self.install_audit(identity, git_metadata_reads=[self.selection | {'retained': [
            *admission.SELECTION_EVIDENCE, 'requests.json']}])
        self.rejected('relies on evidence this record lacks: requests.json')

    def test_file_selection_must_retain_the_source_snapshot_and_archive(self):
        for name in admission.SELECTION_EVIDENCE:
            with self.subTest(name=name):
                retained = [other for other in self.selection['retained'] if other != name]
                self.check(self.selection | {'retained': retained}, 'must retain source-inputs.json and source-archive.json')
        self.check(self.selection | {'retained': ['environment.json']}, 'must retain')

    def test_fields_are_exactly_those_of_the_classification(self):
        cases = [self.read | {'chain': [self.link()]}, self.selection | {'inputs': self.fixed['inputs']},
                 {key: value for key, value in self.selection.items() if key != 'retained'},
                 {key: value for key, value in self.read.items() if key != 'reader'},
                 self.read | {'use': ''}, self.read | {'path': ''}, self.read | {'verdict': 1}, 'run.py']
        for entry in cases:
            with self.subTest(entry=entry):
                self.check(entry, 'malformed Git metadata read')

    def test_reader_and_guards_are_approved_executed_files(self):
        wrong = [{'source': 'config_files', 'name': '.harness-gate/quality.toml', 'digest': 'q' * 64},
                 {'source': 'baseline', 'name': 'sha256', 'digest': 's' * 64},
                 self.reference() | {'name': '/absent'}, self.reference() | {'digest': 'f' * 64}]
        for reference in wrong:
            with self.subTest(reference=reference):
                self.check(self.read | {'reader': reference}, 'binding does not match: run.py')
                self.check(self.selection | {'guards': [reference]}, 'binding does not match: secrets/mod.rs')
        for reference in [self.reference() | {'extra': 1}, self.reference() | {'digest': ''}, None]:
            with self.subTest(reference=reference):
                self.check(self.read | {'reader': reference}, 'malformed')
        for guards in [[], {}, None]:
            with self.subTest(guards=guards):
                self.check(self.selection | {'guards': guards}, 'malformed')

    def test_retained_names_are_a_non_empty_list_of_names(self):
        for retained in [[], [''], [1], 'environment.json']:
            with self.subTest(retained=retained):
                self.check(self.selection | {'retained': retained}, 'malformed')

    def test_fixed_inputs_are_the_approved_baseline_values(self):
        for item, pattern in [({'source': 'baseline', 'name': 'commit', 'value': 'f' * 40}, 'binding does not match'),
                              ({'source': 'baseline', 'name': 'ref', 'value': BASE}, 'binding does not match'),
                              ({'source': 'config_files', 'name': 'commit', 'value': BASE}, 'binding does not match'),
                              ({'source': 'baseline', 'name': 'commit', 'value': ''}, 'malformed'),
                              ({'source': 'baseline', 'name': 'commit'}, 'malformed'),
                              ({'source': 'baseline', 'name': 'commit', 'value': BASE, 'x': 1}, 'malformed')]:
            with self.subTest(item=item):
                self.check(self.fixed | {'inputs': [item]}, pattern)
        self.check(self.fixed | {'inputs': []}, 'malformed')
        self.check(self.fixed, 'binding does not match: run.py:120', self.approval | {'baseline': None})

    def test_chain_links_are_cited_lines_of_approved_files(self):
        for link, pattern in [(self.link(line=True), 'malformed'), (self.link(line='2'), 'malformed'),
                              (self.link(line=0), 'malformed'), (self.link(text=''), 'malformed'),
                              (self.link(extra=1), 'malformed'), (self.link(line=3), 'out of range'),
                              (self.link(line=1), 'does not contain its text'),
                              (self.link(digest='f' * 64), 'binding does not match'),
                              (self.link(source='baseline'), 'binding does not match'),
                              (self.reference('trusted_files', 'tools/gate.py') | {'line': 1, 'text': 'x', 'digest': 'f' * 64},
                               'binding does not match')]:
            with self.subTest(link=link):
                self.check(self.skipped | {'chain': [link]}, pattern)
        self.check(self.skipped | {'chain': []}, 'malformed')

    def test_chain_rereads_the_runtime_file_it_cites(self):
        Path(self.runtime).write_bytes(RUNTIME.replace(b'--all', b'--any'))
        self.check(self.skipped, 'binding does not match')
        binary = self.base / 'binary'
        binary.write_bytes(b'\xff\n')
        link = self.base / 'link'
        link.symlink_to(binary)
        tables = {str(binary): hashlib.sha256(b'\xff\n').hexdigest(), str(link): hashlib.sha256(b'\xff\n').hexdigest()}
        approval = self.approval | {'runtime_files': tables}
        for name, pattern in [(str(binary), 'not text'), (str(link), 'binding does not match')]:
            with self.subTest(name=name):
                self.check(self.read | {'reader': {'source': 'runtime_files', 'name': name, 'digest': tables[name]},
                                        'verdict': admission.NOT_EXECUTED,
                                        'chain': [{'source': 'runtime_files', 'name': name, 'digest': tables[name],
                                                   'line': 1, 'text': 'x'}]}, pattern, approval)


class TargetTest(AdmissionTest):
    def test_dispatch_of_an_open_pr_head_follows_the_pr_rule(self):
        self.passed()
        self.github.routes['/compare/' + SHA + '...' + BASE] = {'status': 'diverged', 'behind_by': 1}
        self.run.update(event='workflow_dispatch', pull_requests=[])
        result = self.publish()
        self.assertEqual((result['status'], result['pull_request']), ('PASS', 7))
        self.assertEqual(len(pins.find(self.pins, admission.subject('owner/repo', 7, SHA, BASE, TREE))), 1)

    def test_dispatch_naming_neither_main_nor_one_open_pr_is_rejected(self):
        self.passed()
        routes = self.github.routes
        routes['/compare/' + SHA + '...' + BASE] = {'status': 'diverged', 'behind_by': 1}
        self.run.update(event='workflow_dispatch', pull_requests=[])
        other = {'number': 8, 'state': 'open', 'head': {'sha': SHA}}
        for listed in ([], [other | {'state': 'closed'}], [other | {'head': {'sha': BASE}}],
                       [{'number': 7, 'state': 'open', 'head': {'sha': SHA}}, other]):
            with self.subTest(listed=listed):
                routes['/commits/' + SHA + '/pulls'] = listed
                self.rejected('neither a main commit nor the head of exactly one open pull request')
        routes['/commits/' + SHA + '/pulls'] = {'message': 'not a list'}
        self.rejected('no pull request list')


class MergeSourceTest(AdmissionTest):
    def setUp(self):
        super().setUp()
        self.validation = self.passed()
        self.install_audit(self.validation)
        self.proof = self.publish()

    def merged(self, parents=None, **pr):
        routes = self.github.routes
        routes['/git/commits/' + MERGE] = {'sha': MERGE, 'tree': {'sha': TREE},
                                           'parents': [{'sha': p} for p in (parents or [BASE])]}
        routes['/git/ref/heads/main'] = {'object': {'type': 'commit', 'sha': MERGE}}
        routes['/compare/' + MERGE + '...' + MERGE] = {'status': 'identical', 'behind_by': 0}
        routes['/commits/' + MERGE + '/pulls'] = [{'number': 7, 'merge_commit_sha': MERGE}]
        routes['/pulls/7'] = {'state': 'closed', 'merged': True, 'number': 7, 'head': {'sha': SHA},
                              'base': {'ref': 'main'}, 'merge_commit_sha': MERGE} | pr
        self.run = {'id': 13, 'run_attempt': 1, 'event': 'push', 'head_sha': MERGE, 'pull_requests': []}

    def test_squash_and_merge_commits_verify_from_the_confirmed_pr_publication(self):
        for parents in ([BASE], [BASE, SHA]):
            with self.subTest(parents=parents):
                self.merged(parents)
                result = self.publish(identity='13/' + str(len(parents)))
                self.assertEqual((result['status'], result['rule'], result['merge_source'], result['validation_id']),
                                 ('PASS', admission.TREE_EQUIVALENCE, self.proof['publication_id'], self.validation))
                self.assertEqual(result['parents'], parents)

    def test_merge_must_be_read_back_as_exactly_this_pr_merge(self):
        cases = [({'merged': False}, None, 'not merged into main'),
                 ({'base': {'ref': 'dev'}}, None, 'not merged into main'),
                 ({'merge_commit_sha': 'f' * 40}, None, 'not merged into main'),
                 ({'head': {'sha': 'bad'}}, None, 'invalid pull request head'),
                 ({}, [SHA], 'does not match the publication of its pull request'),
                 ({}, [BASE, 'f' * 40], 'neither a squash nor a merge'),
                 ({}, [BASE, SHA, 'f' * 40], 'neither a squash nor a merge'),
                 ({}, ['f' * 40], 'does not match the publication of its pull request'),
                 ({'head': {'sha': 'f' * 40}}, [BASE, 'f' * 40], 'does not match the publication of its pull request')]
        for pr, parents, pattern in cases:
            with self.subTest(pr=pr, parents=parents):
                self.merged(parents, **pr)
                # A subject mismatch is a rejection, never mistaken for expired retention.
                self.assertEqual(self.rejected(pattern)['category'], 'rejected')
        self.merged()
        self.github.routes['/git/commits/' + MERGE]['parents'] = []
        self.rejected('neither a squash nor a merge')
        for listed in ([], [{'number': 7, 'merge_commit_sha': MERGE}, {'number': 8, 'merge_commit_sha': MERGE}]):
            with self.subTest(listed=listed):
                self.merged()
                self.github.routes['/commits/' + MERGE + '/pulls'] = listed
                self.rejected('merge commit of exactly one pull request')

    def test_merge_source_needs_its_live_pin_and_the_same_standing_validation(self):
        self.merged()
        pins.release(self.pins, {self.proof['pin_id']})
        # A released proof leaves no retained fact: the absence is not called expired.
        self.assertEqual(self.rejected('no retained fact shows whether it expired')['category'], 'missing')
        self.run = {'id': 12, 'run_attempt': 3, 'event': 'pull_request', 'head_sha': SHA, 'pull_requests': [{'number': 7}]}
        self.github.routes['/pulls/7'] = {'state': 'open', 'number': 7, 'head': {'sha': SHA}, 'base': {'ref': 'main'}}
        self.github.routes['/git/ref/heads/main'] = {'object': {'type': 'commit', 'sha': BASE}}
        self.publish(identity='12/3')
        # Audited for the newer PASS, whose tree has only the older validation's live proof.
        self.install_audit(self.passed(commit='d' * 40))
        self.merged()
        self.rejected('another validation of this tree')

    def lapse(self):
        """Move the clock past the PR proof's TTL without any pin write."""
        moment = pins.current(self.pins)[self.proof['pin_id']]['expires_at_ms'] / 1000
        clock = patch.object(pins, 'time', SimpleNamespace(time=lambda: moment + 1))
        clock.start()
        self.addCleanup(clock.stop)

    def test_lapsed_proof_of_this_subject_is_expired_retention(self):
        self.merged()
        self.lapse()
        self.expired('expired or was released', '13/1')

    def test_live_pins_of_other_subjects_are_no_evidence_of_expiry_or_mismatch(self):
        self.merged()
        # A live main publication of another commit, of this validation, is no fact about this merge's proof.
        record = ledger.verify(self.ledger, self.validation, self.inputs())
        other = admission.subject('owner/repo', None, 'f' * 40, BASE, TREE)
        value = pins.pin(self.pins, 10 ** 9, record, other, '14/1', 3600 * 10, {'merge_source': 'x'})
        pins.confirm(self.pins, value['pin_id'], self.validation)
        pins.release(self.pins, {self.proof['pin_id']})
        self.assertEqual(self.rejected('no retained fact')['category'], 'missing')

    def test_lapsed_fact_dropped_by_a_later_write_is_not_guessed(self):
        self.merged()
        self.lapse()
        pins.release(self.pins, set())
        self.assertEqual(self.rejected('no retained fact')['category'], 'missing')

    def test_main_merge_needs_the_audit_of_this_candidate(self):
        self.merged()
        self.install_audit(self.validation, tree='f' * 40)
        self.rejected('binds another tree')
        self.install_audit(self.validation, environment='f' * 64)
        self.rejected('binds another environment')
        del self.config['equivalence']
        self.rejected('no reviewed equivalence rule')

    def test_merge_source_publication_is_bound_to_its_identity(self):
        self.merged()
        state = pins.load(self.pins)
        state['pins'][self.proof['pin_id']]['publication']['validation_id'] = 'f' * 64
        (self.pins / 'state.json').write_bytes(pins.canonical(state))
        self.assertEqual(self.rejected('differ from its identity')['category'], 'tampered')

    def test_unconfirmed_pr_attempt_is_never_a_merge_source(self):
        pins.release(self.pins, {self.proof['pin_id']})
        record = ledger.verify(self.ledger, self.validation, self.inputs())
        bound = admission.subject('owner/repo', 7, SHA, BASE, TREE)
        pins.pin(self.pins, 10 ** 9, record, bound, '12/9', 7200, {'sha': SHA})
        self.merged()
        self.assertEqual(self.rejected('was never confirmed')['category'], 'rejected')

    def test_same_tree_base_advance_is_reproved_without_a_gate(self):
        advanced = 'e' * 40
        routes = self.github.routes
        routes['/git/ref/heads/main'] = {'object': {'type': 'commit', 'sha': advanced}}
        routes['/compare/' + advanced + '...' + SHA] = {'status': 'ahead', 'behind_by': 0}
        again = self.publish(identity='12/2')
        self.assertEqual((again['status'], again['base'], again['validation_id']), ('PASS', advanced, self.validation))
        self.merged([advanced])
        self.assertEqual(self.publish(identity='13/1')['merge_source'], again['publication_id'])


class PublicationTest(AdmissionTest):
    def unconfirmed(self, pattern):
        self.rejected(pattern)
        self.assertFalse((self.home / 'ledger-anchor.json').exists())
        self.assertEqual(pins.current(self.pins), {})

    def test_pin_that_does_not_fit_the_records_budget_refuses_success(self):
        self.passed()
        self.config['storage_deployment'] = str(self.deployment(1))
        self.unconfirmed('retention not reserved')
        self.assertFalse(any(body['conclusion'] == 'success' for method, path, body in self.github.writes if method == 'PATCH'))

    def test_success_that_does_not_read_back_is_never_confirmed(self):
        self.passed()
        for check in [{'head_sha': SHA, 'external_id': '12/1', 'status': 'completed', 'conclusion': 'failure'},
                      {'head_sha': SHA, 'external_id': '12/2', 'status': 'completed', 'conclusion': 'success'},
                      {'head_sha': BASE, 'external_id': '12/1', 'status': 'completed', 'conclusion': 'success'}]:
            with self.subTest(check=check):
                self.github.routes['/check-runs/9'] = check
                self.unconfirmed('did not read back')

    def test_pin_expiring_across_publication_is_not_confirmed(self):
        self.passed()
        original = pins.confirm

        def later(root, pin_id, validation_id):
            # The in-lock re-read happens after the pin's TTL has passed.
            return original(root, pin_id, validation_id, now=time.time() + 7201)
        with patch.object(pins, 'confirm', side_effect=later), \
                patch.object(admission.ledger, 'record', side_effect=AssertionError('ledger write')):
            self.unconfirmed('missing or expired')
        self.assertEqual(self.github.writes[-1][2]['output']['title'], 'Evidence retention expired; no Gate started')
        self.assertEqual(json.loads((self.home / 'jobs/12-1/failure.json').read_text())['category'],
                         admission.RETENTION_EXPIRED)

    def test_other_pin_faults_keep_their_category(self):
        self.passed()
        with patch.object(pins, 'confirm', side_effect=pins.PinError('tampered', 'invalid pin state')):
            self.assertEqual(self.rejected('invalid pin state')['category'], 'tampered')
        self.assertEqual(self.title(), 'Evidence not admitted; validate locally')

    def test_confirmation_rereads_under_the_held_lock_without_reopening_it(self):
        self.passed()
        opened = []
        original = pins.locked

        def counting(root):
            opened.append(root)
            return original(root)
        with patch.object(pins, 'locked', side_effect=counting):
            self.assertEqual(self.publish()['status'], 'PASS')
        # budget read is lock-free; pin, holding: two acquisitions, never nested.
        self.assertEqual(len(opened), 2)

    def test_publication_holds_the_pin_lock_against_cleanup(self):
        self.passed()
        observed = []

        def probe(*args):
            descriptor = ledger.os.open(self.pins / 'lock', ledger.os.O_RDWR)
            try:
                ledger.fcntl.flock(descriptor, ledger.fcntl.LOCK_EX | ledger.fcntl.LOCK_NB)
                observed.append('free')
            except BlockingIOError:
                observed.append('held')
            finally:
                ledger.os.close(descriptor)
            original(*args)
        original = admission.complete
        with patch.object(admission, 'complete', side_effect=probe):
            self.publish()
        self.assertEqual(observed, ['held'])


class SweepTest(AdmissionTest):
    def sweep(self):
        return admission.sweep(self.config, 'token')

    def test_open_pr_and_main_publications_keep_their_pins(self):
        self.passed()
        result = self.publish()
        self.run.update(event='push', pull_requests=[])
        main = self.publish(identity='12/2')
        self.assertEqual(self.sweep(), [])
        self.assertEqual(set(pins.current(self.pins)), {result['pin_id'], main['pin_id']})

    def test_settled_proofs_release_their_pins_and_publications(self):
        self.passed()
        for pr in [{'state': 'closed', 'merged': False, 'head': {'sha': SHA}}, {'state': 'open', 'head': {'sha': 'f' * 40}}]:
            with self.subTest(pr=pr):
                self.github.routes['/pulls/7'] = {'state': 'open', 'number': 7, 'head': {'sha': SHA}, 'base': {'ref': 'main'}}
                result = self.publish()
                self.github.routes['/pulls/7'] = pr
                self.assertEqual(self.sweep(), [result['pin_id']])
                self.assertEqual(pins.current(self.pins), {})

    def test_interrupted_unconfirmed_attempt_is_released(self):
        identity = self.passed()
        record = ledger.verify(self.ledger, identity, self.inputs())
        value = pins.pin(self.pins, 10 ** 9, record, {'pull_request': None}, '12/1', 3600, {'sha': SHA})
        self.assertEqual(self.sweep(), [value['pin_id']])

    def test_merged_pr_proof_is_handed_over_to_main_verification_after_its_ttl(self):
        self.install_audit(self.passed())
        proof = self.publish()
        created = pins.current(self.pins)[proof['pin_id']]['created_at_ms'] / 1000
        merged = created + 7000
        MergeSourceTest.merged(self)
        self.github.routes['/pulls/7']['merged_at'] = time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime(merged))
        clock = [created + 7100]
        with patch.object(pins, 'time', SimpleNamespace(time=lambda: clock[0])):
            self.assertEqual(self.sweep(), [])
            expires = pins.current(self.pins)[proof['pin_id']]['expires_at_ms']
            self.assertEqual(expires, int(merged) * 1000 + 3600 * 1000)
            # Past the PR TTL, within the audit window after the merge: main still verifies.
            clock[0] = created + 7300
            result = self.publish(identity='13/1')
            self.assertEqual((result['status'], result['merge_source']), ('PASS', proof['publication_id']))
            clock[0] = expires / 1000
            self.assertEqual(self.publish(identity='13/2')['category'], admission.RETENTION_EXPIRED)

    def test_without_the_hand_off_the_pr_ttl_would_reject_main(self):
        self.install_audit(self.passed())
        proof = self.publish()
        created = pins.current(self.pins)[proof['pin_id']]['created_at_ms'] / 1000
        MergeSourceTest.merged(self)
        with patch.object(pins, 'time', SimpleNamespace(time=lambda: created + 7300)):
            self.expired('expired or was released', '13/1')


if __name__ == '__main__':
    unittest.main()
