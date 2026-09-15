# Actual execution readiness

GH-17 preparation attempted to launch another app-server from inside a command
sandbox. The inherited Codex home was read-only; relocating only SQLite was
insufficient. Relocating CODEX_HOME allowed initialization, but the synthetic
mount registry also needed a writable TMPDIR. After both were relocated,
further user namespace creation was rejected. These probes were not equivalent
to the normal host-launched Agent execution path.

The fixed environment operation `execution-readiness` now launches the same
pinned host wrapper, under the same UID and workspace-write command policy, and
performs initialize/configRequirements/read/command/exec with no model turn.
The exact probe checks Codex 0.154.0, Core 0.4.5, UID 1000, target writes, Git
write rejection, hidden host credential paths and enabled network requirements.
Only network requirements are returned, not unrelated host configuration.

Agents request it with `python3 /opt/symphony-env/execution_readiness.py`.
Arguments cannot select commands, paths or policies. The canonical timestamped
result is in the read-only fixture mount; the client verifies sample freshness
and identity. Existing and future fixture provisioning refreshes the helper and
restarts the broker only when its code changed. No AppArmor, namespace policy,
static network allowlist or model permissions were relaxed.

This restores development preparation. It does not prove product-level network
allow/deny/bypass, deployment identity or persistence failure acceptance for
GH-17; those still need implementation and explicit tests.

## DNS configuration in the outer sandbox

The outer launcher hides host `/run`. On systemd-resolved hosts,
`/etc/resolv.conf` is a symlink into that directory. Before the correction,
GH-17 observed a dangling resolver symlink and `socket.gaierror`; the managed
proxy rejected `index.crates.io` with HTTP 403. Its local/private address check
also rejects DNS resolution failures, so this message did not establish that
crates.io resolved to a private address.

`codex_sandbox.py` now resolves the host resolver file before launch and mounts
only that file read-only at its resolved location after isolating `/run`.
Missing resolver configuration fails before starting Codex. No network domains,
proxy exemptions, or namespace restrictions are changed. This is shared by all
issue workspaces and survives installation through the versioned launcher.

Actual GH-17 command sandbox verification on 2026-09-15:

- Original `probe-allowed-network.py`: exit 0, registry config JSON returned.
- `https://example.com`: HTTP 403 (outside the allowlist).
- Proxy-free registry request: failed; direct TCP to the registry's current
  public address on port 443 separately failed with ENETUNREACH (101).
- Readiness helper: UID 1000, pinned tools, writable target, read-only Git,
  host credentials hidden; zero model calls.
- Environment unit suite: 18 tests passed, including symlink/regular/missing
  resolver cases.

The operator proof is `/opt/symphony-env/network-dns-recovery.json` in the
assigned sandbox. These checks repair the development environment; GH-17 must
still implement and verify its product preparation and runtime guards.

## Product preparation acceptance operation

The basic readiness helper intentionally cannot run product adapters. GH-17
subsequently reached that limitation: the host launcher is absent inside the
Agent workspace, so invoking it there fails before any model call.

The generic environment now includes a second, fixed broker action,
`product-preparation-acceptance`, callable through
`python3 /opt/symphony-env/product_preparation_acceptance.py`. Operator installation
snapshots the reviewed product adapter/sampler, pins SHA256, and mounts the
sampler read-only. The host verifies both its snapshot and the workspace source
hashes before and after acceptance. It never imports mutable workspace code on
the host and never accepts a caller-selected command, path or test case.

Six actual sandbox cases passed: ready; missing dependency; mismatched dependency
capability; mismatched locked Core version; read-only preparation directory;
unknown expected network identity. Every case also verifies allowlisted access,
explicit proxy rejection and direct-IP isolation. The canonical host-owned result
records actual UID/cwd/policy, source hashes, network identity, locked tools,
individual failures and zero model calls. Client freshness checking prevents an
old/spoofed writable-spool result from substituting for that canonical receipt.

The sampler uses curl's CONNECT response headers for an explicit
`x-proxy-error: blocked-by-allowlist` assertion. Generic connection failures or
TCP timeouts do not qualify as successful policy-denial/isolation evidence.

Deployment: stop affected execution, review the two Python product files and
host helpers, run `tools/install_symphony_development.py`, reprovision existing
workspaces and run the fixed acceptance client. Re-pin changed protected host
installation inputs through `tools/install_remote_gate.py`. Future workspaces
receive the same reviewed snapshot through normal provisioning. Product source
changes need a new operator review; a receipt for old hashes cannot approve new
code. This operation does not claim completion of Rust integration, retry/storage
acceptance or the full GH-17 quality gate.
