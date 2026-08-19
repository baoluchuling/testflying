# Remove Repository Subpath Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 删除 TestFlying 构建链路中的仓库子目录配置，使所有构建固定从克隆仓库根目录执行。

**Architecture:** 数据库迁移删除 `app_build_settings.repo_subpath`，管理 API 和构建任务快照不再产生 `repoSubpath`。Runner 领取任务后直接把 checkout 根目录写入 `build-input.json` 的 `projectDir`，后台应用详情和构建工作台同步删除该字段。

**Tech Stack:** FastAPI、SQLAlchemy 2、Alembic、PostgreSQL/SQLite、React 19、TypeScript、Vitest、Go。

## Global Constraints

- 一个构建应用始终对应一个独立 Git 仓库，仓库根目录就是应用项目目录。
- 不支持 Monorepo、工作目录映射或仓库内多应用选择。
- 不保留 `repoSubpath` 请求、响应、数据库或 Runner 协议兼容逻辑。
- Git ref、产物类型、Runner 标签、凭据引用和默认参数行为保持不变。
- 中心后台和 Runner 使用同一版本部署。

---

## File Map

- `alembic/versions/20260714_0014_remove_repo_subpath.py`：删除并可回滚 `repo_subpath` 列。
- `src/testflying_api/schema.py`：移除 ORM 字段。
- `src/testflying_api/build_platform.py`：移除保存参数、任务快照字段和路径校验。
- `src/testflying_api/admin_api/schemas.py`：移除管理 API 与 Runner 协议字段。
- `src/testflying_api/admin_api/routes.py`：停止接收和发送仓库子目录。
- `build-runner/internal/runner/workspace.go`：保留 checkout 根目录函数，移除子路径解析。
- `build-runner/internal/runner/loop.go`：固定使用 checkout 根目录。
- `admin-web/src/app/apiClient.ts`：移除 TypeScript 类型字段。
- `admin-web/src/pages/AppDetailPage.tsx`、`BuildAppsPage.tsx`：移除输入、摘要和草稿状态。
- `docs/build-delivery.md`：更新当前构建配置说明。

### Task 1: 删除数据库列与 ORM 字段

**Files:**
- Create: `alembic/versions/20260714_0014_remove_repo_subpath.py`
- Create: `tests/test_remove_repo_subpath_migration.py`
- Modify: `src/testflying_api/schema.py`
- Modify: `tests/test_schema.py`

**Interfaces:**
- Produces: `AppBuildSetting` 不再具有 `repo_subpath`；Alembic head 为 `20260714_0014`。

- [ ] **Step 1: 写迁移与 ORM 失败测试**

新增迁移测试：先升级到 `20260714_0013`，插入 `repo_subpath='legacy/app'` 的配置，再升级 head 并断言列消失；降级到 `20260714_0013` 后断言列恢复且旧记录值为空字符串。

```python
command.upgrade(config, "head")
engine = create_engine(database_url)
assert "repo_subpath" not in {
    column["name"] for column in inspect(engine).get_columns("app_build_settings")
}
engine.dispose()

command.downgrade(config, "20260714_0013")
engine = create_engine(database_url)
assert "repo_subpath" in {
    column["name"] for column in inspect(engine).get_columns("app_build_settings")
}
with engine.connect() as connection:
    value = connection.execute(
        text("SELECT repo_subpath FROM app_build_settings")
    ).scalar_one()
assert value == ""
engine.dispose()
```

在 `test_app_has_one_scalar_build_setting` 中删除两个 `repo_subpath=""`，并增加：

```python
assert "repo_subpath" not in AppBuildSetting.__table__.columns
```

- [ ] **Step 2: 运行测试确认失败**

Run: `PYTHONPATH=src .venv/bin/python -m pytest -q tests/test_remove_repo_subpath_migration.py tests/test_schema.py`

Expected: FAIL，head 和 ORM 仍包含 `repo_subpath`。

- [ ] **Step 3: 实现迁移与 ORM 删除**

