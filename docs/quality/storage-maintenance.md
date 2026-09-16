# Host disk maintenance

Install as the trusted host operator:

```sh
python3 tools/install_storage_maintenance.py
systemctl --user status codexsymphony-storage.timer
cat ~/.local/share/codexsymphony/storage-maintenance/state.json
```

The installer creates a content-addressed release outside issue workspaces, a
15-second systemd user timer, and persistent startup conditions for the Symphony
and remote Gate services. It does not change approved measurement code, policy,
signatures or thresholds. Reinstalling either main service preserves its drop-in.

## Cache lifecycle

The successful Gate runner already deletes its `target`. Maintenance also removes
historical, failed and interrupted runs' `gate-host/runs/run-<12 hex>/target`
directories after a five-minute grace period, regardless of receipt status.
Before deletion it checks same-user processes for Gate launchers, run arguments,
working directories and open descriptors, including orphan workers. Any active
Gate or unreadable worker defers collection. Known OS session managers (systemd,
PAM and sshd) are excluded from descriptor scanning; their children are still
checked independently. UUID directories are never reused
by the Gate launcher. Operators must not manually resume a compiler in an old
retained run; start a fresh Gate run instead.

Only build caches are removed. Raw probe objects/profiles, signatures, approvals,
reports, HTTP binaries, source snapshots, remote receipts, workspace source/evidence and
the shared `~/cargo-target` are retained. Root symlinks and nested mounts are
rejected; nested symlinks cannot redirect recursive deletion. A process lock
prevents overlapping maintenance runs. `state.json` records the last check;
systemd's journal records each cleanup result.

## Disk pressure

Below **12 GiB available**, maintenance stops active managed services before
collection. Stop may interrupt an in-flight turn or check; normal controller and
remote Gate interruption recovery applies, and no interrupted check becomes a
pass. Only services stopped by this guard are remembered and automatically
restarted once available space reaches **20 GiB**. Startup conditions also reject
manual/service restarts below 20 GiB. Intermediate space does not cause restart
loops. The persistent state survives maintenance restarts.

This bounds managed writes through backpressure, not evidence deletion. Evidence
still grows; if it consumes the reserve, scheduling pauses until the operator
archives evidence or increases capacity. Other applications and manually launched
builds are outside these service controls. A 15-second monitor is not a filesystem
quota and cannot guarantee against arbitrary writers exhausting the disk.

## Validation and incident

```sh
python3 -m unittest discover -s tools/tests -p 'test_storage_maintenance.py' -v
journalctl --user -u codexsymphony-storage.service -n 20
```

On 2026-09-15, 19 inactive Gate build caches held 39.64 GiB. Their removal
restored about 40 GiB available from 52 MiB. The host audit is
`gate-host/cache-cleanup-20260915.json`. GH-13 Actions attempt 34961156132/1
failed copying native evidence with `OSError: [Errno 28] No space left on device`;
its retained diagnostics were preserved. A new check is needed after recovery;
this maintenance does not rewrite its result as successful.

## Workspace cache recovery (2026-09-16)

Maintenance also collects `target/debug` and coverage-build `debug/incremental`
from completed issue workspaces. Waiting/blocked workspaces are eligible only
while Symphony is stopped. Active process references, symlink ancestors and
nested mounts prevent collection. Fixed fixture brokers may keep their request
spool open; they are excluded, while their child processes remain scanned.
Coverage binaries, raw profiles, reports and issue source files are retained.

The remote Gate reconciles unfinished host receipts under its exclusive service
lock, including checks whose Actions jobs have already timed out. Interrupted
checks become failures requiring a fresh attempt; they never become successes.
This prevents an abandoned `in_progress` check after a disk-guard stop.
