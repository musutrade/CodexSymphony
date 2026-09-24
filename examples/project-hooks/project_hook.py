#!/usr/bin/env python3
"""Reviewed project hook example. Install an immutable copy outside Agent worktrees."""
import json
import os
from pathlib import Path
import subprocess
import sys


def result(request, status, artifacts=None, error=None):
    keys = ("protocol_version", "requirement_id", "revision", "run_id",
            "resource_id", "invocation_id", "attempt", "config_id")
    response = {key: request[key] for key in keys}
    response["status"] = status
    if status == "success":
        response["artifacts"] = artifacts or []
    else:
        response["error"] = error
    print(json.dumps(response))


def run(request):
    event = request["event"]
    workspace = Path(request["workspace"])
    output = Path(request["output_dir"])
    if event == "before_run":
        # This belongs to the project. The core only checks platform tools,
        # writable storage and declared connectivity.
        subprocess.run(["git", "-C", str(workspace), "rev-parse", "--is-inside-work-tree"],
                       check=True, capture_output=True, timeout=10)
        if "--require-test-db" in sys.argv:
            url = os.environ["TEST_DATABASE_URL"]
            subprocess.run(["psql", url, "-X", "-At", "-c", "select 1"],
                           check=True, capture_output=True, timeout=10)
    elif event == "after_run":
        # Auxiliary output stays in the hook directory, away from the candidate.
        status = subprocess.run(["git", "-C", str(workspace), "status", "--short"],
                                check=True, capture_output=True, text=True, timeout=10)
        (output / "workspace-status.txt").write_text(status.stdout)
        return [{"path": "workspace-status.txt", "kind": "diagnostic"}]
    elif event in ("after_create", "before_remove"):
        if not workspace.is_dir():
            raise FileNotFoundError(workspace)
    else:
        raise ValueError("unsupported hook event")
    return []


if __name__ == "__main__":
    request = json.load(sys.stdin)
    try:
        artifacts = run(request)
        result(request, "success", artifacts)
    except (OSError, ValueError, subprocess.SubprocessError) as failure:
        result(request, "failed", error={"code": "project_dependency_unavailable",
                                         "message": str(failure)[:1000]})
