"""Validate the artifact supervisor's exact ownership marker before creating files."""
from __future__ import annotations

import json
import os
import stat
from pathlib import Path

PROJECT = Path(__file__).resolve().parents[1]


def managed_root() -> Path:
    raw = os.environ.get("ARTIFACT_RUN_ROOT", "")
    root = Path(raw)
    if not raw or not root.is_absolute() or root.resolve(strict=True) != root:
        raise RuntimeError("A canonical managed run root is required")
    if root.is_relative_to(PROJECT) or PROJECT.is_relative_to(root):
        raise RuntimeError("Managed output cannot overlap source")
    for path in (root, *root.parents):
        if path.is_symlink():
            raise RuntimeError("Symlink in managed root")
    root_stat = root.stat()
    if not stat.S_ISDIR(root_stat.st_mode) or root_stat.st_uid != os.getuid():
        raise RuntimeError("Managed root has the wrong owner or type")
    marker = root / ".artifact-lifecycle-owner.json"
    if os.environ.get("ARTIFACT_RUN_OWNERSHIP_MARKER") != str(marker):
        raise RuntimeError("Exact ownership marker required")
    directory = os.open(root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        fd = os.open(marker.name, os.O_RDONLY | os.O_NOFOLLOW, dir_fd=directory)
        try:
            before = os.fstat(fd)
            if (not stat.S_ISREG(before.st_mode) or before.st_uid != os.getuid()
                    or before.st_nlink != 1 or before.st_mode & 0o222 or before.st_size > 4096):
                raise RuntimeError("Unsafe managed ownership marker")
            payload = json.loads(os.read(fd, 4097))
            expected = {"schema_version": 1, "owner": "codex-artifact-lifecycle",
                        "run_id": root.name, "project_root": str(PROJECT)}
            if payload != expected:
                raise RuntimeError("Managed ownership marker does not match this project")
            after = os.fstat(fd)
            current = os.stat(marker.name, dir_fd=directory, follow_symlinks=False)
            def identity(s: os.stat_result) -> tuple[int, ...]:
                return s.st_dev, s.st_ino, s.st_size, s.st_mtime_ns, s.st_mode, s.st_nlink
            if identity(before) != identity(after) or identity(before) != identity(current):
                raise RuntimeError("Managed ownership marker changed")
            if os.fstat(directory).st_ino != root.stat().st_ino:
                raise RuntimeError("Managed root changed")
        finally:
            os.close(fd)
    finally:
        os.close(directory)
    return root
