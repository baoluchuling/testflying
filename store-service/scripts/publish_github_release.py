"""Publish credential-free, verified Connector packages to a GitHub Release."""
from __future__ import annotations

import datetime
import gzip
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import tarfile
import time
import tomllib
import zipfile

from verify_store_refactor import PROJECT, managed_root, source_hashes

REPO = "baoluchuling/testflying"
VERSION = "0.1.0"
TAG = "store-connector-v" + VERSION
_release_id: int | None = None
TARGETS = {
    "linux-amd64": ("store-connector", "docs/deployment-build.json", ".tar.gz"),
    "darwin-arm64": ("store-connector", "docs/deployment-build.json", ".tar.gz"),
    "windows-amd64": ("store-connector.exe", "docs/windows-release-build.json", ".zip"),
}


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def gh(*args: str, read: bool = False, missing_ok: bool = False) -> subprocess.CompletedProcess:
    for attempt in range(3 if read else 1):
        result = subprocess.run(["gh", *args], cwd=PROJECT, text=True,
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        if result.returncode == 0:
            return result
        if missing_ok and "HTTP 404" in result.stderr:
            return result
        if not read or attempt == 2:
            # Do not publish CLI environment, authorization, or arbitrary response bodies.
            kind = "TLS/EOF" if "EOF" in result.stderr or "TLS" in result.stderr else "request"
            raise RuntimeError("GitHub " + kind + " failed: " + " ".join(args[:2]))
        time.sleep(0.5)
    raise AssertionError("unreachable")


def api(path: str, missing_ok: bool = False) -> dict | None:
    result = gh("api", "repos/" + REPO + "/" + path, read=True, missing_ok=missing_ok)
    return json.loads(result.stdout) if result.returncode == 0 else None


def release_for_tag(missing_ok: bool = False) -> dict | None:
    global _release_id
    if _release_id is not None:
        result = api("releases/" + str(_release_id))
        if result["tag_name"] != TAG:
            raise RuntimeError("Release identity changed")
        return result
    result = api("releases/tags/" + TAG, missing_ok=True)
    if result is None:
        # GitHub's tag endpoint does not return unpublished drafts.
        matches = [item for item in api("releases?per_page=100") if item["tag_name"] == TAG]
        if len(matches) > 1:
            raise RuntimeError("Ambiguous draft release identity")
        result = matches[0] if matches else None
    if result is not None:
        _release_id = result["id"]
    elif not missing_ok:
        raise RuntimeError("Expected release was not found")
    return result


def source_file(relative: str) -> bytes:
    path = PROJECT / relative
    if path.resolve(strict=True) != path or not path.is_file():
        raise RuntimeError("Expected a canonical regular source file: " + relative)
    return path.read_bytes()


def write_formal(path: Path, data: bytes) -> None:
    if path.exists():
        if path.is_symlink() or not path.is_file() or path.read_bytes() != data:
            raise RuntimeError("Preserve the existing formal file and review: " + path.name)
        return
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o644)
    with os.fdopen(fd, "wb") as output:
        output.write(data)
        output.flush()
        os.fsync(output.fileno())


def archive(files: dict[str, tuple[bytes, int]], suffix: str) -> bytes:
    output = io.BytesIO()
    if suffix == ".zip":
        with zipfile.ZipFile(output, "w", zipfile.ZIP_DEFLATED) as bundle:
            for name, (data, mode) in sorted(files.items()):
                entry = zipfile.ZipInfo(name, (1980, 1, 1, 0, 0, 0))
                entry.create_system = 3
                entry.external_attr = (0o100000 | mode) << 16
                entry.compress_type = zipfile.ZIP_DEFLATED
                bundle.writestr(entry, data)
    else:
        with gzip.GzipFile(fileobj=output, mode="wb", filename="", mtime=0) as compressed:
            with tarfile.open(fileobj=compressed, mode="w") as bundle:
                for name, (data, mode) in sorted(files.items()):
                    entry = tarfile.TarInfo(name)
                    entry.size, entry.mode = len(data), mode
                    entry.uid = entry.gid = entry.mtime = 0
                    entry.uname = entry.gname = "root"
                    bundle.addfile(entry, io.BytesIO(data))
    return output.getvalue()


def unpack(data: bytes, suffix: str) -> dict[str, bytes]:
    if suffix == ".zip":
        with zipfile.ZipFile(io.BytesIO(data)) as bundle:
            names = bundle.namelist()
            if len(set(names)) != len(names):
                raise RuntimeError("Duplicate archive members")
            return {name: bundle.read(name) for name in names}
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as bundle:
        members = bundle.getmembers()
        if any(not member.isfile() or "/" in member.name for member in members):
            raise RuntimeError("Unexpected archive member")
        if len({member.name for member in members}) != len(members):
            raise RuntimeError("Duplicate archive members")
        return {member.name: bundle.extractfile(member).read() for member in members}


