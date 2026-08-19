from __future__ import annotations

from testflying_api.errors import ApiError


def normalize_store_identifiers(
    *,
    platform: str,
    store_app_id: str | None,
    store_package_name: str | None,
) -> tuple[str | None, str | None]:
    normalized_app_id = _normalize_optional(store_app_id)
    normalized_package = _normalize_optional(store_package_name)
    if platform == "ios":
        if normalized_package is not None:
            raise ApiError(
                "invalid_store_identifier",
                "iOS App 只能填写 App Store Connect App ID，不能填写 Google Play package name。",
                status_code=422,
            )
        return normalized_app_id, None
    if platform == "android":
        if normalized_app_id is not None:
            raise ApiError(
                "invalid_store_identifier",
                (
                    "Android App 只能填写 Google Play package name，"
                    "不能填写 App Store Connect App ID。"
                ),
                status_code=422,
            )
        return None, normalized_package
    return normalized_app_id, normalized_package


def _normalize_optional(value: str | None) -> str | None:
    if value is None:
        return None
    normalized = value.strip()
    return normalized or None
