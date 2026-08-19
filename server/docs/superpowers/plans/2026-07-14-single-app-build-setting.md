# TestFlying Single App Build Setting Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将每个应用的开发环境、线上环境两份构建配置收敛为一份共享配置，同时保留构建时的环境选择并继续向 Runner 和 package-agent 传递环境。

**Architecture:** `AppBuildSetting` 改为与 `App` 一对一，Alembic 迁移按最后更新时间确定性合并旧记录。FastAPI 保存单个配置，创建构建时只接收环境和 Git ref，并在服务端读取共享配置生成不可变快照。React 的应用详情和构建工作区都使用同一份配置，环境只存在于构建操作区。

**Tech Stack:** Python 3.11、FastAPI、SQLAlchemy 2、Alembic、PostgreSQL/SQLite、React 19、TypeScript、Vite、Vitest、pytest、Go。

## Global Constraints

- 每个应用最多存在一条 `AppBuildSetting`。
- `AppBuildSetting` 不得包含环境字段或环境级覆盖。
- `Build.environment` 和 `Build.requested_environment` 继续只允许 `development`、`production`。
- Runner 和 package-agent 协议保持不变，并继续收到本次构建的环境。
- 已创建构建必须保留不可变配置快照，后续编辑共享配置不得影响历史任务。
- 旧双配置管理接口直接删除，不保留兼容别名。
- 数据迁移必须同时支持 PostgreSQL 和 SQLite。
- 所有手工文件编辑使用 `apply_patch`；业务改动先写失败测试，再实现。

---

### Task 1: Migrate AppBuildSetting to a one-to-one model

**Files:**
- Create: `alembic/versions/20260714_0013_single_app_build_setting.py`
- Create: `tests/test_single_app_build_setting_migration.py`
- Modify: `src/testflying_api/schema.py:17-68`
- Modify: `tests/test_schema.py`

**Interfaces:**
- Consumes: existing Alembic revision `20260713_0012` and legacy rows keyed by `(app_id, environment)`.
- Produces: revision `20260714_0013`, `App.build_setting: AppBuildSetting | None`, and a unique constraint named `uq_app_build_settings_app_id`.

- [ ] **Step 1: Add a failing full migration test**

Create a temporary SQLite database, migrate only to `20260713_0012`, insert one app with development and production settings, then upgrade to `20260714_0013`:

```python
from alembic import command
from alembic.config import Config
from sqlalchemy import create_engine, inspect, text


def _alembic_config(database_url: str) -> Config:
    config = Config("alembic.ini")
    config.set_main_option("sqlalchemy.url", database_url)
    return config


def test_single_setting_migration_keeps_latest_row(tmp_path, monkeypatch) -> None:
    database_url = f"sqlite:///{tmp_path / 'migration.db'}"
    monkeypatch.setenv("TESTFLYING_DATABASE_URL", database_url)
    config = _alembic_config(database_url)
    command.upgrade(config, "20260713_0012")
    engine = create_engine(database_url)
    with engine.begin() as connection:
        connection.execute(
            text(
                "INSERT INTO apps "
                "(id, name, bundle_identifier, platform, default_channel, icon_key, "
                "icon_color, added_at) "
                "VALUES ('app-1', 'Demo', 'com.example.demo', 'ios', 'dev', "
                "'app', '#53606E', '2026-07-14 00:00:00')"
            )
        )
        connection.execute(
            text(
                "INSERT INTO app_build_settings "
                "(id, app_id, environment, git_url, repo_subpath, runner_labels_json, "
                "credential_refs_json, artifact_type, optional_defaults_json, updated_at) "
                "VALUES "
                "('setting-dev', 'app-1', 'development', 'git://old', '', '[]', '{}', "
                "'ipa', '{}', '2026-07-14 01:00:00'), "
                "('setting-prod', 'app-1', 'production', 'git://latest', '', '[]', '{}', "
                "'ipa', '{}', '2026-07-14 02:00:00')"
            )
        )

    command.upgrade(config, "20260714_0013")

    with engine.connect() as connection:
        rows = connection.execute(
            text("SELECT id, app_id, git_url FROM app_build_settings")
        ).mappings().all()
    assert rows == [
        {"id": "setting-prod", "app_id": "app-1", "git_url": "git://latest"}
    ]
    assert "environment" not in {
        column["name"] for column in inspect(engine).get_columns("app_build_settings")
    }
```

Add a second test with equal `updated_at` values and reversed insert order; it must retain the development row. Add a third assertion that inserting another row for the same `app_id` raises `IntegrityError`.

- [ ] **Step 2: Run the migration test and verify it fails**