```python
revision = "20260714_0014"
down_revision = "20260714_0013"


def upgrade() -> None:
    with op.batch_alter_table("app_build_settings", recreate="always") as batch_op:
        batch_op.drop_column("repo_subpath")


def downgrade() -> None:
    with op.batch_alter_table("app_build_settings", recreate="always") as batch_op:
        batch_op.add_column(
            sa.Column(
                "repo_subpath",
                sa.String(length=240),
                nullable=False,
                server_default=sa.text("''"),
            )
        )
    with op.batch_alter_table("app_build_settings") as batch_op:
        batch_op.alter_column(
            "repo_subpath",
            existing_type=sa.String(length=240),
            server_default=None,
        )
```

从 `AppBuildSetting` 删除字段。`tests/test_single_app_build_setting_migration.py` 继续保留 `repo_subpath`，因为它验证的是 `20260714_0013` 及更早版本的历史表结构。

- [ ] **Step 4: 运行迁移和 ORM 测试确认通过**

Run: `PYTHONPATH=src .venv/bin/python -m pytest -q tests/test_remove_repo_subpath_migration.py tests/test_single_app_build_setting_migration.py tests/test_schema.py`

Expected: PASS。

- [ ] **Step 5: 提交数据库变更**

```bash
git add alembic/versions/20260714_0014_remove_repo_subpath.py src/testflying_api/schema.py tests/test_remove_repo_subpath_migration.py tests/test_schema.py
git commit -m "refactor(builds): remove repository subpath storage"
```

### Task 2: 删除后端 API 与任务快照字段

**Files:**
- Modify: `src/testflying_api/build_platform.py`
- Modify: `src/testflying_api/admin_api/schemas.py`
- Modify: `src/testflying_api/admin_api/routes.py`
- Modify: `tests/test_build_platform_api.py`
- Modify: `tests/test_build_runner_api.py`
- Modify: `tests/test_admin_api_build_apps.py`

**Interfaces:**
- Consumes: Task 1 中无 `repo_subpath` 的 `AppBuildSetting`。
- Produces: `save_build_setting` 不接收路径；`BuildSettingItem`、`BuildSettingSaveRequest`、`RunnerBuildPayload` 不含路径。

- [ ] **Step 1: 更新 API 测试为无路径协议**

从保存请求删除 `repoSubpath`，删除路径穿越参数化测试，并增加：

```python
assert "repoSubpath" not in payload["state"]["buildSetting"]
assert "repoSubpath" not in development.runner_labels_json
```

Runner poll 的精确响应改为：

```python
assert payload["build"] == {
    "id": "build-agent-queued-1",
    "appId": app.id,
    "platform": "ios",
    "environment": "development",
    "gitUrl": "git@example.com:demo.git",
    "gitRef": "main",
    "artifactType": "ipa",
    "credentialRefs": {"git": "git-main"},
}
```

- [ ] **Step 2: 运行聚焦测试确认失败**

Run: `PYTHONPATH=src .venv/bin/python -m pytest -q tests/test_build_platform_api.py tests/test_build_runner_api.py tests/test_admin_api_build_apps.py`

Expected: FAIL，当前响应和任务快照仍包含 `repoSubpath`。

- [ ] **Step 3: 删除服务、路由和 Pydantic 字段**

`save_build_setting` 删除 `repo_subpath` 参数、标准化和赋值。构建快照结果为：

```python
runner_labels_json={
    "required": list(setting.runner_labels_json or []),
    "credentialRefs": dict(setting.credential_refs_json or {}),
    "artifactType": setting.artifact_type,
    "optionalDefaults": dict(setting.optional_defaults_json or {}),
    "environment": normalized_environment,
    "gitRef": normalized_git_ref,
},
```

删除 `_normalize_repo_subpath`、`_invalid_repo_subpath` 和 `PurePosixPath` import。从三个 API model 删除 `repo_subpath`，路由停止传参，`_runner_build_payload` 停止读取 `runner_data["repoSubpath"]`。

