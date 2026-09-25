# Complete retained Gate quality diagnostics

Read `all-failures.json` for every failing subject, source span/hash, measurement and threshold (11 failures, 10 subjects). `test_result.md`, `verify.stdout` and `verify.stderr` are complete and untruncated. `cargo-coverage.json.gz` contains full native LLVM coverage; open with Python gzip/json or gzip -dc. `backend-bundle.json.gz` retains source/capture identities. `source/` contains the exact failed source snapshots; do not overwrite newer implementation with them. `evidence.json` binds original and exported hashes. Redaction uses the standard host exporter.

Diagnose and fix the demonstrated coverage/CRAP failures; do not lower thresholds, exclude production code or discard failure history. Refresh source-bound evidence and require a new complete exact-tree Gate PASS before publication.
