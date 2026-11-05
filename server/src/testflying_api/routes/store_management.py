from __future__ import annotations

import datetime as dt
import json
import re
import secrets
import time
from collections import deque
from pathlib import Path
from threading import Lock
from typing import Annotated
from uuid import uuid4
from zoneinfo import ZoneInfo, ZoneInfoNotFoundError

from fastapi import APIRouter, Depends, Query, Request
from pydantic import AliasChoices, BaseModel, ConfigDict, Field, ValidationError
from sqlalchemy.orm import Session

from testflying_api.database import get_db_session
from testflying_api.errors import ApiError
from testflying_api.store_image_requirements import validate_store_image
from testflying_api.store_sync import (
    APP_METADATA_SYNC_SCOPES,
    CURRENT_METADATA_VERSION,
    DEFAULT_CONTENT_SET_ID,
    DEFAULT_CONTENT_SET_NAME,
    DEFAULT_LOCALE,
    ConnectorCallError,
    StoreConnectorClient,
    account_connector,
    create_marketing_page,
    marketing_page_for_scope,
    marketing_page_locales,
    metadata_draft_for_scope,
    save_current_app_metadata_draft,
    save_release_note_draft,
    scoped_app,
    sync_existing_current_app_metadata,
    sync_existing_release_notes,
    sync_marketing_page,
)

router = APIRouter(prefix="/v1/store-management", tags=["store-management"])
SessionDep = Annotated[Session, Depends(get_db_session)]

STORE_IMAGE_SLOT_KEYS = {
    "feature_graphic_url",
    "phone_screenshots",
    "tablet_screenshots",
}
STORE_IMAGE_SLOT_ALIASES = {
    "featureGraphicUrl": "feature_graphic_url",
    "feature_graphic_url": "feature_graphic_url",
    "featureGraphic": "feature_graphic_url",
    "phoneScreenshots": "phone_screenshots",
    "phone_screenshots": "phone_screenshots",
    "tabletScreenshots": "tablet_screenshots",
    "tablet_screenshots": "tablet_screenshots",
}
DEFAULT_DIRECT_STORE_SYNC_SCOPES = ["metadata", "release_notes", "store_images"]
DIRECT_STORE_SYNC_SCOPES = [*APP_METADATA_SYNC_SCOPES, "release_notes"]
DEFAULT_DIRECT_MARKETING_SYNC_SCOPES = ["marketing_text", "store_images"]
DIRECT_SYNC_RATE_LIMIT_PER_MINUTE = 10
DIRECT_SYNC_RATE_LIMIT_PER_HOUR = 100
DIRECT_SYNC_IDEMPOTENCY_TTL_SECONDS = 3600

_direct_sync_guard_lock = Lock()
_direct_sync_minute_events: dict[tuple[str, str, str], deque[float]] = {}
_direct_sync_hour_events: dict[tuple[str, str, str], deque[float]] = {}
_direct_sync_idempotency_cache: dict[
    tuple[str, str, str, str, str],
    tuple[float, dict[str, object]],
] = {}
_direct_sync_account_locks: dict[str, Lock] = {}


class CamelModel(BaseModel):
    model_config = ConfigDict(populate_by_name=True)


class ContentSetInput(CamelModel):
    id: str = DEFAULT_CONTENT_SET_ID
    name: str = DEFAULT_CONTENT_SET_NAME


class LocaleMetadataInput(CamelModel):
    locale: str
    keywords: str = ""
    promotional_text: str = Field(
        default="",
        alias="promotionalText",
        validation_alias=AliasChoices("promotionalText", "shortDescription", "short_description"),
    )
    description: str = ""
    store_images: dict[str, object] = Field(default_factory=dict, alias="storeImages")


class MetadataContentSetImport(CamelModel):
    version: str = CURRENT_METADATA_VERSION
    content_set: ContentSetInput = Field(default_factory=ContentSetInput, alias="contentSet")
    source_locale: str = Field(default=DEFAULT_LOCALE, alias="sourceLocale")
    locales: list[LocaleMetadataInput]
    store_images: dict[str, object] = Field(default_factory=dict, alias="storeImages")


class MetadataContentSetResponse(CamelModel):
    content_set: ContentSetInput = Field(alias="contentSet")
    version: str
    locales: list[str]
    saved_drafts: int = Field(alias="savedDrafts")
    uploaded_assets: int = Field(alias="uploadedAssets")
    warnings: list[str] = Field(default_factory=list)


class VersionLocaleDraftInput(CamelModel):
    locale: str
    keywords: str = ""
    promotional_text: str = Field(
        default="",
        alias="promotionalText",
        validation_alias=AliasChoices("promotionalText", "shortDescription", "short_description"),
    )
    description: str = ""
    release_notes: str = Field(default="", alias="releaseNotes")


class StoreVersionDraftImport(CamelModel):
    source_locale: str = Field(default=DEFAULT_LOCALE, alias="sourceLocale")
    locales: list[VersionLocaleDraftInput]


class StoreVersionDraftResponse(CamelModel):
    version: str
    locales: list[str]
    saved_metadata_drafts: int = Field(alias="savedMetadataDrafts")
    saved_release_note_drafts: int = Field(alias="savedReleaseNoteDrafts")
    warnings: list[str] = Field(default_factory=list)


class MarketingPageLocaleInput(CamelModel):
    locale: str
    promotional_text: str = Field(default="", alias="promotionalText")
    store_images: dict[str, object] = Field(default_factory=dict, alias="storeImages")


class MarketingPageImport(CamelModel):
    page_name: str = Field(default="新的自定义产品页面", alias="pageName")
    page_type: str = Field(default="custom_product_page", alias="pageType")
    source_locale: str = Field(default=DEFAULT_LOCALE, alias="sourceLocale")
    deep_link_url: str = Field(default="", alias="deepLinkUrl")
    locales: list[MarketingPageLocaleInput]
    store_images: dict[str, object] = Field(default_factory=dict, alias="storeImages")


class MarketingPageImportResponse(CamelModel):
    page_id: str = Field(alias="pageId")
    page_name: str = Field(alias="pageName")
    page_type: str = Field(alias="pageType")
    status: str
    locales: list[str]
    saved_locales: int = Field(alias="savedLocales")
    uploaded_assets: int = Field(alias="uploadedAssets")
    warnings: list[str] = Field(default_factory=list)


class StoreDirectSyncRequest(CamelModel):
    version: str
    locales: list[str] = Field(default_factory=list)
    scopes: list[str] = Field(default_factory=lambda: list(DEFAULT_DIRECT_STORE_SYNC_SCOPES))
    ios_app_id: str = Field(
        default="",
        alias="iosAppId",
        validation_alias=AliasChoices("iosAppId", "storeAppId", "appleAppId"),
    )
    package_name: str = Field(default="", alias="packageName")
    store_track: str = Field(default="", alias="storeTrack")
    store_version_code: str | int = Field(
        default="",
        alias="storeVersionCode",
        validation_alias=AliasChoices("storeVersionCode", "versionCode"),
    )
    actor: str = "api"
    idempotency_key: str = Field(default="", alias="idempotencyKey")


class MarketingDirectSyncRequest(CamelModel):
    locales: list[str] = Field(default_factory=list)
    scopes: list[str] = Field(default_factory=lambda: list(DEFAULT_DIRECT_MARKETING_SYNC_SCOPES))
    ios_app_id: str = Field(
        default="",
        alias="iosAppId",
        validation_alias=AliasChoices("iosAppId", "storeAppId", "appleAppId"),
    )
    actor: str = "api"
    idempotency_key: str = Field(default="", alias="idempotencyKey")