- [ ] **Step 4: 运行聚焦测试确认通过**

Run: `PYTHONPATH=src .venv/bin/python -m pytest -q tests/test_build_platform_api.py tests/test_build_runner_api.py tests/test_admin_api_build_apps.py`

Expected: PASS。

- [ ] **Step 5: 提交后端协议变更**

```bash
git add src/testflying_api/build_platform.py src/testflying_api/admin_api tests/test_build_platform_api.py tests/test_build_runner_api.py tests/test_admin_api_build_apps.py
git commit -m "refactor(builds): remove repository subpath protocol"
```

### Task 3: 固定 Runner 从仓库根目录构建

**Files:**
- Modify: `build-runner/internal/runner/workspace.go`
- Modify: `build-runner/internal/runner/loop.go`
- Modify: `build-runner/internal/runner/runner_test.go`

**Interfaces:**
- Consumes: Task 2 中不含 `repoSubpath` 的任务。
- Produces: `BuildInput.ProjectDir == CheckoutPath(workspace)`。

- [ ] **Step 1: 更新 Runner 测试协议和目录期望**

任务响应 helper 只接收 Git URL，不再输出 `repoSubpath`。主执行测试改为：

```go
expectedProjectDir := CheckoutPath(workspace)
if gotInput.ProjectDir != expectedProjectDir {
	t.Fatalf("projectDir = %q, want %q", gotInput.ProjectDir, expectedProjectDir)
}
```

删除 `TestSafeProjectDirPathRejectsTraversalAndSymlinkEscape`，所有 source repo fixture 直接在仓库根目录创建测试文件。

- [ ] **Step 2: 运行 Runner 测试确认失败**

Run: `cd build-runner && go test ./internal/runner`

Expected: FAIL，Runner 类型和处理逻辑仍要求 `RepoSubpath`。

- [ ] **Step 3: 删除 Runner 子目录处理**

从 `BuildAssignment` 和 `BuildInput` 删除：

```go
RepoSubpath string `json:"repoSubpath"`
```

从 `workspace.go` 删除 `ProjectDirPath`、`SafeProjectDirPath` 和 `ValidateRepoSubpath`。`handleBuild` 在 checkout 成功后直接设置：

```go
projectDir := CheckoutPath(workspace)
```

删除 `preAgentRepoSubpathFailureClassification` 及其失败分支，写入 `BuildInput` 时不再设置 `RepoSubpath`。

- [ ] **Step 4: 格式化并运行 Runner 测试**

Run:

```bash
cd build-runner
gofmt -w internal/runner/workspace.go internal/runner/loop.go internal/runner/runner_test.go
go test ./...
```

Expected: PASS。

- [ ] **Step 5: 提交 Runner 变更**

```bash
git add build-runner/internal/runner
git commit -m "refactor(runner): build from repository root"
```

### Task 4: 删除后台字段与状态

**Files:**
- Modify: `admin-web/src/app/apiClient.ts`
- Modify: `admin-web/src/pages/AppDetailPage.tsx`
- Modify: `admin-web/src/pages/BuildAppsPage.tsx`
- Modify: `admin-web/src/pages/AppDetailPage.test.tsx`
- Modify: `admin-web/src/pages/BuildAppsPage.test.tsx`
- Modify: `admin-web/src/app/AdminApp.test.tsx`

**Interfaces:**
- Consumes: Task 2 中不含 `repoSubpath` 的 JSON。
- Produces: 构建配置 UI 只展示 Git 仓库、产物类型、节点标签、凭据引用和默认参数。

- [ ] **Step 1: 修改 fixture 并添加不可见断言**

从全部构建配置 fixture 删除 `repoSubpath`。打开应用详情和构建配置弹窗后增加：

```typescript
expect(screen.queryByText('仓库子目录')).toBeNull();
```

保存请求断言使用：

```typescript
expect(JSON.parse(String(request?.[1]?.body))).toEqual({
  gitUrl: 'git@example.com:novelgo/app.git',
  runnerLabels: [],
  credentialRefs: {},
  artifactType: 'apk',
  optionalDefaults: {}
});
```

