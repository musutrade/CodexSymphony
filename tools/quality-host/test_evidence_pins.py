import json
import os
from pathlib import Path
import tempfile
import threading
import unittest

import evidence_pins as pins

HOUR = 3600
PUBLICATION = {'sha': 'a' * 40, 'actions_attempt': '12/1'}


class PinTest(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name).resolve()
        self.gate = self.base / 'evidence/gate'
        self.gate.mkdir(parents=True)
        self.slot = self.base / 'validation/gate'
        self.slot.mkdir(parents=True)
        self.root = self.base / 'evidence/pins'
        self.root.mkdir()
        (self.root / 'lock').touch()

    def run_dir(self, suffix='aaaaaaaaaaaa', timestamp=100, size=10, measurement='measurement-summary.json'):
        """A completed bounded capture, as cleanup and pin admission both recognize it."""
        run = self.gate / ('run-' + suffix)
        (run / 'reports').mkdir(parents=True)
        (run / 'probes/backend/raw').mkdir(parents=True)
        (run / 'probes/backend/raw/profile').write_bytes(b'r' * 1000)
        (run / 'reports/test_result.json').write_bytes(b'p' * size)
        (run / 'source.tar.gz').write_bytes(b'archive')
        (run / 'source-archive.json').write_text(json.dumps({'sha256': pins.file_digest(run / 'source.tar.gz')}))
        (run / 'probes/backend/bundle.json').write_text('{}')
        (run / 'capture-registration.json').write_text(json.dumps(
            {'root': str(run), 'bundle_sha256': pins.file_digest(run / 'probes/backend/bundle.json')}))
        (run / 'measurements.json').write_text('{}')
        (run / measurement).write_text(json.dumps({'coverage_and_crap': 'PASS', 'measurement': str(run / 'measurements.json'),
                                                   'measurement_sha256': pins.file_digest(run / 'measurements.json')}))
        os.utime(run / 'source-archive.json', (timestamp, timestamp))
        return run

    def record(self, run, validation_id='v' * 64):
        path = run / 'reports/test_result.json'
        return {'validation_id': validation_id, 'details': {'run': str(run)},
                'evidence': {'report': {'path': str(path), 'sha256': pins.file_digest(path)}}}

    def pin(self, run, budget=10 ** 6, ttl=HOUR, now=1000, subject=None, **changes):
        return pins.pin(self.root, budget, self.record(run) | changes, subject or {'pull_request': 7}, '12/1', ttl,
                        PUBLICATION, now)


class StateTest(PinTest):
    def test_open_refuses_links_and_fifos_without_hanging(self):
        target = self.base / 'target'
        target.write_text('x')
        (self.base / 'link').symlink_to(target)
        os.mkfifo(self.base / 'fifo')
        for name in ('link', 'fifo'):
            with self.subTest(name=name), self.assertRaisesRegex(pins.PinError, 'not a regular file'):
                pins.open_regular(self.base / name, os.O_RDONLY)
        with self.assertRaises(FileNotFoundError):
            pins.open_regular(self.base / 'absent', os.O_RDONLY)

    def test_lock_root_must_be_canonical_and_installed(self):
        with self.assertRaisesRegex(pins.PinError, 'canonical'):
            with pins.locked(Path('evidence/pins')):
                pass
        (self.base / 'alias').symlink_to(self.root)
        with self.assertRaisesRegex(pins.PinError, 'canonical'):
            with pins.locked(self.base / 'alias'):
                pass
        (self.root / 'lock').unlink()
        with self.assertRaisesRegex(pins.PinError, 'not installed') as caught:
            with pins.locked(self.root):
                pass
        self.assertEqual(caught.exception.category, 'missing')

    def test_state_round_trips_and_invalid_state_is_tampered(self):
        self.assertEqual(pins.load(self.root), pins.empty())
        saved = pins.save(self.root, pins.load(self.root), {})
        self.assertEqual((saved['sequence'], pins.load(self.root)), (1, saved))
        for value in [{'schema': 'other', 'sequence': 0, 'pins': {}}, {'schema': pins.SCHEMA, 'sequence': '0', 'pins': {}},
                      {'schema': pins.SCHEMA, 'sequence': 0, 'pins': []}]:
            with self.subTest(value=value):
                (self.root / 'state.json').write_text(json.dumps(value))
                with self.assertRaisesRegex(pins.PinError, 'invalid pin state'):
                    pins.load(self.root)

    def test_only_canonical_bounded_runs_are_pinnable(self):
        run = self.run_dir()
        self.assertEqual(pins.pinnable(self.root, str(run)), run)
        legacy = self.base / 'gate-host/runs/run-aaaaaaaaaaaa'
        legacy.mkdir(parents=True)
        (self.gate / 'run-bbbbbbbbbbbb').symlink_to(run)
        for candidate in (legacy, self.gate / 'retiring-run-aaaaaaaaaaaa', self.gate / 'run-bbbbbbbbbbbb', ''):
            with self.subTest(candidate=candidate), self.assertRaisesRegex(pins.PinError, 'canonical bounded Gate run'):
                pins.pinnable(self.root, candidate)


