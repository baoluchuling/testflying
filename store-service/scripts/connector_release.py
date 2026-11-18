"""Fail closed before building a changed node without a higher release version.

Only local source and release reports are read. No node, provider or build command runs here.
"""
from __future__ import annotations

import hashlib
import json
from pathlib import Path
import re
import subprocess
import tomllib

from managed_workspace import PROJECT

MANIFEST = "crates/store-connector/Cargo.toml"
RELEASE = "crates/store-protocol/src/release.rs"
API_ITEMS = {
    "crates/store-api/src/lib.rs": ("config", "installer"),
    "crates/store-api/src/screenshots.rs": ("screenshot_installer", "installer_archive"),
}
SEMVER = re.compile(r"(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)", re.ASCII)
RAW_STRING = re.compile(r'r(#{0,255})"')
CHARACTER = re.compile(r"'(?:\\(?:u\{[0-9a-fA-F_]+\}|x[0-9a-fA-F]{2}|.)|[^'\\\n])'")


def version_tuple(value: object) -> tuple[int, int, int]:
    if not isinstance(value, str) or len(value) > 32 or SEMVER.fullmatch(value) is None:
        raise RuntimeError("Connector version must be canonical major.minor.patch (at most 32 bytes)")
    parts = tuple(map(int, value.split(".")))
    if any(part > 4_294_967_295 for part in parts):
        raise RuntimeError("Connector version components must fit unsigned 32-bit integers")
    return parts


def _package_version(read) -> str:
    package = tomllib.loads(read(MANIFEST).decode())["package"]
    value = package["version"]
    if value == {"workspace": True}:
        value = tomllib.loads(read("Cargo.toml").decode())["workspace"]["package"]["version"]
    version_tuple(value)
    return value


def connector_version(project: Path = PROJECT) -> str:
    version = _package_version(lambda path: (project / path).read_bytes())
    constants = re.findall(r'pub\s+const\s+CONNECTOR_VERSION\s*:\s*&str\s*=\s*"([^"]+)"\s*;',
                           (project / RELEASE).read_text())
    if constants != [version]:
        raise RuntimeError("Connector Cargo and protocol CONNECTOR_VERSION must agree")
    lock = tomllib.loads((project / "Cargo.lock").read_text())
    locked = [item["version"] for item in lock["package"] if item["name"] == "store-connector"]
    if locked != [version]:
        raise RuntimeError("Connector Cargo.lock version must agree with Cargo and protocol")
    return version


def _rust_mask(source: str) -> str:
    """Keep Rust punctuation offsets while hiding strings and nested comments."""
    masked = list(source)
    index = 0
    while index < len(source):
        end = index
        if source.startswith("//", index):
            end = source.find("\n", index)
            end = len(source) if end < 0 else end
        elif source.startswith("/*", index):
            depth, end = 1, index + 2
            while depth and end < len(source):
                if source.startswith("/*", end):
                    depth, end = depth + 1, end + 2
                elif source.startswith("*/", end):
                    depth, end = depth - 1, end + 2
                else:
                    end += 1
            if depth:
                raise RuntimeError("Unterminated Rust comment in node input")
        else:
            raw = RAW_STRING.match(source, index)
            if raw:
                closing = '"' + raw[1]
                close = source.find(closing, raw.end())
                if close < 0:
                    raise RuntimeError("Unterminated Rust raw string in node input")
                end = close + len(closing)
            elif source[index] == '"':
                end = index + 1
                while end < len(source) and source[end] != '"':
                    end += 2 if source[end] == "\\" else 1
                end += 1
            elif source[index] == "'":
                char = CHARACTER.match(source, index)
                if char:
                    end = index + len(char[0])
        if end > index:
            masked[index:end] = " " * (end - index)
            index = end
        else:
            index += 1
    return "".join(masked)


def _item_end(mask: str, start: int) -> int:
    opening = re.search(r"[;{]", mask[start:])
    if opening is None:
        raise RuntimeError("Cannot identify Rust node input item")
    index = start + opening.start()
    if mask[index] == ";":
        return index + 1
    depth = 1
    index += 1
    while index < len(mask) and depth:
        depth += (mask[index] == "{") - (mask[index] == "}")
        index += 1
    if depth:
        raise RuntimeError("Unbalanced Rust node input item")
    return index


def _production_rust(data: bytes) -> bytes:
    source = data.decode()
    mask = _rust_mask(source)
    # cfg(test) items may be followed by more production items; never truncate the file.
    ranges = []
    for match in re.finditer(r"#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]", mask):
        end = _item_end(mask, match.end())
        if not ranges or match.start() >= ranges[-1][1]:
            ranges.append((match.start(), end))
    for start, end in reversed(ranges):
        source = source[:start] + source[end:]
    return source.strip().encode()


def _input(path: str) -> bool:
    if path in {"Cargo.toml", "Cargo.lock", "deploy/testflying-prod-ca.crt",
                "examples/connector.json", "examples/connector-google.json",
                "crates/store-api/src/setup/packager.mjs"} or path in API_ITEMS:
        return True
    parts = Path(path).parts
    if path.endswith("/Cargo.toml") and path.startswith("crates/"):
        return True
    if path.startswith(("packaging/connector/", "packaging/screenshot-connector/")):
        return True
    return (len(parts) >= 3 and parts[0] == "crates"
            and parts[1] in {"store-protocol", "store-provider", "store-connector"}
            and (parts[2] in {"src", "examples"} or parts[2:] == ("build.rs",))
            and path.endswith(".rs"))


