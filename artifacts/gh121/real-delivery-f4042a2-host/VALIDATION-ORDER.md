# GH-121 continuation order

The reviewed fixture is bound to f4042a278642e071115a6a90ff0b838da1b792a9. Its receipt is real-delivery evidence for that source only, not coverage/CRAP or complete Gate acceptance.

1. Inspect the current receipt, finalize its evidence, and run complete local_gate for this source.
2. If Gate exposes mapping or quality failures, repair source/tests under unchanged policy. Keep production logic in production roots and test-only modules in the established tests/unit layout. Do not introduce production closures in modified functions.
3. Run complete local_gate as a development diagnostic to resolve all mapping/measurement/quality failures before requesting another host rebind. Do not block merely because the current fixture still binds an earlier source while quality development continues. Never claim that old fixture certifies the changed source.
4. Once source stabilizes with successful measurement/quality verification, freeze it and request the applicable reviewed credential-isolated fixture rebind. Do not request intermediate rebinding solely for each code edit.
5. After final real acceptance and all evidence changes, require a NEW complete exact-tree local_gate PASS before publication. Earlier diagnostic results cannot authorize a different tree. Keep all failure history and budgets.

This changes work order only; it does not weaken acceptance, source binding, credential isolation, Gate thresholds or publication admission.