Run:

```bash
pytest -q tests/test_single_app_build_setting_migration.py
```

Expected: FAIL because revision `20260714_0013` does not exist.

- [ ] **Step 3: Implement the deterministic Alembic migration**

Create revision `20260714_0013` with `down_revision = "20260713_0012"`. In `upgrade()`, select legacy rows in this exact priority order and delete every row after the first row for each app:

```python
legacy = sa.table(
    "app_build_settings",
    sa.column("id", sa.String),
    sa.column("app_id", sa.String),
    sa.column("environment", sa.String),
    sa.column("updated_at", sa.DateTime(timezone=True)),
)
priority = sa.case((legacy.c.environment == "development", 0), else_=1)
rows = bind.execute(
    sa.select(legacy.c.id, legacy.c.app_id)
    .order_by(
        legacy.c.app_id.asc(),
        legacy.c.updated_at.desc(),
        priority.asc(),
        legacy.c.id.asc(),
    )
).all()
keep: set[str] = set()
drop: list[str] = []
for row in rows:
    if row.app_id in keep:
        drop.append(row.id)
    else:
        keep.add(row.app_id)
if drop:
    bind.execute(sa.delete(legacy).where(legacy.c.id.in_(drop)))
```

Then use `op.batch_alter_table("app_build_settings", recreate="always")` to drop `uq_app_build_settings_scope`, drop `environment`, and add `uq_app_build_settings_app_id` on `app_id`. The downgrade adds a non-null `environment` with server default `development`, removes the new constraint, restores `uq_app_build_settings_scope`, and removes the server default.

- [ ] **Step 4: Change the ORM relationship to one-to-one**

Replace `App.build_settings` with:

```python
build_setting: Mapped[AppBuildSetting | None] = relationship(
    back_populates="app",
    cascade="all, delete-orphan",
    single_parent=True,
    uselist=False,
)
```

Remove `environment` from `AppBuildSetting`, set `__table_args__` to
`UniqueConstraint("app_id", name="uq_app_build_settings_app_id")`, and update the reverse relation to `back_populates="build_setting"`.

Add a schema test that creates one `AppBuildSetting`, verifies `app.build_setting` is scalar, and verifies a second setting for the same app raises `IntegrityError`.

- [ ] **Step 5: Run model and migration tests**

Run:

```bash
pytest -q tests/test_single_app_build_setting_migration.py tests/test_schema.py
```

Expected: PASS.

- [ ] **Step 6: Commit the data-model slice**

```bash
git add alembic/versions/20260714_0013_single_app_build_setting.py \
  src/testflying_api/schema.py tests/test_schema.py \
  tests/test_single_app_build_setting_migration.py
git commit -m "refactor(builds): use one setting per app"
```

---

### Task 2: Make FastAPI read shared settings and snapshot them server-side

**Files:**
- Modify: `src/testflying_api/build_platform.py:93-285,693-706`
- Modify: `src/testflying_api/admin_api/schemas.py:669-705,828-837`
- Modify: `src/testflying_api/admin_api/routes.py:340-420`
- Modify: `tests/test_build_platform_api.py`
- Modify: `tests/test_admin_api_build_apps.py`

**Interfaces:**
- Consumes: `App.build_setting` from Task 1.
- Produces: `get_build_setting()`, `PUT /admin/api/apps/{app_id}/build-setting`, a minimal `AgentBuildCreateRequest`, `AppDetailState.build_setting`, and `BuildAppItem.setting`.

- [ ] **Step 1: Rewrite API tests for the shared contract**

Update the tests to require this response shape:

```python
assert response.json()["state"]["buildSetting"] == {
    "gitUrl": "git@example.com:mobile/demo.git",
    "repoSubpath": "apps/demo",
    "runnerLabels": ["ios-release", "mac-mini-1"],
    "credentialRefs": {"git": "git-main", "iosSigning": "ios-dev"},
    "artifactType": "ipa",
    "optionalDefaults": {"gitRef": "main"},
    "updatedAtLabel": response.json()["state"]["buildSetting"]["updatedAtLabel"],
}
```

Save through `PUT /admin/api/apps/{app.id}/build-setting` with no environment in the path or body. Save twice and assert `select(func.count(AppBuildSetting.id)) == 1`.

Before creating a build, save the shared setting. Create development and production builds with only:

```python
{"environment": "development", "gitRef": "main"}
{"environment": "production", "gitRef": "release/1.0"}
```

Assert both builds use the saved `git_url`, and assert their snapshots differ only in `environment` and `gitRef`:

```python
assert development.runner_labels_json["environment"] == "development"
assert production.runner_labels_json["environment"] == "production"
assert development.runner_labels_json["optionalDefaults"] == {"gitRef": "main"}
```

Add an explicit test that an unconfigured app returns `409` with code `build_setting_not_configured`. Remove tests that validate configuration fields on the build-create endpoint; those validations remain on the save endpoint.

Update `tests/test_admin_api_build_apps.py` to expect:

```python
item = response.json()["apps"][0]
assert item["setting"]["gitUrl"] == "git@example.com:mobile/demo.git"
assert item["matchingRunnerCount"] == 1
assert item["hasOnlineRunner"] is True
assert "environments" not in item
```

- [ ] **Step 2: Run focused backend tests and verify the red state**

Run:

```bash
pytest -q tests/test_build_platform_api.py tests/test_admin_api_build_apps.py
```

Expected: FAIL because the old API returns `settings` and `environments`, and build creation still requires full configuration.

- [ ] **Step 3: Replace environment-indexed schemas**

Change the response models to these stable shapes:

```python
class BuildSettingItem(AdminApiModel):
    git_url: str
    repo_subpath: str
    runner_labels: list[str]
    credential_refs: dict[str, str]
    artifact_type: str
    optional_defaults: dict[str, Any]
    updated_at_label: str


class BuildAppItem(AdminApiModel):
    app: BuildAppSummary
    setting: BuildSettingItem
    matching_runner_count: int
    has_online_runner: bool
    latest_build: BuildItem | None


class AgentBuildCreateRequest(AdminApiModel):
    environment: str
    git_ref: str


class AppDetailState(AdminApiModel):
    app: BuildAppSummary
    builds: list[BuildItem]
    build_setting: BuildSettingItem | None
```

Delete `BuildEnvironmentOption` and remove `environment` from `BuildSettingItem`.

- [ ] **Step 4: Implement shared setting queries and server-side snapshots**

Replace `settings_by_environment()` with:

```python
def get_build_setting(session: Session, app_id: str) -> AppBuildSetting | None:
    return session.scalar(
        select(AppBuildSetting).where(AppBuildSetting.app_id == app_id)
    )
```

`save_build_setting()` no longer accepts or parses environment. Its upsert query matches only `app_id`, and a new ID uses `build-setting-{app_id}-{uuid4().hex[:8]}`.

`create_agent_build()` accepts only `app_id`, `environment`, and `git_ref` after `session`. It loads the shared setting and raises:

```python
raise ApiError(
    "build_setting_not_configured",
    "请先配置应用的源码构建设置",
    status_code=409,
)
```

Build the immutable snapshot from the setting:

```python
runner_labels_json={
    "required": list(setting.runner_labels_json or []),
    "repoSubpath": setting.repo_subpath,
    "credentialRefs": dict(setting.credential_refs_json or {}),
    "artifactType": setting.artifact_type,
    "optionalDefaults": dict(setting.optional_defaults_json or {}),
    "environment": normalized_environment,
    "gitRef": normalized_git_ref,
}
```

Set `Build.git_url = setting.git_url`; retain the existing channel and environment mapping.

`build_apps_state()` joins and eager-loads `App.build_setting`, calculates matching nodes once per app, and emits one `setting` plus one match count. `app_detail_state()` emits `build_setting`.

- [ ] **Step 5: Replace the route and minimal build payload**

Change the save route to:

```python
@router.put(
    "/apps/{app_id}/build-setting",
    response_model=AppBuildActionResponse,
    response_model_by_alias=True,
)
```

Remove the environment path argument. In the build route call:

```python
build_platform.create_agent_build(
    session,
    app_id=app_id,
    environment=payload.environment,
    git_ref=payload.git_ref,
)
```

- [ ] **Step 6: Run all build-platform backend tests**

Run:

```bash
pytest -q tests/test_build_platform_api.py tests/test_admin_api_build_apps.py \
  tests/test_build_runner_api.py
```

Expected: PASS, including unchanged Runner assignment behavior.

- [ ] **Step 7: Commit the backend contract slice**

```bash
git add src/testflying_api/build_platform.py src/testflying_api/admin_api/schemas.py \
  src/testflying_api/admin_api/routes.py tests/test_build_platform_api.py \
  tests/test_admin_api_build_apps.py
git commit -m "refactor(builds): snapshot shared app settings"
```

---

### Task 3: Update the shared TypeScript API and app detail page

**Files:**
- Modify: `admin-web/src/app/apiClient.ts:601-685,1321-1348`
- Modify: `admin-web/src/pages/AppDetailPage.tsx`
- Modify: `admin-web/src/pages/AppDetailPage.test.tsx`
- Modify: `admin-web/src/styles/admin.css`