class DirectSyncRunResult(CamelModel):
    run_id: str = Field(alias="runId")
    operation: str
    locale: str
    status: str
    error_code: str | None = Field(default=None, alias="errorCode")
    error_summary: str | None = Field(default=None, alias="errorSummary")


class DirectSyncResponse(CamelModel):
    status: str
    account_id: str = Field(alias="accountId")
    app_id: str = Field(alias="appId")
    version: str | None = None
    page_id: str | None = Field(default=None, alias="pageId")
    scopes: list[str]
    locales: list[str]
    runs: list[DirectSyncRunResult]
    idempotent: bool = False


class StoreLocalesResponse(CamelModel):
    account_id: str = Field(alias="accountId")
    app_id: str = Field(alias="appId")
    platform: str
    version: str = ""
    locales: list[str]


class StoreListingsResponse(CamelModel):
    account_id: str = Field(alias="accountId")
    app_id: str = Field(alias="appId")
    platform: str
    version: str = ""
    listings: list[dict[str, object]]


class StoreImagesResponse(CamelModel):
    account_id: str = Field(alias="accountId")
    app_id: str = Field(alias="appId")
    platform: str
    version: str = ""
    locales: list[dict[str, object]]


class StoreReleasesResponse(CamelModel):
    account_id: str = Field(alias="accountId")
    app_id: str = Field(alias="appId")
    platform: str
    version: str = ""
    releases: list[dict[str, object]]


class StoreReviewsResponse(CamelModel):
    account_id: str = Field(alias="accountId")
    app_id: str = Field(alias="appId")
    platform: str
    reviews: list[dict[str, object]]
    next_page_token: str = Field(default="", alias="nextPageToken")
    filters: dict[str, object] = Field(default_factory=dict)


class ProductPageOptimizationTreatmentInput(CamelModel):
    name: str
    app_icon_name: str = Field(default="", alias="appIconName")
    locales: list[str] = Field(default_factory=list)


class ProductPageOptimizationCreateRequest(CamelModel):
    name: str
    traffic_proportion: int = Field(default=50, alias="trafficProportion")
    locales: list[str] = Field(default_factory=list)
    treatments: list[ProductPageOptimizationTreatmentInput] = Field(default_factory=list)
    ios_app_id: str = Field(
        default="",
        alias="iosAppId",
        validation_alias=AliasChoices("iosAppId", "storeAppId", "appleAppId"),
    )
    idempotency_key: str = Field(default="", alias="idempotencyKey")


class ProductPageOptimizationTreatmentResponse(CamelModel):
    id: str = ""
    name: str
    app_icon_name: str = Field(default="", alias="appIconName")
    locales: list[str] = Field(default_factory=list)


class ProductPageOptimizationResponse(CamelModel):
    id: str
    name: str
    platform: str
    state: str
    traffic_proportion: int = Field(alias="trafficProportion")
    review_required: bool = Field(alias="reviewRequired")
    start_date: str = Field(default="", alias="startDate")
    end_date: str = Field(default="", alias="endDate")
    treatments: list[ProductPageOptimizationTreatmentResponse] = Field(default_factory=list)


class ProductPageOptimizationsResponse(CamelModel):
    account_id: str = Field(alias="accountId")
    app_id: str = Field(alias="appId")
    experiments: list[ProductPageOptimizationResponse]


class ProductPageOptimizationCreateResponse(CamelModel):
    account_id: str = Field(alias="accountId")
    app_id: str = Field(alias="appId")
    experiment: ProductPageOptimizationResponse
    idempotent: bool = False


@router.post(
    "/developer-accounts/{account_id}/apps/{app_id}/metadata-content-sets",
    response_model=MetadataContentSetResponse,
)
async def import_metadata_content_set(
    account_id: str,
    app_id: str,
    request: Request,
    session: SessionDep,
) -> MetadataContentSetResponse:
    _require_static_token(request)
    form = await request.form()
    payload = _metadata_payload_from_form(form)
    app = scoped_app(session, account_id, app_id)
    if app is None:
        raise ApiError("app_not_found", "当前开发者账号下没有这个 App", status_code=404)

    uploaded_assets = await _store_image_assets_from_form(
        form,
        request=request,
        account_id=account_id,
        app_id=app_id,
        platform=app.platform,
        version=CURRENT_METADATA_VERSION,
        content_set_id=DEFAULT_CONTENT_SET_ID,
    )
    rows = _metadata_rows(payload, uploaded_assets)
    for row in rows:
        save_current_app_metadata_draft(
            session,
            account_id=account_id,
            app_id=app_id,
            locale=row["locale"],
            keywords=row["keywords"],
            promotional_text=row["promotional_text"],
            description=row["description"],
            store_images=row["store_images"],
        )
    session.commit()
    return MetadataContentSetResponse(
        contentSet=payload.content_set,
        version=payload.version,
        locales=[row["locale"] for row in rows],
        savedDrafts=len(rows),
        uploadedAssets=sum(
            len(assets) for slots in uploaded_assets.values() for assets in slots.values()
        ),
        warnings=[],
    )


@router.post(
    "/developer-accounts/{account_id}/apps/{app_id}/store-versions/{version}/draft",
    response_model=StoreVersionDraftResponse,
)
async def import_store_version_draft(
    account_id: str,
    app_id: str,
    version: str,
    request: Request,
    session: SessionDep,
) -> StoreVersionDraftResponse:
    _require_static_token(request)
    payload = await _store_version_payload_from_request(request)
    app = scoped_app(session, account_id, app_id)
    if app is None:
        raise ApiError("app_not_found", "当前开发者账号下没有这个 App", status_code=404)

    rows = _version_draft_rows(payload)
    saved_metadata_drafts = 0
    saved_release_note_drafts = 0
    for row in rows:
        if row["save_metadata"]:
            existing = metadata_draft_for_scope(
                session,
                account_id=account_id,
                app_id=app_id,
                platform=app.platform,
                version=CURRENT_METADATA_VERSION,
                locale=str(row["locale"]),
                content_set_id=DEFAULT_CONTENT_SET_ID,
            )
            save_current_app_metadata_draft(
                session,
                account_id=account_id,
                app_id=app_id,
                locale=str(row["locale"]),
                keywords=str(row["keywords"]),
                promotional_text=str(row["promotional_text"]),
                description=str(row["description"]),
                store_images=existing.store_images_json if existing is not None else {},
            )
            saved_metadata_drafts += 1

        save_release_note_draft(
            session,
            account_id=account_id,
            app_id=app_id,
            version=version,
            locale=str(row["locale"]),
            release_notes=str(row["release_notes"]),
        )
        saved_release_note_drafts += 1

    session.commit()
    return StoreVersionDraftResponse(
        version=version,
        locales=[str(row["locale"]) for row in rows],
        savedMetadataDrafts=saved_metadata_drafts,
        savedReleaseNoteDrafts=saved_release_note_drafts,
        warnings=[],
    )


@router.post(
    "/developer-accounts/{account_id}/apps/{app_id}/marketing-pages",
    response_model=MarketingPageImportResponse,
)
async def import_marketing_page(
    account_id: str,
    app_id: str,
    request: Request,
    session: SessionDep,
) -> MarketingPageImportResponse:
    _require_static_token(request)
    form = await request.form()
    payload = _marketing_page_payload_from_form(form)
    app = scoped_app(session, account_id, app_id)
    if app is None:
        raise ApiError("app_not_found", "当前开发者账号下没有这个 App", status_code=404)
    if app.platform != "ios":
        raise ApiError(
            "unsupported_marketing_page",
            "营销页面当前仅支持 App Store Connect",
            status_code=422,
        )

    page_id = f"page-{uuid4().hex[:8]}"
    uploaded_assets = await _store_image_assets_from_form(
        form,
        request=request,
        account_id=account_id,
        app_id=app_id,
        platform=app.platform,
        version="marketing",
        content_set_id=page_id,
    )
    rows = _marketing_page_rows(payload, uploaded_assets)
    page = create_marketing_page(
        session,
        account_id=account_id,
        app_id=app.id,
        page_id=page_id,
        page_name=payload.page_name,
        page_type=payload.page_type,
        deep_link_url=payload.deep_link_url,
        locale_rows=rows,
    )
    session.commit()
    return MarketingPageImportResponse(
        pageId=page.page_id,
        pageName=page.page_name,
        pageType=page.page_type,
        status=page.status,
        locales=[str(row["locale"]) for row in rows],
        savedLocales=len(rows),
        uploadedAssets=_uploaded_asset_count(uploaded_assets),
        warnings=[],
    )


