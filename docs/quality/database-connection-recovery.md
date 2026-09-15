# Fixture connection recovery

GH-16 blocked on 2026-09-15 before implementation: the managed SOCKS database
connection timed out or returned reply 5 (connection refused). Host TCP and
PostgreSQL checks were healthy when inspected; the original initiating event was
not established. The same sandbox's full verification subsequently passed.

The host-owned database launcher now retries transient connection establishment
for at most 30 seconds, with individual attempts capped at five seconds and
backoff capped at two seconds. Policy/authentication rejection fails immediately.
SQL and child commands are never retried. Persistent outages still fail with the
fixture address, attempt count and last error.

Provisioning refreshes the launcher atomically even for existing workspaces.
The fix was installed in the host template and GH-12 through GH-16 environments.
Original installed files are backed up under Symphony's db-retry-recovery-*.
No network allowlists, credentials or quality thresholds changed.

Validation: 12 environment/retry tests passed; full GH-16 sandbox verification
passed, including test/dev SQL and fixture recreation. A real sandbox probe
injected refusal and timeout before connecting through the managed proxy and
successfully executing SELECT 1. Evidence is in GH-16's
artifacts/gh16-preparation/operator-retry-validation.json. This demonstrates
recovery from the reproduced transient failures, not immunity to lasting outages.

Run regression checks with:

```sh
python3 -m unittest discover -s tools/symphony -p 'test_*.py' -v
```
