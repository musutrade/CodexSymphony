#!/usr/bin/env python3
"""Run the repository gate using the exact reviewed release binaries."""
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]


def main():
    expected = (ROOT / "harness-gate-version.lock").read_text().splitlines()
    if expected != ["harness-gate v0.4.6-rc.1", "rust-collector rust-collector-v0.1.0-rc.6"]:
        raise SystemExit("Gate lock changed: review launcher versions/digests together")
    pins = {
        "harness-gate": ("harness-gate 0.4.6-rc.1", "6d5dcf10b8d6b1248679974664a97b29628fa24778047484f18b0ebbe5d27dd3"),
        "harness-gate-rust-collector": ("harness-gate-rust-collector 0.1.0-rc.6", "520e3fc0fa4938694a10abc504e3a5d0f164cf2bf9314cd4a00c5d613bbbf861"),
    }
    binaries = {}
    for name, (version, digest) in pins.items():
        binary = shutil.which(name)
        if binary is None:
            raise SystemExit(f"Missing pinned tool: {name}; see .harness-gate/QUALITY.md")
        actual = subprocess.check_output([binary, "--version"], text=True).strip()
        if actual != version or hashlib.sha256(Path(binary).read_bytes()).hexdigest() != digest:
            raise SystemExit(f"Unrecognized {name} version/digest; reviewed Linux amd64 release required")
        binaries[name] = binary
    arguments = sys.argv[1:] or ["verify", "--profile", "ci", "--all"]
    os.chdir(ROOT)
    os.execv(binaries["harness-gate"], [binaries["harness-gate"], "--project-root", str(ROOT), *arguments])


if __name__ == "__main__":
    main()