@router.get(
    "/developer-accounts/{account_id}/apps/{app_id}/store-locales",
    response_model=StoreLocalesResponse,
)
def list_store_locales(
    account_id: str,
    app_id: str,
    request: Request,
    session: SessionDep,
    version: str = Query(default=""),
    ios_app_id: str = Query(default="", alias="iosAppId"),
    store_app_id: str = Query(default="", alias="storeAppId"),
    apple_app_id: str = Query(default="", alias="appleAppId"),
    package_name: str = Query(default="", alias="packageName"),
) -> StoreLocalesResponse:
    _require_static_token(request)
    app = _scoped_app_or_error(session, account_id=account_id, app_id=app_id)
    connector = _account_connector_or_error(session, account_id)
    target_store_app_id, target_package_name = _query_store_target(
        platform=app.platform,
        ios_app_id=ios_app_id,
        store_app_id=store_app_id,
        apple_app_id=apple_app_id,
        package_name=package_name,
    )
    try:
        locales = StoreConnectorClient().supported_locales(
            connector,
            account_id=account_id,
            app=app,
            version=version.strip(),
            store_app_id=target_store_app_id,
            package_name=target_package_name,
        )
    except ConnectorCallError as error:
        raise _connector_api_error(error) from error
    return StoreLocalesResponse(
        accountId=account_id,
        appId=app.id,
        platform=app.platform,
        version=version.strip(),
        locales=locales,
    )


@router.get(
    "/developer-accounts/{account_id}/apps/{app_id}/store-listings",
    response_model=StoreListingsResponse,
)
def list_store_listings(
    account_id: str,
    app_id: str,
    request: Request,
    session: SessionDep,
    version: str = Query(default=""),
    ios_app_id: str = Query(default="", alias="iosAppId"),
    store_app_id: str = Query(default="", alias="storeAppId"),
    apple_app_id: str = Query(default="", alias="appleAppId"),
    package_name: str = Query(default="", alias="packageName"),
) -> StoreListingsResponse:
    _require_static_token(request)
    app = _scoped_app_or_error(session, account_id=account_id, app_id=app_id)
    connector = _account_connector_or_error(session, account_id)
    target_store_app_id, target_package_name = _query_store_target(
        platform=app.platform,
        ios_app_id=ios_app_id,
        store_app_id=store_app_id,
        apple_app_id=apple_app_id,
        package_name=package_name,
    )
    try:
        raw_response = StoreConnectorClient().store_listings(
            connector,
            account_id=account_id,
            app=app,
            version=version.strip(),
            store_app_id=target_store_app_id,
            package_name=target_package_name,
        )
    except ConnectorCallError as error:
        raise _connector_api_error(error) from error
    listings = raw_response.get("listings")
    return StoreListingsResponse(
        accountId=account_id,
        appId=app.id,
        platform=app.platform,
        version=version.strip(),
        listings=listings if isinstance(listings, list) else [],
    )


@router.get(
    "/developer-accounts/{account_id}/apps/{app_id}/store-images",
    response_model=StoreImagesResponse,
)
def list_store_images(
    account_id: str,
    app_id: str,
    request: Request,
    session: SessionDep,
    version: str = Query(default=""),
    ios_app_id: str = Query(default="", alias="iosAppId"),
    store_app_id: str = Query(default="", alias="storeAppId"),
    apple_app_id: str = Query(default="", alias="appleAppId"),
    package_name: str = Query(default="", alias="packageName"),
) -> StoreImagesResponse:
    _require_static_token(request)
    app = _scoped_app_or_error(session, account_id=account_id, app_id=app_id)
    connector = _account_connector_or_error(session, account_id)
    target_store_app_id, target_package_name = _query_store_target(
        platform=app.platform,
        ios_app_id=ios_app_id,
        store_app_id=store_app_id,
        apple_app_id=apple_app_id,
        package_name=package_name,
    )
    try:
        raw_response = StoreConnectorClient().store_images(
            connector,
            account_id=account_id,
            app=app,
            version=version.strip(),
            store_app_id=target_store_app_id,
            package_name=target_package_name,
        )
    except ConnectorCallError as error:
        raise _connector_api_error(error) from error
    locales = raw_response.get("locales")
    return StoreImagesResponse(
        accountId=account_id,
        appId=app.id,
        platform=app.platform,
        version=version.strip(),
        locales=locales if isinstance(locales, list) else [],
    )


@router.get(
    "/developer-accounts/{account_id}/apps/{app_id}/store-releases",
    response_model=StoreReleasesResponse,
)
def list_store_releases(
    account_id: str,
    app_id: str,
    request: Request,
    session: SessionDep,
    version: str = Query(default=""),
    ios_app_id: str = Query(default="", alias="iosAppId"),
    store_app_id: str = Query(default="", alias="storeAppId"),
    apple_app_id: str = Query(default="", alias="appleAppId"),
    package_name: str = Query(default="", alias="packageName"),
) -> StoreReleasesResponse:
    _require_static_token(request)
    app = _scoped_app_or_error(session, account_id=account_id, app_id=app_id)
    connector = _account_connector_or_error(session, account_id)
    target_store_app_id, target_package_name = _query_store_target(
        platform=app.platform,
        ios_app_id=ios_app_id,
        store_app_id=store_app_id,
        apple_app_id=apple_app_id,
        package_name=package_name,
    )
    try:
        raw_response = StoreConnectorClient().store_releases(
            connector,
            account_id=account_id,
            app=app,
            version=version.strip(),
            store_app_id=target_store_app_id,
            package_name=target_package_name,
        )
    except ConnectorCallError as error:
        raise _connector_api_error(error) from error
    releases = raw_response.get("releases")
    return StoreReleasesResponse(
        accountId=account_id,
        appId=app.id,
        platform=app.platform,
        version=version.strip(),
        releases=releases if isinstance(releases, list) else [],
    )


