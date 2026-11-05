from __future__ import annotations

from datetime import UTC, datetime

from sqlalchemy import JSON, Boolean, DateTime, ForeignKey, Integer, String, Text, UniqueConstraint
from sqlalchemy.orm import DeclarativeBase, Mapped, mapped_column, relationship


def utc_now() -> datetime:
    return datetime.now(UTC)


class Base(DeclarativeBase):
    pass


class App(Base):
    __tablename__ = "apps"
    __table_args__ = (UniqueConstraint("platform", "bundle_identifier"),)

    id: Mapped[str] = mapped_column(String(80), primary_key=True)
    name: Mapped[str] = mapped_column(String(120), nullable=False)
    bundle_identifier: Mapped[str] = mapped_column(String(180), nullable=False)
    platform: Mapped[str] = mapped_column(String(20), nullable=False)
    developer_account_id: Mapped[str | None] = mapped_column(
        ForeignKey("developer_accounts.id", ondelete="SET NULL"),
    )
    store_app_id: Mapped[str | None] = mapped_column(String(120))
    store_package_name: Mapped[str | None] = mapped_column(String(180))
    default_channel: Mapped[str] = mapped_column(String(20), nullable=False, default="dev")
    icon_key: Mapped[str] = mapped_column(String(40), nullable=False, default="app")
    icon_color: Mapped[str] = mapped_column(String(20), nullable=False, default="#53606E")
    added_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True),
        nullable=False,
        default=utc_now,
    )

    builds: Mapped[list[Build]] = relationship(back_populates="app", cascade="all, delete-orphan")
    build_setting: Mapped[AppBuildSetting | None] = relationship(
        back_populates="app",
        cascade="all, delete-orphan",
        single_parent=True,
        uselist=False,
    )
    developer_account: Mapped[DeveloperAccount | None] = relationship(back_populates="apps")


class AppBuildSetting(Base):
    __tablename__ = "app_build_settings"
    __table_args__ = (UniqueConstraint("app_id", name="uq_app_build_settings_app_id"),)

    id: Mapped[str] = mapped_column(String(80), primary_key=True)
    app_id: Mapped[str] = mapped_column(ForeignKey("apps.id", ondelete="CASCADE"), nullable=False)
    git_url: Mapped[str] = mapped_column(String(800), nullable=False)
    runner_labels_json: Mapped[list] = mapped_column(JSON, nullable=False, default=list)
    credential_refs_json: Mapped[dict] = mapped_column(JSON, nullable=False, default=dict)
    artifact_type: Mapped[str] = mapped_column(String(30), nullable=False)
    optional_defaults_json: Mapped[dict] = mapped_column(JSON, nullable=False, default=dict)
    updated_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True),
        nullable=False,
        default=utc_now,
    )

    app: Mapped[App] = relationship(back_populates="build_setting")