class ChargeTest(PinTest):
    def test_charge_is_every_file_but_the_installed_payloads_with_inodes_counted_once(self):
        run = self.run_dir(size=10)
        for name in ('http-server', 'tmp', 'workspace/.harness-gate/reports'):
            (run / name).mkdir(parents=True)
            (run / name / 'payload').write_bytes(b'x' * 500)
        before = pins.charge([run])
        os.link(run / 'reports/test_result.json', run / 'reports/copy.json')
        self.assertEqual(pins.charge([run, run]), before)
        payload = {run / 'probes/backend/raw/profile', run / 'http-server/payload'}
        self.assertEqual(before, sum(p.stat().st_size for p in run.rglob('*')
                                     if p.is_file() and p not in payload and p.name != 'copy.json'))
        self.assertTrue(pins.cleanable(run, run / 'probes/backend/raw/profile'))
        self.assertTrue(pins.cleanable(run, run / 'tmp/payload'))
        self.assertFalse(pins.cleanable(run, run / 'reports/test_result.json'))

    def test_state_charge_covers_the_pending_copy_and_any_sequence(self):
        state = pins.empty() | {'pins': {'a': {'run': 'x'}}}
        self.assertEqual(pins.state_bytes(state), pins.state_bytes(state | {'sequence': 10 ** 18}))
        self.assertGreaterEqual(pins.state_bytes(state), 2 * len(pins.canonical(state | {'sequence': 10 ** 18})))


class CurrentRecordTest(PinTest):
    def test_completed_capture_requires_source_registration_and_measurement_identity(self):
        run = self.run_dir()
        self.assertTrue(pins.completed(run))
        (run / 'measurement-summary.json').rename(run / 'recovery-measurement.json')
        self.assertTrue(pins.completed(run))
        (run / 'recovery-measurement.json').unlink()
        self.assertFalse(pins.completed(run))
        (run / 'capture-registration.json').unlink()
        self.assertFalse(pins.completed(run))

    def test_changed_or_aliased_capture_records_are_tampered(self):
        run = self.run_dir()
        link = self.base / 'link.json'
        link.symlink_to(run / 'measurements.json')
        cases = [('measurement-summary.json', {'coverage_and_crap': 'PASS', 'measurement': str(link),
                                                'measurement_sha256': '0' * 64}, 'canonical'),
                 ('measurement-summary.json', {'coverage_and_crap': 'PASS', 'measurement': str(run / 'measurements.json'),
                                                'measurement_sha256': '0' * 64}, 'measurement changed'),
                 ('capture-registration.json', {'root': 'other'}, 'registration changed'),
                 ('source-archive.json', {'sha256': '0' * 64}, 'archive changed')]
        for name, value, pattern in cases:
            with self.subTest(pattern=pattern):
                (run / name).write_text(json.dumps(value))
                with self.assertRaisesRegex(pins.PinError, pattern):
                    pins.completed(run)

    def test_failed_measurement_is_not_completed(self):
        run = self.run_dir()
        value = json.loads((run / 'measurement-summary.json').read_text()) | {'coverage_and_crap': 'FAIL'}
        (run / 'measurement-summary.json').write_text(json.dumps(value))
        self.assertFalse(pins.completed(run))

    def test_current_record_is_the_newest_completed_unannounced_capture(self):
        self.assertEqual(pins.current_record(self.root), [])
        old = self.run_dir(timestamp=100)
        new = self.run_dir('bbbbbbbbbbbb', timestamp=200, measurement='recovery-measurement.json')
        self.assertEqual(pins.current_record(self.root), [new])
        (self.slot / 'pending-gate.json').write_text(json.dumps({'capture': str(new)}))
        self.assertEqual(pins.current_record(self.root), [old])
        (self.slot / 'pending-capture.json').write_text(json.dumps({'capture': str(old)}))
        self.assertEqual(pins.current_record(self.root), [])
        (self.slot / 'pending-gate.json').unlink()
        (self.slot / 'pending-gate.json').symlink_to(self.slot / 'pending-capture.json')
        with self.assertRaisesRegex(pins.PinError, 'symlink pending'):
            pins.current_record(self.root)

    def test_unsafe_bounded_directories_are_refused(self):
        (self.gate / 'run-unsafe').mkdir()
        with self.assertRaisesRegex(pins.PinError, 'unsafe'):
            pins.candidates(self.gate, set())

    def test_reservation_is_the_union_of_pinned_and_current_records_plus_state(self):
        old = self.run_dir(size=100)
        new = self.run_dir('bbbbbbbbbbbb', timestamp=200, size=200)
        state = pins.empty() | {'pins': {'a': {'run': str(old)}, 'b': {'run': str(new)}}}
        self.assertEqual(pins.reserved(state, [new]), pins.charge([old, new]) + pins.state_bytes(state))
        self.assertEqual(pins.reserved(pins.empty(), []), pins.state_bytes(pins.empty()))
        self.assertEqual(pins.runs(state), {old, new})


