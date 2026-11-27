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
from release_catalog import deployment_outputs,git_source,installer_asset_names


def expected_platform_screenshots(snapshot: dict) -> list[list]:
    if ["platform_screenshots_v1"] in snapshot["migrationMarkers"]:
        return snapshot["platformScreenshotBindings"]
    explicit = dict(snapshot["accountScreenshotBindings"])
    expected = []
    for store, mode, _enabled in snapshot["connectors"]:
        if mode == "screenshot":
            continue
        accounts = [row for row in snapshot["accounts"] if row[3] == store]
        first = accounts[0][4] if accounts else None
        consistent = all(row[4] == first and row[6] is None for row in accounts)
        for platform in ("ios", "android"):
            own = next((row[4] for row in accounts if row[2] == platform), None)
            selected = explicit[store] if store in explicit else first if consistent else own
            expected.append([store, platform, selected, int(store in explicit)])
    return sorted(expected)


def main() -> None:
    root = managed_root()
    if len(sys.argv)>2:
        raise RuntimeError("Use changed, center, store or screenshots")
    mode=sys.argv[1] if len(sys.argv)==2 else "changed"
    documents_only = True
    integrations_only = True
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

    def routing_snapshot() -> dict:
        # Read-only transaction: never copy secret values into local reports.
        # Registration may update base_url during restart, so identity excludes it.
        return json.loads(remote("""
import os,json,hashlib,sqlite3,urllib.parse
env=dict(l.strip().split('=',1) for l in open('/etc/testflying-store-api/service.env') if '=' in l and not l.startswith('#'))
p=env.get('STORE_API_DATABASE','/var/lib/testflying-store-api/registry.sqlite3')
assert os.path.isabs(p) and os.path.isfile(p)
db=sqlite3.connect('file:'+urllib.parse.quote(p,safe='/')+'?mode=ro',uri=True)
db.execute('PRAGMA query_only=ON');db.execute('BEGIN')
tables={r[0] for r in db.execute("SELECT name FROM sqlite_master WHERE type='table'")}
def rows(table,columns='*',order='1'):
 return [list(r) for r in db.execute('SELECT '+columns+' FROM '+table+' ORDER BY '+order)] if table in tables else []
def digest(value):
 def encode(v):
  if isinstance(v,bytes): return {'blobHex':v.hex()}
  raise TypeError('unsupported database value')
 return hashlib.sha256(json.dumps(value,default=encode,sort_keys=True,separators=(',',':')).encode()).hexdigest()
store=rows('app_bindings','platform,bundle_identifier,connector_id','1,2')
shot=rows('screenshot_app_bindings','platform,bundle_identifier,connector_id','1,2')
accounts=rows('accounts','id,name,platform,store_connector_id,screenshot_connector_id,enabled,migration_error','1')
skill_rows=[(v,m,hashlib.sha256(b).hexdigest()) for v,m,b in db.execute('SELECT version,manifest,bundle FROM screenshot_skill_versions ORDER BY version')] if 'screenshot_skill_versions' in tables else []
out={'legacyStoreBindings':store,'legacyScreenshotBindings':shot,
 'legacyStoreSha256':digest(store),'legacyScreenshotSha256':digest(shot),
 'forwarding':rows('platform_forwarding','platform,enabled','1'),
 'connectorIdentitySha256':digest(rows('connectors','id,name,mode,enabled,nonce','1')),
 'connectors':rows('connectors','id,mode,enabled','1'),
 'integrationsSha256':digest(rows('integrations','id,name','1')),
 'integrationGrantsSha256':digest(rows('integration_connectors','integration_id,connector_id','1,2')),
 'screenshotSettingsSha256':digest(rows('screenshot_settings','id,encrypted','1')),
 'screenshotSkillSha256':digest(skill_rows),
 'screenshotSkillActive':rows('screenshot_skill_current','*','1'),
 'accountScreenshotBindings':rows('store_account_screenshot_bindings','store_connector_id,screenshot_connector_id','1'),
 'platformScreenshotBindings':rows('platform_screenshot_bindings','store_connector_id,platform,screenshot_connector_id,explicit','1,2'),
 'accountApps':rows('account_apps','account_id,bundle_identifier','1,2'),
 'accountAppDetails':rows('account_apps','account_id,bundle_identifier,name,store_app_id','1,2'),
 'accountAppSources':rows('account_apps','account_id,bundle_identifier,source','1,2') if 'source' in {r[1] for r in db.execute('PRAGMA table_info(account_apps)')} else [],
 'accountAppRemovals':rows('account_app_removals','account_id,bundle_identifier','1,2'),
 'legacyAndroidCandidates':[list(r) for r in db.execute('''SELECT a.id,b.bundle_identifier FROM app_bindings b
  JOIN accounts a ON a.platform=b.platform AND a.store_connector_id=b.connector_id
  JOIN account_bindings ab ON ab.platform=b.platform AND ab.bundle_identifier=b.bundle_identifier AND ab.account_id=a.id
  JOIN connectors c ON c.id=b.connector_id
  WHERE b.platform='android' AND ab.provisional=1 AND c.mode!='screenshot' AND a.catalog_status!='identity_changed'
    AND NOT EXISTS(SELECT 1 FROM account_app_removals r WHERE r.account_id=a.id AND r.bundle_identifier=b.bundle_identifier)
    AND NOT EXISTS(SELECT 1 FROM account_apps p JOIN accounts other ON other.id=p.account_id
      WHERE other.platform='android' AND p.bundle_identifier=b.bundle_identifier AND p.account_id!=a.id)
  ORDER BY a.id,b.bundle_identifier''')] if 'account_app_removals' in tables else [],
 'accountStates':rows('accounts','id,platform,catalog_status','1'),
 'applicationSwitches':rows('account_bindings','platform,bundle_identifier,suppressed','1,2'),
 'accounts':accounts,'migrationMarkers':rows('account_migrations','name','1'),
 'migrationIssues':rows('account_migration_issues','platform,bundle_identifier,code','1,2')}
db.rollback();db.close()
print(json.dumps(out))
"""))

    def git(*args: str) -> bytes:
        return subprocess.check_output(["git", *args], cwd=PROJECT, stderr=subprocess.DEVNULL)

    commit = git("rev-parse", "HEAD").decode().strip()
    if git("diff", "--name-only", "HEAD", "--", *source_hashes()).strip():
        raise RuntimeError("Commit the Store API source before deploying")
    pushed = subprocess.check_output(["gh", "api", "repos/baoluchuling/testflying/git/ref/heads/refactor/store-service-rust",
                                      "--jq", ".object.sha"], cwd=PROJECT, text=True).strip()
    if pushed != commit:
        raise RuntimeError("Push the deployment commit first")
    verified=json.loads((PROJECT/"docs/verification.json").read_text())
    if source_hashes()!=verified["sourceHashes"]:
        raise RuntimeError("Deployment source differs from the final verified source")
    previous_publication=json.loads((PROJECT/"docs/setup-deployment-verification.json").read_text())
    published,changed_components=deployment_outputs(mode,previous_publication)
    if not changed_components:
        raise RuntimeError("No newly built selected component differs from the publication")
    replace_connectors="store" in changed_components
    restart_center="center" in changed_components
    center_output=published["releases/linux-amd64/store-api"]
    store_output=published["releases/linux-amd64/store-connector"]
    screenshot_output=published["releases/darwin-arm64/screenshot-connector"]
    release_version=store_output["version"]
    release_fingerprint=store_output["sourceFingerprint"]
    release_versions={"center":center_output["version"],"store":release_version,"screenshot":screenshot_output["version"]}
    api=PROJECT/"releases/linux-amd64/store-api"
    expected_api=center_output["sha256"]
    worker_relative="releases/darwin-arm64/screenshot-connector"
    worker=screenshot_output if "screenshot" in changed_components else None
    center_source=lambda relative:git_source(center_output["sourceCommit"],relative)
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
 'apiSha256':hashlib.sha256(open(os.path.join(current,'bin/store-api'),'rb').read()).hexdigest(),
 'servicePid':int(subprocess.check_output(['systemctl','show','testflying-store-api','--property=MainPID','--value']).strip()),
 'binaries':hashes,'available':int(subprocess.check_output(['df','--output=avail','-B1','/opt']).decode().splitlines()[-1])}))