@router.get(
    "/developer-accounts/{account_id}/apps/{app_id}/store-reviews",
    response_model=StoreReviewsResponse,
)
def list_store_reviews(
    account_id: str,
    app_id: str,
    request: Request,
    session: SessionDep,
    ios_app_id: str = Query(default="", alias="iosAppId"),
    store_app_id: str = Query(default="", alias="storeAppId"),
    apple_app_id: str = Query(default="", alias="appleAppId"),
    package_name: str = Query(default="", alias="packageName"),
    date_value: str = Query(default="", alias="date"),
    start_date: str = Query(default="", alias="startDate"),
    end_date: str = Query(default="", alias="endDate"),
    timezone: str = Query(default="Asia/Shanghai"),
    locale: str = Query(default=""),
    territory: str = Query(default=""),
    rating: int | None = Query(default=None, ge=1, le=5),
    page_size: int = Query(default=50, ge=1, le=200, alias="pageSize"),
    page_token: str = Query(default="", alias="pageToken"),
    start_index: str = Query(default="", alias="startIndex"),
    translation_language: str = Query(default="", alias="translationLanguage"),
    sort: str = Query(default=""),
) -> StoreReviewsResponse:
    _require_static_token(request)
    app = _scoped_app_or_error(session, account_id=account_id, app_id=app_id)
    connector = _account_connector_or_error(session, account_id)
    target_store_app_id, target_package_name = _query_store_target(
        platform=app.platform,
        ios_app_id=ios_app_id,
        store_app_id=store_app_id,
        apple_app_id=apple_app_id,
        package_name=package_name,
    )
    try:
        raw_response = StoreConnectorClient().store_reviews(
            connector,
            account_id=account_id,
            app=app,
            store_app_id=target_store_app_id,
            package_name=target_package_name,
            store_query=_store_reviews_connector_query(
                page_size=_store_review_page_size_for_platform(app.platform, page_size),
                page_token=page_token,
                start_index=start_index,
                translation_language=translation_language,
                sort=sort,
            ),
        )
    except ConnectorCallError as error:
        raise _connector_api_error(error) from error
    raw_reviews = raw_response.get("reviews")
    reviews = raw_reviews if isinstance(raw_reviews, list) else []
    filters = _store_review_filters(
        date_value=date_value,
        start_date=start_date,
        end_date=end_date,
        timezone_name=timezone,
        locale=locale,
        territory=territory,
        rating=rating,
    )
    filtered_reviews = [
        review
        for review in reviews
        if isinstance(review, dict) and _store_review_matches(review, filters)
    ]
    return StoreReviewsResponse(
        accountId=account_id,
        appId=app.id,
        platform=app.platform,
        reviews=filtered_reviews,
        nextPageToken=str(raw_response.get("nextPageToken") or ""),
        filters=_store_review_filter_response(filters),
    )


@router.get(
    "/developer-accounts/{account_id}/apps/{app_id}/product-page-optimizations",
    response_model=ProductPageOptimizationsResponse,
)
def list_product_page_optimizations(
    account_id: str,
    app_id: str,
    request: Request,
    session: SessionDep,
    ios_app_id: str = Query(default="", alias="iosAppId"),
    store_app_id: str = Query(default="", alias="storeAppId"),
    apple_app_id: str = Query(default="", alias="appleAppId"),
) -> ProductPageOptimizationsResponse:
    _require_static_token(request)
    app = _product_page_optimization_app(session, account_id=account_id, app_id=app_id)
    connector = _account_connector_or_error(session, account_id)
    target_store_app_id, _target_package_name = _query_store_target(
        platform=app.platform,
        ios_app_id=ios_app_id,
        store_app_id=store_app_id,
        apple_app_id=apple_app_id,
    )
    try:
        raw_response = StoreConnectorClient().product_page_optimizations(
            connector,
            account_id=account_id,
            app=app,
            store_app_id=target_store_app_id,
        )
    except ConnectorCallError as error:
        raise ApiError(
            "connector_call_failed",
            error.message,
            status_code=502,
            retryable=True,
        ) from error
    experiments = raw_response.get("experiments")
    return ProductPageOptimizationsResponse(
        accountId=account_id,
        appId=app.id,
        experiments=experiments if isinstance(experiments, list) else [],
    )


@router.post(
    "/developer-accounts/{account_id}/apps/{app_id}/product-page-optimizations",
    response_model=ProductPageOptimizationCreateResponse,
    status_code=201,
)
def create_product_page_optimization(
    account_id: str,
    app_id: str,
    payload: ProductPageOptimizationCreateRequest,
    request: Request,
    session: SessionDep,
) -> ProductPageOptimizationCreateResponse:
    token = _require_static_token(request)
    cached = _product_page_optimization_cached_response(
        token=token,
        account_id=account_id,
        app_id=app_id,
        idempotency_key=payload.idempotency_key,
    )
    if cached is not None:
        return cached

    account_lock = _begin_direct_sync(token=token, account_id=account_id, app_id=app_id)
    try:
        app = _product_page_optimization_app(session, account_id=account_id, app_id=app_id)
        connector = _account_connector_or_error(session, account_id)
        name = payload.name.strip()
        if not name:
            raise ApiError(
                "invalid_product_page_optimization_name",
                "产品页面优化名称不能为空",
                status_code=422,
            )
        if payload.traffic_proportion <= 0 or payload.traffic_proportion > 100:
            raise ApiError(
                "invalid_traffic_proportion",
                "trafficProportion 必须在 1 到 100 之间",
                status_code=422,
            )
        try:
            raw_response = StoreConnectorClient().create_product_page_optimization(
                connector,
                account_id=account_id,
                app=app,
                name=name,
                traffic_proportion=payload.traffic_proportion,
                locales=_unique_non_empty(tuple(payload.locales)),
                treatments=_product_page_optimization_treatments(payload.treatments),
                store_app_id=payload.ios_app_id.strip() or None,
            )
        except ConnectorCallError as error:
            raise ApiError(
                "connector_call_failed",
                error.message,
                status_code=502,
                retryable=True,
            ) from error
        experiment = raw_response.get("experiment")
        if not isinstance(experiment, dict):
            raise ApiError(
                "invalid_connector_response",
                "connector 没有返回产品页面优化结果",
                status_code=502,
                retryable=True,
            )
        response = ProductPageOptimizationCreateResponse(
            accountId=account_id,
            appId=app.id,
            experiment=experiment,
        )
        _store_product_page_optimization_idempotency_response(
            token=token,
            account_id=account_id,
            app_id=app_id,
            idempotency_key=payload.idempotency_key,
            response=response,
        )
        return response
    finally:
        account_lock.release()


@router.post(
    "/developer-accounts/{account_id}/apps/{app_id}/sync-runs",
    response_model=DirectSyncResponse,
)
def trigger_default_store_sync(
    account_id: str,
    app_id: str,
    payload: StoreDirectSyncRequest,
    request: Request,
    session: SessionDep,
) -> DirectSyncResponse:
    token = _require_static_token(request)
    cached = _direct_sync_cached_response(
        token=token,
        account_id=account_id,
        app_id=app_id,
        endpoint="default-store",
        idempotency_key=payload.idempotency_key,
    )
    if cached is not None:
        return cached

    account_lock = _begin_direct_sync(token=token, account_id=account_id, app_id=app_id)
    try:
        version = payload.version.strip()
        if not version:
            raise ApiError("missing_version", "同步到商店前需要指定 version", status_code=422)
        app = scoped_app(session, account_id, app_id)
        if app is None:
            raise ApiError("app_not_found", "当前开发者账号下没有这个 App", status_code=404)
        store_app_id, package_name = _direct_sync_store_target(payload, platform=app.platform)
        scopes = _normalize_direct_sync_scopes(
            payload.scopes,
            allowed=DIRECT_STORE_SYNC_SCOPES,
        )
        locales = _normalize_requested_locales(payload.locales)
        actor = payload.actor.strip() or "api"
        runs = []
        metadata_scopes = [scope for scope in scopes if scope in APP_METADATA_SYNC_SCOPES]
        for locale in locales:
            if metadata_scopes:
                runs.append(
                    sync_existing_current_app_metadata(
                        session,
                        account_id=account_id,
                        app_id=app.id,
                        version=version,
                        locale=locale,
                        actor=actor,
                        sync_scopes=metadata_scopes,
                        store_app_id=store_app_id,
                        package_name=package_name,
                    )
                )
            if "release_notes" in scopes:
                runs.append(
                    sync_existing_release_notes(
                        session,
                        account_id=account_id,
                        app_id=app.id,
                        version=version,
                        locale=locale,
                        actor=actor,
                        store_track=payload.store_track,
                        store_version_code=str(payload.store_version_code or ""),
                        store_app_id=store_app_id,
                        package_name=package_name,
                    )
                )
        session.commit()
        response = _direct_sync_response(
            account_id=account_id,
            app_id=app.id,
            version=version,
            scopes=scopes,
            locales=locales,
            runs=runs,
        )
        _store_direct_sync_idempotency_response(
            token=token,
            account_id=account_id,
            app_id=app_id,
            endpoint="default-store",
            idempotency_key=payload.idempotency_key,
            response=response,
        )
        return response
    except ApiError:
        session.rollback()
        raise
    finally:
        account_lock.release()