class PinLifecycleTest(PinTest):
    def test_pin_records_identity_and_protects_until_release_or_expiry(self):
        run = self.run_dir()
        value = self.pin(run)
        self.assertEqual((value['run'], value['expires_at_ms'], value['attempt']), (str(run), 1000 * 1000 + HOUR * 1000, '12/1'))
        self.assertEqual((value['publication'], value['publication_id']), (PUBLICATION | {'status': 'unconfirmed'}, pins.PLACEHOLDER))
        self.assertEqual(set(pins.current(self.root, now=1000)), {value['pin_id']})
        self.assertEqual(pins.active(self.root, value['pin_id'], 'v' * 64, now=1000)['run'], str(run))
        for pin_id, validation, now in [(value['pin_id'], 'w' * 64, 1000), ('absent', 'v' * 64, 1000),
                                        (value['pin_id'], 'v' * 64, 1000 + HOUR)]:
            with self.subTest(pin_id=pin_id, now=now), self.assertRaisesRegex(pins.PinError, 'missing or expired'):
                pins.active(self.root, pin_id, validation, now=now)
        self.assertEqual(pins.release(self.root, {value['pin_id']}, now=1000), [value['pin_id']])
        self.assertEqual(pins.current(self.root, now=1000), {})
        self.assertEqual(pins.release(self.root, {value['pin_id']}, now=1000), [])

    def test_expired_pins_are_dropped_on_write_so_state_stays_bounded(self):
        run = self.run_dir()
        first = self.pin(run, ttl=1, now=1000)
        second = self.pin(run, now=2000)
        self.assertEqual(set(pins.load(self.root)['pins']), {second['pin_id']})
        self.assertNotEqual(first['pin_id'], second['pin_id'])
        self.assertEqual(pins.release(self.root, {first['pin_id']}, now=2000), [])

    def test_pin_that_does_not_fit_is_refused_without_writing(self):
        run = self.run_dir(size=100)
        with self.assertRaisesRegex(pins.PinError, 'record bytes') as caught:
            self.pin(run, budget=pins.charge([run]))
        self.assertEqual(caught.exception.category, 'capacity')
        self.assertFalse((self.root / 'state.json').exists())
        self.pin(run, budget=pins.charge([run]) + 10 ** 5)

    def test_reservation_counts_the_state_that_is_written_and_the_newer_current_record(self):
        old = self.run_dir(size=100)
        self.run_dir('bbbbbbbbbbbb', timestamp=200, size=100)
        value = self.pin(old, budget=10 ** 6)
        state = pins.load(self.root)
        needed = pins.reserved(state, pins.current_record(self.root))
        self.assertGreater(needed, pins.charge([old]) + len(pins.canonical(state)))
        pins.release(self.root, {value['pin_id']})
        with self.assertRaisesRegex(pins.PinError, 'record bytes'):
            self.pin(old, budget=needed - 1)
        self.pin(old, budget=needed)

    def test_pin_checks_every_bound_file_is_protected_and_unchanged(self):
        run = self.run_dir()
        raw = run / 'probes/backend/raw/profile'
        other = self.run_dir('bbbbbbbbbbbb')
        cases = [({'raw': {'path': str(raw), 'sha256': pins.file_digest(raw)}}, 'mismatch'),
                 ({'report': self.record(other)['evidence']['report']}, 'mismatch'),
                 ({'report': {'path': str(run / 'absent'), 'sha256': '0' * 64}}, 'missing'),
                 ({'report': {'path': str(run / 'reports/test_result.json'), 'sha256': '0' * 64}}, 'tampered')]
        for evidence, category in cases:
            with self.subTest(category=category), self.assertRaises(pins.PinError) as caught:
                self.pin(run, evidence=evidence)
            self.assertEqual(caught.exception.category, category)


