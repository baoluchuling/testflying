from __future__ import annotations

import hashlib
import json
import re
from dataclasses import dataclass
from datetime import UTC, datetime, timedelta
from typing import Any
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen
from uuid import uuid4

from sqlalchemy import func, select
from sqlalchemy.orm import Session

from testflying_api.config import Settings
from testflying_api.errors import ApiError
from testflying_api.llm_config import (
    LLM_FEATURE_REVIEW_ANALYSIS,
    LlmRuntimeConfig,
    resolve_llm_runtime_config,
)
from testflying_api.schema import (
    App,
    DeveloperAccount,
    StoreConnector,
    StoreReview,
    StoreReviewAnalysisRun,
    StoreReviewFetchRun,
)
from testflying_api.store_sync import StoreConnectorClient, account_connector, scoped_app

DEFAULT_REVIEW_PAGE_SIZE = 20
DEFAULT_REVIEW_ANALYSIS_LIMIT = 120
DEFAULT_REVIEW_ANALYSIS_DAYS = 30
MAX_REVIEW_FETCH_PAGES = 10
REVIEW_ANALYSIS_SEVERITY_ORDER = {"high": 0, "medium": 1, "low": 2}

REVIEW_ANALYSIS_SCHEMA_INSTRUCTION = (
    "你是面向内部产品、QA 和研发团队的商店评论分析助手。"
    "所有输出必须使用简体中文。只返回 JSON，不要返回 Markdown、解释文字或代码块。"
    "JSON 结构必须是："
    '{"summary":"80字以内中文摘要",'
    '"issues":[{"title":"中文短标题",'
    '"severity":"low|medium|high",'
    '"count":1,'
    '"focus":"一句话说明需要关注的问题",'
    '"evidence":["评论原文中的关键证据片段"],'
    '"suggestion":"建议人工排查或优化的下一步",'
    '"representativeReviewIds":["评论ID"]}]}. '
    "不要回复用户评论，不要建议自动修改商店内容。"
    "重点关注产品缺陷、体验阻塞、回归风险、性能问题、登录/支付/播放/加载等可行动问题。"
)


@dataclass(frozen=True)
class StoreReviewFetchResult:
    run: StoreReviewFetchRun
    inserted_count: int
    fetched_count: int
    duplicate_count: int
    page_count: int
    stopped_reason: str


def store_reviews_context(
    session: Session,
    *,
    account_id: str = "",
    app_id: str = "",
    rating: int | None = None,
) -> dict[str, object]:
    review_apps = _review_apps(session)
    selected = _selected_review_app(session, review_apps, account_id=account_id, app_id=app_id)
    selected_account = selected["account"] if selected else None
    selected_app = selected["app"] if selected else None
    reviews: list[StoreReview] = []
    latest_fetch: StoreReviewFetchRun | None = None
    latest_analysis: StoreReviewAnalysisRun | None = None
    analysis_issues: list[dict[str, object]] = []
    stats = {"total": 0, "low": 0, "ios": 0, "android": 0}

    if selected_account and selected_app:
        reviews = list_recent_reviews(
            session,
            account_id=selected_account.id,
            app_id=selected_app.id,
            rating=rating,
            limit=100,
        )
        latest_fetch = latest_review_fetch_run(
            session,
            account_id=selected_account.id,
            app_id=selected_app.id,
        )
        latest_analysis = latest_review_analysis_run(
            session,
            account_id=selected_account.id,
            app_id=selected_app.id,
        )
        analysis_issues = _analysis_issues(latest_analysis)
        stats = review_stats(session, account_id=selected_account.id, app_id=selected_app.id)

    return {
        "review_apps": review_apps,
        "selected_review_app": selected_app,
        "selected_review_account": selected_account,
        "reviews": reviews,
        "review_rating": rating,
        "review_stats": stats,
        "latest_review_fetch": latest_fetch,
        "latest_review_analysis": latest_analysis,
        "analysis_issues": analysis_issues,
        "analysis_boundaries": [
            "只分析问题和优化关注点，不回复评论。",
            "不自动修改商店文案、截图或版本说明。",
            "初始无历史评论时只拉取第一页 20 条。",
            "有历史后遇到已存在且创建日期一致的评论就停止翻页。",
        ],
    }


