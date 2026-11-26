"""Merge maintained Connector guidance into the active Skill without replacing its helpers."""
from __future__ import annotations

import base64
import datetime
import hashlib
import io
import json
import os
from pathlib import Path
import shlex
import subprocess
import sys
import zipfile

from verify_store_refactor import PROJECT, managed_root, source_hashes

BASE_VERSION = "938bf9c1d27c69271ae43cb5efb040d4711453ad7040c2160c14f4e08f92a245"
PATCHES = ("SKILL.md", "references/connector-protocol.md")

REMOTE_PREFIX = """
import base64,hashlib,http.client,json,os,sqlite3,sys,urllib.parse
env=dict(l.strip().split('=',1) for l in open('/etc/testflying-store-api/service.env') if '=' in l and not l.startswith('#'))
db=sqlite3.connect('file:'+urllib.parse.quote(env.get('STORE_API_DATABASE','/var/lib/testflying-store-api/registry.sqlite3'),safe='/')+'?mode=ro',uri=True)
db.execute('PRAGMA query_only=ON')
def active():
 m,b=db.execute('SELECT v.manifest,v.bundle FROM screenshot_skill_versions v JOIN screenshot_skill_current c ON c.version=v.version WHERE c.id=1').fetchone()
 return {'manifest':json.loads(m),'bundle':json.loads(b)}
"""

def remote(code: str, data: bytes | None = None) -> dict:
    p = subprocess.run(["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=8",
                        "-o", "StrictHostKeyChecking=yes", "-o", "LogLevel=ERROR",
                        "testflying-prod", "python3 -c " + shlex.quote(REMOTE_PREFIX + code)],
                       input=data, capture_output=True, timeout=90)
    if p.returncode:
        raise RuntimeError("Skill operation failed; active version must be checked before retry")
    return json.loads(p.stdout)

def canonical(bundle: dict) -> bytes:
    ordered = {"schemaVersion":bundle["schemaVersion"], "name":bundle["name"],
               "files":[{"path":f["path"],"data":f["data"],"executable":f["executable"]}
                        for f in sorted(bundle["files"], key=lambda f:f["path"])]}
    return json.dumps(ordered, separators=(",", ":"), ensure_ascii=False).encode()

def main() -> None:
    root = managed_root()
    if sys.argv[1:] not in (["--check"], ["--publish"]):
        raise RuntimeError("Use --check or --publish")
    baseline = source_hashes()
    source = remote("print(json.dumps(active()))")
    assert source["manifest"]["version"] == BASE_VERSION, "Active Skill changed; review the new base"
    bundle = source["bundle"]
    assert hashlib.sha256(canonical(bundle)).hexdigest() == BASE_VERSION
    original = {f["path"]:dict(f) for f in bundle["files"]}
    files = dict(original)
    for path in PATCHES:
        files[path] = {"path":path, "data":base64.b64encode((PROJECT / "packaging/screenshot-skill" / path).read_bytes()).decode(),
                       "executable":original.get(path,{}).get("executable",False)}
    bundle["files"] = sorted(files.values(),key=lambda f:f["path"])
    expected = hashlib.sha256(canonical(bundle)).hexdigest()
    temporary = root / "tmp" / "mobile-localized-screenshots"
    temporary.mkdir(parents=True)
    archive = io.BytesIO()
    with zipfile.ZipFile(archive,"w",zipfile.ZIP_DEFLATED,allowZip64=False) as out:
        for f in bundle["files"]:
            path = f["path"]
            assert not Path(path).is_absolute() and ".." not in Path(path).parts
            data = base64.b64decode(f["data"],validate=True)
            target = temporary / path
            target.parent.mkdir(parents=True,exist_ok=True)
            target.write_bytes(data)
            target.chmod(0o700 if f["executable"] else 0o600)
            info = zipfile.ZipInfo(path,(2026,11,25,0,0,0))
            info.create_system = 3
            info.external_attr = (0o100755 if f["executable"] else 0o100644) << 16
            info.compress_type = zipfile.ZIP_DEFLATED
            out.writestr(info,data)
    for path,f in original.items():
        if path not in PATCHES:
            assert files[path] == f
    subprocess.run([sys.executable,"-B",os.path.expanduser("~/.codex/skills/.system/skill-creator/scripts/quick_validate.py"),str(temporary)],check=True)
    # Only inspect reference links; never execute scripts supplied by the active bundle.
    import re
    for path in PATCHES:
        for link in re.findall(r"\]\(([^)]+)\)",(temporary/path).read_text()):
            assert (temporary/path).parent.joinpath(link).is_file(), "Missing Skill reference"
    assert source_hashes() == baseline
    if sys.argv[1:] == ["--check"]:
        print(json.dumps({"baseVersion":BASE_VERSION,"expectedVersion":expected,"unchangedFiles":len(set(original)-set(PATCHES)),"validated":True}))
        return
    verification = json.loads((PROJECT/"docs/setup-deployment-verification.json").read_text())
    assert verification["stage"] == "deployed-and-verified"
    assert json.loads((PROJECT/"docs/verification.json").read_text())["sourceHashes"] == baseline
    response = remote("""
request=json.load(sys.stdin)
assert active()['manifest']['version']==request['baseVersion'], 'Active Skill changed'
listen=env.get('STORE_API_LISTEN','127.0.0.1:8200')
host,port=listen.rsplit(':',1)
assert host in ('127.0.0.1','localhost')
connection=http.client.HTTPConnection(host,int(port),timeout=45)
headers={'Authorization':'Bearer '+env['STORE_API_ADMIN_TOKEN'],'X-Forwarded-Proto':'https','Content-Type':'application/zip'}
connection.request('POST','/v1/screenshot-skill',base64.b64decode(request['zip']),headers)
reply=connection.getresponse()
body=reply.read(1024*1024)
assert reply.status==201, 'Skill publication rejected'
after=active()
assert after['manifest']['version']==request['expectedVersion'], 'Published Skill hash differs'
assert after['bundle']==request['bundle'], 'Published Skill content differs'
assert db.execute('SELECT count(*) FROM screenshot_skill_versions WHERE version=?',(request['baseVersion'],)).fetchone()[0]==1
print(json.dumps({'active':after['manifest'],'previousVersionRetained':True}))
""", json.dumps({"baseVersion":BASE_VERSION,"expectedVersion":expected,"bundle":bundle,
                 "zip":base64.b64encode(archive.getvalue()).decode()}).encode())
    assert source_hashes() == baseline
    verification["screenshotSkillPublication"] = {
        "runId":root.name,"finishedAt":datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "baseVersion":BASE_VERSION,"active":response["active"],
        "previousVersionRetained":response["previousVersionRetained"],
        "unchangedFiles":len(set(original)-set(PATCHES)),"updatedFiles":list(PATCHES),
        "realScreenshotTasksCreated":False,"externalCallbacksSent":False,
    }
    (PROJECT/"docs/setup-deployment-verification.json").write_text(json.dumps(verification,ensure_ascii=False,indent=2)+"\n")
    print("Skill published and exact content verified: "+expected,flush=True)

if __name__ == "__main__":
    main()
