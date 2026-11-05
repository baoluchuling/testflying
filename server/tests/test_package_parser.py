from __future__ import annotations

import pytest

from testflying_api.package_parser import (
    PackageParseError,
    parse_apk_metadata,
    parse_ipa_metadata,
)
from tests.fixtures import make_android_apk_bytes, make_ipa_bytes


def test_parse_ipa_metadata() -> None:
    ipa_bytes = make_ipa_bytes()

    metadata = parse_ipa_metadata(ipa_bytes)

    assert metadata.bundle_identifier == "com.example.aurora"
    assert metadata.app_name == "Aurora"
    assert metadata.version == "1.2.3"
    assert metadata.build_number == "45"
    assert metadata.platform == "ios"


def test_parse_apk_metadata() -> None:
    metadata = parse_apk_metadata(make_android_apk_bytes())

    assert metadata.bundle_identifier == "com.example.autoparse"
    assert metadata.app_name == "Auto Parsed"
    assert metadata.version == "4.5.6"
    assert metadata.build_number == "321"
    assert metadata.platform == "android"


def test_parse_apk_metadata_rejects_invalid_apk() -> None:
    with pytest.raises(PackageParseError):
        parse_apk_metadata(b"not-an-apk")