def fetch_store_reviews_incremental(
    session: Session,
    *,
    account_id: str,
    app_id: str,
    page_size: int = DEFAULT_REVIEW_PAGE_SIZE,
    max_pages: int = MAX_REVIEW_FETCH_PAGES,
    client: StoreConnectorClient | None = None,
) -> StoreReviewFetchResult:
    account, app, connector = _review_scope(session, account_id=account_id, app_id=app_id)
    started_at = datetime.now(UTC)
    existing_total = (
        session.scalar(
            select(func.count(StoreReview.id)).where(
                StoreReview.developer_account_id == account.id,
                StoreReview.app_id == app.id,
            )
        )
        or 0
    )
    # 第一次只拉最近一页，避免把历史评论一次性灌满本地库。
    effective_max_pages = 1 if existing_total == 0 else max(1, max_pages)
    effective_page_size = _clamp_page_size(page_size)
    connector_client = client or StoreConnectorClient()
    page_token = ""
    page_count = 0
    fetched_count = 0
    inserted_count = 0
    duplicate_count = 0
    stopped_reason = "no_more_pages"

    try:
        for _ in range(effective_max_pages):
            page_count += 1
            raw_response = connector_client.store_reviews(
                connector,
                account_id=account.id,
                app=app,
                store_app_id=app.store_app_id,
                package_name=app.store_package_name or app.bundle_identifier,
                store_query=_store_review_fetch_query(
                    page_size=effective_page_size,
                    page_token=page_token,
                ),
            )
            raw_reviews = _raw_reviews(raw_response)
            if not raw_reviews:
                stopped_reason = "empty_page"
                break

            page_has_existing_boundary = False
            for raw_review in raw_reviews:
                fetched_count += 1
                normalized = _normalize_store_review(
                    raw_review,
                    account_id=account.id,
                    app_id=app.id,
                    platform=app.platform,
                )
                existing = _existing_review(session, normalized)
                if existing is not None:
                    duplicate_count += 1
                    if _same_created_day(existing.created_at, normalized["created_at"]):
                        page_has_existing_boundary = True
                        break
                    continue

                session.add(StoreReview(**normalized))
                inserted_count += 1

            session.flush()
            if page_has_existing_boundary:
                stopped_reason = "existing_review_same_created_date"
                break

            page_token = str(raw_response.get("nextPageToken") or "").strip()
            if not page_token:
                stopped_reason = "no_more_pages"
                break
        else:
            stopped_reason = "initial_page_only" if existing_total == 0 else "max_pages"

        run = StoreReviewFetchRun(
            id=f"review-fetch-{uuid4().hex}",
            developer_account_id=account.id,
            app_id=app.id,
            connector_id=connector.id,
            platform=app.platform,
            status="succeeded",
            page_count=page_count,
            fetched_count=fetched_count,
            inserted_count=inserted_count,
            duplicate_count=duplicate_count,
            stopped_reason=stopped_reason,
            filters_json={
                "pageSize": effective_page_size,
                "maxPages": effective_max_pages,
                "existingTotalBeforeFetch": existing_total,
            },
            started_at=started_at,
            finished_at=datetime.now(UTC),
        )
        session.add(run)
        session.flush()
    except ApiError as error:
        run = StoreReviewFetchRun(
            id=f"review-fetch-{uuid4().hex}",
            developer_account_id=account.id,
            app_id=app.id,
            connector_id=connector.id,
            platform=app.platform,
            status="failed",
            page_count=page_count,
            fetched_count=fetched_count,
            inserted_count=inserted_count,
            duplicate_count=duplicate_count,
            stopped_reason="failed",
            filters_json={"pageSize": effective_page_size, "maxPages": effective_max_pages},
            started_at=started_at,
            finished_at=datetime.now(UTC),
            error_code=error.code,
            error_summary=error.message,
        )
        session.add(run)
        session.flush()
        raise

    return StoreReviewFetchResult(
        run=run,
        inserted_count=inserted_count,
        fetched_count=fetched_count,
        duplicate_count=duplicate_count,
        page_count=page_count,
        stopped_reason=stopped_reason,
    )