def main() -> None:
    root = managed_root()
    tmp = root / "tmp"
    tmp.mkdir(exist_ok=True)
    if tomllib.loads(source_file("Cargo.toml").decode())["workspace"]["package"]["version"] != VERSION:
        raise RuntimeError("Review the release version and formal declarations")
    subprocess.run(["git", "diff", "--exit-code", "HEAD", "--", "."], cwd=PROJECT,
                   check=True, stdout=subprocess.DEVNULL)
    commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=PROJECT, text=True).strip()
    branch = subprocess.check_output(["git", "branch", "--show-current"], cwd=PROJECT, text=True).strip()
    remote_commit = api("git/ref/heads/" + branch)["object"]["sha"]
    if remote_commit != commit:
        raise RuntimeError("Push this release's source commit before publication")
    baseline = source_hashes()
    verified = json.loads(source_file("docs/verification.json"))
    if verified["sourceHashes"] != baseline or verified["testsPassed"] != 42:
        raise RuntimeError("Rust sources do not match the completed verification")
    reports = {path: json.loads(source_file(path)) for _, path, _ in TARGETS.values()}
    if any(report["sourceHashes"] != baseline for report in reports.values()):
        raise RuntimeError("Build and current source hashes differ")
    latest_before = api("releases/latest")["id"]
    release = release_for_tag(missing_ok=True)
    if release is not None and release["target_commitish"] != commit:
        raise RuntimeError("Existing release belongs to another commit; nothing was overwritten")
    existing_ref = api("git/ref/tags/" + TAG, missing_ok=True)
    if existing_ref is not None and (existing_ref["object"]["type"] != "commit"
                                     or existing_ref["object"]["sha"] != commit):
        raise RuntimeError("Existing tag belongs to another source; nothing was overwritten")

    shared: dict[str, tuple[bytes, int]] = {
        "README.md": (source_file("packaging/connector/README.md"), 0o644),
        "connector.example.json": (source_file("examples/connector.json"), 0o644),
        "connector-google.example.json": (source_file("examples/connector-google.json"), 0o644),
        "testflying-prod-ca.crt": (source_file("deploy/testflying-prod-ca.crt"), 0o644),
    }
    expected_secrets = {
        "connectorId": "use-the-generated-connector-id",
        "token": "use-the-generated-node-token",
        "messageKey": "use-the-generated-message-key",
        "uploadKey": "use-the-generated-upload-key",
    }
    for name in ("connector.example.json", "connector-google.example.json"):
        template = json.loads(shared[name][0])
        if any(template.get(key) != value for key, value in expected_secrets.items()):
            raise RuntimeError("A public template must contain only placeholder node credentials")
        if template["serverUrl"] != "http://127.0.0.1:8200":
            raise RuntimeError("Unexpected public template server URL")
        provider = ({"mode": "live", "googleServiceAccountPath": "/secure/account/service-account.json"}
                    if name == "connector-google.example.json" else
                    {"mode": "live", "appleIssuerId": "your-issuer-id", "appleKeyId": "your-key-id",
                     "applePrivateKeyPath": "/secure/account/AuthKey.p8"})
        if template["provider"] != provider:
            raise RuntimeError("Public provider examples must contain only approved placeholders")
    if b"PRIVATE KEY" in shared["testflying-prod-ca.crt"][0]:
        raise RuntimeError("Only the public certificate can be packaged")
    subprocess.run(["bash", "-n", str(PROJECT / "packaging/connector/start.sh")], check=True)

    final = PROJECT / "releases/github"
    final.mkdir(exist_ok=True)
    if final.resolve(strict=True) != final:
        raise RuntimeError("Formal package directory cannot contain symlinks")
    assets: dict[str, dict] = {}
    inventories = {}
    for target, (executable, report_name, suffix) in TARGETS.items():
        relative = "releases/" + target + "/" + executable
        program = source_file(relative)
        expected = reports[report_name]["outputs"][relative]
        if digest(program) != expected["sha256"] or len(program) != expected["bytes"]:
            raise RuntimeError("Executable differs from verified build: " + target)
        files = {**shared, executable: (program, 0o755)}
        launcher = "start.cmd" if target == "windows-amd64" else "start.sh"
        launcher_data = source_file("packaging/connector/" + launcher)
        if launcher == "start.cmd":
            launcher_data = launcher_data.replace(b"\r\n", b"\n").replace(b"\n", b"\r\n")
        files[launcher] = (launcher_data, 0o755 if launcher == "start.sh" else 0o644)
        data = archive(files, suffix)
        if unpack(data, suffix) != {name: content for name, (content, _) in files.items()}:
            raise RuntimeError("Archive whitelist/content validation failed")
        name = TAG + "-" + target + suffix
        write_formal(final / name, data)
        assets[name] = {"sha256": digest(data), "bytes": len(data), "binarySha256": digest(program)}
        inventories[name] = sorted(files)
        print("PASS " + target + " binary, archive whitelist and placeholder configuration", flush=True)
    sums = "".join(value["sha256"] + "  " + name + "\n" for name, value in sorted(assets.items())).encode()
    sums_name = TAG + "-SHA256SUMS.txt"
    write_formal(final / sums_name, sums)
    assets[sums_name] = {"sha256": digest(sums), "bytes": len(sums)}
    if source_hashes() != baseline:
        raise RuntimeError("Source changed during packaging")
    notes = PROJECT / "docs/releases" / (TAG + ".md")
    if release is None:
        try:
            gh("release", "create", TAG, "--repo", REPO, "--draft", "--latest=false",
               "--target", commit, "--title", "Store Connector v" + VERSION + " (Rust)",
               "--notes-file", str(notes))
        except RuntimeError:
            # A connection can fail after GitHub accepted creation. Only resume this exact draft.
            release = release_for_tag(missing_ok=True)
            if release is None or release["target_commitish"] != commit or not release["draft"]:
                raise
        release = release_for_tag()
    if release["target_commitish"] != commit:
        raise RuntimeError("Release target changed")
    existing = {item["name"]: item for item in release["assets"]}
    if set(existing) - set(assets):
        raise RuntimeError("Release contains unexpected assets; nothing was removed")
    for name in assets:
        if name in existing:
            if existing[name]["size"] != assets[name]["bytes"]:
                raise RuntimeError("Existing asset size mismatch")
            continue
        if not release["draft"]:
            raise RuntimeError("Published release is missing assets; nothing was overwritten")
        try:
            gh("release", "upload", TAG, str(final / name), "--repo", REPO)
        except RuntimeError:
            current = release_for_tag()
            matches = [item for item in current["assets"] if item["name"] == name]
            if len(matches) != 1 or matches[0]["size"] != assets[name]["bytes"]:
                raise
        print("PASS uploaded " + name, flush=True)

    downloaded = None
    for attempt in range(3):
        directory = tmp / ("download-" + str(attempt))
        directory.mkdir()
        try:
            gh("release", "download", TAG, "--repo", REPO, "--dir", str(directory))
            downloaded = directory
            break
        except RuntimeError:
            if attempt == 2:
                raise
    if {path.name for path in downloaded.iterdir()} != set(assets):
        raise RuntimeError("Downloaded asset inventory differs")
    for name, expected in assets.items():
        data = (downloaded / name).read_bytes()
        if len(data) != expected["bytes"] or digest(data) != expected["sha256"]:
            raise RuntimeError("GitHub round-trip checksum mismatch")
        print("PASS GitHub download SHA-256 " + name, flush=True)

    release = release_for_tag()
    if release["draft"]:
        try:
            gh("release", "edit", TAG, "--repo", REPO, "--draft=false", "--latest=false")
        except RuntimeError:
            if release_for_tag()["draft"]:
                raise
    release = release_for_tag()
    ref = api("git/ref/tags/" + TAG)
    if release["draft"] or release["prerelease"] or ref["object"]["sha"] != commit:
        raise RuntimeError("Published release/tag verification failed")
    if api("releases/latest")["id"] != latest_before:
        raise RuntimeError("Legacy Latest Release changed unexpectedly")
    remote_assets = {item["name"]: item for item in release["assets"]}
    if set(remote_assets) != set(assets):
        raise RuntimeError("Published asset inventory differs")
    for name, expected in assets.items():
        remote = remote_assets[name]
        if remote["size"] != expected["bytes"] or remote["state"] != "uploaded":
            raise RuntimeError("Published asset is incomplete")
        remote_digest = remote.get("digest")
        if remote_digest is not None and remote_digest != "sha256:" + expected["sha256"]:
            raise RuntimeError("GitHub-reported SHA-256 differs")
        expected.update({"id": remote["id"], "url": remote["browser_download_url"],
                         "githubDigest": remote_digest})
    report = {
        "runId": root.name, "finishedAt": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "repository": REPO, "tag": TAG, "releaseId": release["id"], "releaseUrl": release["html_url"],
        "publishedAt": release["published_at"], "releaseCommit": commit,
        "binarySourceCommits": sorted({item["sourceCommit"] for item in reports.values()}),
        "sourceHashes": baseline, "sourceVerificationRunId": verified["runId"],
        "testsPassed": verified["testsPassed"], "assets": assets, "archiveInventory": inventories,
        "downloadVerified": True, "credentialFreePackages": True, "legacyLatestUnchanged": True,
        "notVerified": ["Execution on real Windows hardware", "Apple signing/notarization"],
        "cleanup": "See the supervisor cleanup report for this runId; public assets are formal deliveries.",
    }
    (PROJECT / "docs/github-release-verification.json").write_text(json.dumps(report, indent=2) + "\n")
    print("Published and verified: " + release["html_url"], flush=True)


if __name__ == "__main__":
    main()

