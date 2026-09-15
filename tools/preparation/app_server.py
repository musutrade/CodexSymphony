"""Product preparation adapter. initialize/read/command only; never turn/start.

Run this platform-owned file outside the command sandbox, with the same pinned
launcher, effective environment and cwd as the intended Agent execution.
"""
import hashlib
import json
import os
from pathlib import Path
import select
import socket
import subprocess
import sys
import time
import urllib.parse


def identity(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


class Rpc:
    def __init__(self, process):
        self.process = process
        self.pending = b""
        self.sequence = 0

    def call(self, method, params):
        self.sequence += 1
        request = {"id": self.sequence, "method": method, "params": params}
        self.process.stdin.write((json.dumps(request) + "\n").encode())
        self.process.stdin.flush()
        deadline = time.monotonic() + 90
        while time.monotonic() < deadline:
            if b"\n" not in self.pending:
                if not select.select([self.process.stdout], [], [], .2)[0]:
                    continue
                data = os.read(self.process.stdout.fileno(), 65536)
                if not data:
                    raise RuntimeError("app-server exited before probe response")
                self.pending += data
            line, self.pending = self.pending.split(b"\n", 1)
            response = json.loads(line)
            if response.get("id") == self.sequence:
                if "error" in response:
                    raise RuntimeError("app-server rejected " + method)
                return response["result"]
        raise TimeoutError(method)


def run(config):
    policy = {"type": "workspaceWrite", "writableRoots": [], "networkAccess": True}
    probe = config.get("probe_path", str(Path(__file__).with_name("sandbox_probe.py").resolve()))
    # Resolve on the control side: direct DNS is itself normally blocked inside
    # the network namespace. DNS failure there cannot prove TCP isolation.
    host = urllib.parse.urlsplit(config["allowed_url"]).hostname
    config["direct_addresses"] = socket.getaddrinfo(host, 443, type=socket.SOCK_STREAM)
    with subprocess.Popen(config["launcher"] + ["app-server"], cwd=config["workspace"],
                          stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL) as process:
        try:
            rpc = Rpc(process)
            rpc.call("initialize", {"clientInfo": {"name": "product-preparation", "version": "1"},
                                    "capabilities": {"experimentalApi": True}})
            before = rpc.call("configRequirements/read", {}).get("requirements") or {}
            result = rpc.call("command/exec", {"command": ["python3", str(probe), json.dumps(config)],
                                              "cwd": config["workspace"], "sandboxPolicy": policy,
                                              "timeoutMs": 80000})
            after = rpc.call("configRequirements/read", {}).get("requirements") or {}
            return evidence(config, policy, before, after, result)
        finally:
            process.terminate()
            try:
                process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()


def evidence(config, policy, before, after, result):
    if result["exitCode"] != 0:
        raise RuntimeError("sandbox probe failed, exit=" + str(result["exitCode"]))
    sample = json.loads(result["stdout"])
    network = before.get("network") or {}
    actual = identity(network)
    domains = network.get("allowedDomains") or []
    expected_domains = config["allowed_domains"]
    enforced = (network.get("enabled") is True and before == after
                and actual == config["network_identity"]
                and sorted(domains) == sorted(expected_domains))
    if sample["uid"] != config["uid"] or sample["cwd"] != config["workspace"]:
        raise RuntimeError("Agent execution identity mismatch")
    probes = sample.get("network") or {}
    return {"deployment_identity": config["deployment_identity"],
            "sandbox_identity": identity({"uid": sample["uid"], "cwd": sample["cwd"], "policy": policy, "sample": sample}),
            "network": {"configuration_identity": config["deployment_identity"] if enforced else "unknown",
                        "allowed_domains": domains, "enforced": enforced,
                        "allowed_probe": probes.get("allowed_probe", False),
                        "denied_probe": probes.get("denied_probe", False),
                        "direct_connection_rejected": probes.get("direct_connection_rejected", False)},
            "failures": sample["failures"], "sample": sample, "network_identity": actual,
            "execution": {"uid": sample["uid"], "cwd": sample["cwd"], "policy": policy,
                          "model_calls": sample["model_calls"]}}


if __name__ == "__main__":
    print(json.dumps(run(json.load(sys.stdin))))
