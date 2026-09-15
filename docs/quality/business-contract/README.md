# Business contract and task environment recovery

GH-13 was blocked before implementation because HTTP collector rc.3 accepted only
static GET health responses and one generated Angular type. The rc.4 candidate
([source PR #273](https://github.com/musutrade/Harness-Gate/pull/273)) supports closed
nested JSON, numeric versions, POST/PUT/PATCH/DELETE, path parameters, generated
request/response types and all canonical Angular HttpClient consumers.

`api/capture-scenarios.json` supplies data-only requests to the host-built isolated
API. It must cover every declared operation/status; source, binary, observations
and client inventory are bound into fresh evidence. See `api/README.md` for the
scenario and supported-client contract. New business routes still need actual
implementation, scenarios and tests. Unknown features fail closed.

Rust capture now runs all test targets in the server package instead of the fixed
health/startup pair, so future business integration tests contribute coverage.
No coverage/CRAP thresholds or required contract metrics were reduced.

The complete local isolated CI gate passed with 3 producers and 34 evidence
records. `local-acceptance.json` binds the exact candidate source inputs and
retained report; this was a working-tree acceptance, not a claim that GH-13
business behavior is implemented. The final PR receives fresh exact-head remote
checks after host installation. Old approval files and collector versions remain.

The prior per-issue database/source/cache provisioning fix is now also checked in;
see `docs/symphony-environment-provisioning.md`. The remote installer includes the
approved host and dependency location in its installation identity, allowing a
new reviewed collector approval without overwriting an immutable configuration.
