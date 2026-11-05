"""Exercise the macOS installer in an owned sandbox with a fake launchctl."""
from __future__ import annotations

import os
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

        env = {**os.environ, "SUDO_USER": pwd.getpwuid(os.getuid()).pw_name, "FIXTURE_LAUNCHD": str(state)}
        package = fixture / "package"
        package.mkdir()
        (package / "install.sh").write_text(installer)
        (package / "testflying-prod-ca.crt").write_text("fake certificate")
        (package / "store-connector").write_bytes(b"program v1")
        (package / "node-id.txt").write_text("test-node\n")
        config = b'{\n  "connectorId": "test-node", "token": "fixture-only"\n}\n'
        (package / "connector.json").write_bytes(config)
        (package / "credentials").mkdir()
        key_name = "credentials/apple-private-key.p8"
        (package / key_name).write_bytes(b"fixture-only key")

        def run(expected_status: int) -> None:
            result = subprocess.run(["/bin/sh", str(package / "install.sh")], env=env, check=False)
            assert result.returncode == expected_status
            assert (state / "loaded").is_file()
            assert (base / "data/connector.json").read_bytes() == config
            assert (base / "data" / key_name).read_bytes() == b"fixture-only key"
            assert not (base / ".install").exists()

        run(0)
        assert (base / "program/store-connector").read_bytes() == b"program v1"
        (package / "connector.json").unlink()
        (package / key_name).unlink()
        (package / "credentials").rmdir()
        (package / "store-connector").write_bytes(b"program v2")
        run(0)
        assert (base / "program/store-connector").read_bytes() == b"program v2"
        (package / "node-id.txt").write_text("other-node\n")
        run(1)
        (package / "node-id.txt").write_text("test-node\n")
        (package / "store-connector").write_bytes(b"program v3")
        (state / "fail-next").touch()
        run(1)
        assert (base / "program/store-connector").read_bytes() == b"program v2"
    print("macOS installer: migration, program-only upgrade, node mismatch and rollback passed")


if __name__ == "__main__":
    main()
