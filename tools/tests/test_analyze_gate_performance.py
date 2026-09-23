import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


script = Path(__file__).parents[1] / 'analyze_gate_performance.py'
spec = importlib.util.spec_from_file_location('analyze_gate_performance', script)
analyzer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(analyzer)


class GatePerformanceAnalysis(unittest.TestCase):
    def test_nested_harness_output_and_incomplete_child(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp)
            (output / 'capture.stderr').write_text(
                '    Finished `test` profile [unoptimized] target(s) in 1m 52s\n'
                '     Running tests/one.rs (binary)\n'
                '     Running tests/two.rs (binary)\n'
            )
            (output / 'capture.stdout').write_text(
                'running 2 tests\n'
                'running 1 test\n'
                'test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out; finished in 0.25s\n'
                'test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.00s\n'
                'running 3 tests\n'
                'running 1 test\n'  # A child exits without a summary.
                'test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 7.50s\n'
            )
            self.assertEqual(analyzer.rust_stages(output), {
                'compile_seconds': 112.0,
                'test_harness_seconds': 11.5,
                'slowest_test_binaries': [
                    {'name': 'two.rs', 'seconds': 7.5},
                    {'name': 'one.rs', 'seconds': 4.0},
                ],
            })

    def test_run_span_excludes_nested_phase_double_counting(self):
        with tempfile.TemporaryDirectory() as tmp:
            run = Path(tmp) / 'run-example'
            run.mkdir()
            rows = [
                {'phase': 'capture-all', 'started_at': '2026-09-23T00:00:00+00:00', 'duration_ms': 10000},
                {'phase': 'backend-capture', 'started_at': '2026-09-23T00:00:01+00:00', 'duration_ms': 8000},
                {'phase': 'verify', 'started_at': '2026-09-23T00:00:10+00:00', 'duration_ms': 5000},
            ]
            (run / 'timings.jsonl').write_text(''.join(json.dumps(row) + '\n' for row in rows))
            (run / 'cache-restore.json').write_text('{"hit":true}\n')
            (run / 'verify-result.json').write_text('{"exit":0}\n')
            self.assertEqual(analyzer.inspect(run), {
                'run': 'run-example', 'phase_span_seconds': 15.0, 'cache_hit': True,
                'verify_exit': 0, 'backend_capture_seconds': 8.0,
            })


if __name__ == '__main__':
    unittest.main()
