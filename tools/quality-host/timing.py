"""Retained host phase timings, including failures; never record environment values."""
from contextlib import contextmanager
from datetime import datetime, timezone
import json
import time


@contextmanager
def phase(run, name):
    started = time.monotonic()
    record = {'phase': name, 'started_at': datetime.now(timezone.utc).isoformat(), 'status': 'FAIL'}
    try:
        yield
        record['status'] = 'PASS'
    finally:
        record['duration_ms'] = round((time.monotonic() - started) * 1000)
        with (run / 'timings.jsonl').open('a') as output:
            output.write(json.dumps(record) + '\n')
