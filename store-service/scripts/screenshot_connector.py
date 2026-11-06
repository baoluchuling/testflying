"""Claim and execute one macOS screenshot job inside an artifact-managed run."""
from __future__ import annotations

import json
import os
from pathlib import Path
import re
import socket
import ssl
import stat
import subprocess
import sys
import urllib.error
import urllib.parse
import urllib.request
import uuid

from managed_workspace import managed_root

PNG = b"\x89PNG\r\n\x1a\n"
MAX_IMAGE = 16 * 1024 * 1024


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


class JobFailure(Exception):
    pass


def bounded_read(path: Path, limit: int, *, private: bool = False) -> bytes:
    with os.fdopen(os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK), "rb") as file:
        info = os.fstat(file.fileno())
        if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1 or info.st_uid != os.getuid():
            raise ValueError("Expected an owned regular file")
        if private and info.st_mode & 0o077:
            raise ValueError("Connector configuration must be owner-only")
        if info.st_size > limit:
            raise ValueError("File exceeds the allowed size")
        value = file.read(limit + 1)
        if len(value) > limit:
            raise ValueError("File exceeds the allowed size")
        return value


def local_path(value: str, *, directory: bool = False) -> Path:
    path = Path(value)
    if not path.is_absolute():
        raise ValueError("Use absolute local configuration paths")
    path = path.resolve(strict=True)
    if directory and not path.is_dir():
        raise ValueError("Expected a directory")
    if not directory and not path.is_file():
        raise ValueError("Expected a file")
    return path


class Center:
    def __init__(self, config: dict):
        if config.get("kind") != "screenshot":
            raise ValueError("Expected a Screenshot Connector configuration")
        self.node = str(uuid.UUID(config["connectorId"]))
        self.token = config["token"]
        if not re.fullmatch(r"[0-9a-f]{64}", self.token):
            raise ValueError("Invalid node credential")
        self.base = config["serverUrl"].rstrip("/")
        url = urllib.parse.urlsplit(self.base)
        if url.scheme != "https" or not url.hostname or url.username or url.password or url.query or url.fragment:
            raise ValueError("Screenshot Connector requires a fixed HTTPS center address")
        ca = config.get("caFile")
        tls = ssl.create_default_context()
        bundled_ca = Path(__file__).resolve().parents[1] / "testflying-prod-ca.crt"
        if ca:
            tls.load_verify_locations(cafile=local_path(ca))
        elif bundled_ca.is_file():
            tls.load_verify_locations(cafile=bundled_ca)
        self.opener = urllib.request.build_opener(
            NoRedirect(), urllib.request.ProxyHandler({}), urllib.request.HTTPSHandler(context=tls)
        )

    def request(self, method: str, path: str, body: bytes | None = None,
                *, content_type: str | None = None, lease: str | None = None):
        headers = {"Authorization": "Bearer " + self.token}
        if content_type:
            headers["Content-Type"] = content_type
        if lease:
            headers["X-Screenshot-Lease"] = lease
        url = self.base + "/v1/connectors/" + self.node + "/screenshot-jobs/" + path
        request = urllib.request.Request(url, data=body, method=method, headers=headers)
        with self.opener.open(request, timeout=100) as response:
            if response.status == 204:
                return None
            if response.status != 200:
                raise ValueError("Unexpected center response")
            body = response.read(128 * 1024 + 1)
            if len(body) > 128 * 1024:
                raise ValueError("Center response exceeds the allowed size")
            return json.loads(body)


def capture_command(profile: dict, target: Path) -> tuple[list[str], bool]:
    device = profile["deviceId"]
    if profile["platform"] == "ios":
        device = str(uuid.UUID(device)).upper()
        return ["/usr/bin/xcrun", "simctl", "io", device, "screenshot", str(target)], False
    if profile["platform"] == "android" and re.fullmatch(r"emulator-[0-9]{4,5}", device):
        adb = str(local_path(profile["adbPath"]))
        return [adb, "-s", device, "exec-out", "screencap", "-p"], True
    raise ValueError("Configure an iOS simulator UUID or Android emulator serial")