class Build(Base):
    __tablename__ = "builds"

    id: Mapped[str] = mapped_column(String(80), primary_key=True)
    app_id: Mapped[str] = mapped_column(ForeignKey("apps.id", ondelete="CASCADE"), nullable=False)
    version: Mapped[str | None] = mapped_column(String(60), nullable=True)
    build_number: Mapped[str | None] = mapped_column(String(80), nullable=True)
    channel: Mapped[str] = mapped_column(String(20), nullable=False)
    environment: Mapped[str] = mapped_column(String(30), nullable=False)
    requested_environment: Mapped[str] = mapped_column(
        String(30),
        nullable=False,
        default="development",
    )
    platform: Mapped[str] = mapped_column(String(20), nullable=False)
    source: Mapped[str] = mapped_column(String(30), nullable=False, default="upload")
    lifecycle_status: Mapped[str] = mapped_column(String(40), nullable=False, default="succeeded")
    git_url: Mapped[str | None] = mapped_column(String(800))
    git_ref: Mapped[str | None] = mapped_column(String(240))
    commit_sha: Mapped[str | None] = mapped_column(String(80))
    runner_id: Mapped[str | None] = mapped_column(String(80))
    runner_labels_json: Mapped[dict | None] = mapped_column(JSON)
    attempt_count: Mapped[int] = mapped_column(Integer, nullable=False, default=0)
    assignment_lease_expires_at: Mapped[datetime | None] = mapped_column(DateTime(timezone=True))
    failure_classification: Mapped[str | None] = mapped_column(String(80))
    failure_summary: Mapped[str | None] = mapped_column(Text)
    human_action: Mapped[str | None] = mapped_column(Text)
    uploaded_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True),
        nullable=False,
        default=utc_now,
    )
    started_at: Mapped[datetime | None] = mapped_column(DateTime(timezone=True))
    finished_at: Mapped[datetime | None] = mapped_column(DateTime(timezone=True))
    duration_seconds: Mapped[int | None] = mapped_column(Integer)
    note: Mapped[str] = mapped_column(Text, nullable=False, default="")
    status: Mapped[str] = mapped_column(String(30), nullable=False, default="available")
    min_os_version: Mapped[str | None] = mapped_column(String(80))
    expires_at: Mapped[datetime | None] = mapped_column(DateTime(timezone=True))

    app: Mapped[App] = relationship(back_populates="builds")
    artifacts: Mapped[list[Artifact]] = relationship(
        back_populates="build",
        cascade="all, delete-orphan",
    )
    events: Mapped[list[BuildEvent]] = relationship(
        back_populates="build",
        cascade="all, delete-orphan",
    )

    def package_artifact(self) -> Artifact | None:
        return next(
            (artifact for artifact in self.artifacts if artifact.artifact_type == "package"),
            None,
        )


class Artifact(Base):
    __tablename__ = "artifacts"

    id: Mapped[str] = mapped_column(String(80), primary_key=True)
    build_id: Mapped[str] = mapped_column(
        ForeignKey("builds.id", ondelete="CASCADE"),
        nullable=False,
    )
    artifact_type: Mapped[str] = mapped_column(String(30), nullable=False, default="package")
    file_name: Mapped[str] = mapped_column(String(240), nullable=False)
    content_type: Mapped[str] = mapped_column(String(120), nullable=False)
    storage_backend: Mapped[str] = mapped_column(String(20), nullable=False)
    storage_key: Mapped[str] = mapped_column(String(400), nullable=False)
    download_url: Mapped[str] = mapped_column(String(800), nullable=False)
    manifest_url: Mapped[str | None] = mapped_column(String(800))
    install_url: Mapped[str] = mapped_column(String(1000), nullable=False)
    size_bytes: Mapped[int] = mapped_column(Integer, nullable=False, default=0)
    metadata_json: Mapped[dict | None] = mapped_column(JSON)
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True),
        nullable=False,
        default=utc_now,
    )

    build: Mapped[Build] = relationship(back_populates="artifacts")


class BuildRunner(Base):
    __tablename__ = "build_runners"

    id: Mapped[str] = mapped_column(String(80), primary_key=True)
    name: Mapped[str] = mapped_column(String(120), nullable=False)
    token_hash: Mapped[str] = mapped_column(String(240), nullable=False)
    labels_json: Mapped[list] = mapped_column(JSON, nullable=False, default=list)
    capabilities_json: Mapped[dict] = mapped_column(JSON, nullable=False, default=dict)
    status: Mapped[str] = mapped_column(String(40), nullable=False, default="offline")
    version: Mapped[str] = mapped_column(String(80), nullable=False, default="")
    package_agent_version: Mapped[str] = mapped_column(String(80), nullable=False, default="")
    last_seen_at: Mapped[datetime | None] = mapped_column(DateTime(timezone=True))
    current_build_id: Mapped[str | None] = mapped_column(
        ForeignKey("builds.id", ondelete="SET NULL"),
        nullable=True,
    )


