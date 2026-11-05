"""Run private-profile Chrome UI checks only inside the artifact supervisor."""
from __future__ import annotations

import json
import os
from pathlib import Path
import subprocess
import sys
import time

from verify_store_refactor import PROJECT, managed_root


def main() -> None:
    root = managed_root()
    tmp = root / "tmp"
    tmp.mkdir(exist_ok=True)
    if sys.argv[1:] not in ([], ["production"]):
        raise RuntimeError("Use no argument for loopback tests, or production for the installed service")
    env = {**os.environ, "TMPDIR": str(tmp), "XDG_CACHE_HOME": str(tmp / "cache"),
           "NPM_CONFIG_CACHE": str(tmp / "npm-cache"), "PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD": "1"}
    subprocess.run(["npm", "install", "--prefix", str(tmp / "browser"), "--ignore-scripts", "--no-audit",
                    "--no-fund", "--package-lock=false", "playwright-core@1.56.1"], cwd=PROJECT, env=env,
                   check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    if sys.argv[1:] == ["production"]:
        code = "import json; e=dict(l.strip().split('=',1)for l in open('/etc/testflying-store-api/service.env')if '='in l and not l.startswith('#')); print(json.dumps({'url':e['STORE_API_PUBLIC_URL'],'token':e['STORE_API_ADMIN_TOKEN']}))"
        result = subprocess.run(["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=8", "-o",
                                 "StrictHostKeyChecking=yes", "-o", "LogLevel=ERROR", "testflying-prod",
                                 "python3 -c " + __import__("shlex").quote(code)], stdout=subprocess.PIPE,
                                stderr=subprocess.PIPE, check=True, timeout=45)
        private = json.loads(result.stdout)
        env["SETUP_BASE_URL"] = private["url"].replace("https://", "http://", 1)
        env["SETUP_ADMIN_TOKEN"] = private["token"]
    profile = tmp / "chrome-profile"
    executable = os.environ.get("SETUP_CHROME_EXECUTABLE", "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")
    chrome = subprocess.Popen([executable, "--headless=new", "--remote-debugging-port=0",
                               "--remote-debugging-address=127.0.0.1", "--user-data-dir=" + str(profile),
                               "--no-first-run", "--no-default-browser-check", "--disable-background-networking",
                               "--disable-sync", "--disable-component-update", "--disable-breakpad",
                               "--disable-crash-reporter", "--password-store=basic", "--use-mock-keychain"],
                              env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        endpoint = profile / "DevToolsActivePort"
        for _ in range(100):
            if endpoint.exists():
                break
            if chrome.poll() is not None:
                raise RuntimeError("Owned headless Chrome exited before becoming ready")
            time.sleep(0.1)
        else:
            raise RuntimeError("Owned headless Chrome did not become ready")
        port, socket = endpoint.read_text().splitlines()[:2]
        env["SETUP_CDP_ENDPOINT"] = "ws://127.0.0.1:" + port + socket
        subprocess.run(["node", "scripts/verify_setup_ui.mjs"], cwd=PROJECT, env=env, check=True)
    finally:
        chrome.terminate()
        try:
            chrome.wait(timeout=10)
        except subprocess.TimeoutExpired:
            chrome.kill()
            chrome.wait(timeout=5)


if __name__ == "__main__":
    main()
