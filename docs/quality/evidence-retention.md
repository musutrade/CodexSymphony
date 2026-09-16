# Evidence retention

The operator accepts rebuilding historical commits instead of retaining every native
binary and raw coverage profile forever. This does not change Gate acceptance criteria.
A historical PASS remains a historical result; rebuilding creates a new result and is
not a claim to reproduce the original runtime or binary byte for byte.

## Installed policy

The hourly `codexsymphony-archive` service now runs `compact_gate_evidence.py`:

- Retain rebuildable backend raw files and HTTP server binaries for at most the newest
  two runs, within 24 hours, with a 4 GiB aggregate limit. Active runs and recently
  finished runs are exempt until safe to collect. Completed runs have a five-minute
  grace period; incomplete runs have a one-hour grace period and require no live workers.
- Keep final report JSON/Markdown, source identity, input/lockfile hashes, signed
  evidence and diagnostic logs. Before removing report subdirectories, preserve all
  report details plus signatures and capture metadata in `review-record.tar.gz`.
  Verify every archived file by SHA256 before deleting anything.
- Keep `compact-retention.json` with the archive hash, file hashes and original sizes,
  deleted paths, and an explicit statement that original raw payloads are unavailable.
  An interrupted deletion resumes against the original verified review package.
- Stop creating new cold archives of rebuildable binaries. Existing cold archives
  expire after seven days or above 4 GiB (oldest first); retain their original manifests
  and an `.expired.json` tombstone. Expired raw archives cannot be restored.
- Cache cleanup remains separate. This policy does not delete source worktrees, Git
  history, host approvals, tool installations, or historical host acceptance fixtures.
  The 4 GiB limits apply to raw payloads, not all Symphony data.

Review records are much smaller but still grow with the number of runs; their long-term
storage is not advertised as constant or unlimited. Monitor actual retained report
volume before adding a separate policy for those essential records.

## Review and recovery

`python3 tools/compact_gate_evidence.py` prints a dry-run plan. `--apply` applies it.
The service requires both the runtime SSD bind mount and `/data` to exist. It shares
an exclusive lock with the old archive tool and refuses symlink/mount traversal.

To examine compacted report details, extract `review-record.tar.gz` into a new review
directory, retaining `compact-retention.json` to verify the recorded file hashes.
The top-level original final report remains at its existing path for diagnostics.
For code reproduction, fetch the commit recorded in `source-inputs.json`/the final
report, use its checked-in lockfiles and recorded tool/pipeline versions, and rerun Gate.
Uncommitted or environment-dependent behavior may not be reproducible from Git alone;
retained input hashes and logs expose these limits rather than asserting equivalence.