class BuildEvent(Base):
    __tablename__ = "build_events"

    id: Mapped[str] = mapped_column(String(80), primary_key=True)
    build_id: Mapped[str] = mapped_column(
        ForeignKey("builds.id", ondelete="CASCADE"),
        nullable=False,
    )
    runner_id: Mapped[str | None] = mapped_column(
        ForeignKey("build_runners.id", ondelete="SET NULL"),
        nullable=True,
    )
    type: Mapped[str] = mapped_column(String(80), nullable=False)
    message: Mapped[str] = mapped_column(String(500), nullable=False)
    payload_json: Mapped[dict] = mapped_column(JSON, nullable=False, default=dict)
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True),
        nullable=False,
        default=utc_now,
    )

    build: Mapped[Build] = relationship(back_populates="events")


class Device(Base):
    __tablename__ = "devices"

    id: Mapped[str] = mapped_column(String(120), primary_key=True)
    name: Mapped[str] = mapped_column(String(120), nullable=False)
    owner: Mapped[str] = mapped_column(String(120), nullable=False, default="")
    platform: Mapped[str] = mapped_column(String(20), nullable=False)
    status: Mapped[str] = mapped_column(String(40), nullable=False, default="registered")
    status_color: Mapped[str] = mapped_column(String(20), nullable=False, default="#20864A")
    detail: Mapped[str] = mapped_column(String(240), nullable=False, default="")
    udid: Mapped[str] = mapped_column(String(160), nullable=False, default="")
    os_version: Mapped[str] = mapped_column(String(80), nullable=False, default="")
    certificate_status: Mapped[str] = mapped_column(String(120), nullable=False, default="")
    registered_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True),
        nullable=False,
        default=utc_now,
    )


class DeviceBuildVisibility(Base):
    __tablename__ = "device_build_visibility"

    device_id: Mapped[str] = mapped_column(
        ForeignKey("devices.id", ondelete="CASCADE"),
        primary_key=True,
    )
    build_id: Mapped[str] = mapped_column(
        ForeignKey("builds.id", ondelete="CASCADE"),
        primary_key=True,
    )
    reason: Mapped[str] = mapped_column(String(120), nullable=False, default="assigned")
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True),
        nullable=False,
        default=utc_now,
    )


class DeveloperAccount(Base):
    __tablename__ = "developer_accounts"

    id: Mapped[str] = mapped_column(String(80), primary_key=True)
    team_name: Mapped[str] = mapped_column(String(160), nullable=False)
    expires_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), nullable=False)
    status: Mapped[str] = mapped_column(String(40), nullable=False, default="renewal_due")
    renewal_action_label: Mapped[str] = mapped_column(String(40), nullable=False, default="去续费")

    apps: Mapped[list[App]] = relationship(back_populates="developer_account")


class DeveloperAccountApp(Base):
    __tablename__ = "developer_account_apps"

    developer_account_id: Mapped[str] = mapped_column(
        ForeignKey("developer_accounts.id", ondelete="CASCADE"),
        primary_key=True,
    )
    app_id: Mapped[str] = mapped_column(ForeignKey("apps.id", ondelete="CASCADE"), primary_key=True)


class Notification(Base):
    __tablename__ = "notifications"

    id: Mapped[str] = mapped_column(String(80), primary_key=True)
    type: Mapped[str] = mapped_column(String(30), nullable=False)
    section: Mapped[str] = mapped_column(String(80), nullable=False)
    icon_key: Mapped[str] = mapped_column(String(40), nullable=False)
    title: Mapped[str] = mapped_column(String(180), nullable=False)
    subtitle: Mapped[str] = mapped_column(String(280), nullable=False)
    tag: Mapped[str] = mapped_column(String(40), nullable=False)
    tag_color: Mapped[str] = mapped_column(String(20), nullable=False)
    app_id: Mapped[str | None] = mapped_column(ForeignKey("apps.id", ondelete="SET NULL"))
    build_id: Mapped[str | None] = mapped_column(ForeignKey("builds.id", ondelete="SET NULL"))
    device_id: Mapped[str | None] = mapped_column(ForeignKey("devices.id", ondelete="SET NULL"))
    developer_account_id: Mapped[str | None] = mapped_column(
        ForeignKey("developer_accounts.id", ondelete="SET NULL"),
    )
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True),
        nullable=False,
        default=utc_now,
    )


