"""First production installation over the existing verified SSH connection."""
from __future__ import annotations

import datetime
import hashlib
import io
import json
import os
from pathlib import Path
import shlex
import ssl
import subprocess
import tarfile
import time
import urllib.error
import urllib.request
import zipfile

from verify_store_refactor import PROJECT, managed_root, source_hashes

HOST = "testflying-prod"
PUBLIC_EXAMPLE_URL = "https://store-api.example.com/store-api"
BASE_URL = os.environ.get("STORE_API_PUBLIC_URL", PUBLIC_EXAMPLE_URL)
SSH = ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=8", "-o",
       "StrictHostKeyChecking=yes", "-o", "LogLevel=ERROR", HOST]


def remote(command: str, data: bytes | None = None) -> bytes:
    return subprocess.check_output([*SSH, command], input=data)


def main() -> None:
    root = managed_root()
    if not os.environ.get("STORE_API_PUBLIC_URL") or "store-api.example.com" in BASE_URL:
        raise RuntimeError("Set the real STORE_API_PUBLIC_URL privately before deployment")
    tmp = root / "tmp"
    tmp.mkdir(exist_ok=True)
    subprocess.run(["bash", "-n", str(PROJECT / "deploy/install-prod.sh")], check=True)
    build = json.loads((PROJECT / "docs/deployment-build.json").read_text())
    if source_hashes() != build["sourceHashes"]:
        raise RuntimeError("Release source differs from verified build")
    for relative, expected in build["outputs"].items():
        if hashlib.sha256((PROJECT / relative).read_bytes()).hexdigest() != expected["sha256"]:
            raise RuntimeError("Release checksum mismatch")
    commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=PROJECT, text=True).strip()
    release = "/opt/testflying-store-api/versions/" + commit
    files = {
        "bin/store-api": PROJECT / "releases/linux-amd64/store-api",
        "releases/linux-amd64/store-connector": PROJECT / "releases/linux-amd64/store-connector",
        "releases/darwin-arm64/store-connector": PROJECT / "releases/darwin-arm64/store-connector",
        "deployment-build.json": PROJECT / "docs/deployment-build.json",
        "deploy/install-prod.sh": PROJECT / "deploy/install-prod.sh",
        "deploy/testflying-store-api.service": PROJECT / "deploy/testflying-store-api.service",
    }
    archive_path = tmp / "release.tar"
    with tarfile.open(archive_path, "w") as archive:
        checksums = []
        for name, path in files.items():
            member = archive.gettarinfo(str(path), arcname=name)
            member.uid = member.gid = 0
            member.uname = member.gname = "root"
            with path.open("rb") as content_file:
                archive.addfile(member, content_file)
            checksums.append(hashlib.sha256(path.read_bytes()).hexdigest() + "  " + name)
        content = ("\n".join(checksums) + "\n").encode()
        member = tarfile.TarInfo("SHA256SUMS")
        member.size = len(content)
        member.mode = 0o644
        archive.addfile(member, io.BytesIO(content))
    print("Uploading production release " + commit, flush=True)
    remote("test ! -L /opt/testflying-store-api && test ! -e " + shlex.quote(release)
           + " && umask 022 && mkdir -p " + shlex.quote(release)
           + " && tar -xf - -C " + shlex.quote(release), archive_path.read_bytes())
    print(remote("STORE_API_PUBLIC_URL=" + shlex.quote(BASE_URL) + " bash " + shlex.quote(release + "/deploy/install-prod.sh")
                 + " " + shlex.quote(release)).decode(), flush=True)
    # Production credentials stay in memory; subprocess output is never forwarded.
    values = dict(line.split("=", 1) for line in remote(
        "cat /etc/testflying-store-api/service.env").decode().splitlines() if "=" in line)
    ca = remote("cat /etc/bugsmith/zentao-tls/certs/ca.crt")
    context = ssl.create_default_context(cadata=ca.decode())
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}),
                                         urllib.request.HTTPSHandler(context=context))

    def request(path: str, token: str | None = None, method: str = "GET",
                payload: dict | None = None) -> tuple[int, bytes]:
        headers = {}
        if token:
            headers["Authorization"] = "Bearer " + token
        if payload is not None:
            headers["Content-Type"] = "application/json"
        query = urllib.request.Request(BASE_URL + path, method=method, headers=headers,
                                       data=None if payload is None else json.dumps(payload).encode())
        try:
            with opener.open(query, timeout=30) as response:
                return response.status, response.read(128 * 1024 * 1024)
        except urllib.error.HTTPError as error:
            return error.code, error.read(65536)

    def check(condition: bool, label: str) -> None:
        if not condition:
            raise RuntimeError(label + " failed")
        print("PASS " + label, flush=True)
        checks.append(label)

    checks: list[str] = []
    status, body = request("/health")
    check(status == 200 and json.loads(body)["status"] == "ok", "external HTTPS health and certificate")
    check(request("/v1/connectors")[0] == 401, "administrator authentication required")
    admin, api = values["STORE_API_ADMIN_TOKEN"], values["STORE_API_TOKEN"]
    check(request("/v1/connectors", api)[0] == 401, "application token cannot administer nodes")
    status, body = request("/v1/connectors", admin, "POST",
                           {"name": "Deployment verification (no store credentials)"})
    check(status == 201, "node registration")
    node = json.loads(body)
    identifier = node["connector"]["connectorId"]
    process = None
    mapped = False
    try:
        status, body = request("/v1/connectors/" + identifier + "/installation", admin)
        installation = json.loads(body)
        check(status == 200 and installation == node["installation"], "generated node credentials")
        for target in ("linux-amd64", "darwin-arm64"):
            status, body = request("/v1/connectors/" + identifier + "/installer/" + target, admin)
            check(status == 200, target + " installer download")
            with zipfile.ZipFile(io.BytesIO(body)) as archive:
                check(set(archive.namelist()) == {"store-connector", "connector.json", "start.sh"}
                      and json.loads(archive.read("connector.json")) == installation
                      and hashlib.sha256(archive.read("store-connector")).hexdigest()
                      == build["outputs"]["releases/" + target + "/store-connector"]["sha256"],
                      target + " installer identity and binary checksum")
        check(request("/v1/connectors/" + identifier + "/installer/windows-amd64", admin)[0] == 503,
              "unpublished target rejected")
        config_path = tmp / "connector.json"
        installation["listenAddr"]="127.0.0.1:0"
        config_path.write_text(json.dumps(installation))
        config_path.chmod(0o600)
        ca_path = tmp / "ca.crt"
        ca_path.write_bytes(ca)
        log = (tmp / "connector.log").open("wb")
        process = subprocess.Popen([str(PROJECT / "releases/darwin-arm64/store-connector"), str(config_path)],
                                   env={**os.environ, "SSL_CERT_FILE": str(ca_path), "TOKIO_WORKER_THREADS": "2"},
                                   stdout=log, stderr=subprocess.STDOUT)
        log.close()
        connected = False
        for _ in range(40):
            status, body = request("/v1/connectors/" + identifier, admin)
            if status == 200 and json.loads(body).get("connected"):
                connected = True
                break
            if process.poll() is not None:
                break
            time.sleep(0.5)
        check(connected, "real Connector WSS connection through HTTPS proxy")
        binding = "/v1/apps/ios/com.testflying.deployment.smoke/connector"
        check(request(binding, admin)[0] == 404, "verification app is initially unbound")
        check(request(binding, admin, "PUT", {"connectorId": identifier})[0] == 200,
              "application maps to Connector")
        mapped = True
        status, body = request("/v1/store-management/apps/ios/com.testflying.deployment.smoke/store-listings", api)
        check(status == 503 and json.loads(body).get("error", {}).get("code") == "store_credentials_unavailable",
              "API dispatch reaches Connector without real store credentials")
    finally:
        if process is not None and process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
        if mapped:
            request("/v1/apps/ios/com.testflying.deployment.smoke/connector", admin, "DELETE")
        status, _ = request("/v1/connectors/" + identifier, admin, "PATCH", {"enabled": False})
        if status != 200:
            raise RuntimeError("Verification node could not be disabled")
    check(request("/v1/apps/ios/com.testflying.deployment.smoke/connector", admin)[0] == 404,
          "verification app mapping removed")
    status, body = request("/v1/connectors/" + identifier, admin)
    check(status == 200 and not json.loads(body)["enabled"] and not json.loads(body)["connected"],
          "verification node disabled and disconnected")
    service = remote("systemctl is-active testflying-store-api; systemctl is-enabled testflying-store-api").decode().splitlines()
    check(service == ["active", "enabled"], "production service active and enabled at boot")
    # Public CA certificate is a formal delivery file, never a private key.
    (PROJECT / "deploy/testflying-prod-ca.crt").write_bytes(ca)
    report = {"runId": root.name, "finishedAt": datetime.datetime.now(datetime.timezone.utc).isoformat(),
              "deploymentCommit": commit, "binarySourceCommit": build["sourceCommit"],
              "releaseDirectory": release, "baseUrl": PUBLIC_EXAMPLE_URL, "addressRedacted": True,
              "checks": checks, "verificationNodeId": identifier,
              "verificationNode": "disabled, disconnected, no app mapping or store credentials",
              "caSha256": hashlib.sha256(ca).hexdigest(),
              "remainingProductionResources": ["testflying-store-api systemd service", "SQLite registry",
                   "root-only service.env", "release binaries", "TLS proxy route and original config backup"],
              "notVerified": ["Real Apple/Google accounts", "Unpublished Connector targets"],
              "cleanup": "See the supervisor cleanup report for this runId"}
    (PROJECT / "docs/deployment-verification.json").write_text(json.dumps(report, indent=2) + "\n")
    print("Deployment verified; production address omitted", flush=True)


if __name__ == "__main__":
    main()