def analyze_store_reviews(
    session: Session,
    settings: Settings,
    *,
    account_id: str,
    app_id: str,
    limit: int = DEFAULT_REVIEW_ANALYSIS_LIMIT,
    recent_days: int = DEFAULT_REVIEW_ANALYSIS_DAYS,
) -> StoreReviewAnalysisRun:
    account, app, _connector = _review_scope(session, account_id=account_id, app_id=app_id)
    since = datetime.now(UTC) - timedelta(days=max(1, recent_days))
    reviews = list_recent_reviews(
        session,
        account_id=account.id,
        app_id=app.id,
        since=since,
        limit=max(1, limit),
    )
    if not reviews:
        raise ApiError("no_reviews_to_analyze", "当前应用还没有可分析的商店评论", status_code=422)
    started_at = datetime.now(UTC)
    low_rating_count = sum(1 for review in reviews if (review.rating or 0) <= 3)
    try:
        analysis = _analyze_reviews_with_provider(
            session,
            settings,
            app=app,
            reviews=reviews,
        )
    except ApiError as error:
        run = StoreReviewAnalysisRun(
            id=f"review-analysis-{uuid4().hex}",
            developer_account_id=account.id,
            app_id=app.id,
            platform=app.platform,
            status="failed",
            review_count=len(reviews),
            low_rating_count=low_rating_count,
            issue_count=0,
            summary="",
            analysis_json={},
            started_at=started_at,
            finished_at=datetime.now(UTC),
            error_code=error.code,
            error_summary=error.message,
        )
        session.add(run)
        session.flush()
        raise

    issues = analysis.get("issues") if isinstance(analysis, dict) else []
    issue_count = len(issues) if isinstance(issues, list) else 0
    run = StoreReviewAnalysisRun(
        id=f"review-analysis-{uuid4().hex}",
        developer_account_id=account.id,
        app_id=app.id,
        platform=app.platform,
        status="succeeded",
        review_count=len(reviews),
        low_rating_count=low_rating_count,
        issue_count=issue_count,
        summary=str(analysis.get("summary") or "").strip(),
        analysis_json=analysis,
        started_at=started_at,
        finished_at=datetime.now(UTC),
    )
    session.add(run)
    session.flush()
    return run


def list_recent_reviews(
    session: Session,
    *,
    account_id: str,
    app_id: str,
    rating: int | None = None,
    since: datetime | None = None,
    limit: int = 100,
) -> list[StoreReview]:
    statement = select(StoreReview).where(
        StoreReview.developer_account_id == account_id,
        StoreReview.app_id == app_id,
    )
    if rating is not None:
        statement = statement.where(StoreReview.rating == rating)
    if since is not None:
        statement = statement.where(StoreReview.created_at >= since)
    return list(
        session.scalars(
            statement.order_by(
                StoreReview.created_at.desc(),
                StoreReview.fetched_at.desc(),
            ).limit(limit)
        )
    )


def latest_review_fetch_run(
    session: Session,
    *,
    account_id: str,
    app_id: str,
) -> StoreReviewFetchRun | None:
    return session.scalar(
        select(StoreReviewFetchRun)
        .where(
            StoreReviewFetchRun.developer_account_id == account_id,
            StoreReviewFetchRun.app_id == app_id,
        )
        .order_by(StoreReviewFetchRun.started_at.desc())
        .limit(1)
    )


def latest_review_analysis_run(
    session: Session,
    *,
    account_id: str,
    app_id: str,
) -> StoreReviewAnalysisRun | None:
    return session.scalar(
        select(StoreReviewAnalysisRun)
        .where(
            StoreReviewAnalysisRun.developer_account_id == account_id,
            StoreReviewAnalysisRun.app_id == app_id,
        )
        .order_by(StoreReviewAnalysisRun.started_at.desc())
        .limit(1)
    )


def review_stats(session: Session, *, account_id: str, app_id: str) -> dict[str, int]:
    rows = list(
        session.execute(
            select(StoreReview.platform, StoreReview.rating, func.count(StoreReview.id))
            .where(
                StoreReview.developer_account_id == account_id,
                StoreReview.app_id == app_id,
            )
            .group_by(StoreReview.platform, StoreReview.rating)
        )
    )
    stats = {"total": 0, "low": 0, "ios": 0, "android": 0}
    for platform, rating, count in rows:
        value = int(count or 0)
        stats["total"] += value
        if platform in {"ios", "android"}:
            stats[platform] += value
        if rating is not None and int(rating) <= 3:
            stats["low"] += value
    return stats


