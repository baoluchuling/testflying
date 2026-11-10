"""Exercise the macOS installer in an owned sandbox with a fake launchctl."""
from __future__ import annotations

import os
import json
from pathlib import Path
import pwd
import shlex
import subprocess
import sys
import tempfile

from verify_store_refactor import PROJECT, managed_root


def main() -> None:
    root = managed_root()
    if sys.platform != "darwin":
        return
    installer = (PROJECT / "packaging/connector/install.sh").read_text()
    binary = (Path(os.environ["CARGO_TARGET_DIR"]) / "debug/store-connector").resolve(strict=True)
    if not binary.is_relative_to(root) or not binary.is_file():
        raise RuntimeError("Installer tests require the Connector built inside this managed run")
    subprocess.run(["/bin/sh", "-n"], input=installer, text=True, check=True)
    with tempfile.TemporaryDirectory(dir=root / "tmp", prefix="connector-install-") as temporary:
        fixture = Path(temporary)
        base = fixture / "installed"
        plist = fixture / "connector.plist"
        bin_dir = fixture / "bin"
        bin_dir.mkdir()
        state = fixture / "launchd"
        state.mkdir()

        def fake(name: str, commands: str) -> None:
            executable = bin_dir / name
            executable.write_text("#!/bin/bash\nset -eu\n" + commands + "\n")
            executable.chmod(0o700)

        fake("id", 'if [ "$#" -eq 1 ] && [ "$1" = -u ]; then echo 0; else exec /usr/bin/id "$@"; fi')
        fake("stat", """
case "$2" in
  '%u %Lp') printf '0 '; exec /usr/bin/stat -f '%Lp' "$3" ;;
  '%u') echo 0 ;;
  *) exec /usr/bin/stat "$@" ;;
esac""")
        fake("install", """
args=()
while [ "$#" -gt 0 ]; do
    case "$1" in
        -o|-g) shift 2 ;;
        *) args+=("$1"); shift ;;
    esac
done
exec /usr/bin/install "${args[@]}" """.rstrip())
        fake("launchctl", """
case "$1" in
  print) test -f "$FIXTURE_LAUNCHD/loaded" ;;
  bootout) rm "$FIXTURE_LAUNCHD/loaded" ;;
  enable) ;;
  bootstrap)
    if [ -f "$FIXTURE_LAUNCHD/fail-next" ]; then
        rm "$FIXTURE_LAUNCHD/fail-next"
        exit 1
    fi
    /usr/bin/plutil -lint "$3" >/dev/null
    touch "$FIXTURE_LAUNCHD/loaded"
    ;;
  *) exit 1 ;;
esac""")

        substitutions = {
            "PATH=/usr/bin:/bin:/usr/sbin:/sbin": f"PATH={shlex.quote(str(bin_dir))}:/usr/bin:/bin:/usr/sbin:/sbin",
            "base='/Library/Application Support/Testflying Connector'": "base=" + shlex.quote(str(base)),
            'plist="/Library/LaunchDaemons/$label.plist"': "plist=" + shlex.quote(str(plist)),
        }
        for original, replacement in substitutions.items():
            assert installer.count(original) == 1, "Installer fixture path drift"
            installer = installer.replace(original, replacement)

        env = {**os.environ, "SUDO_USER": pwd.getpwuid(os.getuid()).pw_name, "FIXTURE_LAUNCHD": str(state),
               "FIXTURE_CONNECTOR_BINARY": str(binary)}
        package = fixture / "package"
        package.mkdir()
        (package / "install.sh").write_text(installer)
        (package / "testflying-prod-ca.crt").write_text("fake certificate")
        def program(version: int) -> bytes:
            content = ('#!/bin/sh\n# fixture program version ' + str(version) + '\n'
                       'exec "$FIXTURE_CONNECTOR_BINARY" "$@"\n').encode()
            (package / "store-connector").write_bytes(content)
            (package / "store-connector").chmod(0o700)
            return content

        version1 = program(1)
        (package / "node-id.txt").write_text("test-node\n")
        config = {"connectorId": "test-node", "token": "fixture-only original-token", "platformMode": "android",
                  "listenAddr": "127.0.0.1:8100", "provider": {"mode": "live",
                  "googleServiceAccountPath": "credentials/google-service-account.json", "storeProxyUrl": "http://proxy.invalid"}}
        (package / "connector.json").write_text(json.dumps(config))
        (package / "credentials").mkdir()
        google = "credentials/google-service-account.json"
        apple = "credentials/apple-private-key.p8"
        google_bytes = json.dumps({"type": "service_account", "client_email": "fixture@example.invalid",
                                   "private_key": "-----BEGIN PRIVATE KEY-----\nfixture-only key\n-----END PRIVATE KEY-----"}).encode()
        apple_bytes = b"-----BEGIN PRIVATE KEY-----\nfixture-only key\n-----END PRIVATE KEY-----\n"
        (package / google).write_bytes(google_bytes)

        def run(expected_status: int, expected_data: dict[str, bytes] | None = None) -> dict[str, bytes]:
            result = subprocess.run(["/bin/sh", str(package / "install.sh")], env=env, check=False)
            assert result.returncode == expected_status
            assert (state / "loaded").is_file()
            assert not (base / ".install").exists()
            actual = {relative: (base / "data" / relative).read_bytes()
                      for relative in ("connector.json", google, apple, "user-note.txt")
                      if (base / "data" / relative).is_file()}
            if expected_data is not None:
                assert actual == expected_data, "Installed configuration or credentials changed unexpectedly"
            return actual

        initial = run(0)
        assert json.loads(initial["connector.json"]) == config
        assert (base / "program/store-connector").read_bytes() == version1
        (base / "data/user-note.txt").write_bytes(b"preserved user file")
        initial["user-note.txt"] = b"preserved user file"
        (package / "connector.json").unlink()
        (package / google).unlink()
        (package / "credentials").rmdir()
        version2 = program(2)
        run(0, initial)
        assert (base / "program/store-connector").read_bytes() == version2
        (package / "node-id.txt").write_text("other-node\n")
        run(1, initial)
        (package / "node-id.txt").write_text("test-node\n")

        incoming = {"connectorId": "test-node", "token": "incoming identity must not replace original",
                    "provider": {"mode": "live", "appleIssuerId": "fixture-issuer", "appleKeyId": "fixture-key", "applePrivateKeyPath": apple}}
        (package / "connector.json").write_text(json.dumps(incoming))
        (package / "credentials").mkdir()
        version3 = program(3)
        run(1, initial)  # Missing new platform file fails before touching installed data.
        (package / apple).write_bytes(apple_bytes)
        combined = run(0)
        actual = json.loads(combined["connector.json"])
        assert actual["platformMode"] == "all"
        assert actual["token"] == config["token"]
        assert actual["listenAddr"] == config["listenAddr"]
        assert actual["provider"]["googleServiceAccountPath"] == google
        assert actual["provider"]["storeProxyUrl"] == config["provider"]["storeProxyUrl"]
        assert combined[google] == google_bytes and combined[apple] == apple_bytes
        assert combined["user-note.txt"] == initial["user-note.txt"]
        assert (base / "program/store-connector").read_bytes() == version3

        program(4)
        incoming["provider"]["appleKeyId"] = "fixture-rotated-key"
        (package / "connector.json").write_text(json.dumps(incoming))
        (package / apple).write_bytes(apple_bytes.replace(b"fixture-only", b"rotated-fixture"))
        (state / "fail-next").touch()
        run(1, combined)
        assert (base / "program/store-connector").read_bytes() == version3
        updated = run(0)
        assert json.loads(updated["connector.json"])["provider"]["appleKeyId"] == "fixture-rotated-key"
        assert updated[google] == google_bytes
    print("macOS installer: migration, byte-preserving program upgrade, additive platform merge, selected credential update, input rejection and config/credential/program rollback passed")


if __name__ == "__main__":
    main()
