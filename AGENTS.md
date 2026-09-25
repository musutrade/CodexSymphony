# Repository instructions

## Temporary Rust closure convention

Until the approved source collector has verified complete closure support:

- Do not introduce closures in production Rust code, including `|...|`, `move |...|`, and async closures. When editing a production function, replace its closures with equivalent explicit control flow or ordinary named functions.
- Existing, unchanged production functions are outside this migration requirement. Do not rewrite the repository in bulk. Test-only code may use closures; keep production logic in the measured production inventory.
- Preserve behavior, error propagation, short-circuiting, ordering and authority boundaries. Do not move logic into macros, generated code, excluded files or test code to evade measurement.
- Refactor complex code into cohesive named functions and add meaningful tests as needed. CRAP must remain <= 10, required coverage remains unchanged, and missing or unsupported measurements remain failures. Do not add exclusions, claim N/A, infer closure counts from parent execution, or reuse another source's evidence.
- Source-inventory compatibility is only a preliminary check. Changed code must produce exact source-bound LLVM coverage/CRAP measurements and pass the complete Gate on the final exact tree before publication.

This temporary convention addresses observed rc.7 mapping gaps; it does not assert that all closures fail or that ordinary Rust compilation is unsupported. Remove it only after a reviewed collector upgrade passes retained regression cases for field-projection closures, unit-return closures, macro-bodied closures, captures, async closures, and generic monomorphizations, with exact counters and source-bound measurements, followed by a complete Gate PASS. A version change alone does not lift the rule.