def _review_scope(
    session: Session,
    *,
    account_id: str,
    app_id: str,
) -> tuple[DeveloperAccount, App, StoreConnector]:
    account = session.get(DeveloperAccount, account_id)
    if account is None:
        raise ApiError("developer_account_not_found", "开发者账号不存在", status_code=404)
    app = scoped_app(session, account_id, app_id)
    if app is None:
        raise ApiError("app_not_found", "应用不存在或未绑定到当前开发者账号", status_code=404)
    connector = account_connector(session, account_id)
    if connector is None:
        raise ApiError(
            "connector_not_configured",
            "当前开发者账号还没有配置 Connector",
            status_code=422,
        )
    return account, app, connector


def _review_apps(session: Session) -> list[dict[str, object]]:
    apps = list(
        session.scalars(
            select(App)
            .where(App.developer_account_id.is_not(None))
            .order_by(App.added_at.desc(), App.name.asc())
        )
    )
    review_counts = {
        (account_id, app_id): int(count or 0)
        for account_id, app_id, count in session.execute(
            select(
                StoreReview.developer_account_id, StoreReview.app_id, func.count(StoreReview.id)
            ).group_by(StoreReview.developer_account_id, StoreReview.app_id)
        )
    }
    items: list[dict[str, object]] = []
    for app in apps:
        if not app.developer_account_id:
            continue
        account = session.get(DeveloperAccount, app.developer_account_id)
        if account is None:
            continue
        items.append(
            {
                "app": app,
                "account": account,
                "review_count": review_counts.get((account.id, app.id), 0),
            }
        )
    return items


def _selected_review_app(
    session: Session,
    review_apps: list[dict[str, object]],
    *,
    account_id: str,
    app_id: str,
) -> dict[str, object] | None:
    if account_id and app_id:
        app = scoped_app(session, account_id, app_id)
        account = session.get(DeveloperAccount, account_id)
        if app and account:
            return {
                "app": app,
                "account": account,
                "review_count": session.scalar(
                    select(func.count(StoreReview.id)).where(
                        StoreReview.developer_account_id == account.id,
                        StoreReview.app_id == app.id,
                    )
                )
                or 0,
            }
    return review_apps[0] if review_apps else None


def _store_review_fetch_query(*, page_size: int, page_token: str) -> dict[str, str]:
    query = {"pageSize": str(page_size), "sort": "-createdDate"}
    if page_token:
        query["pageToken"] = page_token
    return query


def _raw_reviews(raw_response: dict[str, object]) -> list[dict[str, object]]:
    raw_reviews = raw_response.get("reviews")
    if not isinstance(raw_reviews, list):
        return []
    return [item for item in raw_reviews if isinstance(item, dict)]


def _normalize_store_review(
    raw_review: dict[str, object],
    *,
    account_id: str,
    app_id: str,
    platform: str,
) -> dict[str, Any]:
    created_at = _parse_review_datetime(raw_review.get("createdAt")) or datetime.now(UTC)
    updated_at = _parse_review_datetime(raw_review.get("updatedAt"))
    store_review_id = _store_review_id(raw_review, app_id=app_id, created_at=created_at)
    return {
        "id": f"store-review-{uuid4().hex}",
        "developer_account_id": account_id,
        "app_id": app_id,
        "platform": platform,
        "store_review_id": store_review_id,
        "rating": _safe_int(raw_review.get("rating")),
        "title": _text(raw_review.get("title"))[:240],
        "body": _text(raw_review.get("body")),
        "author_name": (
            _text(raw_review.get("authorName")) or _text(raw_review.get("reviewerNickname"))
        )[:180],
        "locale": _text(raw_review.get("locale"))[:40],
        "territory": _text(raw_review.get("territory"))[:40],
        "app_version": _text(raw_review.get("appVersion"))[:80],
        "created_at": created_at,
        "updated_at": updated_at,
        "fetched_at": datetime.now(UTC),
        "raw_json": raw_review,
    }


