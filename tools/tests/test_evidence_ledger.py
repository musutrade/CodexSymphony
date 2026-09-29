import importlib.util
import json
import os
from pathlib import Path
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('evidence_ledger', ROOT / 'tools/evidence_ledger.py')
ledger = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ledger)
LedgerError = ledger.LedgerError


def inputs(tree='a' * 40, commit='c' * 40, environment='e' * 64):
    return {'approval': 'p' * 64, 'commit': commit, 'environment': environment, 'tree': tree}


class LedgerTest(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.base = Path(self.temporary.name).resolve()
        self.root = self.base / 'ledger'
        self.report = self.base / 'report.json'
        self.report.write_text('{"passed":true}')

    def tearDown(self):
        self.temporary.cleanup()

    def passed(self, value=None, report=None):
        value = value or inputs()
        attempt = ledger.begin(self.root, value)
        return ledger.record_pass(self.root, attempt, value, {'report': str(report or self.report)}, {'run': 'run-1'})

    def failed(self, value=None):
        value = value or inputs()
        return ledger.record_fail(self.root, ledger.begin(self.root, value), value, {}, {})

    def assert_category(self, category, function, *args, **kwargs):
        with self.assertRaises(LedgerError) as caught:
            function(*args, **kwargs)
        self.assertEqual(caught.exception.category, category)

    def events(self):
        return [json.loads(line) for line in (self.root / 'events.jsonl').read_text().splitlines()]

    def rewrite(self, events):
        (self.root / 'events.jsonl').write_bytes(b''.join(ledger.canonical(e) + b'\n' for e in events))


class RecordTest(LedgerTest):
    def test_pass_is_immutable_content_addressed_and_verifies_read_only(self):
        identity = self.passed()
        path = self.root / 'records' / (identity + '.json')
        self.assertEqual(ledger.digest(path.read_bytes()), identity)
        self.assertEqual(path.stat().st_mode & 0o777, 0o400)
        before = {p: p.read_bytes() for p in self.root.rglob('*') if p.is_file()}
        result = ledger.verify(self.root, identity, inputs())
        self.assertEqual((result['validation_id'], result['sequence'], result['outcome']), (identity, 1, 'pass'))
        self.assertEqual(result['evidence']['report']['sha256'], ledger.file_digest(self.report))
        self.assertEqual(result['head']['sequence'], 2)
        self.assertEqual(ledger.current(self.root, inputs())['validation_id'], identity)
        self.assertEqual({p: p.read_bytes() for p in self.root.rglob('*') if p.is_file()}, before)

    def test_identical_outcomes_get_distinct_ids_and_existing_records_are_never_replaced(self):
        first, second = self.passed(), self.passed()
        self.assertNotEqual(first, second)
        with self.assertRaises(FileExistsError):
            ledger.write_new(self.root / 'records' / (first + '.json'), b'{}')
        self.assertEqual([e['kind'] for e in self.events()], ['start', 'pass', 'start', 'pass'])

    def test_invalid_inputs_evidence_and_details_are_rejected_before_writing(self):
        link = self.base / 'link.json'
        link.symlink_to(self.report)
        cases = [(dict(inputs(), extra='x'), {'report': str(self.report)}, {}),
                 ({k: v for k, v in inputs().items() if k != 'commit'}, {'report': str(self.report)}, {}),
                 (dict(inputs(), tree=''), {'report': str(self.report)}, {}),
                 (dict(inputs(), tree=7), {'report': str(self.report)}, {}),
                 ('tree', {'report': str(self.report)}, {}),
                 (inputs(), {}, {}),
                 (inputs(), [str(self.report)], {}),
                 (inputs(), {'report': 'relative.json'}, {}),
                 (inputs(), {'report': str(link)}, {}),
                 (inputs(), {'report': str(self.base / 'absent.json')}, {}),
                 (inputs(), {'report': str(self.base)}, {}),
                 (inputs(), {'report': str(self.report)}, [])]
        attempt = ledger.begin(self.root, inputs())
        for value, evidence, details in cases:
            with self.subTest(value=value, evidence=evidence, details=details):
                self.assert_category('mismatch', ledger.record_pass, self.root, attempt, value, evidence, details)
        self.assertEqual([e['kind'] for e in self.events()], ['start'])
        self.assertEqual(list((self.root / 'records').iterdir()), [])

    def test_conclusion_must_name_a_pending_attempt_of_exactly_the_announced_inputs(self):
        attempt = ledger.begin(self.root, inputs())
        evidence = {'report': str(self.report)}
        for other in ['f' * 64, ledger.begin(self.root, inputs(tree='b' * 40))]:
            with self.subTest(attempt=other):
                self.assert_category('mismatch', ledger.record_pass, self.root, other, inputs(), evidence, {})
        # Execution evidence of commit c can never be rebound to commit d, as PASS or as FAIL.
        other = inputs(commit='d' * 40)
        self.assert_category('mismatch', ledger.record_pass, self.root, attempt, other, evidence, {})
        self.assert_category('mismatch', ledger.record_fail, self.root, attempt, other, {}, {})
        self.assertEqual(list((self.root / 'records').iterdir()), [])
        ledger.record_fail(self.root, attempt, inputs(), {}, {})
        self.assert_category('mismatch', ledger.record_pass, self.root, attempt, inputs(), evidence, {})
        self.assertEqual(sum(1 for _ in (self.root / 'records').iterdir()), 1)

    def test_non_canonical_root_is_rejected(self):
        self.assert_category('tampered', ledger.record_fail, Path('relative'), 'f' * 64, inputs(), {}, {})
        alias = self.base / 'alias'
        alias.symlink_to(self.base)
        self.assert_category('tampered', ledger.verify, alias / 'ledger', 'f' * 64, inputs())
        self.assert_category('tampered', ledger.hold, alias / 'ledger')
        self.root.mkdir()
        (self.root / 'records').symlink_to(self.base)
        self.assert_category('tampered', ledger.begin, self.root, inputs())

    def test_records_directory_replaced_after_writing_is_rejected_on_read(self):
        identity = self.passed()
        moved = self.base / 'moved-records'
        (self.root / 'records').rename(moved)
        (self.root / 'records').symlink_to(moved)
        self.assert_category('tampered', ledger.verify, self.root, identity, inputs())
        self.assert_category('tampered', ledger.load_record, self.root, identity)

    def test_concurrent_appends_keep_one_unbroken_chain(self):
        threads = [threading.Thread(target=self.passed) for _ in range(8)]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join()
        events, digests = ledger.read_events(self.root)
        self.assertEqual([e['sequence'] for e in events], list(range(16)))
        self.assertEqual(len(digests), 16)
        self.assertEqual(sorted(e['kind'] for e in events), ['pass'] * 8 + ['start'] * 8)


class BlockingTest(LedgerTest):
    def test_later_fail_for_same_inputs_blocks_every_earlier_pass(self):
        first, second = self.passed(), self.passed()
        self.failed(inputs())
        for identity in (first, second):
            self.assert_category('blocked', ledger.verify, self.root, identity, inputs())
        self.assert_category('blocked', ledger.current, self.root, inputs())

    def test_fail_of_the_same_executed_inputs_under_another_commit_still_blocks(self):
        identity = self.passed()
        self.failed(inputs(commit='d' * 40))
        self.assert_category('blocked', ledger.verify, self.root, identity, inputs())
        self.assert_category('blocked', ledger.current, self.root, inputs())

    def test_new_complete_pass_after_fail_stands_but_old_pass_stays_blocked(self):
        old = self.passed()
        self.failed(inputs())
        new = self.passed()
        self.assert_category('blocked', ledger.verify, self.root, old, inputs())
        self.assertEqual(ledger.verify(self.root, new, inputs())['sequence'], 5)
        self.assertEqual(ledger.current(self.root, inputs())['validation_id'], new)

    def test_revocation_blocks_and_requires_a_reason_and_a_pass(self):
        identity = self.passed()
        failed = self.failed(inputs())
        self.assert_category('mismatch', ledger.revoke, self.root, identity, '')
        self.assert_category('mismatch', ledger.revoke, self.root, failed, 'not a pass')
        self.assert_category('missing', ledger.revoke, self.root, 'f' * 64, 'unknown')
        renewed = self.passed()
        event = ledger.revoke(self.root, renewed, 'approval withdrawn')
        self.assertEqual((event['kind'], event['reason']), ('revoke', 'approval withdrawn'))
        self.assert_category('blocked', ledger.verify, self.root, renewed, inputs())
        self.assert_category('blocked', ledger.current, self.root, inputs())

    def test_outcomes_for_other_executed_inputs_do_not_block(self):
        identity = self.passed()
        for other in (inputs(tree='b' * 40), inputs(environment='f' * 64)):
            self.failed(other)
            self.assert_category('blocked', ledger.current, self.root, other)
        self.assertEqual(ledger.verify(self.root, identity, inputs())['validation_id'], identity)
        self.assert_category('blocked', ledger.current, self.root, inputs(commit='d' * 40))

    def test_pass_for_different_inputs_or_a_fail_record_is_not_admitted(self):
        identity = self.passed()
        failed = self.failed(inputs())
        self.assert_category('mismatch', ledger.verify, self.root, identity, inputs(commit='d' * 40))
        self.assert_category('mismatch', ledger.verify, self.root, failed, inputs())
        self.assert_category('mismatch', ledger.verify, self.root, 'not-an-id', inputs())
        self.assert_category('missing', ledger.verify, self.root, 'f' * 64, inputs())


class StartTest(LedgerTest):
    def test_started_gate_blocks_older_pass_until_it_concludes(self):
        old = self.passed()
        other = inputs(commit='d' * 40)
        attempt = ledger.begin(self.root, other)
        self.assertTrue(ledger.IDENTITY.fullmatch(attempt))
        self.assertEqual(self.events()[-1]['kind'], 'start')
        self.assert_category('blocked', ledger.verify, self.root, old, inputs())
        self.assert_category('blocked', ledger.current, self.root, inputs())
        concluded = ledger.record_pass(self.root, attempt, other, {'report': str(self.report)}, {})
        self.assertEqual(ledger.current(self.root, other)['validation_id'], concluded)
        self.assert_category('blocked', ledger.current, self.root, inputs())
        new = self.passed()
        self.assertEqual(ledger.current(self.root, inputs())['validation_id'], new)
        self.assert_category('blocked', ledger.verify, self.root, old, inputs())

    def test_crashed_start_without_conclusion_keeps_blocking(self):
        old = self.passed()
        ledger.begin(self.root, inputs())
        self.passed(inputs(tree='b' * 40))
        self.assert_category('blocked', ledger.current, self.root, inputs())
        self.assert_category('blocked', ledger.verify, self.root, old, inputs())

    def test_another_attempts_pass_cannot_mask_an_unconcluded_attempt(self):
        crashed = ledger.begin(self.root, inputs())
        masking = self.passed()
        self.assert_category('blocked', ledger.verify, self.root, masking, inputs())
        self.assert_category('blocked', ledger.current, self.root, inputs())
        ledger.record_fail(self.root, crashed, inputs(), {}, {})
        self.assert_category('blocked', ledger.verify, self.root, masking, inputs())
        self.assertEqual(ledger.pending(ledger.read_events(self.root)[0], ledger.keys(inputs())[1]), set())

    def test_unconcluded_lists_only_pending_attempts_of_the_exact_inputs_read_only(self):
        self.assert_category('missing', ledger.unconcluded, self.root, inputs())
        crashed = ledger.begin(self.root, inputs())
        ledger.begin(self.root, inputs(commit='d' * 40))
        self.passed(inputs(tree='b' * 40))
        before = (self.root / 'events.jsonl').read_bytes()
        self.assertEqual(ledger.unconcluded(self.root, inputs()), [crashed])
        self.assertEqual((self.root / 'events.jsonl').read_bytes(), before)
        ledger.record_fail(self.root, crashed, inputs(), {}, {})
        self.assertEqual(ledger.unconcluded(self.root, inputs()), [])

    def test_only_a_new_complete_pass_stands_after_a_crash_is_concluded(self):
        old = self.passed()
        crashed = ledger.begin(self.root, inputs())
        ledger.record_fail(self.root, crashed, inputs(), {}, {'error': 'terminated'})
        self.assert_category('blocked', ledger.verify, self.root, old, inputs())
        self.assert_category('blocked', ledger.current, self.root, inputs())
        new = self.passed()
        self.assertEqual(ledger.current(self.root, inputs())['validation_id'], new)

    def test_start_of_other_executed_inputs_does_not_block(self):
        identity = self.passed()
        ledger.begin(self.root, inputs(tree='b' * 40))
        self.assertEqual(ledger.current(self.root, inputs())['validation_id'], identity)
        self.assert_category('mismatch', ledger.begin, self.root, {'tree': 'x'})
        self.assert_category('tampered', ledger.begin, Path('relative'), inputs())


class LockTest(LedgerTest):
    def test_fifo_in_place_of_a_ledger_file_is_rejected_without_blocking(self):
        identity = self.passed()
        index = self.root / 'events.jsonl'
        index.rename(self.base / 'events.jsonl')
        os.mkfifo(index)
        self.assert_category('tampered', ledger.read_events, self.root)
        self.assert_category('tampered', ledger.begin, self.root, inputs())
        index.unlink()
        (self.base / 'events.jsonl').rename(index)
        lock = self.root / 'lock'
        lock.unlink()
        os.mkfifo(lock)
        self.assert_category('tampered', ledger.verify, self.root, identity, inputs())


    def test_held_verification_excludes_a_concurrent_revocation(self):
        identity = self.passed()
        worker = threading.Thread(target=ledger.revoke, args=(self.root, identity, 'withdrawn'))
        with ledger.hold(self.root):
            self.assertEqual(ledger.verify(self.root, identity, inputs())['validation_id'], identity)
            worker.start()
            time.sleep(.2)
            self.assertEqual(len(self.events()), 2)
            self.assertEqual(ledger.verify(self.root, identity, inputs())['validation_id'], identity)
        worker.join(timeout=10)
        self.assertEqual([e['kind'] for e in self.events()], ['start', 'pass', 'revoke'])
        self.assert_category('blocked', ledger.verify, self.root, identity, inputs())

    def test_ledger_without_records_has_no_lock_to_hold(self):
        self.root.mkdir()
        with self.assertRaises(LedgerError) as caught:
            with ledger.hold(self.root):
                pass
        self.assertEqual(caught.exception.category, 'missing')

    def test_linked_or_non_regular_lock_and_index_are_rejected(self):
        identity = self.passed()
        target = self.base / 'target'
        target.write_text('')
        lock = self.root / 'lock'
        lock.unlink()
        lock.symlink_to(target)
        self.assert_category('tampered', ledger.begin, self.root, inputs())
        self.assert_category('tampered', ledger.verify, self.root, identity, inputs())
        lock.unlink()
        lock.mkdir()
        self.assert_category('tampered', ledger.begin, self.root, inputs())
        self.assert_category('tampered', ledger.verify, self.root, identity, inputs())
        lock.rmdir()
        index = self.root / 'events.jsonl'
        index.rename(target)
        index.symlink_to(target)
        self.assert_category('tampered', ledger.begin, self.root, inputs())
        self.assert_category('tampered', ledger.read_events, self.root)
        self.assertEqual(len(target.read_text().splitlines()), 2)


class EvidenceTest(LedgerTest):
    def test_missing_evidence_is_recoverable_and_never_recorded_as_fail(self):
        identity = self.passed()
        moved = self.base / 'moved.json'
        self.report.rename(moved)
        self.assert_category('missing', ledger.verify, self.root, identity, inputs())
        self.assertEqual([e['kind'] for e in self.events()], ['start', 'pass'])
        moved.rename(self.report)
        self.assertEqual(ledger.verify(self.root, identity, inputs())['validation_id'], identity)

    def test_changed_or_linked_evidence_is_tampering(self):
        identity = self.passed()
        self.report.write_text('{"passed":false}')
        self.assert_category('tampered', ledger.verify, self.root, identity, inputs())
        self.report.unlink()
        self.report.symlink_to(self.base / 'elsewhere.json')
        self.assert_category('tampered', ledger.verify, self.root, identity, inputs())

    def test_replaced_evidence_parent_directory_is_tampering(self):
        run = self.base / 'run'
        run.mkdir()
        report = run / 'report.json'
        report.write_text('{"passed":true}')
        identity = self.passed(report=report)
        run.rename(self.base / 'other-run')
        run.symlink_to(self.base / 'other-run')
        self.assert_category('tampered', ledger.verify, self.root, identity, inputs())


class TamperTest(LedgerTest):
    def test_edited_or_linked_record_is_rejected(self):
        identity = self.passed()
        path = self.root / 'records' / (identity + '.json')
        path.chmod(0o600)
        path.write_bytes(path.read_bytes().replace(b'run-1', b'run-2'))
        self.assert_category('tampered', ledger.verify, self.root, identity, inputs())
        path.unlink()
        path.symlink_to(self.report)
        self.assert_category('tampered', ledger.verify, self.root, identity, inputs())

    def test_deleted_reordered_or_edited_inner_events_break_the_chain(self):
        self.passed()
        self.failed(inputs())
        original = self.events()
        edits = [lambda e: e[1:],
                 lambda e: [e[1], e[0]],
                 lambda e: [dict(e[0], reason='edited'), e[1]],
                 lambda e: [e[0], dict(e[1], previous=ledger.GENESIS)],
                 lambda e: [dict(e[0], extra=1), e[1]],
                 lambda e: [e[0], dict(e[1], kind='other')]]
        for index, edit in enumerate(edits):
            with self.subTest(index=index):
                self.rewrite(edit([dict(event) for event in original]))
                self.assert_category('tampered', ledger.read_events, self.root)

    def test_partial_or_garbage_appends_are_tampering(self):
        self.passed()
        self.failed(inputs())
        path = self.root / 'events.jsonl'
        data = path.read_bytes()
        path.write_bytes(data[:-5])
        self.assert_category('tampered', ledger.read_events, self.root)
        path.write_bytes(data + b'not json\n')
        self.assert_category('tampered', ledger.read_events, self.root)

    def test_trailing_truncation_is_only_detected_against_an_anchor(self):
        self.passed()
        self.failed(inputs())
        anchor = ledger.head_of(ledger.read_events(self.root)[1])
        original = self.events()
        self.rewrite(original[:2])
        # Without an anchor a dropped trailing start+FAIL is indistinguishable (see module boundary).
        self.assertEqual(ledger.current(self.root, inputs())['sequence'], 1)
        self.assert_category('tampered', ledger.current, self.root, inputs(), anchor=anchor)
        self.rewrite([original[0], dict(original[1], reason='edited')])
        self.assert_category('tampered', ledger.current, self.root, inputs(), anchor=anchor)

    def test_anchor_accepts_extensions_and_rejects_malformed_values(self):
        identity = self.passed()
        anchor = ledger.head_of(ledger.read_events(self.root)[1])
        self.passed(inputs(tree='b' * 40))
        result = ledger.verify(self.root, identity, inputs(), anchor=anchor)
        self.assertEqual((result['sequence'], result['head']['sequence']), (1, 4))
        self.assertEqual(ledger.verify(self.root, identity, inputs(), anchor={'sequence': 0, 'digest': ledger.GENESIS})['sequence'], 1)
        for bad in [{'sequence': 5, 'digest': ledger.head_of(ledger.read_events(self.root)[1])['digest']},
                    {'sequence': 3, 'digest': ledger.GENESIS}, {'sequence': -1, 'digest': ledger.GENESIS},
                    {'sequence': True, 'digest': ledger.GENESIS}, {'sequence': 1}, [], 'head',
                    {'sequence': 1, 'digest': 'f' * 64}]:
            with self.subTest(anchor=bad):
                self.assert_category('tampered', ledger.verify, self.root, identity, inputs(), anchor=bad)

    def test_index_that_binds_a_pass_to_other_inputs_is_tampering(self):
        identity = self.passed()
        for field in ('input_key', 'execution_key'):
            with self.subTest(field=field):
                start, admitted = self.events()[:2]
                self.rewrite([start, dict(admitted, **{field: 'f' * 64})])
                self.assert_category('tampered', ledger.verify, self.root, identity, inputs())

    def test_record_absent_from_index_is_not_admitted(self):
        attempt = ledger.begin(self.root, inputs())
        with patch.object(ledger, 'append'):
            identity = ledger.record_pass(self.root, attempt, inputs(), {'report': str(self.report)}, {})
        self.assert_category('missing', ledger.verify, self.root, identity, inputs())
        self.assert_category('blocked', ledger.current, self.root, inputs())

    def test_record_from_another_schema_is_rejected(self):
        (self.root / 'records').mkdir(parents=True)
        (self.root / 'lock').write_text('')
        data = ledger.canonical({'schema': 'other', 'outcome': 'pass', 'inputs': inputs()})
        identity = ledger.digest(data)
        (self.root / 'records' / (identity + '.json')).write_bytes(data)
        self.assert_category('mismatch', ledger.verify, self.root, identity, inputs())


class DurabilityTest(LedgerTest):
    def test_new_records_index_and_directories_are_synced(self):
        with patch.object(ledger.os, 'fsync', wraps=os.fsync) as fsync:
            self.passed()
        self.assertGreaterEqual(fsync.call_count, 3)


if __name__ == '__main__':
    unittest.main()
