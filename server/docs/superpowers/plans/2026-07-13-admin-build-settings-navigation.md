# TestFlying Build and Settings Navigation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将构建节点和 LLM 配置从一级导航收拢到“构建”和“设置”工作区，并让已接入应用可以从构建页直接触发构建。

**Architecture:** FastAPI 继续提供后台 JSON API，新增数据库优先、环境变量回退的业务设置解析层。React Admin App 使用可链接的二级路由组织构建和设置子页面，旧一级页面和旧页面路由直接删除。构建调度、Runner 协议、LLM 数据表和公开商店 API 保持不变。

**Tech Stack:** Python 3.11、FastAPI、SQLAlchemy 2、Alembic、PostgreSQL/SQLite、React 18、TypeScript、Vite、Vitest、pytest。

## Global Constraints

- 一级导航不得出现“构建节点”和“LLM 配置”，必须新增“设置”。
- `/admin/build-runners` 和 `/admin/llm-config` 页面路由直接删除，不做兼容跳转。
- `/admin/builds` 默认显示应用构建；`/admin/settings` 默认显示通用设置。
- 数据库、MinIO/S3、Static Token、存储目录、CORS 和 Runner 发布目录只读脱敏展示。
- 后台不得改写 Docker Compose、`.env` 或宿主机环境变量。
- 数据库业务设置优先，现有环境变量作为回退，代码默认值作为最终回退。
- LLM 继续使用 `llm_profiles` 和 `llm_feature_bindings`，不迁移数据。
- 密钥留空表示不修改，任何响应和审计日志都不得返回原始密钥。
- 页面切换保持在 React SPA 内，不触发整页刷新。
- 所有手工编辑使用 `apply_patch`，每个任务先写失败测试，再实现，再提交。

---

### Task 1: Add database-backed business settings

**Files:**
- Create: `alembic/versions/20260713_0012_system_settings.py`
- Create: `src/testflying_api/system_settings.py`
- Create: `tests/test_system_settings.py`
- Modify: `src/testflying_api/schema.py`
- Modify: `tests/test_schema.py`

**Interfaces:**
- Consumes: startup `testflying_api.config.Settings` environment values.
- Produces: `SystemSetting`, `EffectiveBusinessSettings`, `effective_business_settings()`, `save_general_settings()`, and `save_notification_settings()`.

- [ ] **Step 1: Write failing schema and precedence tests**

Add tests that require the new table and exact precedence behavior:

```python
def test_schema_contains_system_settings() -> None:
    engine = create_engine_for_url("sqlite:///:memory:")
    Base.metadata.create_all(engine)

    assert "system_settings" in inspect(engine).get_table_names()


def test_effective_business_settings_prefers_database(db_session, test_settings) -> None:
    db_session.add_all(
        [
            SystemSetting(key="dingtalk_enabled", value="false", is_secret=False),
            SystemSetting(
                key="dingtalk_webhook_url",
                value="https://db.example.test/robot/send",
                is_secret=True,
            ),
            SystemSetting(key="dingtalk_secret", value="SEC-db", is_secret=True),
        ]
    )
    db_session.commit()

    effective = effective_business_settings(db_session, test_settings)

    assert effective.dingtalk_enabled is False
    assert effective.dingtalk_webhook_url == "https://db.example.test/robot/send"
    assert effective.dingtalk_secret == "SEC-db"


def test_effective_business_settings_falls_back_to_environment(
    db_session,
    test_settings,
) -> None:
    settings = replace(
        test_settings,
        connector_base_url_template="https://connector-{account_id}.example.test",
        dingtalk_webhook_url="https://env.example.test/robot/send",
        dingtalk_secret="SEC-env",
        dingtalk_timeout_seconds=7.0,
        dingtalk_dispatch_interval_seconds=12.0,
    )

    effective = effective_business_settings(db_session, settings)

    assert effective.connector_base_url_template == (
        "https://connector-{account_id}.example.test"
    )
    assert effective.dingtalk_enabled is True
    assert effective.dingtalk_timeout_seconds == 7.0
    assert effective.dingtalk_dispatch_interval_seconds == 12.0
```

- [ ] **Step 2: Run tests and verify the red state**

Run:

```bash
pytest -q tests/test_system_settings.py tests/test_schema.py
```

Expected: collection fails because `SystemSetting` and `testflying_api.system_settings` do not exist.

- [ ] **Step 3: Add the SQLAlchemy model and migration**

Add this model to `schema.py`:

```python
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
```

Create migration revision `20260713_0012`, down revision `20260710_0011`, with the same columns and a primary key on `key`. Downgrade drops only `system_settings`.

- [ ] **Step 4: Implement the resolver and save functions**

Define the stable interface in `system_settings.py`:

```python
@dataclass(frozen=True)
class EffectiveBusinessSettings:
    connector_base_url_template: str | None
    dingtalk_enabled: bool
    dingtalk_webhook_url: str | None
    dingtalk_secret: str | None
    dingtalk_timeout_seconds: float
    dingtalk_dispatch_interval_seconds: float

    @property
    def dingtalk_configured(self) -> bool:
        return bool(
            self.dingtalk_enabled
            and self.dingtalk_webhook_url
            and self.dingtalk_secret
        )


def effective_business_settings(
    session: Session,
    environment: Settings,
) -> EffectiveBusinessSettings:
    rows = {
        row.key: row
        for row in session.scalars(select(SystemSetting))
        if row.key in DATABASE_SETTING_KEYS
    }
    return EffectiveBusinessSettings(
        connector_base_url_template=_optional_value(
            rows,
            "connector_base_url_template",
            environment.connector_base_url_template,
        ),
        dingtalk_enabled=_boolean_value(
            rows,
            "dingtalk_enabled",
            default=environment.dingtalk_configured,
        ),
        dingtalk_webhook_url=_optional_value(
            rows,
            "dingtalk_webhook_url",
            environment.dingtalk_webhook_url,
        ),
        dingtalk_secret=_optional_value(
            rows,
            "dingtalk_secret",
            environment.dingtalk_secret,
        ),
        dingtalk_timeout_seconds=_positive_float_value(
            rows,
            "dingtalk_timeout_seconds",
            environment.dingtalk_timeout_seconds,
        ),
        dingtalk_dispatch_interval_seconds=_positive_float_value(
            rows,
            "dingtalk_dispatch_interval_seconds",
            environment.dingtalk_dispatch_interval_seconds,
        ),
    )
```

`save_general_settings()` accepts `connector_base_url_template: str | None`.
`save_notification_settings()` accepts `enabled: bool`, optional webhook/secret updates, timeout and interval. Both functions validate before mutating rows, update `updated_at`, create one `AuditLog` with target type `system_settings`, and commit atomically.

Use these exact save signatures:

```python
def save_general_settings(
    session: Session,
    *,
    connector_base_url_template: str | None,
    actor: str,
) -> None:
    normalized = (connector_base_url_template or "").strip()
    _upsert_setting(
        session,
        key="connector_base_url_template",
        value=normalized,
        is_secret=False,
    )
    _record_setting_audit(session, actor=actor, keys=["connector_base_url_template"])
    session.commit()


def save_notification_settings(
    session: Session,
    *,
    enabled: bool,
    webhook_url: str | None,
    secret: str | None,
    timeout_seconds: float,
    dispatch_interval_seconds: float,
    actor: str,
) -> None:
    normalized_timeout = _require_positive(timeout_seconds, "timeout_seconds")
    normalized_interval = _require_positive(
        dispatch_interval_seconds,
        "dispatch_interval_seconds",
    )
    _upsert_setting(session, key="dingtalk_enabled", value=str(enabled).lower())
    if webhook_url is not None and webhook_url.strip():
        _upsert_setting(
            session,
            key="dingtalk_webhook_url",
            value=webhook_url.strip(),
            is_secret=True,
        )
    if secret is not None and secret.strip():
        _upsert_setting(
            session,
            key="dingtalk_secret",
            value=secret.strip(),
            is_secret=True,
        )
    _upsert_setting(
        session,
        key="dingtalk_timeout_seconds",
        value=str(normalized_timeout),
    )
    _upsert_setting(
        session,
        key="dingtalk_dispatch_interval_seconds",
        value=str(normalized_interval),
    )
    _record_setting_audit(
        session,
        actor=actor,
        keys=[
            "dingtalk_enabled",
            "dingtalk_webhook_url",
            "dingtalk_secret",
            "dingtalk_timeout_seconds",
            "dingtalk_dispatch_interval_seconds",
        ],
    )
    session.commit()
```

- [ ] **Step 5: Run migration and resolver tests**

Run:

```bash
pytest -q tests/test_system_settings.py tests/test_schema.py
```

Expected: all selected tests pass.

- [ ] **Step 6: Commit the settings foundation**

```bash
git add alembic/versions/20260713_0012_system_settings.py src/testflying_api/schema.py src/testflying_api/system_settings.py tests/test_system_settings.py tests/test_schema.py
git commit -m "feat(settings): add database business settings"
```

---

### Task 2: Add settings APIs and hot-reload notification delivery

**Files:**
- Create: `tests/test_admin_api_settings.py`
- Modify: `src/testflying_api/admin_api/schemas.py`
- Modify: `src/testflying_api/admin_api/routes.py`
- Modify: `src/testflying_api/webhook_delivery.py`
- Modify: `src/testflying_api/app.py`
- Modify: `tests/test_webhook_delivery.py`
- Modify: `tests/test_admin_api_pages.py`

**Interfaces:**
- Consumes: `effective_business_settings()`, `save_general_settings()`, `save_notification_settings()` from Task 1.
- Produces: `GET /admin/api/settings`, `PUT /admin/api/settings/general`, `PUT /admin/api/settings/notifications`, and `POST /admin/api/settings/notifications/check`.

