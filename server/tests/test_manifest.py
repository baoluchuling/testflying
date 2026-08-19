from __future__ import annotations

import plistlib

from testflying_api.manifest import generate_ios_manifest, itms_services_url


def test_generate_ios_manifest_contains_software_package() -> None:
    manifest = generate_ios_manifest(
        download_url="https://dist.example.test/Aurora.ipa",
        bundle_identifier="com.example.aurora",
        version="1.2.3",
        title="Aurora",
    )

    payload = plistlib.loads(manifest)
    item = payload["items"][0]
    assert item["assets"][0]["kind"] == "software-package"
    assert item["assets"][0]["url"] == "https://dist.example.test/Aurora.ipa"
    assert item["metadata"]["bundle-identifier"] == "com.example.aurora"


def test_itms_services_url_encodes_manifest_url() -> None:
    url = itms_services_url("https://dist.example.test/manifests/Aurora manifest.plist")

    assert url.startswith("itms-services://?action=download-manifest&url=")
    assert "Aurora%20manifest.plist" in url
