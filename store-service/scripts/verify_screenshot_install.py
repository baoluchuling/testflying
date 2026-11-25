"""Exercise screenshot installation without touching real launchd or privacy settings."""
from __future__ import annotations

import json
import os
from pathlib import Path
import pty
import pwd
import shlex
import subprocess
import sys
import tempfile
import time

from managed_workspace import PROJECT, managed_root


def main() -> None:
    root = managed_root()
    original = (PROJECT / "packaging/screenshot-connector/install.sh").read_text()
    subprocess.run(["/bin/sh", "-n"], input=original, text=True, check=True)
    with tempfile.TemporaryDirectory(dir=root / "tmp", prefix="screenshot-install-") as directory:
        fixture = Path(directory)
        account_home = fixture / "account"
        package = account_home / "Library/Application Support/截图 Connector fixture"
        package.mkdir(parents=True, mode=0o700)
        package.chmod(0o700)
        state = fixture / "state"
        state.mkdir()
        launchd = fixture / "LaunchDaemons"
        launchd.mkdir()
        commands = fixture / "bin"
        commands.mkdir()
        node = "11111111-1111-4111-8111-111111111111"
        plist = launchd / f"com.testflying.screenshot-connector.{node}.plist"
        config = package / "connector.json"
        config.write_text(json.dumps({"kind": "screenshot", "connectorId": node, "token": "fixture-only-token"}))
        config.chmod(0o600)
        config_bytes = config.read_bytes()
        (package / "run.sh").write_text("#!/bin/sh\nexit 0\n")
        binary = package / "screenshot-connector"
        binary.write_text("#!/bin/sh\nexit 0\n")
        binary.chmod(0o700)
        event_log = state / "events.jsonl"
        service_path = f"{fixture / 'account bin'}:/usr/bin:/bin:/usr/sbin:/sbin"
        env = {**os.environ, "FIXTURE_PRIVILEGED": "0", "FIXTURE_GUI": "1",
               "HOME": str(fixture / "caller-home"), "PATH": service_path, "FIXTURE_PATH": service_path,
               "FIXTURE_STATE": str(state), "FIXTURE_LOG": str(event_log),
               "FIXTURE_LAUNCHD": str(launchd), "FIXTURE_PLIST": str(plist),
               "FIXTURE_PACKAGE": str(package), "FIXTURE_ACCOUNT": str(account_home),
               "FIXTURE_UID": str(os.getuid()), "FIXTURE_USER": pwd.getpwuid(os.getuid()).pw_name}
        for key in ("SSH_CONNECTION", "SSH_TTY", "SSH_CLIENT"):
            env.pop(key, None)

        def fake(name: str, body: str) -> None:
            path = commands / name
            path.write_text(f"#!{sys.executable} -B\n" +
                "import json,os,sys,subprocess,plistlib\nfrom pathlib import Path\n" +
                "args=sys.argv[1:]\n" +
                f"with open(os.environ['FIXTURE_LOG'],'a') as log: log.write(json.dumps({{'tool':{name!r},'args':args,'privileged':os.environ['FIXTURE_PRIVILEGED']}})+'\\n')\n" + body + "\n")
            path.chmod(0o700)

        fake("id", """
if args==['-u']:
 print('0' if os.environ['FIXTURE_PRIVILEGED']=='1' else os.environ['FIXTURE_UID'])
elif args==['-un']: print(os.environ['FIXTURE_USER'])
elif args==['-u',os.environ['FIXTURE_USER']]: print(os.environ['FIXTURE_UID'])
else: sys.exit(1)
""")
        fake("dscl", "print('NFSHomeDirectory: '+os.environ['FIXTURE_ACCOUNT'])")
        fake("stat", """
if args==['-f','%u %Lp',os.environ['FIXTURE_PLIST']]: print('0 644')
else: os.execv('/usr/bin/stat',['/usr/bin/stat',*args])
""")
        fake("open", """
assert os.environ['FIXTURE_PRIVILEGED']=='0', 'Privacy UI must run as the screenshot account'
sys.exit(int(os.environ.get('FIXTURE_OPEN_FAIL','0')))
""")
        fake("sudo", """
assert args[:2]==['--','/bin/sh']
env={**os.environ,'FIXTURE_PRIVILEGED':'1','SUDO_UID':os.environ['FIXTURE_UID'],'SUDO_USER':os.environ['FIXTURE_USER'],'PATH':'/usr/sbin:/sbin'}
os.execve('/bin/sh',['/bin/sh',*args[2:]],env)
""")
        fake("launchctl", """
state=Path(os.environ['FIXTURE_STATE']); loaded=state/'loaded'; generation=state/'generation'
if args[0]=='print':
 sys.exit(0 if (os.environ['FIXTURE_GUI']=='1' if args[1].startswith('gui/') else loaded.exists()) else 1)
assert os.environ['FIXTURE_PRIVILEGED']=='1'
if args[0]=='enable': sys.exit(0)
if args[0]=='kill':
 assert args[1]=='SIGTERM' and loaded.exists()
elif args[0]=='bootstrap':
 assert args==['bootstrap','system',os.environ['FIXTURE_PLIST']]
 with open(args[2],'rb') as f: p=plistlib.load(f)
 assert p['KeepAlive'] is True and p['RunAtLoad'] is True
 assert p['UserName']==os.environ['FIXTURE_USER']
 assert p['ProgramArguments']==['/bin/sh',os.environ['FIXTURE_PACKAGE']+'/run.sh','--service']
 assert p['EnvironmentVariables']=={'HOME':os.environ['FIXTURE_ACCOUNT'],'PATH':os.environ['FIXTURE_PATH']}
 loaded.touch()
else: sys.exit(1)
generation.write_text(str(int(generation.read_text())+1 if generation.exists() else 1))
""")
        installer = original
        for name, absolute in {"id": "/usr/bin/id", "dscl": "/usr/bin/dscl", "stat": "/usr/bin/stat",
                               "open": "/usr/bin/open", "sudo": "/usr/bin/sudo", "launchctl": "/bin/launchctl"}.items():
            assert absolute in installer
            installer = installer.replace(absolute, shlex.quote(str(commands / name)))
        for before, after in {
            'plist="/Library/LaunchDaemons/$label.plist"': 'plist="$FIXTURE_LAUNCHD/$label.plist"',
            'stage=$(/usr/bin/mktemp "/Library/LaunchDaemons/.$label.XXXXXXXX")':
                'stage=$(/usr/bin/mktemp "$FIXTURE_LAUNCHD/.$label.XXXXXXXX")',
        }.items():
            assert installer.count(before) == 1
            installer = installer.replace(before, after)
        assert "/Library/LaunchDaemons/" not in installer
        script = package / "install.sh"
        script.write_text(installer)

        def events() -> list[dict]:
            return [json.loads(line) for line in event_log.read_text().splitlines()] if event_log.exists() else []

        def generation() -> int:
            path = state / "generation"
            return int(path.read_text()) if path.exists() else 0

        def unchanged() -> None:
            assert config.read_bytes() == config_bytes
            assert not list(launchd.glob(".*")), "Installer left a staging plist"

        def run(args: list[str], expected: int = 0, extra: dict | None = None) -> str:
            result = subprocess.run(["/bin/sh", str(script), *args], stdin=subprocess.DEVNULL,
                                    stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                    env={**env, **(extra or {})}, timeout=20)
            assert result.returncode == expected, result.stdout.decode()
            unchanged()
            return result.stdout.decode()

        def interactive(args: list[str], eof: bool = False, extra: dict | None = None) -> str:
            master, slave = pty.openpty()
            before = len(events())
            previous = generation()
            process = subprocess.Popen(["/bin/sh", str(script), *args], stdin=slave,
                                       stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                       env={**env, **(extra or {})})
            os.close(slave)
            try:
                deadline = time.monotonic() + 10
                while len([e for e in events()[before:] if e['tool'] == 'open']) < 2:
                    assert time.monotonic() < deadline and process.poll() is None
                    time.sleep(0.02)
                time.sleep(0.05)
                assert process.poll() is None and generation() == previous, "Service changed before user acknowledgement"
                assert not any(e['tool'] == 'sudo' for e in events()[before:])
                os.write(master, b'\x04' if eof else b'\n')
                output = process.communicate(timeout=20)[0].decode()
                assert process.returncode == (1 if eof else 0), output
                assert generation() == previous + (0 if eof else 1)
                opened = [e for e in events()[before:] if e['tool'] == 'open']
                assert opened[0]['args'] == ['-R', str(binary)]
                assert opened[1]['args'] == ['x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles']
                unchanged()
                return output
            finally:
                os.close(master)
                if process.poll() is None:
                    process.terminate()
                    process.wait(timeout=5)

        run(["--unknown"], 1)
        assert not plist.exists() and not any(e['tool'] == 'open' for e in events())
        text = run([])
        assert '未确认授权' in text and generation() == 1
        assert not any(e['tool'] == 'open' for e in events())
        original_plist = plist.read_bytes()
        before = len(events())
        run(["--permissions"])
        assert generation() == 1 and not any(e['tool'] in ('open', 'sudo') for e in events()[before:])
        for context in ({"FIXTURE_GUI": "0"}, {"SSH_CONNECTION": "fixture-remote-session"}):
            master, slave = pty.openpty()
            try:
                result = subprocess.run(["/bin/sh", str(script), "--permissions"], stdin=slave,
                                        stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                        env={**env, **context}, timeout=20)
                assert result.returncode == 0 and '未确认授权' in result.stdout.decode()
                assert generation() == 1 and not any(e['tool'] in ('open', 'sudo') for e in events()[before:])
            finally:
                os.close(master)
                os.close(slave)
        interactive(["--permissions"])
        interactive([])
        text = interactive(["--permissions"], extra={"FIXTURE_OPEN_FAIL": "1"})
        assert '请按上方路径手动进入系统设置' in text
        before = len(events())
        previous = generation()
        run(["--restart"], extra={"PATH": "/usr/bin:/bin"})
        assert generation() == previous + 1 and not any(e['tool'] == 'open' for e in events()[before:])
        assert plist.read_bytes() == original_plist
        (state / "loaded").unlink()
        plist.unlink()
        run(["--restart"], 1)
        assert not plist.exists() and not (state / "loaded").exists()
        interactive([], eof=True)
        assert not plist.exists() and not (state / "loaded").exists()
        interactive([])
        assert plist.exists() and (state / "loaded").exists()
        assert not any(e['tool'] == 'open' and e['privileged'] != '0' for e in events())
    print("Screenshot installer: local acknowledgement before service changes, private path selection, headless deferral, UI fallback, EOF cancellation, missing-install rejection, launchd HOME/PATH and restart, and byte-preserving credentials passed")


if __name__ == "__main__":
    main()
