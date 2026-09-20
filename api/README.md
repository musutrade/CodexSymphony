# Initial HTTP contract

`openapi.json` declares health plus repository and Requirement operations. Health
retains JSON responses for HTTP 200 and 503. `baseline.json` establishes the initial reviewed
contract for future comparisons; it is not evidence about an earlier API release.
The trusted host pins the baseline separately from the current source checkout.
Changes to that baseline require policy review and cannot be accepted by a PR's
own test scripts. The collector also compares the Angular consumer's actual AST
and verifies responses from a running backend against this contract.

## Business contract capture

The rc.4 HTTP collector generates all response/request types into the existing
configured generated type file. `getHealth` remains `HealthResponse`; other
operationIds use PascalCase plus `Response` / `Request`. Use canonical Angular
`HttpClient` injection with an explicit generated response type per call.

Declare business capture sequences in `api/capture-scenarios.json` as data:

```json
[
  {"id":"created","method":"POST","path":"/api/requirements","status":201,
   "body":{"title":"Synthetic requirement"}},
  {"id":"edited","method":"PATCH","path":"/api/requirements/{id}","status":200,
   "path_parameters":{"id":{"$response":"created#/id"}},
   "body":{"version":{"$response":"created#/version"},"title":"Edited"}}
]
```

Replace the example bodies with the complete actual Contract. Each path parameter
must be declared in OpenAPI. The trusted host executes requests only against its
newly built isolated localhost API. It supplies the accepted API Origin and
`x-codexsymphony-csrf: 1`. To exercise rejection cases, `headers` may override
Origin/CSRF or remove them using null; Idempotency-Key and If-Match are also
supported. Arbitrary hosts, redirects, shell commands and other headers are rejected.

Every declared operation/status must be observed exactly once. Setup requests may
use `"record": false`; another recorded response must still cover that variant.
The existing health 200/503 probes remain host-owned and must not be repeated in
this file. Scenario bytes, client source inventory, live observations and the built
binary are bound into the new signed capture. A scenario list is not evidence
until those requests have actually run. Unknown schema/client features still fail.

## GH-13 control semantics

Every write requires the configured Host/Origin and `x-codexsymphony-csrf: 1`.
Bodies reject unknown fields. `request_id` binds an operation, object and complete
input; reusing it for different input returns 409. `version` is CAS (0 on create).
Ready additionally checks `repository_version`; withdraw carries that field for
the same control envelope but only checks the Requirement version.

Use GET `/api/repository` to read the singleton registration, policy version and
explicitly unavailable downstream capabilities. PUT registers version 0 or updates
the same remote identity with a reason. POST `/api/requirements` creates a complete
Draft; GET lists and GET `/{id}` reads Contract and snapshots. PATCH `/{id}` is
Draft-only. POST `/{id}/ready` freezes input/policy; POST `/{id}/withdraw` returns
Ready to Draft and preserves snapshots. No Start or Run action exists here.

`authorization_valid` is a fresh-read indicator, not an execution permission token.
Safety revocation permanently invalidates earlier snapshot policy versions.
Restore alone does not revive them; withdraw and review the new policy explicitly.
`creator`/`reviewer` are local-user attribution until M2 authentication is implemented.

Types are regenerated with the installed rc.4 CLI, then formatted using Prettier
and ESLint's existing `--fix` rules on `health-response.ts`. The collector checks
structural equivalence, including interface/type and array notation conversions.
Common middleware rejections (403) have no JSON body; JSON/schema rejection (422)
and business rejection responses use an `error` string. See
[local validation and trusted-host boundary](../docs/quality/gh13/README.md).

## GH-61 advisory generation

`POST /api/draft-generations` accepts an explicit idempotent request and returns a persisted generation record. GET collection/item only read state. Generation has its own usage and bounded Runtime turn; it never grants Ready, creates an AgentRun or writes a PR. Input version CAS protects concurrent Draft edits. See [generation semantics](../docs/draft-generation.md). Contract capture replays a clearly marked terminal SQL fixture and does not call a real model. AC01 real calls are preserved separately.

## GH-62 group review

- GET `/api/drafts/{id}/review` returns the current Draft revision, saved review
  version, parent/children, repository policies, cumulative ledgers, immutable
  authorization history and the unique group queue. Empty review version is 0.
- PUT the same route with `version`, `draft_revision`, `review` saves a CAS review
  revision. Incomplete mappings may be saved for editing but confer no authority.
- POST `/api/drafts/{id}/authorize` with `request_id`, `version`, `draft_revision`
  atomically validates and freezes the exact review/content/policy/budget snapshot
  and upserts one group queue entry. No child Start calls or executable legacy
  requirements are created. `waiting_scheduler` is explicit and
  `business_complete=false` is never inferred from coverage.

All parent ACs are required. Review references bind child IDs, AC IDs, machine
step IDs and the immutable shared Draft revision. `full_chain_acs` records the
user's semantic classification; final integration dependencies are checked.
Repository policy versions are rechecked under the existing review/revocation
lock. `group_budget=null` uses the sum of approved item amounts. A lower explicit
cap is allowed; every amount preserves cumulative used/reserved values across
revision and reauthorization. The empty ledger item_id denotes the group.

404 means the Draft is absent, 409 means conflicting versions/request identity or
an already-authorized review, and 422 means malformed input or failed review
validation. Database failure returns 503 with no partial authorization committed.
The idempotent confirmation response is an original receipt; GET is the current
queue truth after later edits. Authenticated identities remain local-user until M2.
