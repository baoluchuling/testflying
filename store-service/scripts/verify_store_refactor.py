"""Run isolated Rust checks under the installed artifact lifecycle supervisor."""
from __future__ import annotations

import datetime
import hashlib
import json
import os
import re
import subprocess
import sys
from pathlib import Path


from managed_workspace import PROJECT, managed_root


def source_hashes() -> dict[str, str]:
    crates = PROJECT / "crates"
    files = sorted([PROJECT / "Cargo.toml", PROJECT / "Cargo.lock", Path(__file__).resolve(),
                    PROJECT / "docs/store-api.md", PROJECT / "docs/openapi.yaml",
                    PROJECT / "scripts/verify_connector_install.py",
                    PROJECT / "scripts/managed_workspace.py",
                    PROJECT / "deploy/testflying-prod-ca.crt",
                    PROJECT / "packaging/connector/start.cmd", PROJECT / "packaging/connector/start.sh",
                    PROJECT / "packaging/connector/install.cmd", PROJECT / "packaging/connector/install.ps1",
                    PROJECT / "packaging/connector/install.sh",
                    *crates.rglob("Cargo.toml"), *crates.rglob("*.rs")])
    files = sorted(set(files + list((crates / "store-api/src/setup").rglob("*"))
                       + list((PROJECT / "packaging/screenshot-connector").rglob("*"))
                       + list(crates.rglob("*.mjs"))))
    files = [path for path in files if path.is_file()]
    return {str(p.relative_to(PROJECT)): hashlib.sha256(p.read_bytes()).hexdigest() for p in files}


def main() -> int:
    root = managed_root()
    if sys.argv[1:] != ["rust"]:
        raise RuntimeError("Usage: verify_store_refactor.py rust")
    temporary = root / "tmp"
    temporary.mkdir(exist_ok=True)
    env = {**os.environ, "CARGO_TARGET_DIR": str(temporary / "target"),
           "CARGO_HOME": str(temporary / "cargo-home"), "CARGO_INCREMENTAL": "0",
           "CARGO_BUILD_JOBS": "4", "TMPDIR": str(temporary), "RUST_BACKTRACE": "1"}
    commands = [
        ["node", "--test", "crates/store-api/tests/setup-package.test.mjs"],
        ["cargo", "build", "--locked", "--package", "store-connector"],
        [sys.executable, "-B", "scripts/verify_connector_install.py"],
        ["/bin/sh", "-n", "packaging/screenshot-connector/run.sh"],
        ["/bin/sh", "-n", "packaging/screenshot-connector/install.sh"],
        ["cargo", "fmt", "--all", "--", "--check"],
        ["cargo", "test", "--locked", "--workspace"],
        ["cargo", "test", "--locked", "--package", "store-connector", "--example", "screenshot-connector"],
        ["cargo", "clippy", "--locked", "--workspace", "--all-targets", "--", "-D", "warnings"],
        ["cargo", "build", "--locked", "--release", "--package", "store-api"],
        ["cargo", "build", "--locked", "--release", "--package", "store-connector"],
        ["cargo", "build", "--locked", "--release", "--package", "store-connector", "--example", "screenshot-connector"],
    ]
    checks = []
    passed = 0
    baseline = source_hashes()
    for command in commands:
        print("Running: " + " ".join(command), flush=True)
        summaries = []
        with subprocess.Popen(command, cwd=PROJECT, env=env,
                              stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True) as process:
            assert process.stdout is not None
            for line in process.stdout:
                print(line, end="", flush=True)
                if line.startswith("test result:"):
                    summaries.append(line.strip())
                    match = re.search(r"(\d+) passed; (\d+) failed", line)
                    if match:
                        passed += int(match[1])
            status = process.wait()
        checks.append({"command": command, "exitCode": status, "testSummaries": summaries})
        if status:
            return status
    hashes = source_hashes()
    if hashes != baseline:
        print("Source changed during verification; rerun against a stable tree", flush=True)
        return 75
    report = {"runId": root.name, "finishedAt": datetime.datetime.now(datetime.timezone.utc).isoformat(),
              "checks": checks, "testsPassed": passed, "sourceHashes": hashes,
              "notVerified": ["Live store accounts", "Docker build", "Cross-platform CI", "Deployment"],
              "cleanup": "See the supervisor cleanup report for this runId"}
    # This exact report is a declared formal deliverable, outside cleanup ownership.
    (PROJECT / "docs/verification.json").write_text(json.dumps(report, indent=2) + "\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
