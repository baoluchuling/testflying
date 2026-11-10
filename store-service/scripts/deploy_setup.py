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
import zipfile

from verify_store_refactor import PROJECT, managed_root, source_hashes


def existing_connector_source_matches(relative: str, digest: str) -> bool:
    contents = (PROJECT / relative).read_bytes()
    if hashlib.sha256(contents).hexdigest() == digest:
        return True
    # The screenshot example alone uses these dev-dependencies. Require every
    # other line and every resolved package to match the existing store builds.
    if relative == "crates/store-connector/Cargo.toml":
        normal, separator, dev = contents.partition(b"[dev-dependencies]\n")
        for line in (b'libc = "0.2"\n', b"tempfile.workspace = true\n", b"uuid.workspace = true\n"):
            dev = dev.replace(line, b"", 1)
        contents = normal + separator + dev
    elif relative == "Cargo.lock":
        sections = contents.split(b"\n[[package]]\n")
        for index, section in enumerate(sections):
            if section.startswith(b'name = "store-connector"\n'):
                for line in (b' "libc",\n', b' "tempfile",\n', b' "uuid",\n'):
                    section = section.replace(line, b"", 1)
                sections[index] = section
                break
        contents = b"\n[[package]]\n".join(sections)
    return hashlib.sha256(contents).hexdigest() == digest


def main() -> None:
    root = managed_root()
    if sys.argv[1:] not in ([], ["integrations"], ["api-docs"], ["config-update"]):
        raise RuntimeError("Use integrations, api-docs or config-update for a verified rollout")
    replace_connectors = sys.argv[1:] == ["config-update"]
    documents_only = sys.argv[1:] in (["api-docs"], ["config-update"])
    integrations_only = sys.argv[1:] in (["integrations"], ["api-docs"], ["config-update"])
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
    build = json.loads((PROJECT / "docs" / ("deployment-build.json" if replace_connectors else "setup-api-build.json")).read_text())
    verified = json.loads((PROJECT / "docs/verification.json").read_text())
    if source_hashes() != build["sourceHashes"] or source_hashes() != verified["sourceHashes"]:
        raise RuntimeError("API sources differ from the verified release")
    api = PROJECT / "releases/linux-amd64/store-api"
    expected_api = build["outputs"]["releases/linux-amd64/store-api"]["sha256"]
    if hashlib.sha256(api.read_bytes()).hexdigest() != expected_api:
        raise RuntimeError("Linux API checksum mismatch")
    worker_relative = "releases/darwin-arm64/screenshot-connector"
    worker = build["outputs"].get(worker_relative)
    if worker and hashlib.sha256((PROJECT / worker_relative).read_bytes()).hexdigest() != worker["sha256"]:
        raise RuntimeError("macOS Screenshot Connector checksum mismatch")
    snapshot = json.loads(remote("""
import os,json,hashlib,subprocess
e=open('/etc/testflying-store-api/service.env','rb').read()
v=dict(l.strip().split('=',1)for l in e.decode().splitlines()if '='in l and not l.startswith('#'))
current=os.path.realpath('/opt/testflying-store-api/current')
hashes={}
for target,name in [('linux-amd64','store-connector'),('darwin-arm64','store-connector'),('windows-amd64','store-connector.exe')]:
 p=os.path.join(current,'releases',target,name)
 hashes[target]=hashlib.sha256(open(p,'rb').read()).hexdigest()
worker=os.path.join(current,'releases/darwin-arm64/screenshot-connector')
print(json.dumps({'current':current,'environmentSha256':hashlib.sha256(e).hexdigest(),
 'proxySha256':hashlib.sha256(open('/etc/bugsmith/zentao-tls/nginx.conf','rb').read()).hexdigest(),
 'url':v['STORE_API_PUBLIC_URL'],'admin':v['STORE_API_ADMIN_TOKEN'],'token':v['STORE_API_TOKEN'],
 'screenshotUploadConfigured':bool(v.get('STORE_API_SCREENSHOT_UPLOAD_URL')),
 'screenshotConnectorSha256':hashlib.sha256(open(worker,'rb').read()).hexdigest() if os.path.isfile(worker) else None,
 'binaries':hashes,'available':int(subprocess.check_output(['df','--output=avail','-B1','/opt']).decode().splitlines()[-1])}))
"""))
    if snapshot["available"] <= 21474836480:
        raise RuntimeError("Server free-space reserve is insufficient")
    replacements = {}
    previous_publication = json.loads((PROJECT / "docs/setup-deployment-verification.json").read_text())
    for target, report_name in [("linux-amd64", "deployment-build.json"), ("darwin-arm64", "deployment-build.json"),
                                ("windows-amd64", "windows-release-build.json")]:
        old = json.loads((PROJECT / "docs" / report_name).read_text())
        binary_name = "store-connector.exe" if target.startswith("windows") else "store-connector"
        if replace_connectors:
            if snapshot["binaries"][target] != previous_publication["storeConnectorPublication"]["downloadVerified"][target]:
                raise RuntimeError("Published Connector differs from the previous verified publication")
            if old["sourceHashes"] != verified["sourceHashes"]:
                raise RuntimeError("Connector replacement sources differ from the verified center release")
            relative = f"releases/{target}/{binary_name}"
            expected = old["outputs"][relative]["sha256"]
            if hashlib.sha256((PROJECT / relative).read_bytes()).hexdigest() != expected:
                raise RuntimeError("Connector replacement checksum mismatch")
            replacements[target] = {"relative": relative, "sha256": expected, "name": binary_name}
            continue
        if snapshot["binaries"][target] != old["outputs"][f"releases/{target}/{binary_name}"]["sha256"]:
            raise RuntimeError("An existing Connector binary changed")
        for relative, digest in old["sourceHashes"].items():
            if relative.startswith(("crates/store-protocol/", "crates/store-provider/", "crates/store-connector/")) or relative in ("Cargo.toml", "Cargo.lock"):
                if not existing_connector_source_matches(relative, digest):
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
        if worker:
            executable = PROJECT / worker_relative
            member = tarfile.TarInfo("bin/screenshot-connector")
            member.size, member.mode = executable.stat().st_size, 0o755
            with executable.open("rb") as stream:
                output.addfile(member, stream)
        for target, replacement in replacements.items():
            executable = PROJECT / replacement["relative"]
            member = tarfile.TarInfo("bin/connector-" + target)
            member.size, member.mode = executable.stat().st_size, 0o755
            with executable.open("rb") as stream:
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
legacy=os.path.join(old,'releases/darwin-universal')
shutil.copytree(os.path.join(old,'releases'),os.path.join(p,'releases'),symlinks=True,
                ignore=lambda source,names: {'screenshot-connector'} if source==legacy else set())
