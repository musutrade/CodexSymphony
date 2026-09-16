"""Fixed non-model probes, executed *inside* the Agent command sandbox.

The platform supplies reviewed deployment input through stdin/argv, never code.
This module does not install dependencies or change network/system policy.
"""
import errno
import hashlib
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile
import urllib.error
import urllib.request
import urllib.parse

CORE_VERSION = "harness-gate 0.4.5"
CORE_SHA256 = "70721282c751826ed4d57e14bd7de9516e73e833aa058d758dbd2154c0aa5e10"
CODEX_VERSION = "codex-cli 0.154.0"
RESERVE = 256 * 1024 * 1024


def run(command, env=None):
    return subprocess.run(command, env=env, capture_output=True, text=True,
                          timeout=10, check=True).stdout.strip()


def tools(lock=None):
    lock = lock or {"core_version": CORE_VERSION, "core_sha256": CORE_SHA256,
                    "codex_version": CODEX_VERSION}
    identities = []
    for path in (os.environ["PATH"], str(Path.home() / ".cargo/bin") + ":" + os.environ["PATH"]):
        executable = shutil.which("harness-gate", path=path)
        if executable is None:
            raise FileNotFoundError("harness-gate")
        version = run([executable, "--version"])
        digest = hashlib.sha256(Path(executable).read_bytes()).hexdigest()
        if (version, digest) != (lock["core_version"], lock["core_sha256"]):
            raise ValueError("Core version/SHA256 does not match deployment lock")
        identities.append({"path": executable, "version": version, "sha256": digest})
    if run(["codex", "--version"]) != lock["codex_version"]:
        raise ValueError("Codex version does not match deployment lock")
    return identities


def dependency(item):
    # These are administrator-selected capability commands, never repair work.
    command = item["command"]
    if not command or not all(isinstance(arg, str) for arg in command):
        raise ValueError("invalid dependency command")
    output = run(command)
    if item["expected"] not in output:
        raise ValueError("dependency capability mismatch: " + command[0])
    return {"command": command, "output": output[:4096]}


def writable(path):
    directory = Path(path)
    if shutil.disk_usage(directory).free < RESERVE:
        raise OSError(errno.ENOSPC, "control-plane reserve unavailable")
    with tempfile.TemporaryDirectory(prefix=".preparation-", dir=directory) as temporary:
        sample = Path(temporary) / "probe"
        with sample.open("xb") as stream:
            stream.write(b"durability probe")
            stream.flush()
            os.fsync(stream.fileno())
        assert sample.read_bytes() == b"durability probe"
        fd = os.open(temporary, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(fd)
        finally:
            os.close(fd)


def fetch(url):
    try:
        with urllib.request.urlopen(url, timeout=8) as response:
            return response.status, response.read(2048).decode(errors="replace")
    except urllib.error.HTTPError as error:
        return error.code, error.read(2048).decode(errors="replace")
    except urllib.error.URLError as error:
        return 0, str(error.reason)


def network(config):
    allowed = fetch(config["allowed_url"])
    denied = denied_response(config["denied_url"])
    # A TCP refusal or DNS error is not proof of network isolation. Resolve first
    # and require an explicit sandbox routing/permission error for every address.
    addresses = config["direct_addresses"]
    bypass = bool(addresses)
    direct_results = []
    for family, kind, protocol, _, address in addresses:
        with socket.socket(family, kind, protocol) as connection:
            connection.settimeout(2)
            result = connection.connect_ex(tuple(address))
            direct_results.append({"address": address, "errno": result})
            bypass = bypass and result in (errno.ENETUNREACH, errno.EACCES, errno.EPERM)
    return {"allowed_probe": 200 <= allowed[0] < 300,
            "denied_probe": denied[0] == 403 and "x-proxy-error: blocked-by-allowlist" in denied[1].lower(),
            "direct_connection_rejected": bypass,
            "direct_results": direct_results,
            "responses": {"allowed": allowed, "denied": denied}}


def denied_response(url):
    # Preserve the proxy's CONNECT response headers; generic TLS clients often
    # raise before making the managed policy-denial header available.
    result = subprocess.run(["curl", "-sS", "-i", "--max-time", "8", url],
                            capture_output=True, text=True, timeout=10)
    headers = result.stdout[:4096]
    first = headers.splitlines()[0] if headers else ""
    status = 403 if first.startswith("HTTP/") and first.split()[1:2] == ["403"] else 0
    return status, headers


def failure(error, code):
    if isinstance(error, FileNotFoundError):
        code = "preparation_dependency_missing"
    if isinstance(error, OSError) and error.errno == errno.ENOSPC:
        code = "storage_unavailable"
    return {"code": code, "phase": "preparation", "detail": str(error)[:2048],
            "exit_code": getattr(error, "returncode", getattr(error, "errno", None)),
            "evidence": "preparation_history"}


def sample(config):
    result = {"uid": os.getuid(), "cwd": str(Path.cwd()), "failures": [], "model_calls": 0}
    checks = [("tools", lambda: tools(config.get("tool_lock")), "preparation_capability_mismatch")]
    for index, item in enumerate(config["dependencies"]):
        checks.append((f"dependency_{index}", lambda item=item: dependency(item), "preparation_capability_mismatch"))
    for index, path in enumerate(config["writable_paths"]):
        checks.append((f"path_{index}", lambda path=path: writable(path), "preparation_path_unwritable"))
    checks.append(("network", lambda: network(config), "network_scope_unavailable"))
    for name, probe, code in checks:
        try:
            result[name] = probe()
        except (OSError, ValueError, subprocess.SubprocessError) as error:
            result["failures"].append(failure(error, code))
    return result


if __name__ == "__main__":
    import sys
    print(json.dumps(sample(json.loads(sys.argv[1]))))