def _store_review_id(raw_review: dict[str, object], *, app_id: str, created_at: datetime) -> str:
    explicit_id = _text(raw_review.get("id"))
    if explicit_id:
        return explicit_id[:180]
    identity = json.dumps(
        {
            "appId": app_id,
            "createdAt": created_at.isoformat(),
            "rating": raw_review.get("rating"),
            "title": raw_review.get("title"),
            "body": raw_review.get("body"),
            "author": raw_review.get("authorName") or raw_review.get("reviewerNickname"),
        },
        ensure_ascii=False,
        sort_keys=True,
    )
    return hashlib.sha256(identity.encode("utf-8")).hexdigest()


def _existing_review(session: Session, normalized: dict[str, Any]) -> StoreReview | None:
    return session.scalar(
        select(StoreReview)
        .where(
            StoreReview.developer_account_id == normalized["developer_account_id"],
            StoreReview.app_id == normalized["app_id"],
            StoreReview.platform == normalized["platform"],
            StoreReview.store_review_id == normalized["store_review_id"],
        )
        .limit(1)
    )


def _same_created_day(left: datetime, right: datetime) -> bool:
    return _as_utc(left).date() == _as_utc(right).date()


def _as_utc(value: datetime) -> datetime:
    if value.tzinfo is None:
        return value.replace(tzinfo=UTC)
    return value.astimezone(UTC)


def _parse_review_datetime(value: object) -> datetime | None:
    if value is None:
        return None
    raw_value = str(value).strip()
    if not raw_value:
        return None
    if raw_value.endswith("Z"):
        raw_value = raw_value[:-1] + "+00:00"
    try:
        parsed = datetime.fromisoformat(raw_value)
    except ValueError:
        return None
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=UTC)
    return parsed.astimezone(UTC)


def _clamp_page_size(page_size: int) -> int:
    return min(100, max(1, page_size))


def _safe_int(value: object) -> int | None:
    try:
        return int(value)
    except (TypeError, ValueError):
        return None


def _text(value: object) -> str:
    return str(value or "").strip()


def _analysis_issues(latest_analysis: StoreReviewAnalysisRun | None) -> list[dict[str, object]]:
    if latest_analysis is None or latest_analysis.status != "succeeded":
        return []
    issues = latest_analysis.analysis_json.get("issues") if latest_analysis.analysis_json else []
    if not isinstance(issues, list):
        return []
    return _sort_review_analysis_issues([issue for issue in issues if isinstance(issue, dict)])


def _decode_review_analysis_content(content: str) -> dict[str, object]:
    for candidate in _review_analysis_json_candidates(content):
        try:
            decoded: object = json.loads(candidate)
            if isinstance(decoded, str):
                decoded = json.loads(decoded)
        except json.JSONDecodeError:
            continue
        if isinstance(decoded, dict):
            return _normalize_review_analysis(decoded)
    raise ValueError("review analysis response is not a JSON object")


def _review_analysis_json_candidates(content: str) -> list[str]:
    stripped = content.strip()
    candidates = [stripped] if stripped else []
    fenced_matches = re.findall(
        r"```(?:json)?\s*(.*?)```", stripped, flags=re.IGNORECASE | re.DOTALL
    )
    candidates.extend(match.strip() for match in fenced_matches if match.strip())
    start = stripped.find("{")
    end = stripped.rfind("}")
    if start >= 0 and end > start:
        candidates.append(stripped[start : end + 1])
    return candidates


def _normalize_review_analysis(analysis: dict[str, object]) -> dict[str, object]:
    raw_issues = analysis.get("issues")
    issues: list[dict[str, object]] = []
    if isinstance(raw_issues, list):
        for raw_issue in raw_issues:
            if not isinstance(raw_issue, dict):
                continue
            severity = str(raw_issue.get("severity") or "medium").strip().lower()
            if severity not in REVIEW_ANALYSIS_SEVERITY_ORDER:
                severity = "medium"
            representative = raw_issue.get("representativeReviewIds") or raw_issue.get(
                "representative_review_ids"
            )
            issues.append(
                {
                    "title": _text(raw_issue.get("title")) or "未命名关注点",
                    "severity": severity,
                    "count": _safe_int(raw_issue.get("count")) or 0,
                    "focus": _text(raw_issue.get("focus")) or "需要人工确认影响范围。",
                    "evidence": _string_list(raw_issue.get("evidence")),
                    "suggestion": _text(raw_issue.get("suggestion"))
                    or "请结合代表评论和版本信息进一步确认。",
                    "representativeReviewIds": _string_list(representative),
                }
            )
    return {
        "summary": _text(analysis.get("summary")),
        "issues": _sort_review_analysis_issues(issues),
    }