@router.post(
    "/developer-accounts/{account_id}/apps/{app_id}/marketing-pages/{page_id}/sync-runs",
    response_model=DirectSyncResponse,
)
def trigger_marketing_page_sync(
    account_id: str,
    app_id: str,
    page_id: str,
    payload: MarketingDirectSyncRequest,
    request: Request,
    session: SessionDep,
) -> DirectSyncResponse:
    token = _require_static_token(request)
    cached = _direct_sync_cached_response(
        token=token,
        account_id=account_id,
        app_id=app_id,
        endpoint=f"marketing-page:{page_id}",
        idempotency_key=payload.idempotency_key,
    )
    if cached is not None:
        return cached

    account_lock = _begin_direct_sync(token=token, account_id=account_id, app_id=app_id)
    try:
        app = scoped_app(session, account_id, app_id)
        if app is None:
            raise ApiError("app_not_found", "当前开发者账号下没有这个 App", status_code=404)
        page = marketing_page_for_scope(
            session,
            account_id=account_id,
            app_id=app.id,
            page_id=page_id,
        )
        if page is None:
            raise ApiError("marketing_page_not_found", "营销页面不存在", status_code=404)
        store_app_id, _package_name = _direct_sync_store_target(payload, platform=app.platform)
        scopes = _normalize_direct_sync_scopes(
            payload.scopes,
            allowed=DEFAULT_DIRECT_MARKETING_SYNC_SCOPES,
        )
        locales = _normalize_requested_locales(payload.locales)
        existing_locales = marketing_page_locales(session, page.id)
        missing_locales = [locale for locale in locales if locale not in existing_locales]
        if missing_locales:
            raise ApiError(
                "marketing_page_locale_missing",
                f"营销页面缺少语言：{', '.join(missing_locales)}",
                status_code=422,
            )
        actor = payload.actor.strip() or "api"
        runs = [
            sync_marketing_page(
                session,
                account_id=account_id,
                app_id=app.id,
                page_id=page.page_id,
                locale=locale,
                sync_scopes=scopes,
                actor=actor,
                store_app_id=store_app_id,
            )
            for locale in locales
        ]
        session.commit()
        response = _direct_sync_response(
            account_id=account_id,
            app_id=app.id,
            version=page.page_id,
            page_id=page.page_id,
            scopes=scopes,
            locales=locales,
            runs=runs,
        )
        _store_direct_sync_idempotency_response(
            token=token,
            account_id=account_id,
            app_id=app_id,
            endpoint=f"marketing-page:{page_id}",
            idempotency_key=payload.idempotency_key,
            response=response,
        )
        return response
    except ApiError:
        session.rollback()
        raise
    finally:
        account_lock.release()


def _product_page_optimization_app(session: Session, *, account_id: str, app_id: str):
    app = _scoped_app_or_error(session, account_id=account_id, app_id=app_id)
    if app.platform != "ios":
        raise ApiError(
            "unsupported_product_page_optimization",
            "产品页面优化当前仅支持 App Store Connect",
            status_code=422,
        )
    return app


def _scoped_app_or_error(session: Session, *, account_id: str, app_id: str):
    app = scoped_app(session, account_id, app_id)
    if app is None:
        raise ApiError("app_not_found", "当前开发者账号下没有这个 App", status_code=404)
    return app


def _account_connector_or_error(session: Session, account_id: str):
    connector = account_connector(session, account_id)
    if connector is None:
        raise ApiError(
            "connector_missing",
            "当前开发者账号还没有配置 connector",
            status_code=422,
        )
    return connector


def _connector_api_error(error: ConnectorCallError) -> ApiError:
    return ApiError(
        "connector_call_failed",
        error.message,
        status_code=502,
        retryable=True,
    )


def _product_page_optimization_treatments(
    values: list[ProductPageOptimizationTreatmentInput],
) -> list[dict[str, object]]:
    treatments: list[dict[str, object]] = []
    for index, item in enumerate(values):
        name = item.name.strip()
        if not name:
            raise ApiError(
                "invalid_treatment_name",
                f"第 {index + 1} 个 treatment 名称不能为空",
                status_code=422,
            )
        treatments.append(
            {
                "name": name,
                "appIconName": item.app_icon_name.strip(),
                "locales": _unique_non_empty(tuple(item.locales)),
            }
        )
    return treatments


def _require_static_token(request: Request) -> str:
    expected = f"Bearer {request.app.state.settings.static_token}"
    authorization = request.headers.get("Authorization", "")
    if not secrets.compare_digest(authorization, expected):
        raise ApiError("invalid_static_token", "接口 token 不正确", status_code=401)
    return authorization


def _normalize_direct_sync_scopes(values: list[str], *, allowed: list[str]) -> list[str]:
    normalized = [str(value or "").strip() for value in values]
    invalid = [value for value in normalized if value and value not in allowed]
    if invalid:
        raise ApiError(
            "invalid_sync_scopes",
            f"不支持的同步范围：{', '.join(invalid)}",
            status_code=422,
        )
    scopes = _unique_non_empty(tuple(normalized))
    if not scopes:
        raise ApiError(
            "invalid_sync_scopes",
            f"同步范围至少需要包含一个值：{', '.join(allowed)}",
            status_code=422,
        )
    return scopes


def _normalize_requested_locales(values: list[str]) -> list[str]:
    locales = _unique_non_empty(tuple(str(value or "").strip() for value in values))
    if not locales:
        raise ApiError("locales_required", "直接同步接口需要指定 locales", status_code=422)
    return locales


def _begin_direct_sync(*, token: str, account_id: str, app_id: str) -> Lock:
    _check_direct_sync_rate_limit(token=token, account_id=account_id, app_id=app_id)
    with _direct_sync_guard_lock:
        account_lock = _direct_sync_account_locks.setdefault(account_id, Lock())
    if not account_lock.acquire(blocking=False):
        raise ApiError(
            "account_sync_in_progress",
            "当前开发者账号已有同步任务正在执行，请稍后重试",
            status_code=409,
            retryable=True,
        )
    return account_lock


def _check_direct_sync_rate_limit(*, token: str, account_id: str, app_id: str) -> None:
    now = time.monotonic()
    key = (token, account_id, app_id)
    with _direct_sync_guard_lock:
        minute_events = _direct_sync_minute_events.setdefault(key, deque())
        hour_events = _direct_sync_hour_events.setdefault(key, deque())
        _drop_old_events(minute_events, now=now, window_seconds=60)
        _drop_old_events(hour_events, now=now, window_seconds=3600)

        waits: list[float] = []
        if len(minute_events) >= DIRECT_SYNC_RATE_LIMIT_PER_MINUTE:
            waits.append(60 - (now - minute_events[0]))
        if len(hour_events) >= DIRECT_SYNC_RATE_LIMIT_PER_HOUR:
            waits.append(3600 - (now - hour_events[0]))
        if waits:
            retry_after = max(1, int(max(waits)) + 1)
            raise ApiError(
                "rate_limited",
                "同步请求过于频繁，请稍后重试",
                status_code=429,
                retryable=True,
                extra={"retryAfterSeconds": retry_after},
            )

        minute_events.append(now)
        hour_events.append(now)


