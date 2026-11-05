"""Publish and verify the Windows Connector without restarting the Store API."""
from __future__ import annotations

import datetime
import hashlib
import io
import json
import os
from pathlib import Path
import shlex
import ssl
import urllib.error
import urllib.request
import zipfile

from deploy_prod import BASE_URL, PUBLIC_EXAMPLE_URL, remote
from verify_store_refactor import PROJECT, managed_root, source_hashes


def main() -> None:
    root = managed_root()
    if not os.environ.get("STORE_API_PUBLIC_URL") or "store-api.example.com" in BASE_URL:
        raise RuntimeError("Set the real STORE_API_PUBLIC_URL privately before deployment")
    build = json.loads((PROJECT / "docs/windows-release-build.json").read_text())
    if source_hashes() != build["sourceHashes"]:
        raise RuntimeError("Windows release source changed")
    binary = (PROJECT / "releases/windows-amd64/store-connector.exe").read_bytes()
    digest = hashlib.sha256(binary).hexdigest()
    if digest != build["outputs"]["releases/windows-amd64/store-connector.exe"]["sha256"]:
        raise RuntimeError("Windows release checksum mismatch")
    deployed = json.loads((PROJECT / "docs/deployment-verification.json").read_text())
    release = deployed["releaseDirectory"]
    # Keep the checksum-addressed executable as a formal release, then publish its alias.
    directory = release + "/releases/windows-amd64"
    stored = directory + "/store-connector-" + digest + ".exe"
    alias = directory + "/store-connector.exe"
    quote = shlex.quote
    main_pid = remote("systemctl show testflying-store-api -p MainPID --value").decode().strip()
    remote("test \"$(readlink -f /opt/testflying-store-api/current)\" = " + quote(release)
           + " && test ! -e " + quote(alias) + " && test ! -L " + quote(alias)
           + " && test ! -e " + quote(stored)
           + " && umask 022 && mkdir -p " + quote(directory) + " && cat > " + quote(stored), binary)
    measured = remote("sha256sum -- " + quote(stored)).decode().split()[0]
    if measured != digest:
        raise RuntimeError("Uploaded Windows checksum mismatch; alias was not published")
    remote("ln -s " + quote(Path(stored).name) + " " + quote(alias))
    values = dict(line.split("=", 1) for line in remote(
        "cat /etc/testflying-store-api/service.env").decode().splitlines() if "=" in line)
    ca = (PROJECT / "deploy/testflying-prod-ca.crt").read_bytes()
    context = ssl.create_default_context(cadata=ca.decode())
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}),
                                         urllib.request.HTTPSHandler(context=context))

    def request(path: str, method: str = "GET", payload: dict | None = None) -> tuple[int, bytes]:
        headers = {"Authorization": "Bearer " + values["STORE_API_ADMIN_TOKEN"]}
        if payload is not None:
            headers["Content-Type"] = "application/json"
        query = urllib.request.Request(BASE_URL + path, method=method, headers=headers,
                                       data=None if payload is None else json.dumps(payload).encode())
        try:
            with opener.open(query, timeout=30) as response:
                return response.status, response.read(128 * 1024 * 1024)
        except urllib.error.HTTPError as error:
            return error.code, error.read(65536)

    status, body = request("/v1/connectors", "POST",
                           {"name": "Windows download verification (no store credentials)"})
    if status != 201:
        raise RuntimeError("Cannot create installation verification node")
    node = json.loads(body)
    identifier = node["connector"]["connectorId"]
    try:
        status, body = request("/v1/connectors/" + identifier + "/installer/windows-amd64")
        if status != 200:
            raise RuntimeError("Windows installation download failed")
        with zipfile.ZipFile(io.BytesIO(body)) as archive:
            if set(archive.namelist()) != {"store-connector.exe", "connector.json", "start.cmd"}:
                raise RuntimeError("Unexpected Windows archive content")
            if hashlib.sha256(archive.read("store-connector.exe")).hexdigest() != digest:
                raise RuntimeError("Downloaded Windows executable differs from release")
            config = json.loads(archive.read("connector.json"))
            if config != node["installation"] or config["serverUrl"] != BASE_URL:
                raise RuntimeError("Windows installer has incorrect node credentials")
            launcher = archive.read("start.cmd")
            if b"store-connector.exe connector.json\r\n" not in launcher or b'cd /d "%~dp0"' not in launcher:
                raise RuntimeError("Unexpected Windows launcher")
    finally:
        status, _ = request("/v1/connectors/" + identifier, "PATCH", {"enabled": False})
        if status != 200:
            raise RuntimeError("Cannot disable verification node")
    after_pid = remote("systemctl show testflying-store-api -p MainPID --value").decode().strip()
    if not main_pid or main_pid == "0" or after_pid != main_pid:
        raise RuntimeError("API process changed during node release publication")
    # This is the user's actual installation identity, not the disabled verification node.
    delivery_path = PROJECT / "releases/windows-amd64/connector-windows.zip"
    if delivery_path.exists() or delivery_path.is_symlink():
        raise RuntimeError("User installation package already exists; refusing to overwrite")
    status, body = request("/v1/connectors", "POST",
                           {"name": "Windows Connector（待配置账号）"})
    if status != 201:
        raise RuntimeError("Cannot create user installation node")
    delivery_node = json.loads(body)
    delivery_id = delivery_node["connector"]["connectorId"]
    try:
        status, body = request("/v1/connectors/" + delivery_id + "/installer/windows-amd64")
        if status != 200:
            raise RuntimeError("Cannot download user's Windows installation")
        packaged = io.BytesIO()
        with zipfile.ZipFile(io.BytesIO(body)) as source, zipfile.ZipFile(packaged, "w", zipfile.ZIP_DEFLATED) as bundle:
            if hashlib.sha256(source.read("store-connector.exe")).hexdigest() != digest:
                raise RuntimeError("User installation executable checksum mismatch")
            if json.loads(source.read("connector.json")) != delivery_node["installation"]:
                raise RuntimeError("User installation identity mismatch")
            bundle.writestr("store-connector.exe", source.read("store-connector.exe"))
            bundle.writestr("connector.json", source.read("connector.json"))
            bundle.writestr("testflying-prod-ca.crt", ca)
            bundle.writestr("start.cmd", '@echo off\r\ncd /d "%~dp0"\r\nset "SSL_CERT_FILE=%~dp0testflying-prod-ca.crt"\r\nstore-connector.exe connector.json\r\nif errorlevel 1 pause\r\n')
            bundle.writestr("README.txt", (
                "Windows x64 Connector\r\n\r\n"
                "1. 解压到本机目录。安装包已包含专属 connectorId、Token 和中心地址。\r\n"
                "2. 用文本编辑器修改 connector.json 的 provider，填写本机商店账号凭据：\r\n"
                "   Apple: appleIssuerId、appleKeyId、applePrivateKeyPath（P-256 PKCS8 p8 文件路径）。\r\n"
                "   Google: googleServiceAccountPath（服务账号 JSON 路径）。\r\n"
                "   Windows JSON 路径中的反斜线要写成两个，或直接使用正斜线，如 C:/secure/AuthKey.p8。\r\n"
                "3. 双击 start.cmd，窗口保持运行；脚本已指定包内公开 CA 验证 HTTPS/WSS。\r\n"
                "4. 此节点尚未绑定应用。用中心管理接口将 platform + Bundle ID/包名映射到节点。\r\n"
                "   节点 ID: " + delivery_id + "\r\n"
                "   一个账号用一个节点，多个应用可映射到同一节点。\r\n\r\n"
                "不要公开分享整个安装包，connector.json 内含节点凭据。该程序尚未经过 Windows 实机运行验证。\r\n"
            ).encode("utf-8-sig"))
        content = packaged.getvalue()
        descriptor = os.open(delivery_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        with os.fdopen(descriptor, "wb") as output:
            output.write(content)
            output.flush()
            os.fsync(output.fileno())
    except BaseException:
        request("/v1/connectors/" + delivery_id, "PATCH", {"enabled": False})
        raise
    report = {"runId": root.name, "finishedAt": datetime.datetime.now(datetime.timezone.utc).isoformat(),
              "baseUrl": PUBLIC_EXAMPLE_URL, "addressRedacted": True,
              "target": "windows-amd64", "rustTarget": "x86_64-pc-windows-gnu",
              "binarySha256": digest, "releasePath": stored, "downloadHttpStatus": 200,
              "zipFiles": ["store-connector.exe", "connector.json", "start.cmd"],
              "identityVerified": True, "launcherVerified": True, "apiRestarted": False,
              "verificationNodeId": identifier, "verificationNode": "disabled, no application mapping or credentials",
              "deliveryNodeId": delivery_id, "deliveryNode": "enabled, waiting for account configuration and app mapping",
              "deliveryPackage": "releases/windows-amd64/connector-windows.zip",
              "deliveryPackageSha256": hashlib.sha256(content).hexdigest(),
              "deliveryPackageMode": "0600; Git ignored; contains private node credentials",
              "notVerified": ["Execution on a real Windows machine"],
              "cleanup": "See supervisor cleanup report; checksum-addressed executable and alias are formal production files"}
    (PROJECT / "docs/windows-release-verification.json").write_text(json.dumps(report, indent=2) + "\n")
    print("PASS Windows x64 download, executable hash, node credentials and start.cmd; API was not restarted", flush=True)


if __name__ == "__main__":
    main()