assert hashlib.sha256(open(os.path.join(p,'bin/store-api'),'rb').read()).hexdigest()==""" + repr(expected_api) + """
worker_sha=""" + repr(worker["sha256"] if worker else None) + """
if worker_sha:
 staged=os.path.join(p,'bin/screenshot-connector')
 assert hashlib.sha256(open(staged,'rb').read()).hexdigest()==worker_sha
 target=os.path.join(p,'releases/darwin-arm64')
 os.makedirs(target,exist_ok=True)
 os.replace(staged,os.path.join(target,'screenshot-connector'))
replacements=""" + repr(replacements) + """
for target,entry in replacements.items():
 staged=os.path.join(p,'bin/connector-'+target)
 assert hashlib.sha256(open(staged,'rb').read()).hexdigest()==entry['sha256']
 os.replace(staged,os.path.join(p,entry['relative']))
""", payload)
    checks = []
    connections = {}
    public = urllib.parse.urlsplit(snapshot["url"])
    ca = ssl.create_default_context(cafile=str(PROJECT / "deploy/testflying-prod-ca.crt"))

    def request(path: str, token: str | None = None, method: str = "GET", data: dict | None = None,
                scheme: str = "https", *, setup_session: str | None = None,
                cookies: list[str] | None = None) -> tuple[int, bytes]:
        try:
            return request_once(path, token, method, data, scheme, setup_session, cookies)
        except http.client.RemoteDisconnected:
            connection = connections.pop(scheme, None)
            if connection is not None:
                connection.close()
            if method != "GET":
                raise
            return request_once(path, token, method, data, scheme, setup_session, cookies)

    def request_once(path: str, token: str | None, method: str, data: dict | None,
                     scheme: str, setup_session: str | None,
                     cookies: list[str] | None) -> tuple[int, bytes]:
        if scheme not in connections:
            connections[scheme] = (http.client.HTTPSConnection(public.hostname, context=ca, timeout=45)
                                   if scheme == "https" else http.client.HTTPConnection(public.hostname, timeout=45))
        connection = connections[scheme]
        headers = {"Content-Type": "application/json"}
        if token:
            headers["Authorization"] = "Bearer " + token
        if setup_session is not None:
            headers["X-Testflying-Setup"] = "1"
            if setup_session:
                headers["Cookie"] = setup_session
        connection.request(method, public.path.rstrip("/") + path, headers=headers,
                           body=None if data is None else json.dumps(data).encode())
        response = connection.getresponse()
        if cookies is not None:
            cookies.extend(value for key, value in response.getheaders() if key.lower() == "set-cookie")
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
        status, body = request("/v1/connectors", snapshot["admin"])
        check(status == 200, "node connection inventory readable before rollout")
        previously_connected = {node["connectorId"] for node in json.loads(body)["connectors"] if node["connected"]}
        management = "/v1/integrations"
        existing_token = None
        if integrations_only:
            status, body = request(management, snapshot["admin"])
            check(status == 200, "existing integration inventory readable before rollout")
            existing = json.loads(body)["integrations"]
            if existing:
                status, body = request(management + "/" + existing[0]["id"] + "/token", snapshot["admin"])
                check(status == 200, "existing credential retrieved for continuity check")
                existing_token = json.loads(body)["token"]
            for connection in connections.values():
                connection.close()
            connections.clear()
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
        if documents_only:
            status, body = request("/v1/logs?limit=20", snapshot["admin"])
            logs = json.loads(body)
            check(status == 200 and logs["retentionDays"] == 7 and logs["storageHealthy"],
                  "seven-day persistent center log store available")
            check(any(row["event"] == "service.started" for row in logs["entries"]),
                  "center startup and subsequent HTTP requests recorded")
            check(request("/v1/logs", snapshot["token"])[0] == 401,
                  "business credentials cannot read administrator logs")
            for asset in ("logs.mjs", "app.mjs", "index.html"):
                path = "/connector-setup/" + ("" if asset == "index.html" else asset)
                status, content = request(path)
                check(status == 200 and content == (PROJECT / "crates/store-api/src/setup" / asset).read_bytes(),
                      "exact deployed log viewer asset " + asset)
            recovered = not previously_connected
            for _ in range(40):
                status, body = request("/v1/connectors", snapshot["admin"])
                connected = {node["connectorId"] for node in json.loads(body)["connectors"] if node["connected"]}
                if previously_connected <= connected:
                    recovered = True
                    break
                time.sleep(0.5)
            check(recovered, "previously connected nodes recovered after center restart")
            docs = "/v1/store-management/api-docs"
            for asset in ("start.cmd", "install.ps1", "install.sh"):
                status, content = request("/connector-setup/" + asset)
                check(status == 200 and content == (PROJECT / "packaging/connector" / asset).read_bytes(),
                      "exact deployed installation asset " + asset)
            if replacements:
                status, body = request("/v1/connectors", snapshot["admin"])
                check(status == 200, "existing Connector inventory available for download verification")
                connector_id = next(node["connectorId"] for node in json.loads(body)["connectors"]
                                    if node["enabled"] and node.get("kind", "store") == "store")
                for target, replacement in replacements.items():
                    status, package = request(f"/v1/connectors/{connector_id}/installer/{target}", snapshot["admin"])
                    check(status == 200, target + " updated installation package available")
                    with zipfile.ZipFile(io.BytesIO(package)) as archive:
                        check(hashlib.sha256(archive.read(replacement["name"])).hexdigest() == replacement["sha256"],
                              target + " downloaded native installer preparation program verified")
            for scheme in ("http", "https"):
                status, body = request(docs, snapshot["token"], scheme=scheme)
                catalog = json.loads(body)
                check(status == 200 and catalog["contractVersion"] == "2026-11-10",
                      scheme + " business-token documentation available")
                endpoints = catalog["endpoints"]
                check(len(endpoints) == 11
                      and sum(row["group"] == "store" for row in endpoints) == 9
                      and sum(row["group"] == "node" for row in endpoints) == 2
                      and len(catalog["uploadEndpoints"]) == 2,
                      scheme + " nine store and two node endpoints plus upload helpers")
                check(catalog["imageQuery"]["parameters"][2]["enum"] == ["phone", "tablet", "foldable"]
                      and "foldable" not in catalog["imageQuery"]["android"]["devices"],
                      scheme + " language and device options with platform limits")
                for format_name, relative in (("markdown", "docs/store-api.md"), ("openapi", "docs/openapi.yaml")):
                    status, content = request(docs + "?format=" + format_name, snapshot["token"], scheme=scheme)
                    check(status == 200 and content == (PROJECT / relative).read_bytes(),
                          scheme + " exact deployed " + format_name + " document")
                check(request(docs, scheme=scheme)[0] == 401,
                      scheme + " anonymous documentation denied")
            status, body = request(docs + "?format=html", snapshot["token"])
            check(status == 422 and json.loads(body)["error"]["code"] == "invalid_document_format",
                  "unsupported documentation format rejected")
            if existing_token:
                check(request(docs, existing_token)[0] == 200,
                      "existing HTTPS integration token can discover API documentation")
                check(request(docs, existing_token, scheme="http")[0] == 403,
                      "integration documentation access still requires HTTPS")
            after = json.loads(remote("""