class WebhookDelivery(Base):
    __tablename__ = "webhook_deliveries"
    __table_args__ = (
        UniqueConstraint("event_key", name="uq_webhook_deliveries_event_key"),
    )

    id: Mapped[str] = mapped_column(String(80), primary_key=True)
    channel: Mapped[str] = mapped_column(String(30), nullable=False)
    event_key: Mapped[str] = mapped_column(String(240), nullable=False)
    status: Mapped[str] = mapped_column(String(30), nullable=False, default="pending")
    payload_json: Mapped[dict] = mapped_column(JSON, nullable=False, default=dict)
    attempt_count: Mapped[int] = mapped_column(Integer, nullable=False, default=0)
    next_attempt_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True),
        nullable=False,
        default=utc_now,
    )
    last_error: Mapped[str | None] = mapped_column(Text)
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True),
        nullable=False,
        default=utc_now,
    )
    delivered_at: Mapped[datetime | None] = mapped_column(DateTime(timezone=True))


class StoreConnector(Base):
    __tablename__ = "store_connectors"
    __table_args__ = (UniqueConstraint("developer_account_id"),)

    id: Mapped[str] = mapped_column(String(80), primary_key=True)
    developer_account_id: Mapped[str] = mapped_column(
        ForeignKey("developer_accounts.id", ondelete="CASCADE"),
        nullable=False,
    )
    name: Mapped[str] = mapped_column(String(120), nullable=False)
    base_url: Mapped[str] = mapped_column(String(400), nullable=False)
    auth_token: Mapped[str] = mapped_column(String(240), nullable=False)
    status: Mapped[str] = mapped_column(String(40), nullable=False, default="unknown")
    last_checked_at: Mapped[datetime | None] = mapped_column(DateTime(timezone=True))
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True),
        nullable=False,
        default=utc_now,
    )


class StoreReleaseNoteDraft(Base):
    __tablename__ = "store_release_note_drafts"
    __table_args__ = (
        UniqueConstraint("developer_account_id", "app_id", "platform", "version", "locale"),
    )

    id: Mapped[str] = mapped_column(String(80), primary_key=True)
    developer_account_id: Mapped[str] = mapped_column(
        ForeignKey("developer_accounts.id", ondelete="CASCADE"),
        nullable=False,
    )
    app_id: Mapped[str] = mapped_column(ForeignKey("apps.id", ondelete="CASCADE"), nullable=False)
    platform: Mapped[str] = mapped_column(String(20), nullable=False)
    version: Mapped[str] = mapped_column(String(60), nullable=False)
    locale: Mapped[str] = mapped_column(String(40), nullable=False)
    release_notes: Mapped[str] = mapped_column(Text, nullable=False, default="")
    updated_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True),
        nullable=False,
        default=utc_now,
    )


class StoreAppMetadataDraft(Base):
    __tablename__ = "store_app_metadata_drafts"
    __table_args__ = (
        UniqueConstraint(
            "developer_account_id",
            "app_id",
            "platform",
            "version",
            "locale",
            "content_set_id",
            name="uq_store_app_metadata_drafts_scope",
        ),
    )

    id: Mapped[str] = mapped_column(String(80), primary_key=True)
    developer_account_id: Mapped[str] = mapped_column(
        ForeignKey("developer_accounts.id", ondelete="CASCADE"),
        nullable=False,
    )
    app_id: Mapped[str] = mapped_column(ForeignKey("apps.id", ondelete="CASCADE"), nullable=False)
    platform: Mapped[str] = mapped_column(String(20), nullable=False)
    version: Mapped[str] = mapped_column(String(60), nullable=False)
    locale: Mapped[str] = mapped_column(String(40), nullable=False)
    content_set_id: Mapped[str] = mapped_column(String(80), nullable=False, default="default")
    content_set_name: Mapped[str] = mapped_column(
        String(120),
        nullable=False,
        default="默认上架内容",
    )
    title: Mapped[str] = mapped_column(String(120), nullable=False, default="")
    subtitle: Mapped[str] = mapped_column(String(160), nullable=False, default="")
    keywords: Mapped[str] = mapped_column(String(240), nullable=False, default="")
    promotional_text: Mapped[str] = mapped_column(String(240), nullable=False, default="")
    description: Mapped[str] = mapped_column(Text, nullable=False, default="")
    privacy_policy_url: Mapped[str] = mapped_column(String(400), nullable=False, default="")
    support_url: Mapped[str] = mapped_column(String(400), nullable=False, default="")
    marketing_url: Mapped[str] = mapped_column(String(400), nullable=False, default="")
    store_images_json: Mapped[dict[str, object]] = mapped_column(
        JSON,
        nullable=False,
        default=dict,
    )
    updated_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True),
        nullable=False,
        default=utc_now,
    )


