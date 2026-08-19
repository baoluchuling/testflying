from __future__ import annotations

import plistlib
from urllib.parse import quote


def generate_ios_manifest(
    *,
    download_url: str,
    bundle_identifier: str,
    version: str,
    title: str,
) -> bytes:
    payload = {
        "items": [
            {
                "assets": [
                    {
                        "kind": "software-package",
                        "url": download_url,
                    }
                ],
                "metadata": {
                    "bundle-identifier": bundle_identifier,
                    "bundle-version": version,
                    "kind": "software",
                    "title": title,
                },
            }
        ]
    }
    return plistlib.dumps(payload, sort_keys=False)


def itms_services_url(manifest_url: str) -> str:
    encoded_manifest_url = quote(manifest_url, safe="")
    return f"itms-services://?action=download-manifest&url={encoded_manifest_url}"