import json,hashlib,os
p=os.path.realpath('/opt/testflying-store-api/current')
sha=lambda p:hashlib.sha256(open(p,'rb').read()).hexdigest()
print(json.dumps({'current':p,'apiSha256':sha(p+'/bin/store-api'),
 'environmentSha256':sha('/etc/testflying-store-api/service.env'),
 'proxySha256':sha('/etc/bugsmith/zentao-tls/nginx.conf'),
 'screenshotConnectorSha256':sha(p+'/releases/darwin-arm64/screenshot-connector'),
 'binaries':{t:sha(p+'/releases/'+t+'/'+n) for t,n in [('linux-amd64','store-connector'),('darwin-arm64','store-connector'),('windows-amd64','store-connector.exe')]}}))
"""))
            check(after["current"] == release and after["apiSha256"] == expected_api,
                  "verified center release selected")
            check(after["environmentSha256"] == snapshot["environmentSha256"]
                  and after["proxySha256"] == snapshot["proxySha256"],
                  "persistent service and HTTP HTTPS proxy configuration preserved")
            expected_connectors = {target: entry["sha256"] for target, entry in replacements.items()} if replacements else snapshot["binaries"]
            check(after["binaries"] == expected_connectors
                  and after["screenshotConnectorSha256"] == snapshot["screenshotConnectorSha256"],
                  "store Connector release hashes and preserved screenshot Connector verified")
            previous_report = json.loads(report_path.read_text())
            report = {"runId": root.name, "stage": "deployed-and-verified", "production": True,
                      "finishedAt": datetime.datetime.now(datetime.timezone.utc).isoformat(),
                      "deploymentCommit": commit, "binarySourceCommit": build["sourceCommit"],
                      "apiSha256": expected_api, "connectorBinarySha256": expected_connectors,
                      "screenshotConnectorSha256": snapshot["screenshotConnectorSha256"],
                      "screenshotUploadConfigured": snapshot["screenshotUploadConfigured"],
                      "checks": checks, "testNodesCreated": 0, "appBindingsWritten": 0,
                      "actualStoreAccountsUsed": False,
                      "apiDocumentation": {"contractVersion": "2026-11-10", "formats": ["json", "markdown", "openapi"],
                                           "existingIntegrationTokenVerified": bool(existing_token)},
                      "cleanup": "See the supervisor cleanup report for this runId"}
            if "storeConnectorPublication" in previous_report:
                report["storeConnectorPublication"] = previous_report["storeConnectorPublication"]
            if replacements:
                report["storeConnectorPublication"] = {"runId": root.name, "sourceCommit": commit,
                    "downloadVerified": expected_connectors, "serviceConfigPreserved": True,
                    "screenshotBinaryPreserved": True, "configurationMergeSupported": True,
                    "actualStoreAccountsUsed": False}
            report_path.write_text(json.dumps(report, indent=2) + "\n")
            success = True
            return
        for suffix in ("", "app.mjs", "integrations.mjs", "packager.mjs", "style.css", "jszip.min.js", "ca.crt", "start.cmd", "start.sh",
                       "install.cmd", "install.ps1", "install.sh"):
            check(request("/connector-setup/" + suffix)[0] == 200, "setup asset " + (suffix or "index"))
        check(request("/v1/connectors")[0] == 401, "administrator authentication still required")
        setup = "/connector-setup/api/connectors"
        check(request(setup, scheme="http")[0] == 403, "node setup requires HTTPS")
        check(request(setup)[0] == 401, "anonymous node listing rejected")
        check(request(setup, method="POST", data={"name": ""})[0] == 401, "anonymous node creation rejected")
        check(request(setup + "/not-a-node/installer/windows-amd64")[0] == 401,
              "anonymous installer download rejected before lookup")
        check(request(setup, snapshot["token"])[0] == 401, "business token cannot access node setup")
        status, setup_nodes = request(setup, snapshot["admin"])
        check(status == 200, "administrator can access node setup")
        if integrations_only:
            discovery = "/v1/store-management/connectors"
            login = "/connector-setup/api/session"
            cookies = []
            check(request(login, method="POST", scheme="http", setup_session="")[0] == 403,
                  "administrator session login requires HTTPS")
            check(request(login, method="POST", setup_session="")[0] == 401,
                  "anonymous requests cannot issue administrator sessions")
            try:
                check(request(login, snapshot["admin"], "POST", setup_session="", cookies=cookies)[0] == 204,
                      "administrator can establish an HTTPS setup session")
                check(len(cookies) == 1 and cookies[0].startswith("__Secure-tf-setup="),
                      "proxy passes through the setup session cookie")
                check(cookies[0].endswith("; Secure; HttpOnly; SameSite=Strict"),
                      "setup cookie is Secure, HttpOnly and SameSite Strict")
                cookie = cookies[0].split(";", 1)[0]
                check(request(setup, setup_session=cookie)[0] == 200,
                      "setup session restores node management without sending the administrator token again")
                check(request("/connector-setup/api/integrations", setup_session=cookie)[0] == 200,
                      "setup session can list integrations through the scoped browser route")
                check(request(management, setup_session=cookie)[0] == 401,
                      "public management routes still require a bearer token")
            finally:
                if cookies:
                    cookie = cookies[0].split(";", 1)[0]
                    check(request(login, method="DELETE", setup_session=cookie)[0] == 204,
                          "administrator setup session was revoked")
                    check(request(setup, setup_session=cookie)[0] == 401,
                          "revoked setup session cannot be replayed")
            check(request(management, scheme="http")[0] == 403, "credential management requires HTTPS")
            check(request(management)[0] == 401, "anonymous credential management rejected")
            check(request(management, snapshot["token"])[0] == 401, "legacy business token cannot manage credentials")
            status, body = request(discovery, snapshot["token"])
            check(status == 200 and bool(json.loads(body)["connectors"]), "legacy business token can list enabled nodes")
            business_nodes = {row["connectorId"] for row in json.loads(body)["connectors"]}
            connector_id = next(iter(business_nodes))
            status, body = request("/connector-setup/")
            check(status == 200 and b'id="screenshotWorkspace"' in body and b'id="screenshotRefresh"' in body
                  and b'id="integrationNodes"' not in body,
                  "deployed setup includes screenshot download and connection status and issues global tokens")
            installer = "/v1/connectors/" + connector_id + "/screenshot-installer"
            check(request(installer)[0] == 401 and request(installer, snapshot["admin"])[0] == 422,
                  "screenshot installer route is administrator-only and rejects a store node")
            status, body = request(management, snapshot["admin"])
            check(status == 200 and all(row.get("allConnectors") is True and "token" not in row
                                        for row in json.loads(body)["integrations"]),
                  "all existing integrations report full node access without exposing token values")
            if existing_token:
                status, body = request(discovery, existing_token)
                check(status == 200 and {row["connectorId"] for row in json.loads(body)["connectors"]} == business_nodes,
                      "original existing token can access all enabled store nodes")
            name = "Integration verification " + root.name
            token = None
            try:
                status, body = request(management, snapshot["admin"], "POST", {"name": name})
                check(status == 201, "temporary integration credential issued")
                created = json.loads(body)
                check(created["integration"]["allConnectors"] is True, "new integration grants all nodes")
                integration = management + "/" + created["integration"]["id"]
                token = created["token"]
                status, body = request(integration + "/token", snapshot["admin"])
                check(status == 200 and json.loads(body)["token"] == token, "administrator can retrieve the same credential")
                check(request(integration + "/token", token)[0] == 401, "integration cannot retrieve admin credentials")
                status, body = request(discovery, token)
                check(status == 200 and {row["connectorId"] for row in json.loads(body)["connectors"]} == business_nodes,
                      "new integration lists all enabled store nodes")
                status, body = request("/v1/screenshot-jobs", token, "POST",
                                       {"connectorId": connector_id, "prompt": "verify"})
                check((status, json.loads(body)["error"]["code"]) == (422, "invalid_screenshot_connector"),
                      "profile-free screenshot submission validates the node independently of the upload destination")
            finally:
                status, body = request(management, snapshot["admin"])
                check(status == 200, "temporary integration cleanup listing available")
                for item in json.loads(body)["integrations"]:
                    if item["name"] == name:
                        check(request(management + "/" + item["id"], snapshot["admin"], "DELETE")[0] == 204,
                              "temporary integration credential revoked")
            check(token is not None and request(discovery, token)[0] == 401, "revoked integration credential rejected")
            after = json.loads(remote("""
