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
        updater_plist = fixture / "updater.plist"
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
  '%u %l') printf '0 '; exec /usr/bin/stat -f '%l' "$3" ;;
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
        fake("ls", """
[ "$1" = -lde ] && [ "$#" = 2 ] || exit 91
if [ -f "$FIXTURE_LAUNCHD/acl-path" ] && [ "$(cat "$FIXTURE_LAUNCHD/acl-path")" = "$2" ]; then
    printf 'drwxr-xr-x+ 1 root wheel 0 Nov 27 00:00 %s\\n' "$2"
    printf ' 0: user:fixture allow add_file,delete_child\\n'
elif [ "$2" = /Library ]; then
    printf 'drwxr-xr-x+ 1 root wheel 0 Nov 27 00:00 %s\\n' "$2"
    printf ' 0: group:everyone deny delete\\n'
else
    printf 'drwxr-xr-x 1 root wheel 0 Nov 27 00:00 %s\\n' "$2"
fi
""")
        fake("kill", "exit 92")  # No fake job has a real PID; real signal delivery is forbidden.
        fake("sudo", "exit 93")
        fake("launchctl", """
printf '%s\\n' "$*" >> "$FIXTURE_LAUNCHD/calls"
if [ "$1" = print-disabled ]; then
    [ "$2" = system ] && [ "$#" = 2 ] || exit 94
    echo 'disabled services = {'
    for label in com.testflying.connector com.testflying.connector.updater; do
        value=false
        [ ! -f "$FIXTURE_LAUNCHD/disabled-$label" ] || value=true
        printf '  "%s" => %s\\n' "$label" "$value"
    done
    echo '}'
    exit 0
fi
if [ "$1" = bootstrap ]; then
    [ "$2" = system ] && [ "$#" = 3 ] || exit 95
    case "$3" in
        "$FIXTURE_WORKER_PLIST") label=com.testflying.connector ;;
        "$FIXTURE_UPDATER_PLIST") label=com.testflying.connector.updater ;;
        *) exit 96 ;;
    esac
else
    [ "$#" = 2 ] || exit 97
    case "$2" in
        system/com.testflying.connector) label=com.testflying.connector ;;
        system/com.testflying.connector.updater) label=com.testflying.connector.updater ;;
        *) exit 98 ;;
    esac
fi
loaded="$FIXTURE_LAUNCHD/loaded-$label"
disabled="$FIXTURE_LAUNCHD/disabled-$label"
case "$1" in
  print) test -f "$loaded"; echo 'state = running' ;;
  bootout) rm "$loaded" ;;
  enable) rm -f "$disabled" ;;
  disable) touch "$disabled" ;;
  bootstrap)
    if [ -f "$FIXTURE_LAUNCHD/fail-next-$label" ]; then
        rm "$FIXTURE_LAUNCHD/fail-next-$label"
        exit 1
    fi
    [ ! -f "$disabled" ] || exit 99
    /usr/bin/plutil -lint "$3" >/dev/null
    touch "$loaded"
    ;;
  *) exit 1 ;;
esac""")

        substitutions = {
            "PATH=/usr/bin:/bin:/usr/sbin:/sbin": f"PATH={shlex.quote(str(bin_dir))}:/usr/bin:/bin:/usr/sbin:/sbin",
            "base='/Library/Application Support/Testflying Connector'": "base=" + shlex.quote(str(base)),
            'plist="/Library/LaunchDaemons/$label.plist"': "plist=" + shlex.quote(str(plist)),
            'updater_plist="/Library/LaunchDaemons/$updater_label.plist"': "updater_plist=" + shlex.quote(str(updater_plist)),
            'acl_listing=$(LC_ALL=C /bin/ls -lde "$1")': 'acl_listing=$(LC_ALL=C ' + shlex.quote(str(bin_dir / "ls")) + ' -lde "$1")',
            '/bin/kill -0 "$stopped_pid"': shlex.quote(str(bin_dir / "kill")) + ' -0 "$stopped_pid"',
        }
        for original, replacement in substitutions.items():
            expected_occurrences = 2 if original.startswith('acl_listing=') else 1
            assert installer.count(original) == expected_occurrences, "Installer fixture path drift"
            installer = installer.replace(original, replacement)
        assert '/Library/LaunchDaemons/' not in installer, "A real launchd plist path escaped sandbox substitution"

        env = {**os.environ, "SUDO_USER": pwd.getpwuid(os.getuid()).pw_name, "FIXTURE_LAUNCHD": str(state),
               "FIXTURE_CONNECTOR_BINARY": str(binary), "FIXTURE_WORKER_PLIST": str(plist),
               "FIXTURE_UPDATER_PLIST": str(updater_plist)}
        package = fixture / "package"
        package.mkdir()
        (package / "install.sh").write_text(installer)
        (package / "testflying-prod-ca.crt").write_text("fake certificate")
        def program(version: int) -> bytes:
            content = ('#!/bin/sh\n# fixture program version ' + str(version) + '\n'
                       'case "${1:-}" in --prepare-install|--validate-update-trust) ;; *) exit 90 ;; esac\n'
                       'exec "$FIXTURE_CONNECTOR_BINARY" "$@"\n').encode()
            (package / "store-connector").write_bytes(content)
            (package / "store-connector").chmod(0o700)
            return content

        version1 = program(1)
        node = "00000000-0000-4000-8000-000000000001"
        (package / "node-id.txt").write_text(node + "\n")
        trust = {"protocol": 1, "nodeId": node, "serverUrl": "https://fixture.invalid",
                 "target": "darwin-arm64", "publicKey": "04"
                 "6b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c296"
                 "4fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5"}
        (package / "update-trust.json").write_text(json.dumps(trust))
        config = {"connectorId": node, "serverUrl": trust["serverUrl"], "token": "fixture-only original-token", "platformMode": "android",
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

        labels = ("com.testflying.connector", "com.testflying.connector.updater")
        def services() -> dict[str, tuple[bool, bool]]:
            return {label: ((state / f"loaded-{label}").is_file(), (state / f"disabled-{label}").is_file())
                    for label in labels}

        def run(expected_status: int, expected_data: dict[str, bytes] | None = None,
                expected_services: dict[str, tuple[bool, bool]] | None = None) -> dict[str, bytes]:
            result = subprocess.run(["/bin/sh", str(package / "install.sh")], env=env, check=False)
            assert result.returncode == expected_status
            assert services() == (expected_services or {label: (True, False) for label in labels})
            assert not (base / ".install").exists()
            actual = {relative: (base / "data" / relative).read_bytes()
                      for relative in ("connector.json", google, apple, "user-note.txt")
                      if (base / "data" / relative).is_file()}
            if expected_data is not None:
                assert actual == expected_data, "Installed configuration or credentials changed unexpectedly"
            return actual

        (state / "fail-next-com.testflying.connector.updater").touch()
        run(1, {}, {label: (False, False) for label in labels})
        assert not plist.exists() and not updater_plist.exists()
        assert not (base / "program").exists() and not (base / "updater").exists()
        initial = run(0)
        assert json.loads(initial["connector.json"]) == config
        assert (base / "program/store-connector").read_bytes() == version1
        assert (base / "updater/store-connector").read_bytes() == version1
        original_trust = (package / "update-trust.json").read_bytes()
        assert (base / "updater/trust.json").read_bytes() == original_trust
        assert json.loads(subprocess.check_output(["/usr/bin/plutil", "-extract", "ProgramArguments", "json", "-o", "-", str(updater_plist)])) == [str(base / "updater/store-connector"), "--upgrade-manager"]
        (base / "data/user-note.txt").write_bytes(b"preserved user file")
        initial["user-note.txt"] = b"preserved user file"
        (package / "connector.json").unlink()
        (package / google).unlink()
        (package / "credentials").rmdir()
        version2 = program(2)
        run(0, initial)
        assert (base / "program/store-connector").read_bytes() == version2
        assert (base / "updater/store-connector").read_bytes() == version2
        assert (base / "updater/trust.json").read_bytes() == original_trust
        (package / "node-id.txt").write_text("other-node\n")
        run(1, initial)
        (package / "node-id.txt").write_text(node + "\n")

        incoming = {"connectorId": node, "token": "incoming identity must not replace original",
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
        (state / "fail-next-com.testflying.connector").touch()
        run(1, combined)
        assert (base / "program/store-connector").read_bytes() == version3
        assert (base / "updater/store-connector").read_bytes() == version3
        assert (base / "updater/trust.json").read_bytes() == original_trust

        # A failed manager registration/start restores both binaries, trust and
        # each service's independent loaded and disabled states.
        trust["publicKey"] = "03" + trust["publicKey"][2:66]
        (package / "update-trust.json").write_text(json.dumps(trust))
        (state / "disabled-com.testflying.connector").touch()
        (state / "disabled-com.testflying.connector.updater").touch()
        before = services()
        (state / "fail-next-com.testflying.connector.updater").touch()
        run(1, combined, before)
        assert (base / "program/store-connector").read_bytes() == version3
        assert (base / "updater/store-connector").read_bytes() == version3
        assert (base / "updater/trust.json").read_bytes() == original_trust
        for label in labels:
            (state / f"disabled-{label}").unlink()

        for unloaded, disabled, failed in [
            (labels[1], labels[1], labels[1]),
            (labels[0], labels[0], labels[0]),
        ]:
            (state / f"loaded-{unloaded}").unlink()
            (state / f"disabled-{disabled}").touch()
            before = services()
            (state / f"fail-next-{failed}").touch()
            run(1, combined, before)
            assert (base / "updater/store-connector").read_bytes() == version3
            assert (base / "updater/trust.json").read_bytes() == original_trust
            (state / f"loaded-{unloaded}").touch()
            (state / f"disabled-{disabled}").unlink()

        # Invalid trust must fail before either service is stopped.
        good_trust = (package / "update-trust.json").read_bytes()
        trust["nodeId"] = "00000000-0000-4000-8000-000000000099"
        (package / "update-trust.json").write_text(json.dumps(trust))
        (state / "calls").write_text("")
        run(1, combined)
        assert "bootout" not in (state / "calls").read_text()
        (package / "update-trust.json").write_bytes(good_trust)

        # Extended allow ACLs on privileged paths are rejected without changing
        # the program or touching a foreign tree; system deny ACLs remain valid.
        (state / "acl-path").write_text(str(base / "updater"))
        run(1, combined)
        assert (base / "updater/store-connector").read_bytes() == version3
        (state / "acl-path").unlink()
        foreign = fixture / "foreign"
        foreign.mkdir()
        (foreign / "sentinel").write_bytes(b"unrelated user data")
        # A pre-existing lock is not owned by this installer invocation and must
        # survive rejection, including when it points outside the installation.
        install_lock = base / ".install"
        install_lock.symlink_to(foreign, target_is_directory=True)
        rejected = subprocess.run(["/bin/sh", str(package / "install.sh")], env=env, check=False)
        assert rejected.returncode == 1 and install_lock.is_symlink()
        assert (foreign / "sentinel").read_bytes() == b"unrelated user data"
        install_lock.unlink()
        helper = base / "updater/store-connector"
        saved_helper = helper.read_bytes()
        helper.unlink()
        helper.symlink_to(foreign / "sentinel")
        run(1, combined)
        assert (foreign / "sentinel").read_bytes() == b"unrelated user data"
        helper.unlink()
        helper.write_bytes(saved_helper)
        helper.chmod(0o755)
        updated = run(0)
        assert json.loads(updated["connector.json"])["provider"]["appleKeyId"] == "fixture-rotated-key"
        assert updated[google] == google_bytes
        assert (base / "updater/trust.json").read_bytes() == good_trust
        assert (foreign / "sentinel").read_bytes() == b"unrelated user data"
    print("macOS installer: full/program-only migration, real trust validation, dual-service startup/rollback, independent loaded/disabled restoration, helper/trust/config preservation, ACL rejection and cleanup ownership protection passed")


if __name__ == "__main__":
    main()
