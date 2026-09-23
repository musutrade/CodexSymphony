#!/usr/bin/env python3
"""Summarize retained gate timings and Rust test output without running the gate."""
import argparse
from datetime import datetime, timedelta
import json
from pathlib import Path
import re


DEFAULT_RUNS = Path.home() / '.local/share/codexsymphony/gate-host/runs'
TEST_START = re.compile(r'^running (\d+) tests?$')
TEST_END = re.compile(r'^test result: .*?(\d+) passed; (\d+) failed; (\d+) ignored;.*finished in ([0-9.]+)s$')
TEST_BINARY = re.compile(r'^\s+Running (?:unittests |tests/)([^ (]+)')
COMPILE = re.compile(r'^\s+Finished `test` profile .* target\(s\) in (.+)$')
UNIT = re.compile(r'([0-9.]+)([hms])')


def seconds(value):
    units = {'h': 3600, 'm': 60, 's': 1}
    parts = UNIT.findall(value)
    if not parts or ''.join(number + unit for number, unit in parts) != value.replace(' ', ''):
        raise ValueError('unrecognized Cargo duration: ' + value)
    return sum(float(number) * units[unit] for number, unit in parts)


def rust_stages(output):
    """Cargo stderr lists binaries; stdout contains nested test harness output."""
    stderr = (output / 'capture.stderr').read_text(errors='replace').splitlines()
    stdout = (output / 'capture.stdout').read_text(errors='replace').splitlines()
    compiles = [seconds(match.group(1)) for line in stderr if (match := COMPILE.match(line))]
    binaries = [match.group(1) for line in stderr if (match := TEST_BINARY.match(line))]
    stack = []
    durations = []
    for line in stdout:
        if match := TEST_START.match(line):
            stack.append(int(match.group(1)))
        elif (match := TEST_END.match(line)) and stack:
            executed = sum(int(match.group(index)) for index in (1, 2, 3))
            if executed not in stack:
                raise ValueError('Cargo test result does not match a running harness')
            # Child subprocesses may exit before printing a result. Their
            # parent still prints its own complete harness summary.
            while stack.pop() != executed:
                pass
            if not stack:
                durations.append(float(match.group(4)))
    if stack or len(durations) != len(binaries):
        raise ValueError(f'Cargo test output mismatch: {len(binaries)} binaries, {len(durations)} results')
    return {
        'compile_seconds': round(sum(compiles), 2) if compiles else None,
        'test_harness_seconds': round(sum(durations), 2),
        'slowest_test_binaries': [
            {'name': name, 'seconds': duration}
            for duration, name in sorted(zip(durations, binaries), reverse=True)[:5]
        ],
    }


def inspect(run):
    rows = [json.loads(line) for line in (run / 'timings.jsonl').read_text().splitlines()]
    started = min(datetime.fromisoformat(row['started_at']) for row in rows)
    ended = max(datetime.fromisoformat(row['started_at']) + timedelta(milliseconds=row['duration_ms']) for row in rows)
    phases = {row['phase']: row['duration_ms'] / 1000 for row in rows}
    cache_file = run / 'cache-restore.json'
    verify_file = run / 'verify-result.json'
    result = {
        'run': run.name,
        'phase_span_seconds': round((ended - started).total_seconds(), 2),
        'cache_hit': json.loads(cache_file.read_text()).get('hit') if cache_file.exists() else None,
        'verify_exit': json.loads(verify_file.read_text()).get('exit') if verify_file.exists() else None,
        'backend_capture_seconds': round(phases['backend-capture'], 2) if 'backend-capture' in phases else None,
    }
    output = run / 'probes/backend'
    if result['backend_capture_seconds'] is not None and (output / 'capture.stderr').exists() and (output / 'capture.stdout').exists():
        try:
            stages = rust_stages(output)
        except ValueError as error:
            # A failed or interrupted capture may leave incomplete Cargo logs.
            result['rust_stage_error'] = str(error)
        else:
            result.update(stages)
            if stages['compile_seconds'] is not None:
                # This also includes Cargo startup, test-harness gaps and coverage export.
                result['other_backend_seconds'] = round(result['backend_capture_seconds'] - stages['compile_seconds'] - stages['test_harness_seconds'], 2)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('runs', type=Path, nargs='*', help='retained run directories')
    parser.add_argument('--runs-root', type=Path, default=DEFAULT_RUNS)
    parser.add_argument('--limit', type=int, default=5, help='recent runs to inspect when none are named')
    args = parser.parse_args()
    if args.limit < 1:
        parser.error('--limit must be positive')
    runs = args.runs or sorted(
        (path for path in args.runs_root.glob('run-*') if (path / 'timings.jsonl').exists()),
        key=lambda path: path.stat().st_mtime, reverse=True,
    )[:args.limit]
    for run in runs:
        print(json.dumps(inspect(run), ensure_ascii=False))


if __name__ == '__main__':
    main()