class PublicationTest(PinTest):
    def test_confirmation_under_the_held_lock_rechecks_expiry_and_only_shortens_the_entry(self):
        run = self.run_dir()
        value = self.pin(run, now=1000)
        before = len(pins.canonical(pins.load(self.root)))
        with pins.holding(self.root, value['pin_id'], 'v' * 64, now=1000) as root:
            confirmed = pins.confirm(root, value['pin_id'], 'v' * 64, now=1000)
        self.assertEqual(confirmed['publication']['status'], 'confirmed')
        self.assertEqual(confirmed['publication_id'], pins.digest(pins.canonical(confirmed['publication'])))
        self.assertLessEqual(len(pins.canonical(pins.load(self.root))), before)
        other = self.pin(run, now=1000, ttl=2 * HOUR)
        with pins.holding(self.root, other['pin_id'], 'v' * 64, now=1000) as root:
            with self.assertRaisesRegex(pins.PinError, 'missing or expired'):
                pins.confirm(root, other['pin_id'], 'v' * 64, now=1000 + 2 * HOUR)
        self.assertEqual(pins.current(self.root, now=1000)[other['pin_id']]['publication']['status'], 'unconfirmed')

    def test_find_returns_only_live_confirmed_publications_of_the_exact_subject(self):
        run = self.run_dir()
        subject = {'pull_request': 7, 'head': 'h'}
        self.pin(run, now=None, subject=subject)
        self.assertEqual(pins.find(self.root, subject), [])
        value = self.pin(run, now=None, subject=subject, validation_id='w' * 64)
        with pins.holding(self.root, value['pin_id'], 'w' * 64) as root:
            pins.confirm(root, value['pin_id'], 'w' * 64)
        found = pins.find(self.root, subject)
        self.assertEqual([(row['pin_id'], row['validation_id']) for row in found], [(value['pin_id'], 'w' * 64)])
        self.assertEqual(pins.find(self.root, subject | {'head': 'other'}), [])
        state = pins.load(self.root)
        state['pins'][value['pin_id']]['publication']['sha'] = 'forged'
        (self.root / 'state.json').write_bytes(pins.canonical(state))
        with self.assertRaisesRegex(pins.PinError, 'differ from its identity'):
            pins.find(self.root, subject)

    def test_extension_keeps_a_live_pin_but_never_revives_one(self):
        run = self.run_dir()
        value = self.pin(run, now=1000)
        until = value['expires_at_ms'] + 5000
        self.assertTrue(pins.extend(self.root, value['pin_id'], until, now=1000))
        self.assertEqual(pins.current(self.root, now=1000)[value['pin_id']]['expires_at_ms'], until)
        self.assertTrue(pins.extend(self.root, value['pin_id'], 0, now=1000))
        self.assertEqual(pins.current(self.root, now=1000)[value['pin_id']]['expires_at_ms'], until)
        with self.assertRaisesRegex(pins.PinError, 'width'):
            pins.extend(self.root, value['pin_id'], until * 10, now=1000)
        self.assertFalse(pins.extend(self.root, value['pin_id'], until * 2, now=until / 1000))
        self.assertFalse(pins.extend(self.root, 'absent', until, now=1000))