def _sort_review_analysis_issues(issues: list[dict[str, object]]) -> list[dict[str, object]]:
    return sorted(
        issues,
        key=lambda issue: (
            REVIEW_ANALYSIS_SEVERITY_ORDER.get(str(issue.get("severity") or "").lower(), 1),
            -(_safe_int(issue.get("count")) or 0),
            _text(issue.get("title")),
        ),
    )


def _string_list(value: object) -> list[str]:
    if isinstance(value, list):
        return [_text(item) for item in value if _text(item)]
    if isinstance(value, str) and value.strip():
        return [value.strip()]
    return []


def _analyze_reviews_with_provider(
    session: Session,
    settings: Settings,
    *,
    app: App,
    reviews: list[StoreReview],
) -> dict[str, object]:
    runtime = resolve_llm_runtime_config(
        session,
        settings,
        feature_key=LLM_FEATURE_REVIEW_ANALYSIS,
    )
    provider = runtime.provider.strip().lower()
    if provider in {"", "disabled", "none"}:
        raise ApiError(
            "review_analysis_not_configured",
            "评论分析 LLM 服务未配置",
            status_code=503,
        )
    if provider == "mock":
        return _mock_review_analysis(reviews)
    if provider in {"openai", "configured"} and runtime.protocol == "openai_compatible":
        return _review_analysis_with_openai_compatible(runtime, app=app, reviews=reviews)
    if provider == "configured" and runtime.protocol == "claude_compatible":
        return _review_analysis_with_claude_compatible(runtime, app=app, reviews=reviews)
    raise ApiError(
        "unsupported_review_analysis_provider",
        "不支持的评论分析服务配置",
        status_code=422,
    )


def _mock_review_analysis(reviews: list[StoreReview]) -> dict[str, object]:
    low_reviews = [review for review in reviews if (review.rating or 0) <= 3]
    issue_title = "低分评论集中在稳定性和体验问题"
    if low_reviews:
        representative_ids = [review.store_review_id for review in low_reviews[:5]]
        count = len(low_reviews)
    else:
        representative_ids = [review.store_review_id for review in reviews[:3]]
        count = max(1, min(3, len(reviews)))
        issue_title = "当前评论未出现明显低分集中问题"
    return {
        "summary": f"共分析 {len(reviews)} 条评论，发现 {count} 条需要持续关注的反馈。",
        "issues": [
            {
                "title": issue_title,
                "severity": "medium" if low_reviews else "low",
                "count": count,
                "representativeReviewIds": representative_ids,
                "focus": "优先确认评论中反复出现的崩溃、登录、播放或加载类关键词。",
            }
        ],
    }


def _review_analysis_with_openai_compatible(
    runtime: LlmRuntimeConfig,
    *,
    app: App,
    reviews: list[StoreReview],
) -> dict[str, object]:
    if not runtime.api_key:
        raise ApiError(
            "review_analysis_not_configured",
            "评论分析服务缺少 API Key",
            status_code=503,
        )
    payload = {
        "model": runtime.model,
        "temperature": 0.2,
        "response_format": {"type": "json_object"},
        "messages": [
            {
                "role": "system",
                "content": REVIEW_ANALYSIS_SCHEMA_INSTRUCTION,
            },
            {
                "role": "user",
                "content": json.dumps(
                    {
                        "app": {
                            "name": app.name,
                            "platform": app.platform,
                            "bundleIdentifier": app.bundle_identifier,
                        },
                        "reviews": [
                            {
                                "id": review.store_review_id,
                                "rating": review.rating,
                                "title": review.title,
                                "body": review.body,
                                "locale": review.locale,
                                "appVersion": review.app_version,
                                "createdAt": review.created_at.isoformat(),
                            }
                            for review in reviews
                        ],
                    },
                    ensure_ascii=False,
                ),
            },
        ],
    }
    headers = {
        "Content-Type": "application/json",
        "Accept": "application/json",
    }
    headers.update(_auth_headers(runtime))
    request = Request(
        runtime.base_url.rstrip("/") + "/chat/completions",
        data=json.dumps(payload).encode("utf-8"),
        headers=headers,
        method="POST",
    )
    try:
        with urlopen(request, timeout=45) as response:
            response_payload = json.loads(response.read().decode("utf-8"))
    except HTTPError as error:
        raise ApiError(
            "review_analysis_failed",
            f"评论分析服务调用失败（HTTP {error.code}）",
            status_code=502,
        ) from error
    except (TimeoutError, URLError, json.JSONDecodeError) as error:
        raise ApiError(
            "review_analysis_failed",
            "评论分析服务调用失败，请稍后重试",
            status_code=502,
        ) from error
    content = _response_content(response_payload)
    try:
        return _decode_review_analysis_content(content)
    except ValueError as error:
        raise ApiError(
            "review_analysis_failed",
            "评论分析服务返回格式不正确",
            status_code=502,
        ) from error