def _drop_old_events(events: deque[float], *, now: float, window_seconds: float) -> None:
    while events and now - events[0] >= window_seconds:
        events.popleft()


def _direct_sync_cached_response(
    *,
    token: str,
    account_id: str,
    app_id: str,
    endpoint: str,
    idempotency_key: str,
) -> DirectSyncResponse | None:
    normalized_key = idempotency_key.strip()
    if not normalized_key:
        return None
    cache_key = (token, account_id, app_id, endpoint, normalized_key)
    now = time.monotonic()
    with _direct_sync_guard_lock:
        cached = _direct_sync_idempotency_cache.get(cache_key)
        if cached is None:
            return None
        expires_at, body = cached
        if expires_at <= now:
            _direct_sync_idempotency_cache.pop(cache_key, None)
            return None
        response_body = dict(body)
    response_body["idempotent"] = True
    return DirectSyncResponse.model_validate(response_body)


def _store_direct_sync_idempotency_response(
    *,
    token: str,
    account_id: str,
    app_id: str,
    endpoint: str,
    idempotency_key: str,
    response: DirectSyncResponse,
) -> None:
    normalized_key = idempotency_key.strip()
    if not normalized_key:
        return
    cache_key = (token, account_id, app_id, endpoint, normalized_key)
    expires_at = time.monotonic() + DIRECT_SYNC_IDEMPOTENCY_TTL_SECONDS
    body = response.model_dump(by_alias=True)
    with _direct_sync_guard_lock:
        _direct_sync_idempotency_cache[cache_key] = (expires_at, body)


def _product_page_optimization_cached_response(
    *,
    token: str,
    account_id: str,
    app_id: str,
    idempotency_key: str,
) -> ProductPageOptimizationCreateResponse | None:
    normalized_key = idempotency_key.strip()
    if not normalized_key:
        return None
    cache_key = (token, account_id, app_id, "product-page-optimization:create", normalized_key)
    now = time.monotonic()
    with _direct_sync_guard_lock:
        cached = _direct_sync_idempotency_cache.get(cache_key)
        if cached is None:
            return None
        expires_at, body = cached
        if expires_at <= now:
            _direct_sync_idempotency_cache.pop(cache_key, None)
            return None
        response_body = dict(body)
    response_body["idempotent"] = True
    return ProductPageOptimizationCreateResponse.model_validate(response_body)


def _store_product_page_optimization_idempotency_response(
    *,
    token: str,
    account_id: str,
    app_id: str,
    idempotency_key: str,
    response: ProductPageOptimizationCreateResponse,
) -> None:
    normalized_key = idempotency_key.strip()
    if not normalized_key:
        return
    cache_key = (token, account_id, app_id, "product-page-optimization:create", normalized_key)
    expires_at = time.monotonic() + DIRECT_SYNC_IDEMPOTENCY_TTL_SECONDS
    body = response.model_dump(by_alias=True)
    with _direct_sync_guard_lock:
        _direct_sync_idempotency_cache[cache_key] = (expires_at, body)


def _direct_sync_response(
    *,
    account_id: str,
    app_id: str,
    version: str,
    scopes: list[str],
    locales: list[str],
    runs: list[object],
    page_id: str | None = None,
) -> DirectSyncResponse:
    results = [
        DirectSyncRunResult(
            runId=str(run.id),
            operation=str(run.operation),
            locale=str(run.locale),
            status=str(run.status),
            errorCode=run.error_code,
            errorSummary=run.error_summary,
        )
        for run in runs
    ]
    ok = all(result.status == "succeeded" for result in results)
    return DirectSyncResponse(
        status="succeeded" if ok else "completed_with_failures",
        accountId=account_id,
        appId=app_id,
        version=version,
        pageId=page_id,
        scopes=scopes,
        locales=locales,
        runs=results,
    )


def _direct_sync_store_target(
    payload: StoreDirectSyncRequest | MarketingDirectSyncRequest,
    *,
    platform: str,
) -> tuple[str | None, str | None]:
    ios_app_id = payload.ios_app_id.strip()
    package_name = (
        payload.package_name.strip() if isinstance(payload, StoreDirectSyncRequest) else ""
    )
    if platform == "ios":
        return ios_app_id or None, None
    if platform == "android":
        return None, package_name or None
    return None, None


def _query_store_target(
    *,
    platform: str,
    ios_app_id: str = "",
    store_app_id: str = "",
    apple_app_id: str = "",
    package_name: str = "",
) -> tuple[str | None, str | None]:
    target_store_app_id = _first_non_empty(ios_app_id, store_app_id, apple_app_id)
    target_package_name = package_name.strip()
    if platform == "ios":
        return target_store_app_id, None
    if platform == "android":
        return None, target_package_name or None
    return None, None


def _first_non_empty(*values: str) -> str | None:
    for value in values:
        normalized = str(value or "").strip()
        if normalized:
            return normalized
    return None


def _store_reviews_connector_query(
    *,
    page_size: int,
    page_token: str,
    start_index: str,
    translation_language: str,
    sort: str,
) -> dict[str, str]:
    query = {"pageSize": str(page_size)}
    if page_token.strip():
        query["pageToken"] = page_token.strip()
    if start_index.strip():
        query["startIndex"] = start_index.strip()
    if translation_language.strip():
        query["translationLanguage"] = translation_language.strip()
    if sort.strip():
        query["sort"] = sort.strip()
    return query


def _store_review_page_size_for_platform(platform: str, page_size: int) -> int:
    if platform == "android":
        return min(page_size, 100)
    return page_size


def _store_review_filters(
    *,
    date_value: str,
    start_date: str,
    end_date: str,
    timezone_name: str,
    locale: str,
    territory: str,
    rating: int | None,
) -> dict[str, object]:
    normalized_date = date_value.strip()
    normalized_start = start_date.strip()
    normalized_end = end_date.strip()
    if normalized_date and (normalized_start or normalized_end):
        raise ApiError(
            "invalid_review_date_filter",
            "date 不能和 startDate/endDate 同时使用",
            status_code=422,
        )
    zone_name = timezone_name.strip() or "Asia/Shanghai"
    try:
        timezone_info = ZoneInfo(zone_name)
    except ZoneInfoNotFoundError as error:
        raise ApiError(
            "invalid_timezone",
            "timezone 必须是有效的 IANA 时区名称，例如 Asia/Shanghai",
            status_code=422,
        ) from error
    start_at: dt.datetime | None = None
    end_at: dt.datetime | None = None
    if normalized_date:
        selected = _parse_review_date(normalized_date, field_name="date")
        start_at = dt.datetime.combine(selected, dt.time.min, timezone_info)
        end_at = dt.datetime.combine(selected, dt.time.max, timezone_info)
    else:
        if normalized_start:
            selected_start = _parse_review_date(normalized_start, field_name="startDate")
            start_at = dt.datetime.combine(selected_start, dt.time.min, timezone_info)
        if normalized_end:
            selected_end = _parse_review_date(normalized_end, field_name="endDate")
            end_at = dt.datetime.combine(selected_end, dt.time.max, timezone_info)
    if start_at is not None and end_at is not None and start_at > end_at:
        raise ApiError(
            "invalid_review_date_filter",
            "startDate 不能晚于 endDate",
            status_code=422,
        )
    return {
        "startAt": start_at,
        "endAt": end_at,
        "timezone": zone_name,
        "locale": locale.strip(),
        "territory": territory.strip(),
        "rating": rating,
    }


def _parse_review_date(value: str, *, field_name: str) -> dt.date:
    try:
        return dt.date.fromisoformat(value)
    except ValueError as error:
        raise ApiError(
            "invalid_review_date_filter",
            f"{field_name} 必须使用 YYYY-MM-DD 格式",
            status_code=422,
        ) from error


