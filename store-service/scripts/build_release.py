"""Build isolated Linux and Apple Silicon releases for the standalone service."""
from __future__ import annotations

import datetime
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys

from verify_store_refactor import PROJECT, managed_root, source_hashes


def main() -> None:
    root = managed_root()
    if sys.argv[1:] not in ([], ["windows"]) or platform.system() != "Darwin" or platform.machine() != "arm64":
        raise RuntimeError("This release workflow requires an Apple Silicon macOS host")
    windows = sys.argv[1:] == ["windows"]
    target_triple = "x86_64-pc-windows-gnu" if windows else "x86_64-unknown-linux-musl"
    tmp = root / "tmp"
    tmp.mkdir(exist_ok=True)
    baseline = source_hashes()
    source = tmp / "source"
    for relative in baseline:
        if relative.startswith("scripts/"):
            continue
        destination = source / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(PROJECT / relative, destination)
    packages = tmp / "python-packages"
    env = {**os.environ,
           "RUSTUP_HOME": str(tmp / "rustup"), "CARGO_HOME": str(tmp / "cargo-home"),
           "RUSTUP_TOOLCHAIN": "1.94.0", "CARGO_TARGET_DIR": str(tmp / "target"),
           "CARGO_INCREMENTAL": "0", "CARGO_BUILD_JOBS": "3", "TMPDIR": str(tmp),
           "XDG_CACHE_HOME": str(tmp / "cache"), "PIP_CACHE_DIR": str(tmp / "pip-cache"),
           "ZIG_GLOBAL_CACHE_DIR": str(tmp / "zig-global"),
           "ZIG_LOCAL_CACHE_DIR": str(tmp / "zig-local"),
           "CARGO_ZIGBUILD_CACHE_DIR": str(tmp / "zigbuild"),
           "CARGO_ZIGBUILD_ZIG_PATH": str(packages / "ziglang" / "zig"),
           "PYTHONPATH": str(packages), "PYTHONDONTWRITEBYTECODE": "1",
           "PATH": str(packages / "bin") + os.pathsep + os.environ["PATH"]}
    if windows:
        dlltool = Path("/opt/homebrew/opt/llvm/bin/llvm-dlltool")
        if not dlltool.is_file():
            raise RuntimeError("An existing trusted LLVM dlltool is required")
        env["CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS"] = "-C dlltool=" + str(dlltool)
    commands = [
        [sys.executable, "-m", "pip", "install", "--disable-pip-version-check",
         "--no-cache-dir", "--only-binary=:all:", "--target", str(packages),
         "cargo-zigbuild==0.20.1", "ziglang==0.14.1"],
        [shutil.which("rustup") or "rustup", "toolchain", "install", "1.94.0",
         "--profile", "minimal", "--target", target_triple, "--no-self-update"],
    ]
    if windows:
        commands.append(["cargo", "zigbuild", "--locked", "--release", "--target", target_triple,
                         "--package", "store-connector"])
    else:
        commands.extend([
            ["cargo", "zigbuild", "--locked", "--release", "--target", target_triple,
             "--package", "store-api", "--package", "store-connector"],
            ["cargo", "build", "--locked", "--release", "--package", "store-connector"],
        ])
    for command in commands:
        print("Running: " + " ".join(command), flush=True)
        # Resume transient official-toolchain download failures within the same owned cache.
        # Never retry compilation or business requests.
        attempts = 3 if "toolchain" in command and "install" in command else 1
        for attempt in range(attempts):
            try:
                subprocess.run(command, cwd=source, env=env, check=True)
                break
            except subprocess.CalledProcessError:
                if attempt + 1 == attempts:
                    raise
                print("Retrying toolchain download with the existing managed cache", flush=True)
    if source_hashes() != baseline:
        raise RuntimeError("Source changed during release build")
    outputs = {
        "releases/linux-amd64/store-api": tmp / "target/x86_64-unknown-linux-musl/release/store-api",
        "releases/linux-amd64/store-connector": tmp / "target/x86_64-unknown-linux-musl/release/store-connector",
        "releases/darwin-arm64/store-connector": tmp / "target/release/store-connector",
    }
    if windows:
        outputs = {"releases/windows-amd64/store-connector.exe":
                   tmp / "target/x86_64-pc-windows-gnu/release/store-connector.exe"}
    hashes = {}
    for relative, binary in outputs.items():
        description = subprocess.check_output(["file", str(binary)], text=True).strip()
        if ("linux-amd64" in relative and "ELF 64-bit" not in description
                or "darwin-arm64" in relative and "Mach-O 64-bit executable arm64" not in description
                or "windows-amd64" in relative and ("PE32+ executable" not in description or "x86-64" not in description)):
            raise RuntimeError("Unexpected release architecture: " + description)
        imports = []
        if windows:
            details = subprocess.check_output(["/opt/homebrew/opt/llvm/bin/llvm-objdump", "-p", str(binary)], text=True)
            imports = sorted(set(re.findall(r"DLL Name:\s*(\S+)", details)))
            allowed = {"advapi32.dll", "bcrypt.dll", "bcryptprimitives.dll", "crypt32.dll", "kernel32.dll", "msvcrt.dll",
                       "ntdll.dll", "ole32.dll", "secur32.dll", "shell32.dll", "user32.dll",
                       "userenv.dll", "ws2_32.dll", "iphlpapi.dll", "synchronization.dll", "kernelbase.dll"}
            if not imports or any(name.lower() not in allowed and not name.lower().startswith("api-ms-win-") for name in imports):
                raise RuntimeError("Unexpected non-system Windows DLL imports: " + repr(imports))
        target = PROJECT / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        if target.is_symlink():
            raise RuntimeError("Release target cannot be a symlink")
        # Formal release files are outside supervisor cleanup ownership.
        with target.open("wb") as output:
            shutil.copyfileobj(binary.open("rb"), output)
            output.flush()
            os.fsync(output.fileno())
        target.chmod(0o755)
        hashes[relative] = {"sha256": hashlib.sha256(target.read_bytes()).hexdigest(),
                            "bytes": target.stat().st_size, "format": description.split(": ", 1)[-1]}
        if windows:
            hashes[relative]["importedDlls"] = imports
    report = {"runId": root.name,
              "finishedAt": datetime.datetime.now(datetime.timezone.utc).isoformat(),
              "sourceCommit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=PROJECT, text=True).strip(),
              "sourceHashes": baseline, "toolchain": "Rust 1.94.0 / cargo-zigbuild 0.20.1 / Zig 0.14.1",
              "outputs": hashes, "cleanup": "See the supervisor cleanup report for this runId"}
    report_name = "windows-release-build.json" if windows else "deployment-build.json"
    (PROJECT / "docs" / report_name).write_text(json.dumps(report, indent=2) + "\n")
    print("Published formal release files", flush=True)


if __name__ == "__main__":
    main()