- [ ] **Step 2: 运行前端聚焦测试确认失败**

Run: `npm --prefix admin-web test -- --run src/pages/AppDetailPage.test.tsx src/pages/BuildAppsPage.test.tsx src/app/AdminApp.test.tsx`

Expected: FAIL，页面或保存请求仍包含仓库子目录。

- [ ] **Step 3: 删除 TypeScript 与组件字段**

从 `BuildSettingItem`、`BuildSettingSavePayload`、两个页面的 draft 类型、初始化和保存 payload 删除 `repoSubpath`。删除表单输入和配置摘要。构建列表摘要固定为：

```typescript
function settingSummary(item: BuildAppItem): string {
  return item.setting.gitUrl;
}
```

- [ ] **Step 4: 运行前端测试、类型检查和构建**

Run:

```bash
npm --prefix admin-web test -- --run
npm --prefix admin-web run lint
npm --prefix admin-web run build
```

Expected: 全部测试 PASS，TypeScript 无错误，Vite 构建成功。

- [ ] **Step 5: 提交后台变更**

```bash
git add admin-web/src
git commit -m "refactor(admin): remove repository subpath setting"
```

### Task 5: 更新文档并执行全量验证

**Files:**
- Modify: `docs/build-delivery.md`

**Interfaces:**
- Consumes: Tasks 1-4 的最终行为。
- Produces: 当前交付文档只描述单一应用配置和仓库根目录构建。

- [ ] **Step 1: 更新当前构建说明**

将“后台配置构建”说明改为：

```markdown
在“应用构建”页面中从已有应用接入构建。每个应用只维护一份源码构建配置，Runner 克隆 Git 仓库后始终从仓库根目录执行。创建任务时再选择 `development` 或 `production`，环境不会切换配置。

应用配置包含 Git 仓库、Runner 标签、制品类型、凭据引用名和默认参数；构建任务包含 Git ref 和目标环境。
```

- [ ] **Step 2: 检查运行时代码不再含旧字段**

Run:

```bash
rg -n "repo_subpath|repoSubpath|仓库子目录|Repo Subpath" src alembic build-runner admin-web docs/build-delivery.md tests --glob '!src/testflying_api/static/admin-app/**'
```

Expected: 仅迁移往返测试允许出现 `repo_subpath`；运行时代码、Runner、前端和当前交付文档无结果。

- [ ] **Step 3: 执行完整后端与静态检查**

Run:

```bash
PYTHONPATH=src .venv/bin/python -m pytest -q
.venv/bin/python -m ruff check src tests alembic
```

Expected: 全部 PASS，Ruff 输出 `All checks passed!`。

- [ ] **Step 4: 执行完整前端与 Go 验证**

Run:

```bash
npm --prefix admin-web test -- --run
npm --prefix admin-web run lint
npm --prefix admin-web run build
cd build-runner && go test ./...
cd ../connector && go test ./...
```

Expected: 前端测试、类型检查、生产构建和两个 Go 模块全部通过。

- [ ] **Step 5: 验证迁移往返与工作树**

Run:

```bash
tmp_db="$(mktemp -t testflying-repo-root).db"
TESTFLYING_DATABASE_URL="sqlite:///$tmp_db" .venv/bin/alembic upgrade head
TESTFLYING_DATABASE_URL="sqlite:///$tmp_db" .venv/bin/alembic downgrade 20260714_0013
TESTFLYING_DATABASE_URL="sqlite:///$tmp_db" .venv/bin/alembic upgrade head
rm -f "$tmp_db"
git diff --check
git status --short
```

Expected: 三次 Alembic 命令成功，`git diff --check` 无输出，状态只包含预期文档改动。

- [ ] **Step 6: 提交文档**

```bash
git add docs/build-delivery.md docs/superpowers/plans/2026-07-14-remove-repo-subpath.md
git commit -m "docs(builds): document repository root builds"
```
