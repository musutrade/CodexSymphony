# Complete run-98a73620b935 diagnostics

All 693 entries in source-inventory-test-layout.json match the Gate source-inputs manifest. All failing subjects also match current workspace files and retained source snapshots.

Read all-failures.json for the full four failures on three subjects: accepted line 7/9 and region 10/13; settle region 17/25; decode_retained region 18/23. All require 80%. No other failures are listed in the complete test_result.md. verify.stdout/stderr are untruncated. Native LLVM coverage is cargo-coverage.json.gz; open with Python gzip/json. backend-bundle.json.gz and evidence.json preserve capture identities and original/export SHA-256. source/ holds the exact failed source snapshots for inspection, not overwriting newer work.

Fix demonstrated coverage failures, refresh current-source checks/evidence and require a new complete exact-tree PASS before publication. Preserve historical failures and policy.