**Interfaces:**
- Consumes: Task 2 JSON contracts.
- Produces: `AppDetailState.buildSetting`, `BuildAppItem.setting`, `saveAppBuildSetting()`, and minimal `createAgentBuild()` request typing.

- [ ] **Step 1: Rewrite app-detail component tests**

Change the fixture from `settings.development/production` to one `buildSetting`. Assert there is exactly one “构建设置” form and no “测试环境设置” or “线上环境设置” headings.

Capture requests and require:

```typescript
expect(savedSettingsBody).toMatchObject({
  gitUrl: 'git@example.com:any/new-ios.git',
  optionalDefaults: {
    releaseChannel: 'internal',
    notifyGroups: ['qa', 'ios']
  }
});
expect(createdBuildBody).toEqual({
  environment: 'production',
  gitRef: 'release/1.2.0'
});
```

Mock the save URL as `/admin/api/apps/app-ios-demo/build-setting`. Add a test that both environment options remain enabled when one shared setting exists, and a test that “立即构建” is disabled when `buildSetting` is null.

- [ ] **Step 2: Run the app-detail test and verify it fails**

Run:

```bash
npm --prefix admin-web test -- --run src/pages/AppDetailPage.test.tsx
```

Expected: FAIL because the component still renders two environment-specific setting cards and submits full build configuration.

- [ ] **Step 3: Update API types and request functions**

Use these TypeScript shapes:

```typescript
export type BuildSettingItem = {
  gitUrl: string;
  repoSubpath: string;
  runnerLabels: string[];
  credentialRefs: Record<string, string>;
  artifactType: string;
  optionalDefaults: Record<string, unknown>;
  updatedAtLabel: string;
};

export type AppDetailState = {
  app: BuildAppSummary;
  builds: BuildItem[];
  buildSetting: BuildSettingItem | null;
};

export type AgentBuildCreateInput = {
  environment: 'development' | 'production';
  gitRef: string;
};
```

Rename `saveAppBuildSettings(appId, environment, payload)` to
`saveAppBuildSetting(appId, payload)` and use `/admin/api/apps/${appId}/build-setting`.

- [ ] **Step 4: Render one setting form and keep environment only in quick build**

Replace the drafts record with one `SettingDraft`. Initialize it from `payload.buildSetting`, preserve
`payload.buildSetting?.optionalDefaults ?? {}` on save, and submit builds with only environment and Git ref.

Render one `BuildSettingCard` titled “应用构建配置”. Its inputs remain Git URL, Artifact Type, Repo Subpath, Runner Labels, and Credential Refs. The quick-build environment select always contains:

```tsx
<option value="development">开发环境</option>
<option value="production">线上环境</option>
```

The shared summary does not change when the environment changes. Remove environment-specific card tones and obsolete CSS selectors; retain existing panel spacing and responsive behavior.

- [ ] **Step 5: Run app-detail tests, lint, and production build**

Run:

```bash
npm --prefix admin-web test -- --run src/pages/AppDetailPage.test.tsx
npm --prefix admin-web run lint
npm --prefix admin-web run build
```

Expected: all commands PASS.

- [ ] **Step 6: Commit the app-detail slice**

```bash
git add admin-web/src/app/apiClient.ts admin-web/src/pages/AppDetailPage.tsx \
  admin-web/src/pages/AppDetailPage.test.tsx admin-web/src/styles/admin.css
git commit -m "refactor(admin): share app build settings"
```

---

### Task 4: Simplify the build workspace configuration and verify end to end

**Files:**
- Modify: `admin-web/src/pages/BuildAppsPage.tsx`
- Modify: `admin-web/src/pages/BuildAppsPage.test.tsx`
- Modify: `admin-web/src/styles/admin.css`
- Modify: `docs/superpowers/specs/2026-07-13-admin-build-settings-navigation-design.md`

**Interfaces:**
- Consumes: `BuildAppItem.setting`, `matchingRunnerCount`, `hasOnlineRunner`, `saveAppBuildSetting()`, and minimal `createAgentBuild()` from Tasks 2-3.
- Produces: one build-setting dialog per app, a build-time environment selector, and a build workspace with no environment-specific configuration state.

- [ ] **Step 1: Rewrite build-workspace tests around one configuration**

Use this fixture shape:

```typescript
const buildAppsState = {
  total: 1,
  availableApps: [novelGoSummary],
  apps: [{
    app: appSummary,
    setting: {
      gitUrl: 'git@example.com:lookrva/ios.git',
      repoSubpath: 'ios-app',
      runnerLabels: ['mobile-release'],
      credentialRefs: { git: 'git-main' },
      artifactType: 'ipa',
      optionalDefaults: { gitRef: 'main' },
      updatedAtLabel: '2026-07-14 10:00'
    },
    matchingRunnerCount: 1,
    hasOnlineRunner: true,
    latestBuild: null
  }]
};
```

Assert the build form defaults to development, can switch to production, and sends:

```typescript
expect(createdBuildBody).toEqual({
  environment: 'production',
  gitRef: 'release/latest'
});
```

Open “编辑构建配置” and assert there is no `aria-label="构建环境"` navigation and only one “保存构建配置” button. Test the in-place onboarding flow uses `/build-setting`, then refreshes the configured list while preserving the selected app. Keep the existing test for “saved but refresh failed” and update only its expected shared-config wording.

- [ ] **Step 2: Run the build-workspace tests and verify the red state**

Run:

```bash
npm --prefix admin-web test -- --run src/pages/BuildAppsPage.test.tsx
```

Expected: FAIL because `BuildAppsPage` still indexes settings by environment.

- [ ] **Step 3: Remove environment-specific state from BuildAppsPage**

Keep `environment` as a `BuildEnvironment` with default `development`, independent from the selected app. Replace `selectedEnvironment` with `selectedApp.setting`. `submitBuild()` sends:

```typescript
await createAgentBuild(selectedApp.app.id, {
  environment,
  gitRef: gitRef.trim()
});
```

Use `defaultGitRef(selectedApp.setting)` and keep Git ref when refreshing the same app. The app list displays repository, platform, shared Runner labels, one match summary, and latest build; remove the list of configured environment labels.

The environment select always shows both values:

```tsx
<select value={environment} onChange={(event) => selectEnvironment(event.target.value)}>
  <option value="development">开发环境</option>
  <option value="production">线上环境</option>
</select>
```

- [ ] **Step 4: Reduce BuildSettingsDialog to one draft**

Load `payload.buildSetting` into one draft, remove `BuildEnvironment`, `drafts`, segmented environment navigation, and `environmentLabel()` from configuration UI. Save through `saveAppBuildSetting()` and show `构建配置已保存`.

The footer reads `此配置同时用于开发环境和线上环境；具体环境在发起构建时选择。` The primary action label is always `保存构建配置` or `保存中...`.

- [ ] **Step 5: Update the superseded navigation spec sentence**

In `2026-07-13-admin-build-settings-navigation-design.md`, replace environment-specific wording in “应用构建” and the old non-goal with a direct reference to the approved single-setting spec:

```markdown
应用源码构建配置采用每个应用一份共享配置；开发环境和线上环境只在发起构建时选择。
详细数据和接口约束见 `2026-07-14-single-app-build-setting-design.md`。
```

- [ ] **Step 6: Run focused frontend and backend regression tests**

Run:

```bash
npm --prefix admin-web test -- --run \
  src/pages/BuildAppsPage.test.tsx src/pages/AppDetailPage.test.tsx
pytest -q tests/test_single_app_build_setting_migration.py \
  tests/test_build_platform_api.py tests/test_admin_api_build_apps.py \
  tests/test_build_runner_api.py
```

Expected: PASS.

- [ ] **Step 7: Run complete repository verification**

Run:

```bash
pytest -q
ruff check src tests
python -m compileall -q src
npm --prefix admin-web test -- --run
npm --prefix admin-web run lint
npm --prefix admin-web run build
(cd build-runner && go test ./...)
(cd connector && go test ./...)
git diff --check
```

Expected: all commands exit `0`. Record the exact pytest and Vitest counts in the completion report.

- [ ] **Step 8: Perform browser verification**

Start the local API and admin bundle using the repository's existing development commands. Verify at desktop and narrow desktop widths:

1. “接入已有应用” opens one shared configuration form.
2. Editing a configured app never shows development/production configuration tabs.
3. The build form still switches between development and production.
4. Switching environment does not change repository or credential summary.
5. Saving and creating a build show in-place loading, success, and error states without page navigation or layout jump.

- [ ] **Step 9: Commit the completed workspace slice**

```bash
git add admin-web/src/pages/BuildAppsPage.tsx \
  admin-web/src/pages/BuildAppsPage.test.tsx admin-web/src/styles/admin.css \
  docs/superpowers/specs/2026-07-13-admin-build-settings-navigation-design.md
git commit -m "refactor(admin): configure builds once per app"
```

After all commits, push `main`, wait for GitHub Actions to succeed, deploy only the API service in `/root/testflight-server`, and verify `/health`, the deployed commit, and that the Connector container was not recreated.
