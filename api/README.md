# Initial HTTP contract

`openapi.json` declares the current GET `/api/health` operation, including JSON
responses for HTTP 200 and 503. `baseline.json` establishes the initial reviewed
contract for future comparisons; it is not evidence about an earlier API release.
The trusted host pins the baseline separately from the current source checkout.
Changes to that baseline require policy review and cannot be accepted by a PR's
own test scripts. The collector also compares the Angular consumer's actual AST
and verifies responses from a running backend against this contract.
