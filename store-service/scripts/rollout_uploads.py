"""Upgrade the standalone production API and verify central file streaming."""
from __future__ import annotations

import base64
import datetime
import hashlib
import http.client
import io
import json
import os
from pathlib import Path
import shlex
import ssl
import struct
import subprocess
import sys
import tarfile
import time
import urllib.parse
import zipfile
import zlib

from deploy_prod import BASE_URL, PUBLIC_EXAMPLE_URL, remote
from verify_store_refactor import PROJECT, managed_root, source_hashes


def main() -> None:
    root = managed_root()
    if not os.environ.get("STORE_API_PUBLIC_URL") or "store-api.example.com" in BASE_URL:
        raise RuntimeError("Set the real STORE_API_PUBLIC_URL privately before deployment")
    tmp = root / "tmp"
    tmp.mkdir(exist_ok=True)
    private_path = PROJECT / "releases/windows-amd64/connector-windows-v3.zip"
    if private_path.exists() or private_path.is_symlink():
        raise RuntimeError("The v3 private package already exists; preserve it and review before another rollout")
    commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=PROJECT, text=True).strip()
    subprocess.run(["git", "diff", "--exit-code", "HEAD", "--", "."], cwd=PROJECT, check=True,
                   stdout=subprocess.DEVNULL)
    pushed = subprocess.check_output(["git", "ls-remote", "origin", "refs/heads/refactor/store-service-rust"],
                                     cwd=PROJECT, text=True).split()[0]
    if pushed != commit:
        raise RuntimeError("Push the current release commit before deployment")
    build = json.loads((PROJECT / "docs/deployment-build.json").read_text())
    windows = json.loads((PROJECT / "docs/windows-release-build.json").read_text())
    verified = json.loads((PROJECT / "docs/verification.json").read_text())
    if not source_hashes() == build["sourceHashes"] == windows["sourceHashes"] == verified["sourceHashes"]:
        raise RuntimeError("Release sources differ from the tested sources")
    runtime = {
        "bin/store-api": ("releases/linux-amd64/store-api", build),
        "releases/linux-amd64/store-connector": ("releases/linux-amd64/store-connector", build),
        "releases/darwin-arm64/store-connector": ("releases/darwin-arm64/store-connector", build),
        "releases/windows-amd64/store-connector.exe": ("releases/windows-amd64/store-connector.exe", windows),
    }
    for relative, report in runtime.values():
        if hashlib.sha256((PROJECT / relative).read_bytes()).hexdigest() != report["outputs"][relative]["sha256"]:
            raise RuntimeError("Local release checksum mismatch")
    if int(remote("df --output=avail -B1 /opt | tail -1")) <= 21474836480:
        raise RuntimeError("Server free-space reserve is insufficient")
    previous = remote("readlink -f /opt/testflying-store-api/current").decode().strip()
    release = "/opt/testflying-store-api/versions/" + commit
    if not previous.startswith("/opt/testflying-store-api/versions/") or previous == release:
        raise RuntimeError("Review the existing release before running this upgrade")
    secret_before = remote("cat /etc/testflying-store-api/service.env")
    values = dict(line.split("=", 1) for line in secret_before.decode().splitlines() if "=" in line)
    ca = (PROJECT / "deploy/testflying-prod-ca.crt").read_bytes()
    context = ssl.create_default_context(cadata=ca.decode())
    origin = urllib.parse.urlsplit(BASE_URL)
    connection = None

    def request(path: str, token: str | None = None, method: str = "GET", data: dict | None = None) -> tuple[int, bytes]:
        nonlocal connection
        headers = {"Content-Type": "application/json"}
        if token:
            headers["Authorization"] = "Bearer " + token
        if connection is None:
            connection = http.client.HTTPSConnection(origin.hostname, origin.port, context=context, timeout=45)
        try:
            connection.request(method, origin.path.rstrip("/") + path,
                               body=None if data is None else json.dumps(data).encode(), headers=headers)
            response = connection.getresponse()
            body = response.read(128 * 1024 * 1024 + 1)
            if len(body) > 128 * 1024 * 1024:
                raise RuntimeError("Response exceeds the reviewed download budget")
            status = response.status
            if response.will_close:
                connection.close()
                connection = None
            return status, body
        except (OSError, http.client.HTTPException):
            connection.close()
            connection = None
            raise

    admin, api_token = values["STORE_API_ADMIN_TOKEN"], values["STORE_API_TOKEN"]
    delivery_id = json.loads((PROJECT / "docs/windows-release-verification.json").read_text())["deliveryNodeId"]
    status, body = request("/v1/connectors/" + delivery_id + "/installation", admin)
    if status != 200:
        raise RuntimeError("Existing delivery node is disabled; review before upgrading its package")
    original_identity = json.loads(body)
    original_proxy = remote("cat /etc/bugsmith/zentao-tls/nginx.conf")
    archive_path = tmp / "release.tar"
    source = subprocess.check_output(["git", "archive", "--format=tar", "--prefix=source/",
                                      commit + ":store-service"], cwd=PROJECT)
    hashes: dict[str, str] = {}
    with tarfile.open(archive_path, "w") as out:
        with tarfile.open(fileobj=io.BytesIO(source), mode="r:") as source_tar:
            for member in source_tar.getmembers():
                if not ((member.name == "source" and member.isdir()) or member.name.startswith("source/")) or ".." in Path(member.name).parts:
                    raise RuntimeError("Unexpected committed archive path")
                if not (member.isfile() or member.isdir()):
                    raise RuntimeError("Source archive cannot contain links or special files")
                member.uid = member.gid = 0
                member.uname = member.gname = "root"
                if member.isfile():
                    data = source_tar.extractfile(member).read()
                    hashes[member.name] = hashlib.sha256(data).hexdigest()
                    out.addfile(member, io.BytesIO(data))
                else:
                    out.addfile(member)

        def add(name: str, data: bytes, mode: int = 0o644) -> None:
            member = tarfile.TarInfo(name)
            member.size, member.mode = len(data), mode
            member.uname = member.gname = "root"
            out.addfile(member, io.BytesIO(data))
            hashes[name] = hashlib.sha256(data).hexdigest()

        for name, (relative, _) in runtime.items():
            add(name, (PROJECT / relative).read_bytes(), 0o755)
        metadata = {"deploymentCommit": commit, "binarySourceCommit": build["sourceCommit"],
                    "previousRelease": previous, "runId": root.name, "sha256": dict(hashes)}
        add("deployment.json", (json.dumps(metadata, indent=2) + "\n").encode())
        add("nginx.before-upload.conf", original_proxy, 0o600)
        checksums = "".join(digest + "  " + name + "\n" for name, digest in hashes.items()).encode()
        member = tarfile.TarInfo("SHA256SUMS")
        member.size, member.mode = len(checksums), 0o644
        member.uname = member.gname = "root"
        out.addfile(member, io.BytesIO(checksums))
    if archive_path.stat().st_size >= 64 * 1024 * 1024:
        raise RuntimeError("Release exceeds the reviewed upload budget")
    quote = shlex.quote
    print("Uploading tested release " + commit, flush=True)
    remote("test ! -e " + quote(release) + " && umask 022 && mkdir -p " + quote(release)
           + " && tar -xf - -C " + quote(release), archive_path.read_bytes())
    remote("cd " + quote(release) + " && sha256sum --check SHA256SUMS >/dev/null")
    checks: list[str] = []

    def check(condition: bool, label: str) -> None:
        if not condition:
            raise RuntimeError(label + " failed")
        checks.append(label)
        print("PASS " + label, flush=True)

    def select(target: str, suffix: str) -> None:
        script = "import os; p=" + repr("/opt/testflying-store-api/selector-" + commit + suffix)
        script += "; os.symlink(" + repr(target) + ",p); os.replace(p,'/opt/testflying-store-api/current')"
        remote("python3 -c " + quote(script))

    def write_remote(path: str, data: bytes) -> None:
        script = "import sys; from pathlib import Path; Path(" + repr(path) + ").write_bytes(sys.stdin.buffer.read())"
        remote("python3 -c " + quote(script), data)

    selected = proxy_changed = success = False
    node_id = None
    mapped = False
    process = None
    app = "com.testflying.upload.verification"
    binding = "/v1/apps/ios/" + app + "/connector"
    try:
        text = original_proxy.decode()
        start, end = text.index("        location /store-api/ {"), text.index("        location / {", text.index("        location /store-api/ {"))
        block = text[start:end]
        if "proxy_request_buffering" not in block:
            if block.count("client_max_body_size 2m;") != 1 or block.count("proxy_read_timeout 90s;") != 1:
                raise RuntimeError("Unexpected existing proxy configuration")
            block = block.replace("client_max_body_size 2m;", "client_max_body_size 64m;\n            proxy_request_buffering off;\n            client_body_buffer_size 64k;")
        else:
            check("proxy_request_buffering off;" in block and "client_max_body_size 64m;" in block,
                  "existing proxy already permits streaming uploads")
        for previous_timeout in ("90s", "660s"):
            block = block.replace("proxy_read_timeout " + previous_timeout + ";", "proxy_read_timeout 1860s;")
        if block.count("proxy_read_timeout 1860s;") != 1:
            raise RuntimeError("Unexpected proxy timeout; review before changing it")
        write_remote("/etc/bugsmith/zentao-tls/nginx.conf", (text[:start] + block + text[end:]).encode())
        proxy_changed = True
        remote("docker exec zentao-tls-proxy nginx -t && docker exec zentao-tls-proxy nginx -s reload")
        select(release, "-activate")
        selected = True
        remote("systemctl restart testflying-store-api")
        healthy = False
        for _ in range(20):
            try:
                status, body = request("/health")
                if status == 200 and json.loads(body).get("status") == "ok":
                    healthy = True
                    break
            except OSError:
                pass
            time.sleep(0.5)
        check(healthy, "production HTTPS health")
        check(secret_before == remote("cat /etc/testflying-store-api/service.env"), "persistent API credentials unchanged")
        status, body = request("/v1/connectors/" + delivery_id + "/installation", admin)
        delivery_config = json.loads(body)
        check(status == 200 and delivery_config["connectorId"] == original_identity["connectorId"]
              and delivery_config["token"] == original_identity["token"]
              and "messageKey" in delivery_config and "uploadKey" in delivery_config,
              "existing node identity preserved with separate protocol keys")
        check(request(binding, admin)[0] == 404, "verification app is unbound")
        status, body = request("/v1/connectors", admin, "POST", {"name": "Central upload verification (mock only)"})
        check(status == 201, "name-only node creation")
        created = json.loads(body)
        node_id = created["connector"]["connectorId"]
        config = created["installation"]
        for target, name in [("linux-amd64", "store-connector"), ("darwin-arm64", "store-connector"), ("windows-amd64", "store-connector.exe")]:
            status, body = request("/v1/connectors/" + node_id + "/installer/" + target, admin)
            check(status == 200, target + " installer download")
            report = windows if target == "windows-amd64" else build
            with zipfile.ZipFile(io.BytesIO(body)) as bundle:
                check(json.loads(bundle.read("connector.json")) == config
                      and hashlib.sha256(bundle.read(name)).hexdigest() == report["outputs"]["releases/" + target + "/" + name]["sha256"],
                      target + " executable and private config")
        config["provider"] = {"mode": "mock"}
        config["listenAddr"] = "127.0.0.1:0"
        config_path = tmp / "connector.json"
        fd = os.open(config_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        with os.fdopen(fd, "w") as output:
            json.dump(config, output)
        log = (tmp / "connector.log").open("wb")
        process = subprocess.Popen([str(PROJECT / "releases/darwin-arm64/store-connector"), str(config_path)],
                                   env={**os.environ, "TMPDIR": str(tmp), "SSL_CERT_FILE": str(PROJECT / "deploy/testflying-prod-ca.crt"), "TOKIO_WORKER_THREADS": "2"},
                                   stdout=log, stderr=subprocess.STDOUT)
        log.close()
        connected = False
        for _ in range(40):
            status, body = request("/v1/connectors/" + node_id, admin)
            if status == 200 and json.loads(body).get("connected"):
                connected = True
                break
            if process.poll() is not None:
                break
            time.sleep(0.5)
        check(connected, "real encrypted v2 Connector WSS authentication")
        check(request(binding, admin, "PUT", {"connectorId": node_id})[0] == 200, "verification application mapping")
        mapped = True

        def png_chunk(kind: bytes, data: bytes) -> bytes:
            return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xffffffff)

        raw = b"".join(b"\x00" + os.urandom(1024 * 3) for _ in range(1024))
        image = b"\x89PNG\r\n\x1a\n" + png_chunk(b"IHDR", struct.pack(">IIBBBBB", 1024, 1024, 8, 2, 0, 0, 0))
        image += png_chunk(b"IDAT", zlib.compress(raw, 0)) + png_chunk(b"IEND", b"")
        image_path = tmp / "screen.png"
        image_path.write_bytes(image)
        check(len(image) > 2 * 1024 * 1024, "fixture exceeds the JSON-only limit")
        status, body = request("/v1/store-management/apps/ios/" + app + "/uploads", api_token, "POST",
                               {"fileName": "screen.png", "contentType": "image/png", "sizeBytes": len(image),
                                "sha256": hashlib.sha256(image).hexdigest()})
        check(status == 201, "production upload preparation")
        prepared = json.loads(body)
        encoded = prepared["uploadToken"].split(".")[1]
        claims = json.loads(base64.urlsafe_b64decode(encoded + "=" * (-len(encoded) % 4)))
        check(claims["exp"] - claims["iat"] == 1800 and prepared["expiresAt"] == claims["exp"],
              "production upload credentials expire after thirty minutes")
        uploaded = subprocess.run([sys.executable, "-B", "examples/upload-image.py", "--base-url", BASE_URL,
                                   "--platform", "ios", "--bundle-id", app, "--file", str(image_path),
                                   "--ca-file", "deploy/testflying-prod-ca.crt"], cwd=PROJECT,
                                  env={**os.environ, "STORE_API_TOKEN": api_token, "PYTHONDONTWRITEBYTECODE": "1"},
                                  stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, timeout=1860)
        if uploaded.returncode:
            raise RuntimeError("External upload example failed; no credentials or body were logged")
        asset = json.loads(uploaded.stdout)
        check(asset["sizeBytes"] == len(image) and asset["sha256"] == hashlib.sha256(image).hexdigest(),
              "3 MiB local file streamed through center to encrypted WSS node")
        payload = {"operation": "update_app_metadata", "version": "1.0", "locale": "en-US", "syncScopes": ["store_images"],
                   "metadata": {"storeImages": {"phone_screenshots": {"assets": [{"assetId": asset["assetId"]}]}}}}
        status, body = request("/v1/store-management/apps/ios/" + app + "/sync", api_token, "POST", payload)
        result = json.loads(body)
        check(status == 200 and result["status"] == "succeeded" and result["storeState"]["mock"] is True,
              "mock sync consumes uploaded asset without vendor traffic")
        check(request("/v1/store-management/apps/ios/" + app + "/sync", api_token, "POST", payload)[0] == 404,
              "consumed asset is no longer reusable")
        pid = remote("systemctl show testflying-store-api -p MainPID --value").decode().strip()
        check(pid.isdigit() and int(pid) > 0 and remote("readlink -f /proc/" + pid + "/exe").decode().strip() == release + "/bin/store-api",
              "running API belongs to pushed deployment commit")
        check(json.loads(remote("curl --fail --silent http://127.0.0.1:8000/health"))["status"] == "ok", "legacy API health")

        status, body = request("/v1/connectors/" + delivery_id + "/installer/windows-amd64", admin)
        check(status == 200, "existing delivery node Windows v3 package")
        old_zip = PROJECT / "releases/windows-amd64/connector-windows-v2.zip"
        if not old_zip.is_file():
            old_zip = PROJECT / "releases/windows-amd64/connector-windows.zip"
        if old_zip.is_file():
            with zipfile.ZipFile(old_zip) as previous_zip:
                old_config = json.loads(previous_zip.read("connector.json"))
                if old_config["connectorId"] != delivery_id:
                    raise RuntimeError("Existing private package belongs to another node")
                for field in ("provider", "listenAddr", "requestTimeoutSeconds", "baseUrl"):
                    if field in old_config:
                        delivery_config[field] = old_config[field]
        private = io.BytesIO()
        with zipfile.ZipFile(io.BytesIO(body)) as original, zipfile.ZipFile(private, "w", zipfile.ZIP_DEFLATED) as bundle:
            executable = original.read("store-connector.exe")
            check(hashlib.sha256(executable).hexdigest() == windows["outputs"]["releases/windows-amd64/store-connector.exe"]["sha256"], "private Windows v3 executable checksum")
            bundle.writestr("store-connector.exe", executable)
            bundle.writestr("connector.json", json.dumps(delivery_config, indent=2))
            bundle.writestr("testflying-prod-ca.crt", ca)
            bundle.writestr("start.cmd", '@echo off\r\ncd /d "%~dp0"\r\nset "SSL_CERT_FILE=%~dp0testflying-prod-ca.crt"\r\nstore-connector.exe connector.json\r\nif errorlevel 1 pause\r\n')
            bundle.writestr("README.txt", ("Windows x64 Connector - 30 minute uploads\r\n解压后配置 connector.json 中的商店凭据，再双击 start.cmd。\r\n节点身份和协议密钥保持不变；现有 v2 配置可继续使用，只需更新程序。\r\n上传凭据及单次传输窗口均为30分钟，完成后图片保留15分钟供同步使用。\r\n不要公开分享配置。Windows 实机执行尚未验证。\r\n").encode("utf-8-sig"))
        private_bytes = private.getvalue()
        fd = os.open(private_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        with os.fdopen(fd, "wb") as output:
            output.write(private_bytes)
            output.flush()
            os.fsync(output.fileno())
        metadata.update({"finishedAt": datetime.datetime.now(datetime.timezone.utc).isoformat(), "checks": checks,
                         "uploadTokenSeconds": 1800, "uploadTimeoutSeconds": 1800, "proxyReadTimeoutSeconds": 1860,
                         "baseUrl": PUBLIC_EXAMPLE_URL, "addressRedacted": True,
                         "imageBytes": len(image), "imageSha256": hashlib.sha256(image).hexdigest(),
                         "verificationNodeId": node_id, "vendorTraffic": False, "deliveryNodeId": delivery_id,
                         "privatePackage": str(private_path.relative_to(PROJECT)), "privatePackageSha256": hashlib.sha256(private_bytes).hexdigest(),
                         "notVerified": ["Real store accounts", "Execution on a real Windows machine"]})
        success = True
    finally:
        if process is not None and process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
        cleanup_errors = []
        if mapped:
            try:
                check(request(binding, admin, "DELETE")[0] == 204, "verification app mapping removed")
            except Exception as error:
                cleanup_errors.append(error)
        if node_id is not None:
            try:
                check(request("/v1/connectors/" + node_id, admin, "PATCH", {"enabled": False})[0] == 200,
                      "verification node disabled")
            except Exception as error:
                cleanup_errors.append(error)
        if cleanup_errors:
            success = False
        if connection is not None:
            connection.close()
            connection = None
        if not success:
            if selected:
                select(previous, "-rollback")
                remote("systemctl restart testflying-store-api")
            if proxy_changed:
                write_remote("/etc/bugsmith/zentao-tls/nginx.conf", original_proxy)
                remote("docker exec zentao-tls-proxy nginx -t && docker exec zentao-tls-proxy nginx -s reload")
            print("Previous production state restored after unsuccessful verification", flush=True)
        if cleanup_errors:
            raise RuntimeError("Verification cleanup needs review after rollback") from cleanup_errors[0]
    # Persist the completed cleanup checks too; no secrets, file bodies or JWTs enter the report.
    metadata["cleanup"] = "Local verification process stopped; mapping removed; verification node disabled. See supervisor run cleanup report."
    metadata["checks"] = checks
    write_remote(release + "/upload-verification.json", (json.dumps(metadata, indent=2) + "\n").encode())
    (PROJECT / "docs/upload-deployment-verification.json").write_text(json.dumps(metadata, indent=2) + "\n")
    print("Upload-capable API deployed and verified; production address omitted", flush=True)


if __name__ == "__main__":
    main()
