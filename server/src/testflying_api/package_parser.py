from __future__ import annotations

import plistlib
from dataclasses import dataclass
from io import BytesIO
from tempfile import NamedTemporaryFile
from zipfile import BadZipFile, ZipFile

from pyaxmlparser import APK


class PackageParseError(ValueError):
    pass


@dataclass(frozen=True)
class PackageMetadata:
    app_name: str
    bundle_identifier: str
    version: str
    build_number: str
    platform: str


def parse_ipa_metadata(content: bytes) -> PackageMetadata:
    try:
        with ZipFile(BytesIO(content)) as archive:
            info_plist_name = _find_info_plist(archive)
            raw_plist = archive.read(info_plist_name)
    except (BadZipFile, KeyError) as error:
        raise PackageParseError("IPA 包结构不正确") from error

    plist = plistlib.loads(raw_plist)
    bundle_identifier = _required_plist_value(plist, "CFBundleIdentifier")
    version = _required_plist_value(plist, "CFBundleShortVersionString")
    build_number = _required_plist_value(plist, "CFBundleVersion")
    app_name = (
        _optional_plist_value(plist, "CFBundleDisplayName")
        or _optional_plist_value(plist, "CFBundleName")
        or bundle_identifier
    )
    return PackageMetadata(
        app_name=app_name,
        bundle_identifier=bundle_identifier,
        version=version,
        build_number=build_number,
        platform="ios",
    )


def parse_apk_metadata(content: bytes, *, app_name: str | None = None) -> PackageMetadata:
    try:
        with NamedTemporaryFile(suffix=".apk") as package_file:
            package_file.write(content)
            package_file.flush()
            apk = APK(package_file.name)
            if not apk.is_valid_APK():
                raise PackageParseError("APK 包结构不正确")
            package_name = _required_apk_value(apk.package, "packageName")
            version_name = _required_apk_value(apk.version_name, "versionName")
            version_code = _required_apk_value(apk.version_code, "versionCode")
            parsed_app_name = (
                _optional_text_value(app_name)
                or _optional_text_value(apk.application)
                or _optional_text_value(apk.get_app_name())
                or package_name
            )
    except PackageParseError:
        raise
    except Exception as error:
        raise PackageParseError("APK 包结构不正确") from error

    return PackageMetadata(
        app_name=parsed_app_name,
        bundle_identifier=package_name,
        version=version_name,
        build_number=version_code,
        platform="android",
    )


def _find_info_plist(archive: ZipFile) -> str:
    for name in archive.namelist():
        if name.startswith("Payload/") and name.endswith(".app/Info.plist"):
            return name
    raise PackageParseError("IPA 缺少 Payload/*.app/Info.plist")


def _required_plist_value(plist: dict[object, object], key: str) -> str:
    value = _optional_plist_value(plist, key)
    if value is None:
        raise PackageParseError(f"Info.plist 缺少 {key}")
    return value


def _optional_plist_value(plist: dict[object, object], key: str) -> str | None:
    value = plist.get(key)
    return value if isinstance(value, str) and value else None


def _required_apk_value(value: object, field_name: str) -> str:
    parsed_value = _optional_text_value(value)
    if parsed_value is None:
        raise PackageParseError(f"APK 缺少 {field_name}")
    return parsed_value


def _optional_text_value(value: object) -> str | None:
    if value is None:
        return None
    parsed_value = str(value).strip()
    return parsed_value or None
