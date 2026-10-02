"""Synthetic local receiver: persist once, then lose the first response."""
import json
import os
import sqlite3
import sys

event = json.load(sys.stdin)
assert not any(name.startswith('PG') for name in os.environ)
with sqlite3.connect(sys.argv[1]) as connection:
    connection.execute('CREATE TABLE IF NOT EXISTS inbox(event_id INTEGER PRIMARY KEY, payload TEXT NOT NULL)')
    connection.execute('INSERT OR IGNORE INTO inbox VALUES(?,?)',
                       (event['event_id'], json.dumps(event, sort_keys=True)))
if event['attempt'] == 1:
    # Durable acceptance happened; the caller receives no acknowledgement.
    raise SystemExit(1)
print(json.dumps({'protocol_version': 1, 'event_id': event['event_id'], 'status': 'accepted'}))