class ConcurrencyTest(PinTest):
    def blocked_until_exit(self, context, action):
        """action started while context holds the pin lock completes only after context exits."""
        done = threading.Event()
        worker = threading.Thread(target=lambda: (action(), done.set()))
        with context:
            worker.start()
            self.assertFalse(done.wait(0.2))
        worker.join(5)
        self.assertTrue(done.is_set())

    def test_cleanup_excludes_pin_creation_extension_and_release(self):
        run = self.run_dir()
        self.blocked_until_exit(pins.cleanup(self.root), lambda: self.pin(run, now=None))
        value = self.pin(run, now=None)
        self.blocked_until_exit(pins.cleanup(self.root), lambda: pins.extend(self.root, value['pin_id'], 0))
        self.blocked_until_exit(pins.cleanup(self.root), lambda: pins.release(self.root, {value['pin_id']}))

    def test_publication_hold_excludes_cleanup_and_sees_the_live_pin(self):
        run = self.run_dir()
        value = self.pin(run, now=None)
        seen = []
        holding = pins.holding(self.root, value['pin_id'], 'v' * 64)

        def clean():
            with pins.cleanup(self.root) as state:
                seen.append(pins.runs(state))
        self.blocked_until_exit(holding, clean)
        self.assertEqual(seen, [{run}])
        with self.assertRaisesRegex(pins.PinError, 'missing or expired'):
            with pins.holding(self.root, value['pin_id'], 'w' * 64):
                pass

    def test_cleanup_drops_expired_pins_and_an_absent_root_protects_nothing(self):
        with pins.cleanup(self.base / 'absent') as state:
            self.assertEqual(state['pins'], {})
        self.assertFalse((self.base / 'absent').exists())
        run = self.run_dir()
        self.pin(run, ttl=1, now=1000)
        with pins.cleanup(self.root, now=3000) as state:
            self.assertEqual(state['pins'], {})
        self.assertEqual(pins.load(self.root)['pins'], {})
        sequence = pins.load(self.root)['sequence']
        with pins.cleanup(self.root, now=3000):
            pass
        self.assertEqual(pins.load(self.root)['sequence'], sequence)


class BudgetTest(PinTest):
    def deployment(self, source, files=None):
        release = self.base / 'release'
        release.mkdir(exist_ok=True)
        entry = release / 'compact_gate_evidence.py'
        entry.write_text(source)
        path = self.base / 'deployment.json'
        path.write_text(json.dumps({'release': str(release),
                                    'files': {str(entry): pins.file_digest(entry)} if files is None else files}))
        return path, entry

    def test_installed_defaults_are_evaluated_without_execution(self):
        source = 'GIB=1024**3\nraise SystemExit\ndef maintain(apply=False, keep=2, hours=24, budget=4*GIB):\n    pass\n'
        path, _ = self.deployment(source)
        self.assertEqual((pins.records_budget(path), pins.retention_hours(path)), (4 * 1024 ** 3, 24))
        path, _ = self.deployment('def maintain(budget=10+2-1):\n    pass\n')
        self.assertEqual(pins.records_budget(path), 11)
        with self.assertRaisesRegex(pins.PinError, 'no maintain hours'):
            pins.retention_hours(path)

    def test_changed_unapproved_or_unsupported_policy_is_refused(self):
        path, entry = self.deployment('def maintain(budget=1):\n    pass\n')
        entry.write_text('def maintain(budget=2):\n    pass\n')
        with self.assertRaisesRegex(pins.PinError, 'policy changed'):
            pins.records_budget(path)
        path, _ = self.deployment('def maintain(budget=1):\n    pass\n', files={})
        with self.assertRaisesRegex(pins.PinError, 'not approved'):
            pins.records_budget(path)
        value = json.loads(path.read_text()) | {'release': 'release'}
        path.write_text(json.dumps(value))
        with self.assertRaisesRegex(pins.PinError, 'canonical'):
            pins.records_budget(path)
        for source, pattern in [('def other(budget=1):\n    pass\n', 'no maintain budget'),
                                ('def maintain(keep=1):\n    pass\n', 'no maintain budget'),
                                ('def maintain(budget=size()):\n    pass\n', 'unsupported'),
                                ('def maintain(budget=1.5):\n    pass\n', 'unsupported'),
                                ('def maintain(budget=UNKNOWN):\n    pass\n', 'unsupported'),
                                ('def maintain(budget=2//1):\n    pass\n', 'unsupported'),
                                ('def maintain(budget=2**65):\n    pass\n', 'unsupported')]:
            with self.subTest(source=source), self.assertRaisesRegex(pins.PinError, pattern):
                pins.records_budget(self.deployment(source)[0])


if __name__ == '__main__':
    unittest.main()