def _canonical(path: str, data: bytes) -> bytes:
    if path in API_ITEMS:
        source = data.decode()
        mask = _rust_mask(source)
        items = []
        for name in API_ITEMS[path]:
            matches = list(re.finditer(r"\bfn\s+" + name + r"\s*\(", mask))
            if len(matches) != 1:
                raise RuntimeError("Cannot identify node packaging function: " + name)
            start = matches[0].start()
            items.append(source[start:_item_end(mask, start)])
        return "\n".join(items).encode()
    return _production_rust(data) if path.endswith(".rs") else data


def _fingerprint(hashes: dict[str, str]) -> str:
    return hashlib.sha256(json.dumps(hashes, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def connector_source_hashes(project: Path = PROJECT) -> dict[str, str]:
    candidates = [project / path for path in (
        "Cargo.toml", "Cargo.lock", "deploy/testflying-prod-ca.crt",
        "examples/connector.json", "examples/connector-google.json",
        "crates/store-api/src/setup/packager.mjs", *API_ITEMS)]
    candidates.extend((project / "crates").glob("*/Cargo.toml"))
    candidates.extend(project / "crates" / name / "build.rs" for name in
                      ("store-protocol", "store-provider", "store-connector"))
    for relative in ("packaging", "crates/store-protocol/src", "crates/store-provider/src",
                     "crates/store-connector/src", "crates/store-connector/examples"):
        candidates.extend((project / relative).rglob("*"))
    paths = sorted({path for path in candidates
                    if path.is_file() and _input(path.relative_to(project).as_posix())})
    return {path.relative_to(project).as_posix(): hashlib.sha256(
        _canonical(path.relative_to(project).as_posix(), path.read_bytes())).hexdigest() for path in paths}


def connector_source_fingerprint(project: Path = PROJECT) -> str:
    return _fingerprint(connector_source_hashes(project))


def _git_source(project: Path, commit: str, relative: str) -> bytes:
    if not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise RuntimeError("Historical node release requires an exact sourceCommit")
    prefix = subprocess.check_output(["git", "rev-parse", "--show-prefix"], cwd=project, text=True).strip()
    try:
        return subprocess.check_output(["git", "show", commit + ":" + prefix + relative],
                                       cwd=project, stderr=subprocess.DEVNULL)
    except subprocess.CalledProcessError as error:
        raise RuntimeError("Cannot recover historical node release source: " + relative) from error


def _previous(report: dict, project: Path) -> tuple[str, str]:
    version = report.get("connectorVersion")
    if version is not None:
        version_tuple(version)
        hashes = report.get("connectorSourceHashes")
        if not isinstance(hashes, dict) or not hashes or any(
            not isinstance(path, str) or not _input(path) or not isinstance(value, str)
            or re.fullmatch(r"[0-9a-f]{64}", value) is None for path, value in hashes.items()
        ) or report.get("connectorSourceFingerprint") != _fingerprint(hashes):
            raise RuntimeError("Invalid previous node release fingerprint")
        return version, report["connectorSourceFingerprint"]

    recorded = report.get("sourceHashes", {})
    commit = report.get("sourceCommit", "")

    def historical(path: str) -> bytes:
        data = _git_source(project, commit, path)
        if hashlib.sha256(data).hexdigest() != recorded.get(path):
            raise RuntimeError("Historical node manifest differs from its sourceCommit: " + path)
        return data

    version = _package_version(historical)
    hashes = {}
    for path, digest in recorded.items():
        if _input(path):
            try:
                data = _git_source(project, commit, path)
            except RuntimeError:
                # A file first added in an uncommitted historical build has no blob.
                # Preserve the recorded input as unknown, so a version bump is required.
                hashes[path] = "legacy-raw:" + digest
                continue
            # Old builds could precede their source commit. Unknown production bytes must
            # count as changed, rather than trusting the current checkout as the old node.
            hashes[path] = (hashlib.sha256(_canonical(path, data)).hexdigest()
                            if hashlib.sha256(data).hexdigest() == digest else "legacy-raw:" + digest)
    if not hashes:
        raise RuntimeError("Historical node release has no recorded source inputs")
    return version, _fingerprint(hashes)


def guard_connector_release(report_paths: list[Path], *, api_only: bool = False,
                            project: Path = PROJECT) -> dict:
    version = connector_version(project)
    hashes = connector_source_hashes(project)
    fingerprint = _fingerprint(hashes)
    if not report_paths:
        raise RuntimeError("Previous platform node release report is required")
    for path in report_paths:
        previous_version, previous_fingerprint = _previous(json.loads(path.read_text()), project)
        if version_tuple(version) < version_tuple(previous_version):
            raise RuntimeError("Connector version downgrade rejected: " + str(path))
        changed = fingerprint != previous_fingerprint
        if api_only and (changed or version != previous_version):
            raise RuntimeError("Node inputs/version changed; rebuild all Connector platforms before an API-only release")
        if changed and version_tuple(version) <= version_tuple(previous_version):
            raise RuntimeError("Node inputs changed; increase Connector version before building: " + str(path))
    return {"connectorVersion": version, "connectorSourceFingerprint": fingerprint,
            "connectorSourceHashes": hashes}