class StoreImageSuite(Base):
    __tablename__ = "store_image_suites"
    __table_args__ = (
        UniqueConstraint(
            "developer_account_id",
            "app_id",
            "platform",
            "suite_id",
            name="uq_store_image_suites_scope",
        ),
    )

    id: Mapped[str] = mapped_column(String(80), primary_key=True)
    developer_account_id: Mapped[str] = mapped_column(
        ForeignKey("developer_accounts.id", ondelete="CASCADE"),
        nullable=False,
    )
    app_id: Mapped[str] = mapped_column(ForeignKey("apps.id", ondelete="CASCADE"), nullable=False)
    platform: Mapped[str] = mapped_column(String(20), nullable=False)
    suite_id: Mapped[str] = mapped_column(String(80), nullable=False, default="default")
    suite_name: Mapped[str] = mapped_column(String(120), nullable=False, default="默认商店图")
    source: Mapped[str] = mapped_column(String(40), nullable=False, default="api")
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True),
        nullable=False,
        default=utc_now,
    )
    updated_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True),
        nullable=False,
        default=utc_now,
    )

    locales: Mapped[list[StoreImageSuiteLocale]] = relationship(
        back_populates="image_suite",
        cascade="all, delete-orphan",
    )


class StoreImageSuiteLocale(Base):
    __tablename__ = "store_image_suite_locales"
    __table_args__ = (
        UniqueConstraint("image_suite_id", "locale", name="uq_store_image_suite_locales_scope"),
    )

    id: Mapped[str] = mapped_column(String(80), primary_key=True)
    image_suite_id: Mapped[str] = mapped_column(
        ForeignKey("store_image_suites.id", ondelete="CASCADE"),
        nullable=False,
    )
    locale: Mapped[str] = mapped_column(String(40), nullable=False)
    store_images_json: Mapped[dict[str, object]] = mapped_column(
        JSON,
        nullable=False,
        default=dict,
    )
    updated_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True),
        nullable=False,
        default=utc_now,
    )

    image_suite: Mapped[StoreImageSuite] = relationship(back_populates="locales")


class StoreMarketingPage(Base):
    __tablename__ = "store_marketing_pages"
    __table_args__ = (
        UniqueConstraint(
            "developer_account_id",
            "app_id",
            "platform",
            "page_id",
            name="uq_store_marketing_pages_scope",
        ),
    )

    id: Mapped[str] = mapped_column(String(80), primary_key=True)
    developer_account_id: Mapped[str] = mapped_column(
        ForeignKey("developer_accounts.id", ondelete="CASCADE"),
        nullable=False,
    )
    app_id: Mapped[str] = mapped_column(ForeignKey("apps.id", ondelete="CASCADE"), nullable=False)
    platform: Mapped[str] = mapped_column(String(20), nullable=False)
    page_id: Mapped[str] = mapped_column(String(80), nullable=False)
    page_name: Mapped[str] = mapped_column(String(160), nullable=False)
    page_type: Mapped[str] = mapped_column(
        String(60),
        nullable=False,
        default="custom_product_page",
    )
    status: Mapped[str] = mapped_column(String(40), nullable=False, default="draft")
    apple_page_id: Mapped[str] = mapped_column(String(120), nullable=False, default="")
    deep_link_url: Mapped[str] = mapped_column(String(500), nullable=False, default="")
    keywords: Mapped[str] = mapped_column(String(240), nullable=False, default="")
    store_images_json: Mapped[dict[str, object]] = mapped_column(JSON, nullable=False, default=dict)
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True),
        nullable=False,
        default=utc_now,
    )
    updated_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True),
        nullable=False,
        default=utc_now,
    )

    locales: Mapped[list[StoreMarketingPageLocale]] = relationship(
        back_populates="marketing_page",
        cascade="all, delete-orphan",
    )