def _review_analysis_with_claude_compatible(
    runtime: LlmRuntimeConfig,
    *,
    app: App,
    reviews: list[StoreReview],
) -> dict[str, object]:
    if not runtime.api_key:
        raise ApiError(
            "review_analysis_not_configured",
            "评论分析服务缺少 API Key",
            status_code=503,
        )
    payload = {
        "model": runtime.model,
        "max_tokens": 2000,
        "temperature": 0.2,
        "system": REVIEW_ANALYSIS_SCHEMA_INSTRUCTION,
        "messages": [
            {
                "role": "user",
                "content": json.dumps(
                    {
                        "app": {
                            "name": app.name,
                            "platform": app.platform,
                            "bundleIdentifier": app.bundle_identifier,
                        },
                        "reviews": [
                            {
                                "id": review.store_review_id,
                                "rating": review.rating,
                                "title": review.title,
                                "body": review.body,
                                "locale": review.locale,
                                "appVersion": review.app_version,
                                "createdAt": review.created_at.isoformat(),
                            }
                            for review in reviews
                        ],
                    },
                    ensure_ascii=False,
                ),
            }
        ],
    }
    headers = {
        "Content-Type": "application/json",
        "Accept": "application/json",
        "anthropic-version": "2023-06-01",
    }
    headers.update(_auth_headers(runtime))
    request = Request(
        _claude_messages_url(runtime.base_url),
        data=json.dumps(payload).encode("utf-8"),
        headers=headers,
        method="POST",
    )
    try:
        with urlopen(request, timeout=45) as response:
            response_payload = json.loads(response.read().decode("utf-8"))
    except HTTPError as error:
        raise ApiError(
            "review_analysis_failed",
            f"评论分析服务调用失败（HTTP {error.code}）",
            status_code=502,
        ) from error
    except (TimeoutError, URLError, json.JSONDecodeError) as error:
        raise ApiError(
            "review_analysis_failed",
            "评论分析服务调用失败，请稍后重试",
            status_code=502,
        ) from error
    content = _response_content(response_payload)
    try:
        return _decode_review_analysis_content(content)
    except ValueError as error:
        raise ApiError(
            "review_analysis_failed",
            "评论分析服务返回格式不正确",
            status_code=502,
        ) from error


def _response_content(response_payload: dict[str, object]) -> str:
    content = response_payload.get("content")
    if isinstance(content, list):
        text_parts = [
            str(item.get("text") or "")
            for item in content
            if isinstance(item, dict) and item.get("type") == "text"
        ]
        if text_parts:
            return "\n".join(text_parts).strip()
    choices = response_payload.get("choices")
    if not isinstance(choices, list) or not choices:
        return ""
    first_choice = choices[0]
    if not isinstance(first_choice, dict):
        return ""
    message = first_choice.get("message")
    if not isinstance(message, dict):
        return ""
    return str(message.get("content") or "").strip()


def _auth_headers(runtime: LlmRuntimeConfig) -> dict[str, str]:
    if not runtime.api_key:
        return {}
    if runtime.auth_header == "api-key":
        return {"api-key": runtime.api_key}
    if runtime.auth_header == "x-api-key":
        return {"x-api-key": runtime.api_key}
    return {"Authorization": f"Bearer {runtime.api_key}"}


def _claude_messages_url(base_url: str) -> str:
    normalized = base_url.rstrip("/")
    if normalized.endswith("/v1"):
        return normalized + "/messages"
    return normalized + "/v1/messages"
