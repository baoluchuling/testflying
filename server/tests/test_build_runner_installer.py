from __future__ import annotations

import hashlib
import json
import os
import shlex
import subprocess
import sys
import zipfile
from pathlib import Path

import pytest

from testflying_api.runner_releases import RunnerReleaseManifest

REPO_ROOT = Path(__file__).resolve().parents[1]
SCRIPT = REPO_ROOT / "scripts/build_runner_installer.sh"
SYSTEM_INSTALL_ROOT = "/Library/Application Support/TestFlying/build-runner"


@pytest.mark.parametrize(
    ("field", "value"),
    [
        ("runnerId", ""),
        ("token", " "),
        ("serverUrl", None),
        ("rootDir", ""),
        ("labels", []),
    ],
)
def test_installer_rejects_invalid_config_before_creating_output(
    tmp_path: Path,
    field: str,
    value: object,
) -> None:
    config = _valid_config(tmp_path)
    if value is None:
        config.pop(field)
    else:
        config[field] = value
    config_path = tmp_path / "runner.json"
    config_path.write_text(json.dumps(config), encoding="utf-8")
    output_dir = tmp_path / "installer-output"
    env = _fake_tool_environment(tmp_path)

    result = subprocess.run(
        [str(SCRIPT), str(config_path), str(output_dir), "0.2.0"],
        text=True,
        capture_output=True,
        env=env,
        check=False,
    )

    assert result.returncode != 0
    assert not output_dir.exists()


def test_installer_builds_two_binary_pkg_and_matching_release(tmp_path: Path) -> None:
    config_path = tmp_path / "runner.json"
    config_path.write_text(json.dumps(_valid_config(tmp_path)), encoding="utf-8")
    output_dir = tmp_path / "installer-output"
    env = _fake_tool_environment(tmp_path)

    result = subprocess.run(
        [str(SCRIPT), str(config_path), str(output_dir), "0.2.0"],
        text=True,
        capture_output=True,
        env=env,
        check=False,
    )

    assert result.returncode == 0, result.stderr
    tool_log = Path(env["TEST_TOOL_LOG"]).read_text(encoding="utf-8")
    for command in ("go ", "python3.11 -m PyInstaller", "ditto ", "pkgbuild "):
        assert command in tool_log
    fake_ditto = Path(env["PATH"].split(os.pathsep, 1)[0]) / "ditto"
    assert "/usr/bin/ditto" not in fake_ditto.read_text(encoding="utf-8")

    fallback = output_dir / "build-runner-macos"
    assert (fallback / "testflying-build-runner").stat().st_mode & 0o111
    assert (fallback / "package-agent").stat().st_mode & 0o111
    staged_config = json.loads((fallback / "config.json").read_text(encoding="utf-8"))
    assert staged_config["packageAgentBin"] == f"{SYSTEM_INSTALL_ROOT}/package-agent"
    assert staged_config["version"] == "0.2.0"
    assert staged_config["packageAgentVersion"] == "0.2.0"

    release_dir = output_dir / "darwin" / "arm64"
    manifest = json.loads((release_dir / "release.json").read_text(encoding="utf-8"))
    bundle = release_dir / manifest["bundleFile"]
    digest = hashlib.sha256(bundle.read_bytes()).hexdigest()
    assert manifest == {
        "version": "0.2.0",
        "runnerVersion": "0.2.0",
        "packageAgentVersion": "0.2.0",
        "platform": "darwin",
        "arch": "arm64",
        "bundleFile": bundle.name,
        "sha256": digest,
    }
    loaded_manifest = RunnerReleaseManifest.load(output_dir, "darwin", "arm64")
    assert loaded_manifest.bundle_path == bundle.resolve()
    assert (release_dir / f"{bundle.name}.sha256").read_text(encoding="utf-8").split()[0] == digest
    with zipfile.ZipFile(bundle) as archive:
        assert sorted(archive.namelist()) == ["package-agent", "testflying-build-runner"]

    package = output_dir / "TestFlyingBuildRunner-0.2.0-darwin-arm64.pkg"
    assert package.is_file()
    assert f"{SYSTEM_INSTALL_ROOT}/package-agent" in tool_log
    assert f"{SYSTEM_INSTALL_ROOT}/testflying-build-runner" in tool_log


