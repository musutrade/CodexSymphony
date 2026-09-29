import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch


SPEC = importlib.util.spec_from_file_location('gate_selftest_entry',
                                            Path(__file__).parents[1] / 'gate_selftest.py')
entry = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(entry)


class GateSelftestTests(unittest.TestCase):
    def test_ledger_regressions_are_required_and_run_in_isolated_interpreters(self):
        with patch.object(entry.subprocess, 'run') as run:
            entry.main()
        patterns = [call.args[0][-2] for call in run.call_args_list]
        self.assertIn('test_evidence_ledger.py', patterns)
        self.assertIn('test_gate_selftest.py', patterns)
        for call in run.call_args_list:
            self.assertEqual(call.args[0][1:5], ['-B', '-m', 'unittest', 'discover'])
            self.assertIs(call.kwargs['check'], True)

    def test_missing_required_suite_fails_before_running_any_test(self):
        with tempfile.TemporaryDirectory() as directory:
            with patch.object(entry, 'ROOT', Path(directory)), patch.object(entry.subprocess, 'run') as run:
                with self.assertRaisesRegex(ValueError, 'missing gate regression suite'):
                    entry.main()
                run.assert_not_called()

    def test_failed_suite_stops_subsequent_suites(self):
        error = subprocess.CalledProcessError(1, ['fixture-test'])
        with patch.object(entry.subprocess, 'run', side_effect=error) as run:
            with self.assertRaises(subprocess.CalledProcessError):
                entry.main()
            self.assertEqual(run.call_count, 1)
