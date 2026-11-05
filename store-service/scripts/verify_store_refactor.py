"""Run isolated Rust checks under the installed artifact lifecycle supervisor."""
from __future__ import annotations

import datetime
import hashlib
import json
import os
import re
import stat
import subprocess
import sys
from pathlib import Path


PROJECT = Path(__file__).resolve().parents[1]


def source_hashes() -> dict[str, str]:
    crates = PROJECT / "crates"
    files = sorted([PROJECT / "Cargo.toml", PROJECT / "Cargo.lock", Path(__file__).resolve(),
                    *crates.rglob("Cargo.toml"), *crates.rglob("*.rs")])
    return {str(p.relative_to(PROJECT)): hashlib.sha256(p.read_bytes()).hexdigest() for p in files}


def managed_root() -> Path:
    raw = os.environ.get("ARTIFACT_RUN_ROOT", "")
    root = Path(raw)
    if not raw or not root.is_absolute() or root.resolve(strict=True) != root:
        raise RuntimeError("A canonical managed run root is required")
    if root.is_relative_to(PROJECT) or PROJECT.is_relative_to(root):
        raise RuntimeError("Managed output cannot overlap source")
    for path in (root, *root.parents):
        if path.is_symlink():
            raise RuntimeError("Symlink in managed root")
    root_stat = root.stat()
    if not stat.S_ISDIR(root_stat.st_mode) or root_stat.st_uid != os.getuid():
        raise RuntimeError("Managed root has the wrong owner or type")
    marker = root / ".artifact-lifecycle-owner.json"
    if os.environ.get("ARTIFACT_RUN_OWNERSHIP_MARKER") != str(marker):
        raise RuntimeError("Exact ownership marker required")
    directory = os.open(root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        fd = os.open(marker.name, os.O_RDONLY | os.O_NOFOLLOW, dir_fd=directory)
        try:
            before = os.fstat(fd)
            if (not stat.S_ISREG(before.st_mode) or before.st_uid != os.getuid()
                    or before.st_nlink != 1 or before.st_mode & 0o222 or before.st_size > 4096):
                raise RuntimeError("Unsafe managed ownership marker")
            payload = json.loads(os.read(fd, 4097))
            expected = {"schema_version": 1, "owner": "codex-artifact-lifecycle",
                        "run_id": root.name, "project_root": str(PROJECT)}
            if payload != expected:
                raise RuntimeError("Managed ownership marker does not match this project")
            after = os.fstat(fd)
            current = os.stat(marker.name, dir_fd=directory, follow_symlinks=False)
            def identity(s: os.stat_result) -> tuple[int, ...]:
                return s.st_dev, s.st_ino, s.st_size, s.st_mtime_ns, s.st_mode, s.st_nlink
            if identity(before) != identity(after) or identity(before) != identity(current):
                raise RuntimeError("Managed ownership marker changed")
            if os.fstat(directory).st_ino != root.stat().st_ino:
                raise RuntimeError("Managed root changed")
        finally:
            os.close(fd)
    finally:
        os.close(directory)
    return root


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
        ["cargo", "fmt", "--all", "--", "--check"],
        ["cargo", "test", "--locked", "--workspace"],
        ["cargo", "clippy", "--locked", "--workspace", "--all-targets", "--", "-D", "warnings"],
        ["cargo", "build", "--locked", "--release", "--package", "store-api"],
        ["cargo", "build", "--locked", "--release", "--package", "store-connector"],
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