def _store_review_matches(review: dict[str, object], filters: dict[str, object]) -> bool:
    rating = filters.get("rating")
    if rating is not None and _int_value(review.get("rating")) != int(rating):
        return False
    locale = str(filters.get("locale") or "").strip()
    if locale and str(review.get("locale") or "").strip().lower() != locale.lower():
        return False
    territory = str(filters.get("territory") or "").strip()
    if territory and str(review.get("territory") or "").strip().lower() != territory.lower():
        return False
    start_at = filters.get("startAt")
    end_at = filters.get("endAt")
    if start_at is not None or end_at is not None:
        timestamp = _review_timestamp(review)
        if timestamp is None:
            return False
        if start_at is not None and timestamp < start_at:
            return False
        if end_at is not None and timestamp > end_at:
            return False
    return True


def _review_timestamp(review: dict[str, object]) -> dt.datetime | None:
    raw_value = str(review.get("createdAt") or review.get("updatedAt") or "").strip()
    if not raw_value:
        return None
    if raw_value.endswith("Z"):
        raw_value = raw_value[:-1] + "+00:00"
    try:
        parsed = dt.datetime.fromisoformat(raw_value)
    except ValueError:
        return None
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=dt.UTC)
    return parsed.astimezone(dt.UTC)


def _int_value(value: object) -> int | None:
    try:
        return int(value)
    except (TypeError, ValueError):
        return None


def _store_review_filter_response(filters: dict[str, object]) -> dict[str, object]:
    start_at = filters.get("startAt")
    end_at = filters.get("endAt")
    return {
        "startAt": start_at.isoformat() if isinstance(start_at, dt.datetime) else "",
        "endAt": end_at.isoformat() if isinstance(end_at, dt.datetime) else "",
        "timezone": filters.get("timezone") or "",
        "locale": filters.get("locale") or "",
        "territory": filters.get("territory") or "",
        "rating": filters.get("rating"),
    }


def _reset_direct_sync_guards_for_tests() -> None:
    with _direct_sync_guard_lock:
        _direct_sync_minute_events.clear()
        _direct_sync_hour_events.clear()
        _direct_sync_idempotency_cache.clear()
        _direct_sync_account_locks.clear()


def _metadata_payload_from_form(form: object) -> MetadataContentSetImport:
    raw_metadata = str(getattr(form, "get", lambda _key, _default=None: None)("metadata") or "")
    if not raw_metadata.strip():
        raise ApiError("missing_metadata", "metadata 字段不能为空", status_code=422)
    try:
        decoded = json.loads(raw_metadata)
        return MetadataContentSetImport.model_validate(decoded)
    except (json.JSONDecodeError, ValidationError) as error:
        raise ApiError(
            "invalid_metadata",
            f"metadata JSON 格式不正确：{error}",
            status_code=422,
        ) from error


async def _store_version_payload_from_request(request: Request) -> StoreVersionDraftImport:
    try:
        decoded = await request.json()
        return StoreVersionDraftImport.model_validate(decoded)
    except (json.JSONDecodeError, ValidationError) as error:
        raise ApiError(
            "invalid_metadata",
            f"metadata JSON 格式不正确：{error}",
            status_code=422,
        ) from error


def _marketing_page_payload_from_form(form: object) -> MarketingPageImport:
    raw_metadata = str(getattr(form, "get", lambda _key, _default=None: None)("metadata") or "")
    if not raw_metadata.strip():
        raise ApiError("missing_metadata", "metadata 字段不能为空", status_code=422)
    try:
        decoded = json.loads(raw_metadata)
        return MarketingPageImport.model_validate(decoded)
    except (json.JSONDecodeError, ValidationError) as error:
        raise ApiError(
            "invalid_metadata",
            f"metadata JSON 格式不正确：{error}",
            status_code=422,
        ) from error


async def _store_image_assets_from_form(
    form: object,
    *,
    request: Request,
    account_id: str,
    app_id: str,
    platform: str,
    version: str | None = None,
    content_set_id: str | None = None,
) -> dict[str, dict[str, list[dict[str, object]]]]:
    assets_by_locale: dict[str, dict[str, list[dict[str, object]]]] = {}
    if not hasattr(form, "multi_items"):
        return assets_by_locale

    for field_name, value in form.multi_items():
        if not isinstance(field_name, str) or not field_name.startswith("storeImageFiles__"):
            continue
        parts = field_name.split("__", 2)
        if len(parts) != 3:
            continue
        slot_key = _normalize_slot_key(parts[1])
        locale = parts[2].strip()
        if slot_key is None or not locale:
            continue
        filename = str(getattr(value, "filename", "") or "").strip()
        if not filename or not hasattr(value, "read"):
            continue
        content = await value.read()
        if not content:
            continue
        content_type = str(getattr(value, "content_type", "") or "application/octet-stream")
        validation = validate_store_image(
            platform=platform,
            slot_key=slot_key,
            filename=filename,
            content_type=content_type,
            content=content,
        )
        if not validation.valid:
            raise ApiError(
                "store_image_invalid",
                f"{locale} {Path(filename).name}: {validation.message}",
                status_code=422,
            )
        folder = _store_image_storage_folder(
            account_id=account_id,
            app_id=app_id,
            version=str(version or ""),
            content_set_id=str(content_set_id or DEFAULT_CONTENT_SET_ID),
            locale=locale,
            slot_key=slot_key,
        )
        stored = request.app.state.artifact_storage.save(
            folder,
            filename,
            content,
            content_type=content_type,
        )
        assets_by_locale.setdefault(locale, {}).setdefault(slot_key, []).append(
            {
                "fileName": Path(filename).name,
                "contentType": content_type,
                "sizeBytes": len(content),
                "storageKey": stored.storage_key,
                "downloadUrl": stored.download_url,
                "width": validation.image.width if validation.image else None,
                "height": validation.image.height if validation.image else None,
                "format": validation.image.format if validation.image else None,
                "validationMessage": validation.message,
                "matchedLabel": validation.matched_label,
            }
        )
    return assets_by_locale


def _version_draft_rows(payload: StoreVersionDraftImport) -> list[dict[str, object]]:
    locale_inputs = [item for item in payload.locales if item.locale.strip()]
    if not locale_inputs:
        raise ApiError("invalid_metadata", "locales 至少需要包含一个语言", status_code=422)
    source_locale = payload.source_locale.strip() or locale_inputs[0].locale.strip()
    source = next(
        (item for item in locale_inputs if item.locale.strip() == source_locale),
        locale_inputs[0],
    )
    source_has_metadata = bool(
        source.keywords.strip() or source.promotional_text.strip() or source.description.strip()
    )
    rows: list[dict[str, object]] = []
    seen: set[str] = set()
    for item in locale_inputs:
        locale = item.locale.strip()
        if locale in seen:
            continue
        seen.add(locale)
        description = item.description.strip() or source.description.strip()
        save_metadata = bool(
            source_has_metadata
            or item.keywords.strip()
            or item.promotional_text.strip()
            or item.description.strip()
        )
        if save_metadata and not description:
            raise ApiError(
                "invalid_metadata",
                f"{locale} 的 description 不能为空",
                status_code=422,
            )
        rows.append(
            {
                "locale": locale,
                "save_metadata": save_metadata,
                "keywords": item.keywords.strip() or source.keywords.strip(),
                "promotional_text": (
                    item.promotional_text.strip() or source.promotional_text.strip()
                ),
                "description": description,
                "release_notes": item.release_notes.strip() or source.release_notes.strip(),
            }
        )
    return rows