import json,hashlib,os
print(json.dumps({'environmentSha256':hashlib.sha256(open('/etc/testflying-store-api/service.env','rb').read()).hexdigest(),
'proxySha256':hashlib.sha256(open('/etc/bugsmith/zentao-tls/nginx.conf','rb').read()).hexdigest(),
'current':os.path.realpath('/opt/testflying-store-api/current')}))
"""))
            check(after["current"] == release, "new API release selected")
            check(after["environmentSha256"] == snapshot["environmentSha256"], "persistent API configuration unchanged")
            check(after["proxySha256"] == snapshot["proxySha256"], "HTTP and HTTPS proxy configuration unchanged")
            expected_worker = worker["sha256"] if worker else snapshot["screenshotConnectorSha256"]
            if expected_worker:
                check(remote("""
import hashlib
print(hashlib.sha256(open('/opt/testflying-store-api/current/releases/darwin-arm64/screenshot-connector','rb').read()).hexdigest())
""").decode().strip() == expected_worker, "published Apple Silicon Screenshot Connector checksum")
                screenshot_node = next((node for node in json.loads(setup_nodes)["connectors"]
                                        if node["kind"] == "screenshot" and node["enabled"]), None)
                if screenshot_node:
                    screenshot_id = screenshot_node["connectorId"]
                    status, package = request(f"/v1/connectors/{screenshot_id}/screenshot-installer", snapshot["admin"])
                    check(status == 200, "administrator can download an existing screenshot node installer")
                    with zipfile.ZipFile(io.BytesIO(package)) as archive:
                        check(set(archive.namelist()) == {"connector.json", "screenshot-connector", "install.sh",
                                                        "run.sh", "README.md", "testflying-prod-ca.crt"},
                              "screenshot package contains the native worker and its installation files")
                        config = json.loads(archive.read("connector.json"))
                        check(archive.read("install.sh") == (PROJECT / "packaging/screenshot-connector/install.sh").read_bytes(),
                              "download contains the current LaunchDaemon installer script")
                        check("projectRoots" not in config and "profiles" not in config,
                              "download supports machine-local discovery without predefined projects or profiles")
                        check({"codexPath", "codexHome", "codexPermissionProfile"}.isdisjoint(config),
                              "download uses the installed Codex without machine-specific settings")
                        executable = archive.read("screenshot-connector")
                        check(executable.startswith(b"\xcf\xfa\xed\xfe\x0c\0\0\x01"),
                              "download contains a single-architecture Apple Silicon Mach-O")
                        check(config["connectorId"] == screenshot_id and len(config["messageKey"]) == 64
                              and archive.getinfo("install.sh").external_attr >> 16 & 0o700 == 0o700
                              and hashlib.sha256(executable).hexdigest() == expected_worker,
                              "download contains the node's WSS configuration, installer and verified macOS worker")
            report = {"runId": root.name, "stage": "deployed-and-verified", "production": True,
                      "finishedAt": datetime.datetime.now(datetime.timezone.utc).isoformat(),
                      "deploymentCommit": commit, "binarySourceCommit": build["sourceCommit"],
                      "apiSha256": expected_api, "connectorBinarySha256": snapshot["binaries"],
                      "checks": checks, "testNodesCreated": 0, "appBindingsWritten": 0,
                      "screenshotUploadConfigured": snapshot["screenshotUploadConfigured"],
                      "screenshotConnectorSha256": expected_worker,
                      "temporaryIntegrationRevoked": True, "cleanup": "See the supervisor cleanup report for this runId"}
            report_path.write_text(json.dumps(report, indent=2) + "\n")
            success = True
            return
        status, body = request(setup, snapshot["admin"], method="POST",
                               data={"name": "Setup deployment verification (mock only)"})
        check(status == 201, "temporary verification node created")
        created = json.loads(body)
        node = created["connector"]["connectorId"]
        status, body = request(setup, snapshot["admin"])
        check(status == 200 and any(item["connectorId"] == node for item in json.loads(body)["connectors"]),
              "administrator list includes the verification node")
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
                  "baseUrl": "https://store-api.example.com/store-api", "addressRedacted": True,
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
