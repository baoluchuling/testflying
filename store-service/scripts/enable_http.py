"""Enable the separate HTTP ingress and verify uploads through a mock-only node."""
from __future__ import annotations

import hashlib
import http.client
import json
import os
from pathlib import Path
import shlex
import ssl
import struct
import subprocess
import time
import urllib.parse
import uuid
import zlib

from verify_store_refactor import PROJECT, managed_root


def main() -> None:
    root = managed_root()
    tmp = root / "tmp"
    tmp.mkdir(exist_ok=True)
    ssh = ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=8", "-o",
           "StrictHostKeyChecking=yes", "-o", "LogLevel=ERROR",
           os.environ.get("STORE_API_SSH_HOST", "testflying-prod")]

    def remote(code: str, data: bytes | None = None) -> bytes:
        command = "python3 -c " + shlex.quote(code)
        result = subprocess.run(ssh + [command], input=data, stdout=subprocess.PIPE,
                                stderr=subprocess.PIPE, timeout=45)
        if result.returncode:
            raise RuntimeError("Verified SSH command failed")
        return result.stdout

    snapshot = json.loads(remote("""
import json,os,subprocess
p='/etc/bugsmith/zentao-tls/nginx.conf'
e=open('/etc/testflying-store-api/service.env').read()
print(json.dumps({'config':open(p).read(),'environment':e,
 'pid':subprocess.check_output(['systemctl','show','testflying-store-api','-p','MainPID','--value']).decode().strip(),
 'network':json.loads(subprocess.check_output(['docker','inspect','zentao-tls-proxy']))[0]['HostConfig']['NetworkMode']}))
"""))
    if snapshot["network"] != "host":
        raise RuntimeError("Review proxy networking before opening HTTP")
    values = dict(line.split("=", 1) for line in snapshot["environment"].splitlines()
                  if "=" in line and not line.startswith("#"))
    public = urllib.parse.urlsplit(values["STORE_API_PUBLIC_URL"])
    if public.scheme != "https" or public.path.rstrip("/") != "/store-api":
        raise RuntimeError("Review the configured HTTPS origin")
    ca = ssl.create_default_context(cafile=str(PROJECT / "deploy/testflying-prod-ca.crt"))
    admin, token = values["STORE_API_ADMIN_TOKEN"], values["STORE_API_TOKEN"]
    connections: dict[str, http.client.HTTPConnection] = {}

    def request(path: str, credential: str | None = None, method: str = "GET",
                data: dict | bytes | None = None, scheme: str = "http",
                content_type: str = "application/json", prefix: bool = True) -> tuple[int, bytes]:
        if scheme not in connections:
            connections[scheme] = (http.client.HTTPSConnection(public.hostname, context=ca, timeout=45)
                                   if scheme == "https" else
                                   http.client.HTTPConnection(public.hostname, timeout=45))
        connection = connections[scheme]
        headers = {"Content-Type": content_type}
        if credential:
            headers["Authorization"] = "Bearer " + credential
        body = json.dumps(data).encode() if isinstance(data, dict) else data
        try:
            connection.request(method, public.path.rstrip("/") + path if prefix else path,
                               body=body, headers=headers)
            response = connection.getresponse()
            payload = response.read(16 * 1024 * 1024 + 1)
            if len(payload) > 16 * 1024 * 1024:
                raise RuntimeError("Response size bound")
            status = response.status
            if response.will_close:
                connection.close()
                del connections[scheme]
            return status, payload
        except (OSError, http.client.HTTPException):
            connection.close()
            connections.pop(scheme, None)
            raise RuntimeError("HTTP verification request failed") from None

    report_path = PROJECT / "releases/http-deployment-2026-11-05.json"
    checks: list[str] = []
    backup = "/etc/bugsmith/zentao-tls/nginx.before-store-api-http-" + root.name + ".conf"
    report = {"runId": root.name, "stage": "verifying", "checks": checks,
              "proxyBackup": backup, "certificateVerificationDisabled": False,
              "rustBinaryChanged": False, "actualStoreAccountsUsed": False}

    def save() -> None:
        report_path.write_text(json.dumps(report, indent=2) + "\n")
        os.chmod(report_path, 0o600)

    def check(condition: bool, label: str) -> None:
        if not condition:
            raise RuntimeError(label)
        checks.append(label)
        save()

    original = snapshot["config"].encode()
    check(request("/health", scheme="https")[0] == 200, "existing HTTPS health")
    root_status = request("/", scheme="https", prefix=False)[0]
    # Do not reuse this idle socket after the HTTP upload/WSS checks.
    connections.pop("https").close()
    snippet = (PROJECT / "deploy/http-store-api.conf").read_text()
    if "listen 80;" in snapshot["config"] or not snapshot["config"].rstrip().endswith("}"):
        raise RuntimeError("Review existing HTTP or unexpected proxy layout")
    candidate = (snapshot["config"].rstrip()[:-1] + "\n" +
                 "\n".join("    " + line if line else "" for line in snippet.splitlines()) + "\n}\n").encode()
    report["proxyBeforeSha256"] = hashlib.sha256(original).hexdigest()
    report["proxyAfterSha256"] = hashlib.sha256(candidate).hexdigest()
    changed = success = False
    process = None
    identifier = None
    app = "com.testflying.httpverification." + uuid.uuid4().hex
    binding = "/v1/apps/ios/" + app + "/connector"
    try:
        # Preserve the inode of the bind-mounted proxy configuration.
        changed = True
        remote("""
import sys,os,hashlib,subprocess
p='/etc/bugsmith/zentao-tls/nginx.conf'
b=""" + repr(backup) + """
old=open(p,'rb').read()
assert hashlib.sha256(old).hexdigest()==""" + repr(report["proxyBeforeSha256"]) + """
assert not os.path.exists(b) and not os.path.islink(p)
fd=os.open(b,os.O_WRONLY|os.O_CREAT|os.O_EXCL,384)
with os.fdopen(fd,'wb')as f:f.write(old)
new=sys.stdin.buffer.read()
assert hashlib.sha256(new).hexdigest()==""" + repr(report["proxyAfterSha256"]) + """
with open(p,'wb')as f:f.write(new)
test=subprocess.call(['docker','exec','zentao-tls-proxy','nginx','-t'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
if test:
 with open(p,'wb')as f:f.write(old)
 raise SystemExit(1)
subprocess.check_call(['docker','exec','zentao-tls-proxy','nginx','-s','reload'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
""", candidate)
        healthy = False
        for _ in range(10):
            try:
                if request("/health")[0] == 200:
                    healthy = True
                    break
            except RuntimeError:
                pass
            time.sleep(0.2)
        check(healthy, "public HTTP health without TLS")
        check(request("/v1/connectors")[0] == 401, "HTTP administrator authentication required")
        check(request("/v1/connectors", admin)[0] == 200, "HTTP administrator token accepted")
        check(request("/v1/store-management/apps/ios/" + app + "/store-listings")[0] == 401,
              "HTTP business authentication required")
        status, body = request("/v1/connectors", admin, "POST", {"name": "HTTP verification (mock only)"})
        check(status == 201, "HTTP verification node created")
        created = json.loads(body)
        identifier = created["connector"]["connectorId"]
        config = created["installation"]
        # Runtime configuration stores the HTTPS base; Connector converts it to WSS.
        check(config["serverUrl"].startswith("https://"), "new node configuration retains HTTPS for WSS")
        config.update({"provider": {"mode": "mock"}, "listenAddr": "127.0.0.1:0"})
        configuration = tmp / "connector.json"
        fd = os.open(configuration, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        with os.fdopen(fd, "w") as output:
            json.dump(config, output)
        process = subprocess.Popen([str(PROJECT / "releases/darwin-arm64/store-connector"), str(configuration)],
                                   env={**os.environ, "TMPDIR": str(tmp), "TOKIO_WORKER_THREADS": "2",
                                        "SSL_CERT_FILE": str(PROJECT / "deploy/testflying-prod-ca.crt")},
                                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        connected = False
        for _ in range(40):
            status, body = request("/v1/connectors/" + identifier, admin)
            if status == 200 and json.loads(body).get("connected"):
                connected = True
                break
            if process.poll() is not None:
                break
            time.sleep(0.5)
        check(connected, "mock Connector connected through existing encrypted WSS")
        check(request(binding, admin, "PUT", {"connectorId": identifier})[0] == 200,
              "HTTP app mapping created")
        path = "/v1/store-management/apps/ios/" + app
        check(request(path + "/store-listings?version=1.0", token)[0] == 200,
              "HTTP business request through WSS mock node")
        def png_chunk(kind: bytes, content: bytes) -> bytes:
            return struct.pack(">I", len(content)) + kind + content + struct.pack(">I", zlib.crc32(kind + content))
        raw = b"".join(b"\0" + os.urandom(1024 * 3) for _ in range(1024))
        image = (b"\x89PNG\r\n\x1a\n" + png_chunk(b"IHDR", struct.pack(">IIBBBBB", 1024, 1024, 8, 2, 0, 0, 0)) +
                 png_chunk(b"IDAT", zlib.compress(raw, 0)) + png_chunk(b"IEND", b""))
        sha = hashlib.sha256(image).hexdigest()
        file = {"fileName": "screen.png", "contentType": "image/png", "sizeBytes": len(image), "sha256": sha}
        status, body = request(path + "/uploads", token, "POST", file)
        check(status == 201, "HTTP upload credential issued")
        ticket = json.loads(body)
        upload_url = urllib.parse.urlsplit(ticket["uploadUrl"])
        check(upload_url.scheme == "http" and upload_url.hostname == public.hostname,
              "HTTP uploadUrl stays on the HTTP center")
        status, body = request(upload_url.path, ticket["uploadToken"], "PUT", image,
                               content_type="image/png", prefix=False)
        uploaded = json.loads(body)
        check(status == 201 and uploaded["sha256"] == sha and uploaded["sizeBytes"] == len(image),
              "image larger than 2 MiB streamed over HTTP and encrypted WSS")
        payload = {"operation": "update_app_metadata", "version": "1.0", "locale": "en-US",
                   "syncScopes": ["store_images"], "metadata": {"storeImages": {
                       "phone_screenshots": {"assets": [{"assetId": ticket["assetId"]}]}}}}
        status, body = request(path + "/sync", token, "POST", payload)
        check(status == 200 and json.loads(body).get("status") == "succeeded", "HTTP mock sync consumed image")
        status, body = request(path + "/uploads", token, "POST", file, scheme="https")
        check(status == 201 and json.loads(body)["uploadUrl"].startswith("https://"),
              "HTTPS uploadUrl retains HTTPS")
        check(request("/health", scheme="https")[0] == 200, "existing HTTPS health preserved")
        check(request("/", scheme="https", prefix=False)[0] == root_status, "other HTTPS application preserved")
        unchanged = json.loads(remote("""
import json,subprocess
print(json.dumps({'environment':open('/etc/testflying-store-api/service.env').read(),
'pid':subprocess.check_output(['systemctl','show','testflying-store-api','-p','MainPID','--value']).decode().strip()}))
"""))
        check(unchanged == {"environment": snapshot["environment"], "pid": snapshot["pid"]},
              "API process and all persistent credentials unchanged")
        report.update({"imageBytes": len(image), "imageSha256": sha, "stage": "verified"})
        success = True
    finally:
        if identifier:
            try:
                cleanup_status = request(binding, admin, "DELETE")[0]
                report["verificationMappingRemoved"] = cleanup_status in (200, 204, 404)
            except RuntimeError:
                report["verificationMappingRemoved"] = False
            try:
                disabled = request("/v1/connectors/" + identifier, admin, "PATCH", {"enabled": False})[0]
                report["verificationNodeDisabled"] = disabled == 200
            except RuntimeError:
                report["verificationNodeDisabled"] = False
            success = success and report["verificationNodeDisabled"] and report["verificationMappingRemoved"]
        if process is not None:
            process.terminate()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
        if not success and changed:
            remote("""
import sys,subprocess
with open('/etc/bugsmith/zentao-tls/nginx.conf','wb')as f:f.write(sys.stdin.buffer.read())
subprocess.check_call(['docker','exec','zentao-tls-proxy','nginx','-t'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
subprocess.check_call(['docker','exec','zentao-tls-proxy','nginx','-s','reload'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
""", original)
            report["rolledBack"] = True
        report["success"] = success
        save()
        for connection in connections.values():
            connection.close()
    if not success:
        raise RuntimeError("HTTP verification or resource cleanup did not complete")


if __name__ == "__main__":
    main()