- [ ] **Step 1: Write failing API redaction and save tests**

Require the API to mask secrets and preserve secrets when an update sends an empty field:

```python
def test_settings_api_never_returns_secrets(client, db_session) -> None:
    db_session.add_all(
        [
            SystemSetting(
                key="dingtalk_webhook_url",
                value="https://oapi.test/robot/send?access_token=never-return",
                is_secret=True,
            ),
            SystemSetting(key="dingtalk_secret", value="SEC-never-return", is_secret=True),
        ]
    )
    db_session.commit()

    response = client.get("/admin/api/settings", headers=_admin_headers())

    assert response.status_code == 200
    assert response.json()["notifications"]["webhookConfigured"] is True
    assert response.json()["notifications"]["secretConfigured"] is True
    assert "never-return" not in response.text


def test_notification_settings_blank_secret_keeps_existing(client, db_session) -> None:
    db_session.add(
        SystemSetting(key="dingtalk_secret", value="SEC-existing", is_secret=True)
    )
    db_session.commit()

    response = client.put(
        "/admin/api/settings/notifications",
        headers=_admin_headers(),
        json={
            "enabled": True,
            "webhookUrl": "https://oapi.test/robot/send?access_token=updated",
            "secret": "",
            "timeoutSeconds": 5,
            "dispatchIntervalSeconds": 10,
        },
    )

    assert response.status_code == 200
    assert db_session.get(SystemSetting, "dingtalk_secret").value == "SEC-existing"
    assert "SEC-existing" not in response.text
```

- [ ] **Step 2: Run API tests and verify they fail with 404**

Run:

```bash
pytest -q tests/test_admin_api_settings.py tests/test_webhook_delivery.py tests/test_admin_api_pages.py
```

Expected: `/admin/api/settings` returns 404.

- [ ] **Step 3: Add exact settings response models**

Add these schema families:

```python
class GeneralSettingsState(AdminApiModel):
    connector_base_url_template: str
    source: str


class NotificationSettingsState(AdminApiModel):
    enabled: bool
    configured: bool
    webhook_configured: bool
    secret_configured: bool
    timeout_seconds: float
    dispatch_interval_seconds: float
    pending_delivery_count: int
    dead_delivery_count: int
    source: str


class RuntimeEnvironmentItem(AdminApiModel):
    key: str
    label: str
    group: str
    source: str
    value_label: str
    configured: bool
    sensitive: bool
    restart_required: bool


class SettingsState(AdminApiModel):
    general: GeneralSettingsState
    notifications: NotificationSettingsState
    runtime: list[RuntimeEnvironmentItem]
```

Add requests `GeneralSettingsSaveRequest` and `NotificationSettingsSaveRequest`, plus a response with `message` and `state`.

- [ ] **Step 4: Implement settings routes and runtime descriptors**

Add a fixed descriptor registry for every runtime environment variable named in the design. Sensitive descriptors return only `已配置` or `未配置`. Database URL returns `已配置` and never returns username, password, host query, or the raw DSN. Non-sensitive values may return their normalized value.

`POST /settings/notifications/check` resolves current effective credentials and calls:

```python
send_dingtalk_markdown(
    webhook_url=effective.dingtalk_webhook_url or "",
    secret=effective.dingtalk_secret or "",
    title="TestFlying 配置检查",
    markdown="### TestFlying 配置检查\n\n钉钉通知连接正常。",
    timeout_seconds=effective.dingtalk_timeout_seconds,
)
```

Return `422 notification_not_configured` when URL or secret is missing and a redacted `502 notification_check_failed` when DingTalk rejects the request.

- [ ] **Step 5: Make the delivery loop resolve database settings each cycle**

Change `dispatch_due_deliveries()` to resolve `EffectiveBusinessSettings` inside its database session. Start `run_delivery_loop()` unconditionally in `app.py`; a disabled or unconfigured channel returns without sending. Resolve the interval after each cycle so a changed interval applies without restart.

Update build completion/poll routes to pass `effective.dingtalk_configured` instead of `request.app.state.settings.dingtalk_configured`. Update Connector URL generation to use `effective.connector_base_url_template`.

- [ ] **Step 6: Verify API, hot reload, and secret redaction**

Run:

```bash
pytest -q tests/test_admin_api_settings.py tests/test_webhook_delivery.py tests/test_admin_api_pages.py tests/test_build_runner_api.py tests/test_active_connector.py
```

Expected: all selected tests pass and response bodies contain no test secret values.

- [ ] **Step 7: Commit settings APIs and runtime changes**

```bash
git add src/testflying_api/admin_api/schemas.py src/testflying_api/admin_api/routes.py src/testflying_api/webhook_delivery.py src/testflying_api/app.py tests/test_admin_api_settings.py tests/test_webhook_delivery.py tests/test_admin_api_pages.py
git commit -m "feat(settings): add runtime configuration workspace api"
```

