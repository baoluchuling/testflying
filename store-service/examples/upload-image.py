#!/usr/bin/env python3
"""Upload a local image through the central API; no store mutation is performed."""
from __future__ import annotations

import argparse
import hashlib
import json
import mimetypes
import os
from pathlib import Path
import ssl
import urllib.parse
import urllib.request


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base-url", required=True, help="Central Store API URL, including its path prefix")
    parser.add_argument("--platform", choices=["ios", "android"], required=True)
    parser.add_argument("--bundle-id", required=True)
    parser.add_argument("--file", type=Path, required=True)
    parser.add_argument("--ca-file", type=Path)
    args = parser.parse_args()
    token = os.environ.get("STORE_API_TOKEN")
    if not token:
        parser.error("Set STORE_API_TOKEN in the calling process environment")
    base = args.base_url.rstrip("/")
    origin = urllib.parse.urlsplit(base)
    if origin.scheme != "https" or not origin.hostname or origin.username or origin.password or origin.query or origin.fragment:
        parser.error("Use a central HTTPS URL without embedded credentials or query")
    context = ssl.create_default_context(cafile=None if args.ca_file is None else str(args.ca_file))
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), urllib.request.HTTPSHandler(context=context))
    mime = mimetypes.guess_type(args.file.name)[0]
    if mime not in {"image/png", "image/jpeg"}:
        parser.error("Choose a PNG or JPEG file")
    with args.file.open("rb") as source:
        before = os.fstat(source.fileno())
        if not 0 < before.st_size <= 64 * 1024 * 1024:
            parser.error("Image must be between 1 byte and 64 MiB")
        digest = hashlib.sha256()
        while chunk := source.read(64 * 1024):
            digest.update(chunk)
        after = os.fstat(source.fileno())
        if (before.st_size, before.st_mtime_ns) != (after.st_size, after.st_mtime_ns):
            parser.error("Image changed while calculating its checksum")
        platform = args.platform
        app = urllib.parse.quote(args.bundle_id, safe="")
        data = json.dumps({"fileName": args.file.name, "contentType": mime,
                           "sizeBytes": before.st_size, "sha256": digest.hexdigest()}).encode()
        prepare = urllib.request.Request(f"{base}/v1/store-management/apps/{platform}/{app}/uploads", data=data,
                                         headers={"Authorization": "Bearer " + token, "Content-Type": "application/json"}, method="POST")
        with opener.open(prepare, timeout=30) as response:
            prepared = json.load(response)
        target = urllib.parse.urlsplit(prepared["uploadUrl"])
        if (target.scheme, target.netloc) != (origin.scheme, origin.netloc):
            raise RuntimeError("Upload URL does not belong to the central API")
        source.seek(0)
        upload = urllib.request.Request(prepared["uploadUrl"], data=source, method="PUT", headers={
            "Authorization": "Bearer " + prepared["uploadToken"], "Content-Type": mime,
            "Content-Length": str(before.st_size)})
        with opener.open(upload, timeout=1860) as response:
            result = json.load(response)
    # Upload credentials remain in process memory; print only the resulting asset metadata.
    print(json.dumps(result, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
