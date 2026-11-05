"""Roll out the API-only setup page while retaining Connector binaries and secrets."""
from __future__ import annotations

import datetime
import hashlib
import http.client
import io
import json
import os
import shlex
import ssl
import subprocess
import sys
import tarfile
import time
import urllib.parse

from verify_store_refactor import PROJECT, managed_root, source_hashes


def main() -> None:
    root = managed_root()
    tmp = root / "tmp"
    tmp.mkdir(exist_ok=True)
    ssh = ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=8", "-o",
           "StrictHostKeyChecking=yes", "-o", "LogLevel=ERROR", "testflying-prod"]

    def remote(code: str, data: bytes | None = None) -> bytes:
        result = subprocess.run(ssh + ["python3 -c " + shlex.quote(code)], input=data,
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=90)
        if result.returncode:
            raise RuntimeError("Verified deployment SSH operation failed")
        return result.stdout

    def git(*args: str) -> bytes:
        return subprocess.check_output(["git", *args], cwd=PROJECT, stderr=subprocess.DEVNULL)

    commit = git("rev-parse", "HEAD").decode().strip()
    if git("diff", "--name-only", "HEAD", "--", *source_hashes()).strip():
        raise RuntimeError("Commit the Store API source before deploying")
    pushed = subprocess.check_output(["gh", "api", "repos/baoluchuling/testflying/git/ref/heads/refactor/store-service-rust",
                                      "--jq", ".object.sha"], cwd=PROJECT, text=True).strip()
    if pushed != commit:
        raise RuntimeError("Push the deployment commit first")
    build = json.loads((PROJECT / "docs/setup-api-build.json").read_text())
    verified = json.loads((PROJECT / "docs/verification.json").read_text())
    if source_hashes() != build["sourceHashes"] or source_hashes() != verified["sourceHashes"]:
        raise RuntimeError("API sources differ from the verified release")
    api = PROJECT / "releases/linux-amd64/store-api"
    expected_api = build["outputs"]["releases/linux-amd64/store-api"]["sha256"]
    if hashlib.sha256(api.read_bytes()).hexdigest() != expected_api:
        raise RuntimeError("Linux API checksum mismatch")
    snapshot = json.loads(remote("""
import os,json,hashlib,subprocess
e=open('/etc/testflying-store-api/service.env','rb').read()
v=dict(l.strip().split('=',1)for l in e.decode().splitlines()if '='in l and not l.startswith('#'))
current=os.path.realpath('/opt/testflying-store-api/current')
hashes={}
for target,name in [('linux-amd64','store-connector'),('darwin-arm64','store-connector'),('windows-amd64','store-connector.exe')]:
 p=os.path.join(current,'releases',target,name)
 hashes[target]=hashlib.sha256(open(p,'rb').read()).hexdigest()
print(json.dumps({'current':current,'environmentSha256':hashlib.sha256(e).hexdigest(),
 'proxySha256':hashlib.sha256(open('/etc/bugsmith/zentao-tls/nginx.conf','rb').read()).hexdigest(),
 'url':v['STORE_API_PUBLIC_URL'],'admin':v['STORE_API_ADMIN_TOKEN'],'token':v['STORE_API_TOKEN'],
 'binaries':hashes,'available':int(subprocess.check_output(['df','--output=avail','-B1','/opt']).decode().splitlines()[-1])}))
"""))
    if snapshot["available"] <= 21474836480:
        raise RuntimeError("Server free-space reserve is insufficient")
    for target, report_name in [("linux-amd64", "deployment-build.json"), ("darwin-arm64", "deployment-build.json"),
                                ("windows-amd64", "windows-release-build.json")]:
        old = json.loads((PROJECT / "docs" / report_name).read_text())
        binary_name = "store-connector.exe" if target.startswith("windows") else "store-connector"
        if snapshot["binaries"][target] != old["outputs"][f"releases/{target}/{binary_name}"]["sha256"]:
            raise RuntimeError("An existing Connector binary changed")
        for relative, digest in old["sourceHashes"].items():
            if relative.startswith(("crates/store-protocol/", "crates/store-provider/", "crates/store-connector/")) or relative in ("Cargo.toml", "Cargo.lock"):
                if hashlib.sha256((PROJECT / relative).read_bytes()).hexdigest() != digest:
                    raise RuntimeError("Connector source compatibility changed")
    source_archive = git("archive", "--format=tar", "--prefix=source/", commit + ":store-service")
    archive = io.BytesIO()
    with tarfile.open(fileobj=archive, mode="w") as output:
        with tarfile.open(fileobj=io.BytesIO(source_archive), mode="r:") as source:
            for member in source:
                if not (member.isfile() or member.isdir()) or ".." in member.name.split("/"):
                    raise RuntimeError("Unexpected committed source member")
                output.addfile(member, source.extractfile(member) if member.isfile() else None)
        member = tarfile.TarInfo("bin/store-api")
        member.size, member.mode = api.stat().st_size, 0o755
        with api.open("rb") as stream:
            output.addfile(member, stream)
    payload = archive.getvalue()
    if len(payload) > 64 * 1024 * 1024:
        raise RuntimeError("Deployment archive exceeds the reviewed upload budget")
    release = "/opt/testflying-store-api/versions/" + commit
    previous = snapshot["current"]
    if not previous.startswith("/opt/testflying-store-api/versions/") or previous == release:
        raise RuntimeError("Review the current deployment before replacing it")
    remote("""
import os,sys,tarfile,io,shutil,hashlib
p=""" + repr(release) + """
old=""" + repr(previous) + """
assert os.path.realpath('/opt/testflying-store-api/current')==old
assert not os.path.lexists(p)
os.mkdir(p,493)
with tarfile.open(fileobj=io.BytesIO(sys.stdin.buffer.read()),mode='r:')as t:
 for m in t.getmembers():
  assert (m.isfile()or m.isdir())and not m.name.startswith('/')and '..'not in m.name.split('/')
 t.extractall(p)
shutil.copytree(os.path.join(old,'releases'),os.path.join(p,'releases'),symlinks=True)
assert hashlib.sha256(open(os.path.join(p,'bin/store-api'),'rb').read()).hexdigest()==""" + repr(expected_api), payload)
    checks = []
    connections = {}
    public = urllib.parse.urlsplit(snapshot["url"])
    ca = ssl.create_default_context(cafile=str(PROJECT / "deploy/testflying-prod-ca.crt"))

    def request(path: str, token: str | None = None, method: str = "GET", data: dict | None = None,
                scheme: str = "https") -> tuple[int, bytes]:
        if scheme not in connections:
            connections[scheme] = (http.client.HTTPSConnection(public.hostname, context=ca, timeout=45)
                                   if scheme == "https" else http.client.HTTPConnection(public.hostname, timeout=45))
        connection = connections[scheme]
        headers = {"Content-Type": "application/json"}
        if token:
            headers["Authorization"] = "Bearer " + token
        connection.request(method, public.path.rstrip("/") + path, headers=headers,
                           body=None if data is None else json.dumps(data).encode())
        response = connection.getresponse()
        body = response.read(16 * 1024 * 1024 + 1)
        if len(body) > 16 * 1024 * 1024:
            raise RuntimeError("Deployment response budget exceeded")
        if response.will_close:
            connection.close()
            connections.pop(scheme)
        return response.status, body

    def check(condition: bool, label: str) -> None:
        if not condition:
            raise RuntimeError(label)
        checks.append(label)

    def select(target: str, expected: str) -> None:
        remote("""
import os,subprocess
p='/opt/testflying-store-api/setup-selector-""" + root.name + """'
current=os.path.realpath('/opt/testflying-store-api/current')
assert current in (""" + repr(expected) + "," + repr(target) + """ )
assert not os.path.lexists(p)
os.symlink(""" + repr(target) + """,p)
os.replace(p,'/opt/testflying-store-api/current')
subprocess.check_call(['systemctl','restart','testflying-store-api'])
""")

    changed = success = False
    node = None
    process = None
    app = "com.testflying.setupverification." + root.name
    mapped = []
    report_path = PROJECT / "docs/setup-deployment-verification.json"
    try:
        changed = True
        select(release, previous)
        healthy = False
        for _ in range(20):
            try:
                if request("/health", scheme="http")[0] == 200:
                    healthy = True
                    break
            except (OSError, http.client.HTTPException):
                for connection in connections.values():
                    connection.close()
                connections.clear()
            time.sleep(0.5)
        check(healthy, "HTTP health after API-only rollout")
        check(request("/health", scheme="https")[0] == 200, "HTTPS health with certificate verification")
        for suffix in ("", "app.mjs", "packager.mjs", "style.css", "jszip.min.js", "ca.crt", "start.cmd", "start.sh"):
            check(request("/connector-setup/" + suffix)[0] == 200, "setup asset " + (suffix or "index"))
        check(request("/v1/connectors")[0] == 401, "administrator authentication still required")
        setup = "/connector-setup/api/connectors"
        status, body = request(setup, method="POST", data={"name": "Setup deployment verification (mock only)"})
        check(status == 201, "temporary verification node created")
        created = json.loads(body)
        node = created["connector"]["connectorId"]
        status, body = request(setup)
        check(status == 200 and any(item["connectorId"] == node for item in json.loads(body)["connectors"]),
              "public list includes the verification node")
        config = created["installation"]
        config.update({"provider": {"mode": "mock"}, "listenAddr": "127.0.0.1:0"})
        private = tmp / "connector.json"
        fd = os.open(private, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        with os.fdopen(fd, "w") as stream:
            json.dump(config, stream)
        process = subprocess.Popen([str(PROJECT / "releases/darwin-arm64/store-connector"), str(private)],
                                   env={**os.environ, "TMPDIR": str(tmp), "TOKIO_WORKER_THREADS": "2",
                                        "SSL_CERT_FILE": str(PROJECT / "deploy/testflying-prod-ca.crt")},
                                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        connected = False
        for _ in range(40):
            status, body = request("/v1/connectors/" + node, snapshot["admin"])
            if status == 200 and json.loads(body).get("connected"):
                connected = True
                break
            if process.poll() is not None:
                break
            time.sleep(0.5)
        check(connected, "existing Connector binary connects through encrypted WSS")
        for platform in ("ios", "android"):
            binding = "/v1/apps/" + platform + "/" + app + "/connector"
            check(request(binding, snapshot["admin"])[0] == 404, "verification mapping unused " + platform)
            check(request(binding, snapshot["admin"], "PUT", {"connectorId": node})[0] == 200, "same node mapped for " + platform)
            mapped.append(binding)
            check(request("/v1/store-management/apps/" + platform + "/" + app + "/store-listings", snapshot["token"])[0] == 200,
                  "business request routed for " + platform)
        after = json.loads(remote("""
import json,hashlib,os
print(json.dumps({'environmentSha256':hashlib.sha256(open('/etc/testflying-store-api/service.env','rb').read()).hexdigest(),
'proxySha256':hashlib.sha256(open('/etc/bugsmith/zentao-tls/nginx.conf','rb').read()).hexdigest(),
'current':os.path.realpath('/opt/testflying-store-api/current')}))
"""))
        check(after["environmentSha256"] == snapshot["environmentSha256"], "all persistent API credentials unchanged")
        check(after["proxySha256"] == snapshot["proxySha256"], "HTTP HTTPS and other proxy routes unchanged")
        check(after["current"] == release, "new API release selected")
        report = {"runId": root.name, "stage": "api-deployed", "finishedAt": datetime.datetime.now(datetime.timezone.utc).isoformat(),
                  "deploymentCommit": commit, "binarySourceCommit": build["sourceCommit"], "checks": checks,
                  "baseUrl": "http://store-api.example.com/store-api", "addressRedacted": True,
                  "apiSha256": expected_api, "connectorBinarySha256": snapshot["binaries"],
                  "actualStoreAccountsUsed": False, "cleanup": "See the supervisor cleanup report for this runId"}
        report_path.write_text(json.dumps(report, indent=2) + "\n")
        # Verify the real, deployed browser page before accepting the rollout.
        subprocess.run([sys.executable, "-B", "scripts/verify_setup_ui.py", "production"], cwd=PROJECT, check=True)
        success = True
    finally:
        cleanup_ok = True
        for binding in mapped:
            try:
                if request(binding, snapshot["admin"], "DELETE")[0] not in (200, 204, 404):
                    cleanup_ok = False
            except (OSError, http.client.HTTPException, RuntimeError):
                cleanup_ok = False
        if node:
            try:
                if request("/v1/connectors/" + node, snapshot["admin"], "PATCH", {"enabled": False})[0] != 200:
                    cleanup_ok = False
            except (OSError, http.client.HTTPException, RuntimeError):
                cleanup_ok = False
        if process is not None:
            process.terminate()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
        if changed and not (success and cleanup_ok):
            select(previous, release)
        for connection in connections.values():
            connection.close()
    if not cleanup_ok:
        raise RuntimeError("Verification resources were not fully cleared")
    report = json.loads(report_path.read_text())
    if report.get("stage") != "browser-verified" or not report.get("verificationNodesDisabled"):
        raise RuntimeError("Production browser verification did not complete")
    report.update({"stage": "deployed-and-verified", "verificationNodeDisabled": True,
                   "verificationMappingsRemoved": True})
    report_path.write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    main()
