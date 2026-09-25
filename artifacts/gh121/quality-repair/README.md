# GH-121 delivery quality repair

This continuation preserves the implementation and historical evidence from
`8f5ed89af8d098bf34f9d816fdcf50d9252f2ef4`. The retained prior exact LLVM diagnostic
reported 15 subjects violating required coverage or CRAP thresholds. It did not
report a complete Gate PASS.

## Source changes

- Split repository target validation, generic submit admission, reviewed registry
  resolution, Hook call construction, durable invocation claiming, result admission,
  process intent/replay/heartbeat handling, and per-invocation recovery into named
  functions. Preserve validation order, short-circuiting, original invocation IDs,
  persist-before-send, retained errors, and frozen inputs.
- Separate native GitHub merge evidence checks and publication operation dispatch.
  Replace edited production closures with explicit control flow or named functions.
- Reap the existing std child through a named async polling function; process stop
  authority still comes from the existing supervisor's exact quiescence receipt.
  The detached reaper never supplies a stop proof or permission to retry.
- Add controlled tests for startup/stop deadlines, reaping, registry substitution,
  original merge reconciliation, missing stop proofs, recovery identity mismatch,
  cancellation while a delivery Hook runs, credential/configuration failures, and
  a matching remote lease whose nonexistent ancestor is rejected by local Git
  before a network write.

No quality policy, collector, scope exclusions, CI workflow, database migration,
external write permission or production deployment configuration was changed.
Unchanged legacy functions remain outside the temporary closure migration.
Migration/rollback remains `docs/delivery-extensions.md`; original frozen tasks,
unknown operation records and cumulative budgets are retained. Actual local_git
execution remains GH-105.

## Evidence interpretation

`preliminary-inventory.json` comes from the checked-in rc.7 collector archive's
inventory executable. It accepts all 138 production files and bounds changed
function complexity at 10. This is only source compatibility/complexity analysis;
it is NOT LLVM coverage, a CRAP result, or a complete Gate verdict.

`source-inventory.json` binds the final source/config/test/documentation bytes and
modes. Final test outcomes and hashes are recorded in `summary.json` after checks
finish. `workspace-tests.log` is an earlier passing diagnostic before final test
additions and the last decomposition. `focused.log` is invalid as passing evidence:
its fixture overlapped a subsequent fixture recreation. Earlier unit compilation
errors were fixed without adding dependencies. `workspace-tests-cancellation-placement.log`
retains the new cancellation test placed before later success assertions: production
correctly invalidates validation permanently on cancellation. The test was moved
after those assertions and now also asserts durable invalidation. Those logs remain retained.

The existing real-core acceptance receipt binds product commit 8f5ed89. It cannot
certify this changed source. The supplied `REAL-DELIVERY-RUNNER.md` requires host
candidate installation/rebinding before a new real-core regression. The host must
build/review without credentials and retain the bounded disposable repository,
credential boundary, original intent ledger, and migrated-core submit path.

After source-bound real regression, finish the evidence manifest and run a NEW
complete exact-tree local_gate. Inspect all LLVM/coverage/CRAP results and repair
actual failures without weakening policy. No final Gate PASS, publication or CI
acceptance is claimed here.
