# GH-121 test layout repair

Gate run-0fc9aa8603b4 produced LLVM coverage but failed exact mapping of inline cfg(test) functions. The complete exported diagnostics are retained here with the original manifest and source identities. All flagged functions are test-only. Three modules now follow the existing tests/unit layout; production function contents and test bodies are unchanged, with no measurement exclusion or policy modification.

The prepared full Rust suite passed 310 top-level tests across 39 reported suites, including real Runtime. Formatting, strict Clippy and Gate config diagnostics passed. The relocation is not yet certified by a new complete Gate.

The real normal GitHub receipt for cae3b48 remains valid only for its named source. The supplied host runner instructions require the new exact commit/tree to be reviewed and rebound before new-source real acceptance. Resume there, then finalize evidence and run complete local_gate; repair remaining measured failures without changing thresholds. No remote publication, CI acceptance, merge or production deployment is claimed. Historical evidence and budgets are preserved.