---

### Task 3: Add the build application workspace API

**Files:**
- Modify: `src/testflying_api/admin_api/schemas.py`
- Modify: `src/testflying_api/admin_api/routes.py`
- Modify: `src/testflying_api/build_platform.py`
- Create: `tests/test_admin_api_build_apps.py`

**Interfaces:**
- Consumes: `AppBuildSetting`, `BuildRunner`, `Build`, and existing `POST /admin/api/apps/{app_id}/builds`.
- Produces: `GET /admin/api/builds/apps` returning `BuildAppsState`.

- [ ] **Step 1: Write failing configured-app and runner-match tests**

Create one configured app, one unconfigured app, an online matching runner, and an offline runner. Require only the configured app to appear:

```python
def test_build_apps_returns_only_configured_apps_with_runner_match(
    client,
    db_session,
) -> None:
    configured = _app(db_session, "app-configured", "ios")
    _app(db_session, "app-unconfigured", "android")
    _setting(db_session, configured.id, "development", ["ios-release"])
    _runner(db_session, "runner-online", "online", ["ios-release"], ["ios"])
    _runner(db_session, "runner-offline", "offline", ["ios-release"], ["ios"])

    response = client.get("/admin/api/builds/apps", headers=_admin_headers())

    assert response.status_code == 200
    assert response.json()["total"] == 1
    item = response.json()["apps"][0]
    assert item["app"]["id"] == "app-configured"
    assert item["environments"][0]["environment"] == "development"
    assert item["environments"][0]["matchingRunnerCount"] == 1
    assert item["environments"][0]["hasOnlineRunner"] is True
```

- [ ] **Step 2: Run the new test and verify 404**

Run:

```bash
pytest -q tests/test_admin_api_build_apps.py
```

Expected: `/admin/api/builds/apps` returns 404.

- [ ] **Step 3: Add response types and service query**

Add exact schema types:

```python
class BuildEnvironmentOption(AdminApiModel):
    environment: str
    environment_label: str
    setting: BuildSettingItem
    matching_runner_count: int
    has_online_runner: bool


class BuildAppItem(AdminApiModel):
    app: BuildAppSummary
    environments: list[BuildEnvironmentOption]
    latest_build: BuildItem | None


class BuildAppsState(AdminApiModel):
    apps: list[BuildAppItem]
    total: int
```

Implement `build_apps_state(session)` with eager loading, deterministic app-name ordering, latest-build selection, and per-environment runner matching. A runner matches only when status is `online` or `busy`, platform is listed in capabilities, and all required labels are present.

- [ ] **Step 4: Verify API filtering and matching**

Run:

```bash
pytest -q tests/test_admin_api_build_apps.py tests/test_build_platform_api.py tests/test_admin_api_pages.py
```

Expected: all selected tests pass.

- [ ] **Step 5: Commit the build application API**

```bash
git add src/testflying_api/admin_api/schemas.py src/testflying_api/admin_api/routes.py src/testflying_api/build_platform.py tests/test_admin_api_build_apps.py
git commit -m "feat(builds): expose configured build applications"
```

---

### Task 4: Restructure global navigation and route parsing

**Files:**
- Create: `admin-web/src/pages/NotFoundPage.tsx`
- Create: `admin-web/src/pages/BuildWorkspacePage.tsx`
- Create: `admin-web/src/pages/SettingsPage.tsx`
- Modify: `admin-web/src/app/routes.tsx`
- Modify: `admin-web/src/app/routes.test.tsx`
- Modify: `admin-web/src/app/AdminApp.tsx`
- Modify: `admin-web/src/app/AdminApp.test.tsx`
- Modify: `src/testflying_api/admin_api/routes.py`
- Modify: `tests/test_admin_api.py`

**Interfaces:**
- Consumes: current bootstrap nav and `history.pushState` navigation model.
- Produces: top-level `builds`, `settings`, and `not-found` route keys plus subroute parsers.

- [ ] **Step 1: Write failing navigation tests**

Require the new navigation and direct removal of old routes:

```typescript
it('uses build and settings workspaces without old top-level routes', () => {
  expect(routeKeyFromPath('/admin/builds/apps')).toBe('builds');
  expect(routeKeyFromPath('/admin/builds/runners')).toBe('builds');
  expect(routeKeyFromPath('/admin/settings/llm')).toBe('settings');
  expect(routeKeyFromPath('/admin/build-runners')).toBe('not-found');
  expect(routeKeyFromPath('/admin/llm-config')).toBe('not-found');
});

it('parses exact build and settings subroutes', () => {
  expect(buildViewFromPath('/admin/builds')).toBe('apps');
  expect(buildViewFromPath('/admin/builds/history')).toBe('history');
  expect(settingsViewFromPath('/admin/settings')).toBe('general');
  expect(settingsViewFromPath('/admin/settings/runtime')).toBe('runtime');
});
```