class StoreMarketingPageLocale(Base):
    __tablename__ = "store_marketing_page_locales"
    __table_args__ = (
        UniqueConstraint(
            "marketing_page_id",
            "locale",
            name="uq_store_marketing_page_locales_scope",
        ),
    )

    id: Mapped[str] = mapped_column(String(80), primary_key=True)
    marketing_page_id: Mapped[str] = mapped_column(
        ForeignKey("store_marketing_pages.id", ondelete="CASCADE"),
        nullable=False,
    )
    locale: Mapped[str] = mapped_column(String(40), nullable=False)
    promotional_text: Mapped[str] = mapped_column(String(240), nullable=False, default="")
    store_images_json: Mapped[dict[str, object]] = mapped_column(
        JSON,
        nullable=False,
        default=dict,
    )
    updated_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True),
        nullable=False,
        default=utc_now,
    )

    marketing_page: Mapped[StoreMarketingPage] = relationship(back_populates="locales")


class StorePreflightCheck(Base):
    __tablename__ = "store_preflight_checks"

    id: Mapped[str] = mapped_column(String(80), primary_key=True)
    developer_account_id: Mapped[str] = mapped_column(
        ForeignKey("developer_accounts.id", ondelete="CASCADE"),
        nullable=False,
    )
    app_id: Mapped[str] = mapped_column(ForeignKey("apps.id", ondelete="CASCADE"), nullable=False)
    connector_id: Mapped[str] = mapped_column(
        ForeignKey("store_connectors.id", ondelete="CASCADE"),
        nullable=False,
    )
    platform: Mapped[str] = mapped_column(String(20), nullable=False)
    operation: Mapped[str] = mapped_column(String(60), nullable=False)
    version: Mapped[str] = mapped_column(String(60), nullable=False)
    locale: Mapped[str] = mapped_column(String(40), nullable=False)
    request_hash: Mapped[str] = mapped_column(String(64), nullable=False)
    can_sync: Mapped[bool] = mapped_column(Boolean, nullable=False)
    reason_code: Mapped[str | None] = mapped_column(String(80))
    message: Mapped[str] = mapped_column(String(280), nullable=False)
    store_state_json: Mapped[dict[str, object]] = mapped_column(JSON, nullable=False, default=dict)
    checked_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True),
        nullable=False,
        default=utc_now,
    )
    expires_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), nullable=False)