def test_postinstall_and_unpacked_fallback_manage_console_user_launch_agent() -> None:
    postinstall = (REPO_ROOT / "build-runner/packaging/postinstall").read_text(encoding="utf-8")
    fallback = (REPO_ROOT / "build-runner/packaging/install.command").read_text(encoding="utf-8")

    for expected in (
        "stat -f '%Su' /dev/console",
        'id -u "${CONSOLE_USER}"',
        "NFSHomeDirectory",
        "ThrottleInterval",
        'chown "${CONSOLE_USER}:${CONSOLE_GROUP}" "${INSTALL_ROOT}"',
        'chmod 600 "${CONFIG_PATH}"',
        'launchctl bootstrap "gui/${CONSOLE_UID}"',
        'launchctl kickstart -k "gui/${CONSOLE_UID}/com.testflying.build-runner"',
    ):
        assert expected in postinstall
    assert "package-agent" in fallback
    assert "ThrottleInterval" in fallback


def _valid_config(tmp_path: Path) -> dict[str, object]:
    return {
        "runnerId": "runner-mac-1",
        "name": "Mac Runner 1",
        "token": "runner-secret-token",
        "serverUrl": "https://testflying.example.test",
        "rootDir": str(tmp_path / "runner-root"),
        "packageAgentBin": "/ignored/package-agent",
        "version": "dev",
        "packageAgentVersion": "dev",
        "labels": ["ios-release"],
        "llmAdapters": ["codex", "claude"],
        "capacity": 1,
    }


def _fake_tool_environment(tmp_path: Path) -> dict[str, str]:
    bin_dir = tmp_path / "fake-bin"
    bin_dir.mkdir()
    log_path = tmp_path / "tools.log"
    zip_program = (
        "import sys\n"
        "import zipfile\n"
        "from pathlib import Path\n"
        "source = Path(sys.argv[1])\n"
        "target = Path(sys.argv[2])\n"
        "with zipfile.ZipFile(target, 'w') as archive:\n"
        "    for item in sorted(source.iterdir()):\n"
        "        if item.is_file():\n"
        "            archive.write(item, item.name)\n"
    )
    _write_tool(
        bin_dir / "go",
        """
        output=""
        while [[ $# -gt 0 ]]; do
          if [[ "$1" == "-o" ]]; then output="$2"; shift 2; continue; fi
          shift
        done
        printf 'go %s\n' "$output" >> "$TEST_TOOL_LOG"
        mkdir -p "$(dirname "$output")"
        printf 'fake-runner' > "$output"
        chmod +x "$output"
        """,
    )
    _write_tool(
        bin_dir / "python3.11",
        f"""
        if [[ "${{1:-}}" == "-m" && "${{2:-}}" == "PyInstaller" ]]; then
          printf 'python3.11 %s\n' "$*" >> "$TEST_TOOL_LOG"
          shift 2
          distpath=""
          name=""
          while [[ $# -gt 0 ]]; do
            case "$1" in
              --distpath) distpath="$2"; shift 2 ;;
              --name) name="$2"; shift 2 ;;
              *) shift ;;
            esac
          done
          mkdir -p "$distpath"
          printf 'fake-package-agent' > "$distpath/$name"
          chmod +x "$distpath/$name"
          exit 0
        fi
        exec {shlex.quote(sys.executable)} "$@"
        """,
    )
    _write_tool(
        bin_dir / "ditto",
        """
        printf 'ditto %s\n' "$*" >> "$TEST_TOOL_LOG"
        arguments=("$@")
        source="${arguments[$((${#arguments[@]} - 2))]}"
        target="${arguments[$((${#arguments[@]} - 1))]}"
        python3.11 -c "$TEST_FAKE_ZIP_PROGRAM" "$source" "$target"
        """,
    )
    _write_tool(
        bin_dir / "shasum",
        """
        printf 'shasum %s\n' "$*" >> "$TEST_TOOL_LOG"
        exec /usr/bin/shasum "$@"
        """,
    )
    _write_tool(
        bin_dir / "pkgbuild",
        """
        printf 'pkgbuild %s\n' "$*" >> "$TEST_TOOL_LOG"
        root=""
        previous=""
        for argument in "$@"; do
          if [[ "$previous" == "--root" ]]; then root="$argument"; fi
          previous="$argument"
        done
        if [[ -n "$root" ]]; then find "$root" -type f -print >> "$TEST_TOOL_LOG"; fi
        output="${!#}"
        mkdir -p "$(dirname "$output")"
        printf 'fake-pkg' > "$output"
        """,
    )
    _write_tool(
        bin_dir / "uname",
        """
        if [[ "${1:-}" == "-m" ]]; then printf 'arm64\n'; else exec /usr/bin/uname "$@"; fi
        """,
    )
    env = os.environ.copy()
    env["PATH"] = f"{bin_dir}:{env['PATH']}"
    env["TEST_TOOL_LOG"] = str(log_path)
    env["TEST_FAKE_ZIP_PROGRAM"] = zip_program
    return env


def _write_tool(path: Path, body: str) -> None:
    path.write_text("#!/usr/bin/env bash\nset -euo pipefail\n" + body, encoding="utf-8")
    path.chmod(0o755)