"""))
    routing_before = routing_snapshot() if documents_only else None
    if snapshot["apiSha256"]!=previous_publication["apiSha256"]:
        raise RuntimeError("Running API differs from the last verified publication")
    if snapshot["screenshotConnectorSha256"]!=previous_publication["screenshotConnectorSha256"]:
        raise RuntimeError("Published Screenshot Connector differs from its last verified publication")
    if snapshot["binaries"]!=previous_publication["connectorBinarySha256"]:
        raise RuntimeError("Published Store programs differ from their last verified publication")
    if snapshot["available"]<=21474836480:
        raise RuntimeError("Server free-space reserve is insufficient")
    replacements={}
    for target in ("linux-amd64","darwin-arm64","windows-amd64"):
        name="store-connector.exe" if target.startswith("windows") else "store-connector"
        relative=f"releases/{target}/{name}"
        output=published[relative]
        if output["sha256"]!=snapshot["binaries"][target]:
            replacements[target]={"relative":relative,"sha256":output["sha256"],"name":name}
    source_archive = git("archive", "--format=tar", "--prefix=source/", commit + ":store-service")
    archive = io.BytesIO()
    with tarfile.open(fileobj=archive, mode="w") as output:
        with tarfile.open(fileobj=io.BytesIO(source_archive), mode="r:") as source:
            for member in source:
                if not (member.isfile() or member.isdir()) or ".." in member.name.split("/"):
                    raise RuntimeError("Unexpected committed source member")
                output.addfile(member, source.extractfile(member) if member.isfile() else None)
        for kind,node_output in (("store",store_output),("screenshot",screenshot_output)):
            source_directory="connector" if kind=="store" else "screenshot-connector"
            for name in installer_asset_names(kind):
                data=git_source(node_output["sourceCommit"],"packaging/"+source_directory+"/"+name)
                member=tarfile.TarInfo("node-assets/"+kind+"/"+name)
                member.size,member.mode=len(data),0o644
                output.addfile(member,io.BytesIO(data))
        if restart_center:
            member=tarfile.TarInfo("bin/store-api")
            member.size,member.mode=api.stat().st_size,0o755
            with api.open("rb") as stream:output.addfile(member,stream)
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
import os,sys,tarfile,io,shutil,hashlib,json
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
assets=os.path.join(p,'releases/installer-assets')
if os.path.exists(assets): shutil.rmtree(assets)
os.replace(os.path.join(p,'node-assets'),assets)
if not os.path.isfile(os.path.join(p,'bin/store-api')):
 shutil.copy2(os.path.join(old,'bin/store-api'),os.path.join(p,'bin/store-api'))
catalog=""" + repr({"store":release_versions["store"],"screenshot":release_versions["screenshot"]}) + """
with open(os.path.join(p,'releases/connector-versions.json'),'w') as f: json.dump(catalog,f)
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
if """ + repr(restart_center) + """ :
 subprocess.check_call(['systemctl','restart','testflying-store-api'])
""")

    changed = success = False
    report_path = PROJECT / "docs/setup-deployment-verification.json"
    try:
        status, body = request("/v1/connectors", snapshot["admin"])
        check(status == 200, "node connection inventory readable before rollout")
        previously_connected = {node["connectorId"] for node in json.loads(body)["connectors"] if node["connected"]}
        routing_baseline = []
        routing_excluded = []
        expected_legacy_android = []
        if routing_before is not None:
            node_states = {row[0]: row[1:] for row in routing_before["connectors"]}
            account_rows = {row[0]: row for row in routing_before["accounts"]}
            states = {row[0]: row[2] for row in routing_before["accountStates"]}
            switches = {(p, app): off for p, app, off in routing_before["applicationSwitches"]}
            candidates = {}
            if ["android_legacy_bindings_v1"] not in routing_before["migrationMarkers"]:
                existing_apps = {tuple(row) for row in routing_before["accountApps"]}
                for account_id, identifier in routing_before["legacyAndroidCandidates"]:
                    valid = (1 <= len(identifier) <= 180 and not identifier.startswith('.')
                             and not identifier.endswith('.') and '..' not in identifier
                             and all(c.isascii() and (c.isalnum() or c in '._-') for c in identifier))
                    if valid and (account_id, identifier) not in existing_apps:
                        expected_legacy_android.append([account_id, identifier])
            for account_id, identifier in routing_before["accountApps"] + expected_legacy_android:
                account = account_rows[account_id]
                candidates.setdefault((account[2], identifier), []).append(account)
            keys = set(candidates) | {(p, app) for p, app, _ in routing_before["legacyStoreBindings"]}
            for platform, identifier in sorted(keys):
                owners = candidates.get((platform, identifier), [])
                account = owners[0] if len(owners) == 1 else None
                error = ("app_not_found" if not owners else "account_ambiguous" if len(owners) > 1 else
                         "app_forwarding_disabled" if switches.get((platform, identifier), False) else
                         "account_disabled" if not account[5] else
                         "identity_changed" if states[account[0]] == "identity_changed" else None)
                status = 503 if error in ("app_forwarding_disabled", "account_disabled") else 409 if error in ("account_ambiguous", "identity_changed") else 404 if error else 200
                store_id, screenshot_id = (account[3], account[4]) if account else (None, None)
                store_error = error or ("connector_disabled" if not node_states.get(store_id, [None, False])[1] else None)
                shot_error = error or (("legacy_screenshot_conflict" if account and account[6] else "screenshot_app_not_found") if not screenshot_id else
                                      "screenshot_connector_disabled" if not node_states.get(screenshot_id, [None, False])[1] else None)
                routing_baseline.append({"platform": platform, "bundleIdentifier": identifier,
                    "appPath": "/v1/apps/" + urllib.parse.quote(platform, safe="") + "/" + urllib.parse.quote(identifier, safe=""),
                    "accountId": account[0] if account else None, "ownershipStatus": status, "ownershipError": error,
                    "storeConnectorId": store_id, "storeError": store_error,
                    "screenshotConnectorId": screenshot_id, "screenshotError": shot_error})
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
            status, body = request("/v1/screenshot-settings", snapshot["admin"])
            screenshot_settings = json.loads(body)
            execution = screenshot_settings.get("execution", {})
            check(status == 200 and set(execution) == {"preparationMinutes", "perImageMinutes"}
                  and type(execution["preparationMinutes"]) is int and 0 <= execution["preparationMinutes"] <= 1440
                  and type(execution["perImageMinutes"]) is int and 1 <= execution["perImageMinutes"] <= 1440,
                  "administrator can read bounded dynamic screenshot execution budget")
            status, body = request("/connector-setup/api/screenshot-settings", snapshot["admin"])
            check(status == 200 and json.loads(body) == screenshot_settings,
                  "screenshot budget mirrors HTTPS developer session route")
            check(request("/v1/screenshot-settings", snapshot["token"])[0] == 401,
                  "business credentials cannot read screenshot execution and destination settings")
            status, body = request("/v1/logs?limit=20", snapshot["admin"])
            logs = json.loads(body)
            check(status == 200 and logs["retentionDays"] == 7 and logs["storageHealthy"],
                  "seven-day persistent center log store available")
            status, body = request("/v1/logs?event=service.started&limit=1", snapshot["admin"])
            check(status == 200 and bool(json.loads(body)["entries"]),
                  "center startup and subsequent HTTP requests recorded")
            check(request("/v1/logs", snapshot["token"])[0] == 401,
                  "business credentials cannot read administrator logs")
            for asset in ("logs.mjs", "app.mjs", "accounts.mjs", "versions.mjs", "skills.mjs", "packager.mjs", "index.html", "style.css", "forwarding.mjs", "screenshot-settings.mjs"):
                path = "/connector-setup/" + ("" if asset == "index.html" else asset)
                status, content = request(path)
                check(status == 200 and content == center_source("crates/store-api/src/setup/"+asset),
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
            status, body = request("/v1/connectors", snapshot["admin"])
            version_response=json.loads(body)
            node_versions=version_response.get("connectors",[])
            check(version_response.get("versions")==release_versions and version_response.get("centerVersion")==release_versions["center"],
                  "center and each node kind expose their own published version")
            check(status == 200 and all(node.get("latestConnectorVersion") == release_versions["screenshot" if node.get("kind")=="screenshot" else "store"]
                  and node.get("versionStatus") in ("current", "outdated", "newer", "unknown")
                  and ((node.get("connectorVersion") is None) == (node.get("versionStatus") == "unknown"))
                  for node in node_versions),
                  "authenticated node version fields distinguish unknown old programs from published version")
            # Never query a real Firebase account during rollout. A missing node proves
            # route/auth wiring while failing before any provider command is dispatched.
            for suffix in ("projects", "projects/firebase-probe/android-apps"):
                probe = "/v1/connectors/00000000-0000-0000-0000-000000000000/firebase/" + suffix
                check(request(probe, snapshot["token"])[0] == 401,
                      "business token cannot call Firebase administrator diagnostic " + suffix)
                status, body = request(probe, snapshot["admin"])
                check(status == 404 and json.loads(body).get("error", {}).get("code") == "connector_not_found",
                      "Firebase administrator route rejects an absent node without provider access " + suffix)
            docs = "/v1/store-management/api-docs"
            for asset in ("start.cmd", "install.ps1", "install.sh"):
                status, content = request("/connector-setup/" + asset)
                check(status == 200 and content == git_source(store_output["sourceCommit"],"packaging/connector/"+asset),
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
                check(status == 200 and catalog["contractVersion"] == "2026-11-27",
                      scheme + " business-token documentation available")
                check(catalog["sync"]["googleBatch"]["commitCount"] == 1
                      and catalog["sync"]["googleBatch"]["minimumStoreConnectorVersion"] == "0.2.9"
                      and catalog["sync"]["googleBatch"]["async"] is True,
                      scheme + " multi-language Google sync contract available")
                endpoints = catalog["endpoints"]
                check(len(endpoints) == 23
                      and sum(row["group"] == "store" for row in endpoints) == 11
                      and sum(row["group"] == "node" for row in endpoints) == 2
                      and sum(row["group"] == "screenshot" for row in endpoints) == 6
                      and sum(row["group"] == "account" for row in endpoints) == 4
                      and catalog["endpointCounts"] == {"store": 11, "node": 2, "screenshot": 6, "account": 4, "uploads": 2}
                      and len(catalog["uploadEndpoints"]) == 2,
                      scheme + " eleven store, two node, six screenshot and four account endpoints plus two upload helpers")
                account_contract = {(row["method"], row["path"]) for row in endpoints if row["group"] == "account"}
                check(account_contract == {("GET", "/v1/store-management/accounts"),
                                           ("GET", "/v1/store-management/accounts/{accountId}/apps"),
                                           ("GET", "/v1/apps/{platform}/{bundleIdentifier}/account"),
                                           ("PUT", "/v1/apps/{platform}/{bundleIdentifier}/account")}
                      and catalog["accounts"]["queryCallsProviders"] is False
                      and catalog["accounts"]["lastCatalogAtUnit"] == "unix_seconds",
                      scheme + " exact four account endpoints and cached directory contract")
                check(catalog["imageQuery"]["parameters"][1]["enum"] == ["phone", "tablet", "foldable"]
                      and "foldable" not in catalog["imageQuery"]["android"]["devices"],
                      scheme + " language and device options with platform limits")
                for format_name, relative in (("markdown", "docs/store-api.md"), ("openapi", "docs/openapi.yaml")):
                    status, content = request(docs + "?format=" + format_name, snapshot["token"], scheme=scheme)
                    check(status == 200 and content == center_source(relative),
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
import json,hashlib,os,subprocess
p=os.path.realpath('/opt/testflying-store-api/current')
sha=lambda p:hashlib.sha256(open(p,'rb').read()).hexdigest()
print(json.dumps({'current':p,'apiSha256':sha(p+'/bin/store-api'),
 'servicePid':int(subprocess.check_output(['systemctl','show','testflying-store-api','--property=MainPID','--value']).strip()),
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
            if not restart_center:
                check(after["servicePid"]==snapshot["servicePid"],"node-only publication preserves the running center process")
            expected_connectors = {**snapshot["binaries"],**{target:entry["sha256"] for target,entry in replacements.items()}}
            check(after["binaries"] == expected_connectors
                  and after["screenshotConnectorSha256"] == (worker["sha256"] if worker else snapshot["screenshotConnectorSha256"]),
                  "store and screenshot Connector publication hashes verified")
            account_migration = None
            if routing_before is not None:
                routing_after = routing_snapshot()
                for key in ("legacyStoreBindings", "legacyScreenshotBindings", "forwarding",
                            "connectorIdentitySha256", "integrationsSha256",
                            "integrationGrantsSha256", "screenshotSettingsSha256", "screenshotSkillSha256", "screenshotSkillActive", "applicationSwitches", "accountScreenshotBindings", "accountAppRemovals"):
                    check(routing_after[key] == routing_before[key], "read-only migration preserves " + key)
                check(all(row in routing_after["accounts"] for row in routing_before["accounts"]),
                      "preexisting account IDs, names, node relations and enabled states preserved")
                check(["accounts_v1"] in routing_after["migrationMarkers"],
                      "transactional account migration marker recorded")
                check(["android_manual_apps_v1"] in routing_after["migrationMarkers"],
                      "Android manual directory migration recorded")
                check(["android_legacy_bindings_v1"] in routing_after["migrationMarkers"],
                      "legacy Android directory backfill marker recorded")
                check(["platform_screenshots_v1"] in routing_after["migrationMarkers"],
                      "platform screenshot migration marker recorded")
                check(routing_after["platformScreenshotBindings"] == expected_platform_screenshots(routing_before),
                      "both platform selections retain the previous screenshot choice without changing split settings")
                check({tuple(row) for row in routing_after["accountApps"]}
                      == {tuple(row) for row in routing_before["accountApps"] + expected_legacy_android},
                      "application membership changes only by audited legacy Android additions")
                after_details = {(a, b): (name, store_id) for a, b, name, store_id in routing_after["accountAppDetails"]}
                after_source_map = {(a, b): source for a, b, source in routing_after["accountAppSources"]}
                check(all(after_details.get((a, b)) == (b, None) and after_source_map.get((a, b)) == "retained"
                          for a, b in expected_legacy_android), "restored legacy Android membership keeps exact account and identifier")
                check(all(row in routing_after["accountAppDetails"] for row in routing_before["accountAppDetails"]),
                      "preexisting application names, identifiers and store IDs preserved")
                android_ids = {row[0] for row in routing_before["accounts"] if row[2] == "android"}
                before_sources = {(a, b): source for a, b, source in routing_before["accountAppSources"]}
                after_sources = {(a, b): source for a, b, source in routing_after["accountAppSources"]}
                check(all(after_sources.get((a, b)) == (before_sources.get((a, b), "catalog") if routing_before["accountAppSources"] else "retained")
                          for a, b in routing_before["accountApps"] if a in android_ids),
                      "existing Android apps retained across future discovery")
                status, content = request("/v1/accounts", snapshot["admin"])
                check(status == 200, "administrator account migration inventory readable")
                account_inventory = json.loads(content)
                status, content = request("/v1/screenshot-skill", snapshot["admin"])
                skill_inventory = json.loads(content)
                check(status == 200 and set(skill_inventory) == {"active", "versions"}
                      and isinstance(skill_inventory["versions"], list), "administrator Skill publication inventory readable")
                status, content = request("/connector-setup/api/screenshot-skill", snapshot["admin"])
                check(status == 200 and json.loads(content) == skill_inventory, "Skill inventory mirrors HTTPS developer session route")
                check(request("/v1/screenshot-skill", snapshot["token"])[0] == 401, "business token cannot manage Skill versions")
                account_by_id = {account["id"]: account for account in account_inventory["accounts"]}
                status, content = request("/connector-setup/api/accounts", snapshot["admin"])
                check(status == 200 and json.loads(content)["accounts"] == account_inventory["accounts"],
                      "developer account inventory mirrors administrator relations")
                status, content = request("/v1/store-management/accounts", snapshot["token"])
                public_accounts = json.loads(content).get("accounts", [])
                check(status == 200 and {row["id"] for row in public_accounts} == set(account_by_id)
                      and all(set(row) == {"id", "name", "platform", "enabled", "catalogStatus", "catalogError", "lastCatalogAt"}
                              for row in public_accounts), "public cached account inventory omits node IDs and secrets")
                for account_id in account_by_id:
                    status, content = request("/v1/store-management/accounts/" + urllib.parse.quote(account_id, safe="") + "/apps", snapshot["token"])
                    rows = json.loads(content).get("apps", [])
                    check(status == 200 and all(set(row) == {"platform", "bundleIdentifier", "name", "storeAppId", "accountId", "source", "enabled", "resolutionStatus", "screenshotEnabled", "screenshotResolutionStatus"}
                                                and row["source"] in ("catalog", "manual", "retained")
                                                and isinstance(row["enabled"], bool) and isinstance(row["screenshotEnabled"], bool)
                                                and isinstance(row["screenshotResolutionStatus"], str) for row in rows),
                          "cached account app schema includes separate screenshot resolution " + account_id)
                verified_routes = []
                for route in routing_baseline:
                    status, content = request(route["appPath"] + "/account", snapshot["token"])
                    resolved_body = json.loads(content)
                    check(status == route["ownershipStatus"] and
                          (resolved_body.get("error", {}).get("code") == route["ownershipError"] if route["ownershipError"]
                           else resolved_body.get("account", {}).get("id") == route["accountId"]),
                          "directory-only ownership and application switch " + route["appPath"])
                    for suffix, error_key, id_key in [("/connector", "storeError", "storeConnectorId"),
                                                      ("/screenshot-connector", "screenshotError", "screenshotConnectorId")]:
                        status, content = request(route["appPath"] + suffix, snapshot["admin"])
                        body = json.loads(content)
                        error = route[error_key]
                        expected_status = (503 if error in ("app_forwarding_disabled", "account_disabled", "connector_disabled", "screenshot_connector_disabled")
                                           else 409 if error in ("account_ambiguous", "identity_changed", "legacy_screenshot_conflict")
                                           else 404 if error else 200)
                        actual_id = body.get("connectorId") if suffix == "/connector" else body.get("connector", {}).get("connectorId")
                        check(status == expected_status and
                              (body.get("error", {}).get("code") == error if error else actual_id == route[id_key]),
                              "exact catalog route or expected blocked result " + route["appPath"] + suffix)
                    verified_routes.append({key: value for key, value in route.items() if key != "appPath"})
                account_migration = {
                    "verificationMode": "read-only cached GET and SQLite query-only transaction",
                    "providerCalls": 0, "refreshRequests": 0, "accountsCreatedByChecks": 0, "bindingsWrittenByChecks": 0,
                    "legacyStoreBindings": routing_before["legacyStoreBindings"],
                    "legacyScreenshotBindings": routing_before["legacyScreenshotBindings"],
                    "preservedHashes": {key: routing_after[key] for key in ("legacyStoreSha256", "legacyScreenshotSha256", "connectorIdentitySha256", "integrationsSha256", "integrationGrantsSha256", "screenshotSettingsSha256")},
                    "forwarding": routing_after["forwarding"], "accountsBefore": len(routing_before["accounts"]),
                    "accountsAfter": len(routing_after["accounts"]), "migrationMarkers": routing_after["migrationMarkers"],
                    "legacyAndroidRestored": [{"accountId": a, "bundleIdentifier": b} for a, b in expected_legacy_android],
                    "migrationIssues": account_inventory["migrationIssues"],
                    "accountMigrationErrors": [{"accountId": account["id"], "code": account["migrationError"]}
                                               for account in account_by_id.values() if account["migrationError"]],
                    "platformScreenshotBindings": routing_after["platformScreenshotBindings"],
                    "verifiedOriginalRoutes": verified_routes, "unavailableOrInconsistentBeforeRollout": routing_excluded,
                }
            previous_report = json.loads(report_path.read_text())
            report = {"runId": root.name, "stage": "deployed-and-verified", "production": True,
                      "finishedAt": datetime.datetime.now(datetime.timezone.utc).isoformat(),
                      "deploymentCommit": commit, "binarySourceCommit": center_output["sourceCommit"],
                      "apiSha256": expected_api, "connectorBinarySha256": expected_connectors,
                      "connectorVersion": release_version, "connectorSourceFingerprint": release_fingerprint,
                      "centerVersion":release_versions["center"],"screenshotConnectorVersion":release_versions["screenshot"],
                      "componentVersions":release_versions,"publishedOutputs":published,
                      "changedComponents":sorted(changed_components),"centerRestarted":restart_center,
                      "firebaseDiagnostics": {"minimumConnectorVersion": "0.2.1", "publishedConnectorVersion": release_version,
                                              "readOnly": True, "actualFirebaseAccountsQueried": False,
                                              "automaticMembershipChanges": False, "publicCoreEndpointCountUnchanged": True},
                      "screenshotConnectorSha256": worker["sha256"] if worker else snapshot["screenshotConnectorSha256"],
                      "screenshotUploadConfigured": snapshot["screenshotUploadConfigured"],
                      "screenshotExecution": {**execution, "timingProtocol": 1, "deliveryGraceSeconds": 360,
                                              "productionScreenshotTasksCreated": 0},
                      "checks": checks, "testNodesCreated": 0, "appBindingsWritten": 0,
                      "actualStoreAccountsUsed": False,
                      "apiDocumentation": {"contractVersion": "2026-11-27", "coreEndpoints": 23, "uploadHelpers": 2, "formats": ["json", "markdown", "openapi"],
                                           "existingIntegrationTokenVerified": bool(existing_token)},
                      "cleanup": "See the supervisor cleanup report for this runId"}
            if account_migration is not None:
                report["accountMigration"] = account_migration
            if "storeConnectorPublication" in previous_report:
                report["storeConnectorPublication"] = previous_report["storeConnectorPublication"]
            for key in ("screenshotSkillPublication", "screenshotSkillLiveTest", "accountScreenshotBinding"):
                if key in previous_report:
                    report[key] = previous_report[key]
            if replacements:
                report["storeConnectorPublication"] = {"runId": root.name, "sourceCommit": commit,
                    "downloadVerified": expected_connectors, "serviceConfigPreserved": True,
                    "screenshotBinaryPreserved": worker is None, "configurationMergeSupported": True,
                    "actualStoreAccountsUsed": False}
            report_path.write_text(json.dumps(report, indent=2) + "\n")
            success = True
            return

    finally:
        if changed and not success:
            select(previous,release)



if __name__ == "__main__":
    main()
