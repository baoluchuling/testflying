"""Store release compatibility exports and shared release source parsing."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path
import re

from managed_workspace import PROJECT

MANIFEST = "crates/store-connector/Cargo.toml"
RELEASE = "crates/store-protocol/src/release.rs"
SEMVER = re.compile(r"(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)", re.ASCII)
RAW_STRING = re.compile(r'r(#{0,255})"')
CHARACTER = re.compile(r"'(?:\\(?:u\{[0-9a-fA-F_]+\}|x[0-9a-fA-F]{2}|.)|[^'\\\n])'")
TEST_ATTRIBUTE = re.compile(
    r"#\s*(?P<inner>!)?\s*\[\s*(?:cfg\s*\(\s*test\s*\)|"
    r"(?:tokio\s*::\s*)?test(?:\s*\([^\]]*\))?)\s*\]"
)
PRECEDING_ATTRIBUTES = re.compile(r"(?:^[ \t]*#\s*\[[^\[\]\n]*\][ \t]*\n)+[ \t]*\Z", re.MULTILINE)


def version_tuple(value: object) -> tuple[int, int, int]:
    if not isinstance(value, str) or len(value) > 32 or SEMVER.fullmatch(value) is None:
        raise RuntimeError("Connector version must be canonical major.minor.patch (at most 32 bytes)")
    parts = tuple(map(int, value.split(".")))
    if any(part > 4_294_967_295 for part in parts):
        raise RuntimeError("Connector version components must fit unsigned 32-bit integers")
    return parts


def connector_version(project: Path = PROJECT) -> str:
    from component_release import component_version

    return component_version("store", project)


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
            masked[index:end] = re.sub(r"[^\n]", " ", source[index:end])
            index = end
        else:
            index += 1
    return "".join(masked)


def _item_end(mask: str, start: int) -> int:
    delimiters = []
    body = False
    for index in range(start, len(mask)):
        char = mask[index]
        if char == ";" and not delimiters:
            return index + 1
        if char in "([{":
            body = body or (char == "{" and not delimiters)
            delimiters.append(char)
        elif char in ")]}":
            if not delimiters or delimiters.pop() != {")": "(", "]": "[", "}": "{"}[char]:
                break
            if body and not delimiters:
                return index + 1
    raise RuntimeError("Cannot identify complete Rust node input item")


def _production_rust(data: bytes) -> bytes:
    source = data.decode()
    mask = _rust_mask(source)
    ranges = []
    for match in TEST_ATTRIBUTE.finditer(mask):
        if ranges and match.start() < ranges[-1][1]:
            continue
        if match["inner"]:
            prefix = mask[:match.start()]
            if prefix.count("{") != prefix.count("}"):
                raise RuntimeError("Cannot scope a nested inner Rust test attribute")
            return b""
        start = match.start()
        end = _item_end(mask, match.end())
        line_start = source.rfind("\n", 0, start) + 1
        line_end = source.find("\n", end)
        following = mask[end:] if line_end < 0 else mask[end:line_end]
        semicolon = re.match(r"[ \t]*;", following)
        if semicolon is not None:
            end += semicolon.end()
            following = following[semicolon.end():]
        if not source[line_start:start].strip() and not following.strip():
            preceding = PRECEDING_ATTRIBUTES.search(mask[:start])
            start = max(preceding.start() if preceding else line_start, ranges[-1][1] if ranges else 0)
            end = len(source) if line_end < 0 else line_end + 1
        ranges.append((start, end))
    for start, end in reversed(ranges):
        source = source[:start] + source[end:]
    return source.strip().encode()


def _fingerprint(hashes: dict[str, str]) -> str:
    return hashlib.sha256(json.dumps(hashes, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def connector_source_hashes(project: Path = PROJECT) -> dict[str, str]:
    from component_release import component_source_hashes

    return component_source_hashes("store", project)


def connector_source_fingerprint(project: Path = PROJECT) -> str:
    from component_release import component_source_fingerprint

    return component_source_fingerprint("store", project)


def guard_connector_release(report_paths: list[Path], *, api_only: bool = False,
                            project: Path = PROJECT) -> dict:
    from component_release import guard_component

    metadata = guard_component("store", report_paths, project=project, require_unchanged=api_only)
    return {"connectorVersion": metadata["version"], "connectorSourceFingerprint": metadata["sourceFingerprint"],
            "connectorSourceHashes": metadata["sourceHashes"]}
