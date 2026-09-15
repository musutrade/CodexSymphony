# Initial HTTP contract

`openapi.json` declares the current GET `/api/health` operation, including JSON
responses for HTTP 200 and 503. `baseline.json` establishes the initial reviewed
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