class StoreSyncRun(Base):
    __tablename__ = "store_sync_runs"

    id: Mapped[str] = mapped_column(String(80), primary_key=True)
    developer_account_id: Mapped[str] = mapped_column(
        ForeignKey("developer_accounts.id", ondelete="CASCADE"),
        nullable=False,
    )
    app_id: Mapped[str] = mapped_column(ForeignKey("apps.id", ondelete="CASCADE"), nullable=False)
    connector_id: Mapped[str] = mapped_column(
        ForeignKey("store_connectors.id", ondelete="CASCADE"),
        nullable=False,
    )
    draft_id: Mapped[str | None] = mapped_column(
        ForeignKey("store_release_note_drafts.id", ondelete="SET NULL"),
    )
    metadata_draft_id: Mapped[str | None] = mapped_column(
        ForeignKey("store_app_metadata_drafts.id", ondelete="SET NULL"),
    )
    platform: Mapped[str] = mapped_column(String(20), nullable=False)
    operation: Mapped[str] = mapped_column(String(60), nullable=False)
    version: Mapped[str] = mapped_column(String(60), nullable=False)
    locale: Mapped[str] = mapped_column(String(40), nullable=False)
    status: Mapped[str] = mapped_column(String(40), nullable=False)
    started_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True),
        nullable=False,
        default=utc_now,
    )
    finished_at: Mapped[datetime | None] = mapped_column(DateTime(timezone=True))
    error_code: Mapped[str | None] = mapped_column(String(80))
    error_summary: Mapped[str | None] = mapped_column(String(280))
    sync_scopes_json: Mapped[dict[str, object]] = mapped_column(JSON, nullable=False, default=dict)
    payload_snapshot_json: Mapped[dict[str, object]] = mapped_column(
        JSON,
        nullable=False,
        default=dict,
    )


class StoreReview(Base):
    __tablename__ = "store_reviews"
    __table_args__ = (
        UniqueConstraint(
            "developer_account_id",
            "app_id",
            "platform",
            "store_review_id",
            name="uq_store_reviews_scope",
        ),
    )

    id: Mapped[str] = mapped_column(String(80), primary_key=True)
    developer_account_id: Mapped[str] = mapped_column(
        ForeignKey("developer_accounts.id", ondelete="CASCADE"),
        nullable=False,
    )
    app_id: Mapped[str] = mapped_column(ForeignKey("apps.id", ondelete="CASCADE"), nullable=False)
    platform: Mapped[str] = mapped_column(String(20), nullable=False)
    store_review_id: Mapped[str] = mapped_column(String(180), nullable=False)
    rating: Mapped[int | None] = mapped_column(Integer)
    title: Mapped[str] = mapped_column(String(240), nullable=False, default="")
    body: Mapped[str] = mapped_column(Text, nullable=False, default="")
    author_name: Mapped[str] = mapped_column(String(180), nullable=False, default="")
    locale: Mapped[str] = mapped_column(String(40), nullable=False, default="")
    territory: Mapped[str] = mapped_column(String(40), nullable=False, default="")
    app_version: Mapped[str] = mapped_column(String(80), nullable=False, default="")
    created_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), nullable=False)
    updated_at: Mapped[datetime | None] = mapped_column(DateTime(timezone=True))
    fetched_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True),
        nullable=False,
        default=utc_now,
    )
    raw_json: Mapped[dict[str, object]] = mapped_column(JSON, nullable=False, default=dict)


class StoreReviewFetchRun(Base):
    __tablename__ = "store_review_fetch_runs"

    id: Mapped[str] = mapped_column(String(80), primary_key=True)
    developer_account_id: Mapped[str] = mapped_column(
        ForeignKey("developer_accounts.id", ondelete="CASCADE"),
        nullable=False,
    )
    app_id: Mapped[str] = mapped_column(ForeignKey("apps.id", ondelete="CASCADE"), nullable=False)
    connector_id: Mapped[str | None] = mapped_column(
        ForeignKey("store_connectors.id", ondelete="SET NULL"),
    )
    platform: Mapped[str] = mapped_column(String(20), nullable=False)
    status: Mapped[str] = mapped_column(String(40), nullable=False)
    page_count: Mapped[int] = mapped_column(Integer, nullable=False, default=0)
    fetched_count: Mapped[int] = mapped_column(Integer, nullable=False, default=0)
    inserted_count: Mapped[int] = mapped_column(Integer, nullable=False, default=0)
    duplicate_count: Mapped[int] = mapped_column(Integer, nullable=False, default=0)
    stopped_reason: Mapped[str] = mapped_column(String(80), nullable=False, default="")
    filters_json: Mapped[dict[str, object]] = mapped_column(JSON, nullable=False, default=dict)
    started_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True),
        nullable=False,
        default=utc_now,
    )
    finished_at: Mapped[datetime | None] = mapped_column(DateTime(timezone=True))
    error_code: Mapped[str | None] = mapped_column(String(80))
    error_summary: Mapped[str | None] = mapped_column(String(280))