Backend bootstrap test must assert keys equal the expected ordered list and contain `settings` but not `build-runners` or `llm-config`.

- [ ] **Step 2: Run route tests and verify they fail**

Run:

```bash
npm --prefix admin-web test -- --run src/app/routes.test.tsx src/app/AdminApp.test.tsx
pytest -q tests/test_admin_api.py
```

Expected: old route keys and bootstrap nav still appear.

- [ ] **Step 3: Implement route parsers and the new bootstrap nav**

Use these route types:

```typescript
export type AdminRouteKey =
  | 'dashboard'
  | 'uploads'
  | 'apps'
  | 'accounts'
  | 'store-reviews'
  | 'api-docs'
  | 'builds'
  | 'devices'
  | 'app-logs'
  | 'notifications'
  | 'settings'
  | 'not-found';

export type BuildView = 'apps' | 'history' | 'runners';
export type SettingsView = 'general' | 'notifications' | 'llm' | 'runtime';
```

Unknown and removed paths return `not-found`. Build nav path is `/admin/builds/apps`; settings nav path is `/admin/settings/general`. `NotFoundPage` displays “页面不存在” and a button that uses SPA navigation to `/admin`.

Create minimal, compilable workspace containers in this task. `BuildWorkspacePage` renders the three
secondary navigation buttons, the current `BuildsPage` for `history`, the current
`BuildRunnersPage` for `runners`, and an empty-state placeholder for `apps`. `SettingsPage`
renders the four secondary navigation buttons, the existing `LlmConfigPage` for `llm`, and
empty-state placeholders for the other views. Tasks 5 and 7 replace those placeholders with the
real pages.

- [ ] **Step 4: Verify shell navigation without document reload**

Run:

```bash
npm --prefix admin-web test -- --run src/app/routes.test.tsx src/app/AdminApp.test.tsx
pytest -q tests/test_admin_api.py
```

Expected: all selected tests pass.

- [ ] **Step 5: Commit navigation restructuring**

```bash
git add admin-web/src/app/routes.tsx admin-web/src/app/routes.test.tsx admin-web/src/app/AdminApp.tsx admin-web/src/app/AdminApp.test.tsx admin-web/src/pages/NotFoundPage.tsx admin-web/src/pages/BuildWorkspacePage.tsx admin-web/src/pages/SettingsPage.tsx src/testflying_api/admin_api/routes.py tests/test_admin_api.py
git commit -m "feat(admin): group build and settings navigation"
```

---

### Task 5: Build the application and history workspace UI

**Files:**
- Modify: `admin-web/src/pages/BuildWorkspacePage.tsx`
- Create: `admin-web/src/pages/BuildAppsPage.tsx`
- Create: `admin-web/src/pages/BuildAppsPage.test.tsx`
- Rename: `admin-web/src/pages/BuildsPage.tsx` to `admin-web/src/pages/BuildHistoryPage.tsx`
- Rename: `admin-web/src/pages/BuildsPage.test.tsx` to `admin-web/src/pages/BuildHistoryPage.test.tsx`
- Modify: `admin-web/src/app/apiClient.ts`
- Modify: `admin-web/src/app/AdminApp.tsx`
- Modify: `admin-web/src/styles/admin.css`

**Interfaces:**
- Consumes: `GET /admin/api/builds/apps`, existing build create API, and `BuildView` from Task 4.
- Produces: `BuildWorkspacePage`, `BuildAppsPage`, and `BuildHistoryPage`.

- [ ] **Step 1: Write the failing build workspace interaction test**

Mock one app with development and production settings. Assert application selection, environment selection, Git ref input, offline warning, and build submission:

```typescript
it('selects a configured app and creates a build without leaving the page', async () => {
  const user = userEvent.setup();
  render(<BuildAppsPage />);

  await user.click(await screen.findByRole('button', { name: /lookrva/ }));
  await user.selectOptions(screen.getByLabelText('构建环境'), 'production');
  await user.clear(screen.getByLabelText('Git ref'));
  await user.type(screen.getByLabelText('Git ref'), 'release/1.2.0');
  expect(screen.getByText('当前无匹配在线节点')).toBeTruthy();

  await user.click(screen.getByRole('button', { name: '立即构建' }));

  expect(await screen.findByText('构建任务已创建')).toBeTruthy();
  expect(screen.getByText('build-agent-123')).toBeTruthy();
  expect(location.pathname).toBe('/admin/builds/apps');
});
```

- [ ] **Step 2: Run the test and verify missing components**

Run:

```bash
npm --prefix admin-web test -- --run src/pages/BuildAppsPage.test.tsx
```

Expected: import fails because `BuildAppsPage` does not exist.

- [ ] **Step 3: Add typed API methods**

Add `BuildAppItem`, `BuildEnvironmentOption`, `BuildAppsState`, and:

```typescript
export function loadBuildAppsState(): Promise<BuildAppsState> {
  return getJson<BuildAppsState>('/admin/api/builds/apps');
}
```

Reuse `createAgentBuild()` for submission. Do not duplicate request construction; submit the selected environment's server-provided setting.

- [ ] **Step 4: Implement the build workspace**

`BuildWorkspacePage` owns the three secondary navigation buttons and renders the exact subpage from `buildViewFromPath(location.pathname)`. It listens to `admin:navigation` and `popstate` so browser back/forward updates the selected subpage.

`BuildAppsPage` uses a stable two-column layout: configured app list on the left, selected build form on the right. If no apps exist, render “还没有接入构建的应用” and an SPA button to `/admin/apps`. Disable duplicate submit while loading. After success, keep the selected app and show the returned build ID/status.

Move the current record table to `BuildHistoryPage` without changing artifact actions.

- [ ] **Step 5: Verify build workspace behavior and layout types**

Run:

```bash
npm --prefix admin-web test -- --run src/pages/BuildAppsPage.test.tsx src/pages/BuildHistoryPage.test.tsx src/app/AdminApp.test.tsx
npm --prefix admin-web run lint
```

Expected: selected tests and TypeScript checks pass.

- [ ] **Step 6: Commit the build application workspace**

```bash
git add admin-web/src/pages/BuildWorkspacePage.tsx admin-web/src/pages/BuildAppsPage.tsx admin-web/src/pages/BuildAppsPage.test.tsx admin-web/src/pages/BuildHistoryPage.tsx admin-web/src/pages/BuildHistoryPage.test.tsx admin-web/src/app/apiClient.ts admin-web/src/app/AdminApp.tsx admin-web/src/styles/admin.css
git commit -m "feat(admin): add application build workspace"
```

---

### Task 6: Move and complete the runner configuration UI

**Files:**
- Modify: `src/testflying_api/admin_api/schemas.py`
- Modify: `src/testflying_api/admin_api/routes.py`
- Modify: `src/testflying_api/runner_releases.py`
- Modify: `tests/test_build_runner_api.py`
- Modify: `admin-web/src/app/apiClient.ts`
- Modify: `admin-web/src/pages/BuildRunnersPage.tsx`
- Modify: `admin-web/src/pages/BuildRunnersPage.test.tsx`
- Modify: `admin-web/src/styles/admin.css`

**Interfaces:**
- Consumes: existing runner provision API and release manifests.
- Produces: runner release status in `BuildRunnerItem`, `provisionBuildRunner()`, and one-time configuration UI.

- [ ] **Step 1: Write failing provision and version-status tests**

Backend test requires the provision response to expose the token only once and the list response to omit it. Frontend test requires “新增节点”, submission, and copyable config:

```typescript
it('provisions a runner and shows its token only in the result panel', async () => {
  const user = userEvent.setup();
  render(<BuildRunnersPage />);

  await user.click(screen.getByRole('button', { name: '新增节点' }));
  await user.type(screen.getByLabelText('节点 ID'), 'runner-mac-2');
  await user.type(screen.getByLabelText('节点名称'), 'Mac mini 2');
  await user.type(screen.getByLabelText('节点标签'), 'ios-release');
  await user.click(screen.getByRole('button', { name: '生成节点配置' }));

  expect(await screen.findByText('请立即保存，关闭后无法再次查看')).toBeTruthy();
  expect(screen.getByText('runner-secret-once')).toBeTruthy();
  expect(screen.getByRole('button', { name: '复制配置 JSON' })).toBeTruthy();
});
```

- [ ] **Step 2: Run tests and verify missing controls/status**

Run:

```bash
pytest -q tests/test_build_runner_api.py
npm --prefix admin-web test -- --run src/pages/BuildRunnersPage.test.tsx
```

Expected: frontend cannot find “新增节点”; runner list lacks release status fields.

- [ ] **Step 3: Extend runner state without changing runner protocol**

Add fields to `BuildRunnerItem`:

```python
latest_version: str
update_status: str
update_status_label: str
```

Load the relevant manifest for the runner's declared platform/arch when available. Use `current`, `outdated`, and `unknown`; missing manifests produce `unknown`, not an API error.

Keep `RunnerProvisionResponse.token` unchanged. The UI derives the one-time config JSON from the response, `location.origin`, labels, platforms, LLM adapters, capacity `1`, and the documented macOS root directory. It never persists the raw token in React global state or local storage.

- [ ] **Step 4: Implement runner provisioning controls**

Add a focused modal/sheet with fields for runner ID, name, labels, platform (`ios` or `android` capability selection), architecture (`arm64` or `amd64`), and LLM adapters. On success replace the form with the one-time result and copy buttons. Closing the result clears the token from component state.

Render version state in the node table and keep all node UI under `/admin/builds/runners`.

- [ ] **Step 5: Verify provisioning and secret lifetime**

Run:

```bash
pytest -q tests/test_build_runner_api.py tests/test_runner_releases.py
npm --prefix admin-web test -- --run src/pages/BuildRunnersPage.test.tsx
npm --prefix admin-web run lint
```

Expected: all selected tests pass and runner list payloads do not contain `token`.

- [ ] **Step 6: Commit runner configuration UI**

```bash
git add src/testflying_api/admin_api/schemas.py src/testflying_api/admin_api/routes.py src/testflying_api/runner_releases.py tests/test_build_runner_api.py admin-web/src/app/apiClient.ts admin-web/src/pages/BuildRunnersPage.tsx admin-web/src/pages/BuildRunnersPage.test.tsx admin-web/src/styles/admin.css
git commit -m "feat(admin): add runner configuration workspace"
```

---

### Task 7: Build the settings workspace UI

**Files:**
- Modify: `admin-web/src/pages/SettingsPage.tsx`
- Create: `admin-web/src/pages/GeneralSettingsPage.tsx`
- Create: `admin-web/src/pages/NotificationSettingsPage.tsx`
- Create: `admin-web/src/pages/RuntimeSettingsPage.tsx`
- Create: `admin-web/src/pages/SettingsPage.test.tsx`
- Modify: `admin-web/src/pages/LlmConfigPage.tsx`
- Modify: `admin-web/src/pages/LlmConfigPage.test.tsx`
- Modify: `admin-web/src/pages/NotificationsPage.tsx`
- Modify: `admin-web/src/pages/OrdinaryPages.test.tsx`
- Modify: `admin-web/src/app/apiClient.ts`
- Modify: `admin-web/src/app/AdminApp.tsx`
- Modify: `admin-web/src/styles/admin.css`

**Interfaces:**
- Consumes: Task 2 settings APIs, current LLM APIs, and `SettingsView` from Task 4.
- Produces: complete `/admin/settings/*` workspace and a notification-list-only page.

- [ ] **Step 1: Write failing settings UI tests**

Require secondary routing, secret preservation, notification check, and runtime redaction:

```typescript
it('saves notification settings and checks DingTalk without exposing secrets', async () => {
  const user = userEvent.setup();
  history.replaceState(null, '', '/admin/settings/notifications');
  render(<SettingsPage />);

  expect(await screen.findByText('通知设置')).toBeTruthy();
  expect(screen.getByText('密钥已配置')).toBeTruthy();
  expect(screen.queryByText('SEC-never-return')).toBeNull();

  await user.clear(screen.getByLabelText('请求超时'));
  await user.type(screen.getByLabelText('请求超时'), '8');
  await user.click(screen.getByRole('button', { name: '保存配置' }));
  expect(await screen.findByText('通知配置已保存')).toBeTruthy();

  await user.click(screen.getByRole('button', { name: '检查配置' }));
  expect(await screen.findByText('钉钉配置检查消息已发送')).toBeTruthy();
});

it('renders infrastructure values as read-only masked state', async () => {
  history.replaceState(null, '', '/admin/settings/runtime');
  render(<SettingsPage />);

  expect(await screen.findByText('TESTFLYING_DATABASE_URL')).toBeTruthy();
  expect(screen.getByText('已配置')).toBeTruthy();
  expect(screen.queryByText('postgres-secret')).toBeNull();
  expect(screen.queryByRole('textbox')).toBeNull();
});
```

- [ ] **Step 2: Run tests and verify missing settings workspace**

Run:

```bash
npm --prefix admin-web test -- --run src/pages/SettingsPage.test.tsx src/pages/LlmConfigPage.test.tsx src/pages/OrdinaryPages.test.tsx
```

Expected: the test fails because general, notification, and runtime placeholders do not contain the required forms and environment rows.

- [ ] **Step 3: Add typed settings client methods**

Add `SettingsState`, request types, and exact methods:

```typescript
export function loadSettingsState(): Promise<SettingsState> {
  return getJson<SettingsState>('/admin/api/settings');
}

export function saveGeneralSettings(
  payload: GeneralSettingsPayload
): Promise<SettingsActionResponse> {
  return putJson<SettingsActionResponse>('/admin/api/settings/general', payload);
}

export function saveNotificationSettings(
  payload: NotificationSettingsPayload
): Promise<SettingsActionResponse> {
  return putJson<SettingsActionResponse>('/admin/api/settings/notifications', payload);
}

export function checkNotificationSettings(): Promise<SettingsActionResponse> {
  return postJson<SettingsActionResponse>('/admin/api/settings/notifications/check', {});
}
```

- [ ] **Step 4: Implement settings secondary pages**

`SettingsPage` owns secondary navigation and renders one child page at a time. Load shared settings state once and replace it with each successful write response.

`GeneralSettingsPage` edits only Connector URL template. `NotificationSettingsPage` edits enabled state, URL, optional secret, timeout and interval, and shows delivery counts. `RuntimeSettingsPage` groups read-only rows and renders no inputs. Embed the existing `LlmConfigPage` unchanged in behavior under the LLM child route.