def capture(root: Path, config: dict, job: dict) -> bytes:
    tmp = root / "tmp"
    tmp.mkdir(exist_ok=True)
    target = tmp / "screenshot.png"
    try:
        profile = config["profiles"][job["profile"]]
        project = local_path(profile["projectRoot"], directory=True)
        codex = local_path(config["codexPath"])
        codex_home = local_path(config["codexHome"], directory=True)
        permission = config["codexPermissionProfile"]
        if not re.fullmatch(r"[a-zA-Z][a-zA-Z0-9_-]{0,63}", permission):
            raise ValueError("A dedicated Codex permissions profile is required")
        command, capture_stdout = capture_command(profile, target)
        if capture_stdout:
            # Use an operator-started ADB server. The job does not own that daemon.
            with socket.create_connection(("127.0.0.1", 5037), timeout=2):
                pass
    except (KeyError, OSError, TypeError, ValueError):
        raise JobFailure("profile_unavailable") from None

    schema = tmp / "codex-result-schema.json"
    schema.write_text(json.dumps({
        "type": "object", "properties": {"ready": {"type": "boolean"}},
        "required": ["ready"], "additionalProperties": False,
    }), encoding="utf-8")
    result = tmp / "codex-result.json"
    env = {key: os.environ[key] for key in ("HOME", "PATH", "USER", "LOGNAME", "LANG", "DEVELOPER_DIR", "ANDROID_HOME", "ANDROID_SDK_ROOT") if key in os.environ}
    env.update({
        "CODEX_HOME": str(codex_home), "TMPDIR": str(tmp),
        "XDG_CACHE_HOME": str(tmp / "cache"), "GRADLE_USER_HOME": str(tmp / "gradle"),
        "XCODE_DERIVED_DATA_PATH": str(tmp / "derived-data"),
    })
    instructions = (
        "Prepare an app screen for a screenshot on the configured simulator. Decide whether the "
        "request needs building, installing, launching, or UI interaction. Use only the indicated "
        "simulator. The project is a local source reference. Keep generated files in the current "
        "working directory, use a temporary copy of source if a build needs to write in-tree, "
        "and use the explicit DerivedData or Gradle cache under this directory. "
        "Do not start or stop other devices or upload files. Leave the requested screen visible. "
        "Report ready=true only when that screen is ready for capture.\n"
        f"Project: {project}\nPlatform: {profile['platform']}\nSimulator: {command[2] if capture_stdout else command[3]}\n"
        f"Task:\n{job['prompt']}"
    )
    try:
        subprocess.run(
            [str(codex), "--ask-for-approval", "never", "--ignore-user-config",
             "-c", "default_permissions=" + json.dumps(permission), "exec",
             "--ephemeral", "--skip-git-repo-check", "--output-schema", str(schema),
             "--output-last-message", str(result), "-"],
            cwd=tmp, env=env, input=instructions.encode(), stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL, timeout=24 * 60, check=True,
        )
        if json.loads(bounded_read(result, 4096)) != {"ready": True}:
            raise ValueError("Codex did not prepare the requested screen")
    except (OSError, ValueError, subprocess.SubprocessError):
        raise JobFailure("codex_failed") from None

    try:
        if capture_stdout:
            with target.open("xb") as output:
                subprocess.run(command, cwd=tmp, env=env, stdout=output, stderr=subprocess.DEVNULL,
                               timeout=45, check=True)
        else:
            if target.exists() or target.is_symlink():
                raise ValueError("Screenshot destination is already occupied")
            subprocess.run(command, cwd=tmp, env=env, stdout=subprocess.DEVNULL,
                           stderr=subprocess.DEVNULL, timeout=45, check=True)
        image = bounded_read(target, MAX_IMAGE)
        if not image.startswith(PNG) or len(image) < 33:
            raise ValueError("Simulator did not return a PNG")
        return image
    except (OSError, ValueError, subprocess.SubprocessError):
        raise JobFailure("capture_failed") from None


def run_once(root: Path, config: dict) -> None:
    center = Center(config)
    job = center.request("POST", "claim")
    if job is None:
        return
    job_id = str(uuid.UUID(job["jobId"]))
    lease = str(uuid.UUID(job["lease"]))
    try:
        image = capture(root, config, job)
    except JobFailure as error:
        center.request("POST", job_id + "/failure", json.dumps({"code": str(error)}).encode(),
                       content_type="application/json", lease=lease)
        print("Screenshot job failed: " + str(error))
        return
    center.request("PUT", job_id + "/image", image, content_type="image/png", lease=lease)
    print("Screenshot delivered")


def self_test(root: Path) -> None:
    from unittest.mock import patch

    ios = {"platform": "ios", "deviceId": "99999999-9999-4999-8999-999999999999"}
    android = {"platform": "android", "deviceId": "emulator-5554", "adbPath": sys.executable}
    target = Path("/fixture/screenshot.png")
    assert capture_command(ios, target) == (
        ["/usr/bin/xcrun", "simctl", "io", ios["deviceId"], "screenshot", str(target)], False
    )
    assert capture_command(android, target)[0][1:] == ["-s", "emulator-5554", "exec-out", "screencap", "-p"]
    try:
        capture_command({**android, "deviceId": "physical-phone"}, target)
    except ValueError:
        pass
    else:
        raise AssertionError("A real Android device must never be selected")

    config = {"codexPath": sys.executable, "codexHome": str(root),
              "codexPermissionProfile": "fixture",
              "profiles": {"iphone": {**ios, "projectRoot": str(root)}}}
    prompt = "打开设置，保持这一页。"
    png = PNG + bytes(25)

    def fake_command(command, **kwargs):
        if "--output-last-message" in command:
            assert prompt.encode() in kwargs["input"] and prompt not in command
            assert kwargs["cwd"] == root / "tmp"
            Path(command[command.index("--output-last-message") + 1]).write_text('{"ready":true}')
        else:
            assert command[:5] == ["/usr/bin/xcrun", "simctl", "io", ios["deviceId"], "screenshot"]
            Path(command[-1]).write_bytes(png)

    with patch.object(subprocess, "run", fake_command):
        assert capture(root, config, {"profile": "iphone", "prompt": prompt}) == png


def main() -> int:
    try:
        root = managed_root()
        if sys.argv[1:] == ["--self-test"]:
            self_test(root)
            return 0
        if sys.platform != "darwin" or len(sys.argv) != 2:
            raise ValueError("Use a private Screenshot Connector config on macOS")
        config = json.loads(bounded_read(Path(sys.argv[1]), 64 * 1024, private=True))
        run_once(root, config)
        return 0
    except urllib.error.HTTPError as error:
        print(f"Screenshot Connector center request failed ({error.code})", file=sys.stderr)
    except (KeyError, OSError, RuntimeError, TypeError, ValueError):
        print("Screenshot Connector configuration or execution failed", file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
