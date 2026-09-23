# Project hook examples

These configurations exercise the reviewed P2/P3 hook path. `no-database.json`
prepares a Git workspace without asking for a project database. The
`optional-test-db.json` variant checks a separately provisioned PostgreSQL test
service with `psql` during `before_run`. The platform's own PostgreSQL remains
independent of either choice.

Install a reviewed, immutable copy of `project_hook.py` at
`/opt/codexsymphony/project-hooks/project_hook.py`. Verify its SHA-256 is
`7c219e3b31c7378fbd71099b70c6a805deb15e9d9993196afa97b389bec174df`.
The script needs the listed interpreter and, for the database variant, `psql`
and `TEST_DATABASE_URL` in the trusted development environment. These examples
do not supply credentials.

Put the selected JSON `hooks` array in the **repository review document** and
the identical array in the operator-owned Runtime configuration under
`preparation.hook_allowlist`. Set `preparation.dependencies` to `[]` for these
projects. The product also suppresses legacy project dependency probes when a
reviewed `before_run` hook applies; projects without hooks retain the old probe
path. The Runtime launch, baseline, platform tool checks, writable storage and
declared network checks remain configured as before. A changed script, argv or
role requires a new reviewed repository version and an updated deployment
allowlist. Existing Run snapshots keep their prior hook identity.

`after_run` writes `workspace-status.txt` into its own output directory. It
does not change the preserved candidate. `before_remove` only checks the
resource; the core still decides whether deletion is allowed and checks again
after the hook. Normal cancellation and hook failure retain the reviewed
operation record for diagnosis.

## Migration and rollback

Migration `0031_project_hooks.sql` adds durable hook snapshot and invocation
records. Existing repository JSON without `hooks` maps to the existing no-hook
path. New repository versions may add hooks after review; frozen in-flight Runs
continue with their original identities. Disable new hook enrollment by stopping
new claims and removing hooks only through a new review. Do not erase an
in-flight invocation, especially an unknown result. To run an older binary that
does not understand the `hooks` field, first stop new claims, confirm process
quiescence and preservation, and complete or isolate hook-enabled work under a
compatible binary. The additive table migration need not be reversed for that
operational rollback.