Remove `DingTalkSetup` from `NotificationsPage`; replace it with a compact “通知渠道在设置中管理” link using SPA navigation.

- [ ] **Step 5: Verify settings interactions and SPA routing**

Run:

```bash
npm --prefix admin-web test -- --run src/pages/SettingsPage.test.tsx src/pages/LlmConfigPage.test.tsx src/pages/OrdinaryPages.test.tsx src/app/AdminApp.test.tsx
npm --prefix admin-web run lint
npm --prefix admin-web run build
```

Expected: all tests, type checks, and Vite build pass.

- [ ] **Step 6: Commit the settings workspace**

```bash
git add admin-web/src/pages/SettingsPage.tsx admin-web/src/pages/GeneralSettingsPage.tsx admin-web/src/pages/NotificationSettingsPage.tsx admin-web/src/pages/RuntimeSettingsPage.tsx admin-web/src/pages/SettingsPage.test.tsx admin-web/src/pages/LlmConfigPage.tsx admin-web/src/pages/LlmConfigPage.test.tsx admin-web/src/pages/NotificationsPage.tsx admin-web/src/pages/OrdinaryPages.test.tsx admin-web/src/app/apiClient.ts admin-web/src/app/AdminApp.tsx admin-web/src/styles/admin.css
git commit -m "feat(admin): add unified settings workspace"
```

---

### Task 8: Complete regression coverage, documentation, and deployment verification

**Files:**
- Modify: `README.md`
- Modify: `docs/build-delivery.md`
- Modify: `docs/api-contract.md`
- Modify: `tests/test_admin_spa.py`

**Interfaces:**
- Consumes: all routes, APIs, migrations, and UI from Tasks 1-7.
- Produces: updated operator documentation and full release evidence.

- [ ] **Step 1: Add final route and packaged-SPA regression tests**

Extend the SPA nested-route test with the exact new paths:

```python
def test_admin_spa_fallback_serves_build_and_settings_routes(client: TestClient) -> None:
    for path in (
        "/admin/builds/apps",
        "/admin/builds/history",
        "/admin/builds/runners",
        "/admin/settings/general",
        "/admin/settings/notifications",
        "/admin/settings/llm",
        "/admin/settings/runtime",
    ):
        response = client.get(path, headers=_admin_headers())
        _assert_admin_spa_shell(response)
```

The React route tests from Task 4 remain the acceptance check that `/admin/build-runners` and `/admin/llm-config` render `NotFoundPage` rather than old pages.

- [ ] **Step 2: Update operator documentation**

Document these exact paths:

```text
/admin/builds/apps       已接入应用和构建触发
/admin/builds/history    构建记录与制品
/admin/builds/runners    Runner 预配和状态
/admin/settings/general  Connector 默认模板
/admin/settings/notifications  钉钉配置
/admin/settings/llm      LLM 模型和功能绑定
/admin/settings/runtime  只读运行环境
```

Explain database-first/environment-fallback precedence and state that `.env` is never rewritten by the admin UI.

- [ ] **Step 3: Run the complete local verification suite**

Run each command independently:

```bash
ruff check src tests alembic
pytest -q
npm --prefix admin-web run lint
npm --prefix admin-web test -- --run
npm --prefix admin-web run build
python -m compileall -q src tests
```

Then run:

```bash
go test ./...
```

from both `connector/` and `build-runner/`.

Expected: all commands exit zero.

- [ ] **Step 4: Validate migration round trip and Compose files**

Run these migration commands against an isolated SQLite database:

```bash
env TESTFLYING_DATABASE_URL=sqlite:////tmp/testflying-settings-migration.db alembic upgrade head
env TESTFLYING_DATABASE_URL=sqlite:////tmp/testflying-settings-migration.db alembic downgrade 20260710_0011
env TESTFLYING_DATABASE_URL=sqlite:////tmp/testflying-settings-migration.db alembic upgrade head
```

Then run:

```bash
docker compose -f docker-compose.yml config
docker compose -f docker-compose.local.yml config
```

Expected: both upgrades reach `20260713_0012`, downgrade reaches `20260710_0011`, and both Compose files validate.

- [ ] **Step 5: Commit documentation and final regressions**

```bash
git add README.md docs/build-delivery.md docs/api-contract.md tests/test_admin_spa.py
git commit -m "docs(admin): document build and settings workspaces"
```

- [ ] **Step 6: Push, verify CI, and deploy**

Push `main`, wait for the GitHub Actions backend and package jobs to succeed, then update `/root/testflight-server`. Because this feature includes database migration, backend code, and frontend assets, rebuild the Docker Compose services rather than only pulling source. Verify `/health`, `/admin/api/bootstrap`, `/admin/api/settings`, and `/admin/api/builds/apps` on the deployed server without printing secrets.