def _marketing_page_rows(
    payload: MarketingPageImport,
    uploaded_assets: dict[str, dict[str, list[dict[str, object]]]],
) -> list[dict[str, object]]:
    locale_inputs = {item.locale.strip(): item for item in payload.locales if item.locale.strip()}
    locales = _unique_non_empty(
        [
            *locale_inputs.keys(),
            *uploaded_assets.keys(),
            payload.source_locale,
        ]
    )
    if not locales:
        raise ApiError("invalid_metadata", "locales 至少需要包含一个语言", status_code=422)
    source_locale = payload.source_locale.strip() or locales[0]
    source = locale_inputs.get(source_locale) or locale_inputs.get(locales[0])
    source_text = source.promotional_text.strip() if source is not None else ""
    source_images = _marketing_images_for_locale(payload, source_locale, source, uploaded_assets)
    rows: list[dict[str, object]] = []
    for locale in locales:
        item = locale_inputs.get(locale)
        promotional_text = (
            item.promotional_text.strip() if item is not None else ""
        ) or source_text
        rows.append(
            {
                "locale": locale,
                "promotional_text": promotional_text,
                "store_images": _merge_store_images(
                    _marketing_images_for_locale(payload, locale, item, uploaded_assets),
                    source_images,
                ),
            }
        )
    return rows


def _metadata_rows(
    payload: MetadataContentSetImport,
    uploaded_assets: dict[str, dict[str, list[dict[str, object]]]],
) -> list[dict[str, object]]:
    locale_inputs = [item for item in payload.locales if item.locale.strip()]
    if not locale_inputs:
        raise ApiError("invalid_metadata", "locales 至少需要包含一个语言", status_code=422)
    source_locale = payload.source_locale.strip() or locale_inputs[0].locale.strip()
    source = next(
        (item for item in locale_inputs if item.locale.strip() == source_locale),
        locale_inputs[0],
    )
    source_images = _images_for_locale(payload, source, uploaded_assets)
    rows: list[dict[str, object]] = []
    seen: set[str] = set()
    for item in locale_inputs:
        locale = item.locale.strip()
        if locale in seen:
            continue
        seen.add(locale)
        description = item.description.strip() or source.description.strip()
        if not description:
            raise ApiError(
                "invalid_metadata",
                f"{locale} 的 description 不能为空",
                status_code=422,
            )
        rows.append(
            {
                "locale": locale,
                "keywords": item.keywords.strip() or source.keywords.strip(),
                "promotional_text": (
                    item.promotional_text.strip() or source.promotional_text.strip()
                ),
                "description": description,
                "store_images": _merge_store_images(
                    _images_for_locale(payload, item, uploaded_assets),
                    source_images,
                ),
            }
        )
    return rows


def _images_for_locale(
    payload: MetadataContentSetImport,
    item: LocaleMetadataInput,
    uploaded_assets: dict[str, dict[str, list[dict[str, object]]]],
) -> dict[str, object]:
    locale = item.locale.strip()
    merged = _normalize_store_image_payload(_top_level_images_for_locale(payload, locale))
    merged.update(_normalize_store_image_payload(item.store_images))
    for slot_key, assets in uploaded_assets.get(locale, {}).items():
        slot = dict(merged.get(slot_key) if isinstance(merged.get(slot_key), dict) else {})
        slot["urls"] = _string_list(slot.get("urls"))
        slot["assets"] = _asset_list(slot.get("assets")) + assets
        merged[slot_key] = slot
    return merged


def _top_level_images_for_locale(
    payload: MetadataContentSetImport,
    locale: str,
) -> dict[str, object]:
    raw_images = payload.store_images
    locale_images = raw_images.get(locale)
    if isinstance(locale_images, dict):
        return locale_images
    if any(_normalize_slot_key(key) for key in raw_images):
        return raw_images
    return {}


def _marketing_images_for_locale(
    payload: MarketingPageImport,
    locale: str,
    item: MarketingPageLocaleInput | None,
    uploaded_assets: dict[str, dict[str, list[dict[str, object]]]],
) -> dict[str, object]:
    merged = _normalize_store_image_payload(_marketing_top_level_images_for_locale(payload, locale))
    if item is not None:
        merged.update(_normalize_store_image_payload(item.store_images))
    for slot_key, assets in uploaded_assets.get(locale, {}).items():
        slot = dict(merged.get(slot_key) if isinstance(merged.get(slot_key), dict) else {})
        slot["urls"] = _string_list(slot.get("urls"))
        slot["assets"] = _asset_list(slot.get("assets")) + assets
        merged[slot_key] = slot
    return merged


def _marketing_top_level_images_for_locale(
    payload: MarketingPageImport,
    locale: str,
) -> dict[str, object]:
    raw_images = payload.store_images
    locale_images = raw_images.get(locale)
    if isinstance(locale_images, dict):
        return locale_images
    if any(_normalize_slot_key(key) for key in raw_images):
        return raw_images
    return {}


def _normalize_store_image_payload(raw_images: dict[str, object] | None) -> dict[str, object]:
    normalized: dict[str, object] = {}
    for raw_key, raw_value in (raw_images or {}).items():
        slot_key = _normalize_slot_key(raw_key)
        if slot_key is None:
            continue
        urls = raw_value.get("urls") if isinstance(raw_value, dict) else raw_value
        assets = raw_value.get("assets") if isinstance(raw_value, dict) else None
        normalized[slot_key] = {
            "urls": _string_list(urls),
            "assets": _asset_list(assets),
        }
    return normalized


def _merge_store_images(
    images: dict[str, object],
    fallback: dict[str, object],
) -> dict[str, object]:
    merged: dict[str, object] = {}
    for slot_key in STORE_IMAGE_SLOT_KEYS:
        current = images.get(slot_key)
        merged[slot_key] = (
            current if _has_store_image_value(current) else fallback.get(slot_key, {})
        )
    return merged


def _has_store_image_value(value: object) -> bool:
    if not isinstance(value, dict):
        return bool(_string_list(value))
    return bool(_string_list(value.get("urls")) or _asset_list(value.get("assets")))


def _normalize_slot_key(raw_key: object) -> str | None:
    return STORE_IMAGE_SLOT_ALIASES.get(str(raw_key or "").strip())


def _unique_non_empty(values: list[str] | tuple[str, ...]) -> list[str]:
    result: list[str] = []
    for value in values:
        normalized = str(value or "").strip()
        if normalized and normalized not in result:
            result.append(normalized)
    return result


def _string_list(value: object) -> list[str]:
    if isinstance(value, list | tuple):
        raw_values = [str(item or "") for item in value]
    else:
        raw_values = str(value or "").splitlines()
    return [item.strip() for item in raw_values if item.strip()]


def _asset_list(value: object) -> list[dict[str, object]]:
    if not isinstance(value, list | tuple):
        return []
    return [item for item in value if isinstance(item, dict)]


def _store_image_storage_folder(
    *,
    account_id: str,
    app_id: str,
    version: str,
    content_set_id: str,
    locale: str,
    slot_key: str,
) -> str:
    return "/".join(
        [
            "store-assets",
            _safe_storage_part(account_id),
            _safe_storage_part(app_id),
            _safe_storage_part(content_set_id),
            _safe_storage_part(version),
            _safe_storage_part(locale),
            _safe_storage_part(slot_key),
        ]
    )


def _uploaded_asset_count(
    uploaded_assets: dict[str, dict[str, list[dict[str, object]]]],
) -> int:
    return sum(len(assets) for slots in uploaded_assets.values() for assets in slots.values())


def _safe_storage_part(value: str) -> str:
    normalized = re.sub(r"[^A-Za-z0-9._-]+", "-", value.strip())
    return normalized.strip("-") or "default"