class StoreReviewAnalysisRun(Base):
    __tablename__ = "store_review_analysis_runs"

    id: Mapped[str] = mapped_column(String(80), primary_key=True)
    developer_account_id: Mapped[str] = mapped_column(
        ForeignKey("developer_accounts.id", ondelete="CASCADE"),
        nullable=False,
    )
    app_id: Mapped[str] = mapped_column(ForeignKey("apps.id", ondelete="CASCADE"), nullable=False)
    platform: Mapped[str] = mapped_column(String(20), nullable=False)
    status: Mapped[str] = mapped_column(String(40), nullable=False)
    review_count: Mapped[int] = mapped_column(Integer, nullable=False, default=0)
    low_rating_count: Mapped[int] = mapped_column(Integer, nullable=False, default=0)
    issue_count: Mapped[int] = mapped_column(Integer, nullable=False, default=0)
    summary: Mapped[str] = mapped_column(Text, nullable=False, default="")
    analysis_json: Mapped[dict[str, object]] = mapped_column(JSON, nullable=False, default=dict)
    started_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True),
        nullable=False,
        default=utc_now,
    )
    finished_at: Mapped[datetime | None] = mapped_column(DateTime(timezone=True))
    error_code: Mapped[str | None] = mapped_column(String(80))
    error_summary: Mapped[str | None] = mapped_column(String(280))


class LlmProfile(Base):
    __tablename__ = "llm_profiles"

    id: Mapped[str] = mapped_column(String(80), primary_key=True)
    name: Mapped[str] = mapped_column(String(120), nullable=False)
    protocol: Mapped[str] = mapped_column(String(40), nullable=False)
    base_url: Mapped[str] = mapped_column(String(500), nullable=False)
    model: Mapped[str] = mapped_column(String(160), nullable=False)
    api_key: Mapped[str] = mapped_column(Text, nullable=False, default="")
    auth_header: Mapped[str] = mapped_column(String(80), nullable=False, default="")
    status: Mapped[str] = mapped_column(String(40), nullable=False, default="unchecked")
    last_checked_at: Mapped[datetime | None] = mapped_column(DateTime(timezone=True))
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True),
        nullable=False,
        default=utc_now,
    )
    updated_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True),
        nullable=False,
        default=utc_now,
    )


class LlmFeatureBinding(Base):
    __tablename__ = "llm_feature_bindings"

    feature_key: Mapped[str] = mapped_column(String(80), primary_key=True)
    primary_profile_id: Mapped[str | None] = mapped_column(
        ForeignKey("llm_profiles.id", ondelete="SET NULL"),
    )
    fallback_profile_id: Mapped[str | None] = mapped_column(
        ForeignKey("llm_profiles.id", ondelete="SET NULL"),
    )
    updated_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True),
        nullable=False,
        default=utc_now,
    )


class SystemSetting(Base):
    __tablename__ = "system_settings"

    key: Mapped[str] = mapped_column(String(120), primary_key=True)
    value: Mapped[str] = mapped_column(Text, nullable=False, default="")
    is_secret: Mapped[bool] = mapped_column(Boolean, nullable=False, default=False)
    updated_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True),
        nullable=False,
        default=utc_now,
    )


class AuditLog(Base):
    __tablename__ = "audit_logs"

    id: Mapped[str] = mapped_column(String(80), primary_key=True)
    developer_account_id: Mapped[str | None] = mapped_column(
        ForeignKey("developer_accounts.id", ondelete="SET NULL"),
    )
    actor: Mapped[str] = mapped_column(String(120), nullable=False)
    action: Mapped[str] = mapped_column(String(120), nullable=False)
    target_type: Mapped[str] = mapped_column(String(80), nullable=False)
    target_id: Mapped[str] = mapped_column(String(120), nullable=False)
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True),
        nullable=False,
        default=utc_now,
    )
