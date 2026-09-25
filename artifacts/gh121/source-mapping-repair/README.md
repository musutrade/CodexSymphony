# GH-121 exact LLVM mapping repair

Product commit: `8f5ed89af8d098bf34f9d816fdcf50d9252f2ef4`
Product tree: `78bb742caf4221fbcd8901343d2307f0a6126fb4`

The retained prior run `run-0d758608b081` had successful native coverage execution but could not produce source measurements. LLVM did not expose individual entry records for field-projection and unit-return closures. The macro-bodied identity predicate's native record began at `[44,32]`, outside the collector's approved closure anchors. Parent execution is not a valid substitute for these missing counters.

This repair changes production source, not the installed rc.7 collector or its policy:

- `github_credentials::separate_check_identity` uses explicit loops with the same required/pre/post check order and identity exclusion.
- `github_merge_adapter::submit` discards the successful reply after `await?`, preserving the original error.
- `github_publication_adapter::submit` preserves the existing swallowed remote-error branch and propagates all other errors with `?`, without a unit-return closure.
- A new test checks identity separation for both contract phases. Existing full coverage tests exercise publication/merge behavior.
- Root `AGENTS.md` and the workflow add the user-requested temporary prohibition on new production closures or closures in modified production functions. Tests may use them; unrelated historical functions are not a mass rewrite. No excluded logic or lower thresholds is allowed. The active host workflow was updated too.

The complete backend capture is retained under host operator `gh121-collector-compatibility/mapping-run-v2`. It uses the current approved Gate isolation/database setup, original source collector, fresh immutable build snapshot, complete cargo llvm-cov suite and retained native objects/counters. The first local capture launcher rejected a pre-created empty cache destination before test execution; the second uses a fresh run and proper approved cache restore. No failed measurements were reused.

The summary and compressed measurements in this directory contain exact source-bound measurements and all local threshold candidates, not a signed complete Gate verdict. The real delivery fixture for this new source is separate from PR #7; read `../host-real-core-8f5ed89-acceptance.json` and `../real-delivery-core-8f5ed89/files.json` (finalized: PR #8, pinned test-job Success).

After inspecting all evidence, finalize declarations and run a NEW complete `local_gate`. Coverage/CRAP violations are code/test work to repair without lowering thresholds. Publication still requires the complete final exact-tree PASS. Further source edits invalidate applicable source-bound evidence.
