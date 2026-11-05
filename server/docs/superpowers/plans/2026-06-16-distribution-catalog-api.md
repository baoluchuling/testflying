# 分发目录 API 实现计划

> **给执行代理的要求：**执行本计划时必须使用 `superpowers:subagent-driven-development`（如果当前环境支持子代理）或 `superpowers:executing-plans`。所有任务都使用复选框语法跟踪。

**目标：**把 `testflying-api` 实现成一个无用户态的内部应用分发目录 API，只提供包、应用、构建、设备可见性、账号续费、通知和下载地址这些服务端事实。

**架构：**FastAPI 提供只读目录接口和包上传接口；SQLAlchemy 保存应用、构建、制品、设备、开发者账号和服务端通知等分发事实。移动端继续负责安装状态、暂停/下载中状态、排序、通知已读、筛选、tab、sheet、滚动位置等所有客户端状态。

**技术栈：**Python 3.11、FastAPI、Pydantic v2、SQLAlchemy 2.x、Alembic、PostgreSQL、MinIO/S3、pytest、httpx、ruff、Docker、Docker Compose。第一版默认用 PostgreSQL 和 MinIO/S3；本地轻量测试可以兼容 SQLite 和本地 data 目录。

---

## 范围和非目标

服务端只作为“分发事实”的权威来源：

- 应用和构建。
- 构建环境分类：`development` 或 `production`。
- 构建元数据：bundle id 或 package name、版本号、build number、平台、更新说明、发布时间。
- 制品信息：IPA/APK 存储路径、iOS manifest 地址、Android 下载地址。
- 开发者账号续费事实。
- 设备登记事实和构建可见性规则。
- 服务端产生的通知 feed，例如新包发布、账号即将到期。

服务端不能保存用户态或客户端状态：

- 不保存是否已安装。
- 不保存安装中、暂停中、下载中。
- 不保存下载进度。
- 不保存用户自定义排序。
- 不保存通知已读/未读。
- 不保存当前 tab、筛选、sheet、滚动位置。
- 不保存“某台设备装过某个包”这类本地使用痕迹。

以下旧接口不实现：

- `POST /v1/test-distribution/builds/{buildId}/install-tasks`
- `PATCH /v1/test-distribution/install-tasks/{taskId}`
- `GET /v1/test-distribution/install-tasks/{taskId}`
- `PUT /v1/test-distribution/users/me/build-sort-order`
- `PATCH /v1/test-distribution/notifications/{notificationId}`
- `POST /v1/test-distribution/notifications/mark-all-read`

## 文件结构

在 `/Users/admin/ai_project/apps/testflying-api` 中创建或修改这些文件：

- `pyproject.toml`：增加 SQLAlchemy、Alembic、psycopg、boto3、python-multipart 和测试依赖。
- `src/testflying_api/config.py`：配置对象，包含数据库地址、公开基础 URL、存储 backend、S3/MinIO 参数、本地存储路径、静态 token。
- `src/testflying_api/app.py`：FastAPI app factory。
- `src/testflying_api/main.py`：从 app factory 导出 `app`。
- `src/testflying_api/errors.py`：统一 API 错误模型和异常处理。
- `src/testflying_api/database.py`：SQLAlchemy engine、session factory、依赖注入。
- `src/testflying_api/schema.py`：SQLAlchemy 数据表模型。
- `src/testflying_api/domain.py`：和 FastAPI 解耦的领域 dataclass / enum。
- `src/testflying_api/catalog_repository.py`：查询应用、构建、设备、账号、通知。
- `src/testflying_api/catalog_service.py`：组合 workspace，处理设备可见性。
- `src/testflying_api/storage.py`：S3/MinIO 制品存储、本地兼容存储和公开 URL 生成。
- `src/testflying_api/package_parser.py`：解析 IPA 包信息；Android 第一版使用上传时附带的 metadata。
- `src/testflying_api/manifest.py`：生成 iOS `manifest.plist`。
- `src/testflying_api/routes/health.py`：健康检查接口。
- `src/testflying_api/routes/workspace.py`：workspace 聚合接口。
- `src/testflying_api/routes/uploads.py`：包上传接口。
- `src/testflying_api/routes/devices.py`：设备读取和登记链接接口。
- `src/testflying_api/routes/accounts.py`：开发者账号接口。
- `src/testflying_api/routes/notifications.py`：通知 feed 接口。
- `alembic.ini`：Alembic 配置。
- `alembic/env.py`：迁移环境。
- `alembic/versions/*.py`：数据库迁移。
- `tests/conftest.py`：测试 app 和临时数据库 fixture。
- `tests/test_workspace.py`：workspace 契约测试。
- `tests/test_uploads.py`：上传和制品 URL 测试。
- `tests/test_devices.py`：设备可见性测试。
- `tests/test_notifications.py`：通知 feed 测试，明确不写已读状态。
- `README.md`：本地启动、测试、客户端连接说明。
- `Dockerfile`：第一版部署镜像。
- `.dockerignore`：排除本地缓存、测试文件和数据目录。
- `docker-compose.yml`：第一版全栈部署入口，默认启动 API、PostgreSQL、MinIO 和 bucket 初始化容器。
- `docker-compose.local.yml`：本地轻量测试入口，使用 SQLite 和本地 `./data`。

## 阶段零：第一版 Docker 全栈部署入口

### 任务 0：Docker Compose 全栈支持

**文件：**

- 新建：`Dockerfile`
- 新建：`.dockerignore`
- 新建：`docker-compose.yml`
- 新建：`docker-compose.local.yml`
- 修改：`README.md`

- [ ] **步骤 1：创建 Dockerfile**

要求：

- 基于 `python:3.11-slim`。
- 安装当前项目运行依赖。
- 使用非 root 用户运行。
- 暴露 `8000` 端口。
- 默认启动命令为：

```bash
uvicorn testflying_api.main:app --host 0.0.0.0 --port 8000
```

- [ ] **步骤 2：创建 .dockerignore**

必须排除：

- `.git/`
- `.venv/`
- `.pytest_cache/`
- `.ruff_cache/`
- `__pycache__/`
- `data/`
- `tests/`
- `docs/`

- [ ] **步骤 3：创建默认 docker-compose.yml**

第一版默认必须包含这些服务：

- `api`：FastAPI 服务，端口映射 `8000:8000`。
- `postgres`：PostgreSQL 16，保存分发事实。
- `minio`：S3 兼容对象存储，保存 IPA/APK 和 manifest。
- `minio-init`：启动后创建 `testflying` bucket。

`api` 默认环境变量：

- `TESTFLYING_DATABASE_URL=postgresql+psycopg://testflying:testflying@postgres:5432/testflying`
- `TESTFLYING_PUBLIC_BASE_URL=http://localhost:8000`
- `TESTFLYING_STORAGE_BACKEND=s3`
- `TESTFLYING_S3_ENDPOINT_URL=http://minio:9000`
- `TESTFLYING_S3_PUBLIC_BASE_URL=http://localhost:9000/testflying`
- `TESTFLYING_S3_BUCKET=testflying`
- `TESTFLYING_S3_ACCESS_KEY_ID=testflying`
- `TESTFLYING_S3_SECRET_ACCESS_KEY=testflying-secret`
- `TESTFLYING_STATIC_TOKEN=dev-token`

PostgreSQL 和 MinIO 的默认账号密码只能用于开发和内网试部署，正式部署前必须改掉。

- [ ] **步骤 4：创建本地轻量 docker-compose.local.yml**

本地环境不完整时，允许使用 SQLite 和本地 data 目录：

- 只启动 `api`。
- `TESTFLYING_DATABASE_URL=sqlite:////app/data/testflying.db`
- `TESTFLYING_STORAGE_BACKEND=local`
- `TESTFLYING_STORAGE_ROOT=/app/data/artifacts`
- 挂载 `./data:/app/data`。

- [ ] **步骤 5：更新 README**

README 必须把 Docker 部署放在本地开发前面：

```bash
docker compose up --build
curl http://localhost:8000/health
```

README 还必须写清：

- 默认 Compose 是 API + PostgreSQL + MinIO。
- MinIO 控制台地址是 `http://localhost:9001`。
- 真实 iOS OTA 下载需要设备可访问的 HTTPS 公开地址。
- 本地轻量测试可以运行：

```bash
docker compose -f docker-compose.local.yml up --build
```

同时补充无 Compose 插件时的直接 Docker 轻量启动方式：

```bash
docker build -t testflying-api:latest .
docker run -d --name testflying-api -p 8000:8000 -v "$(pwd)/data:/app/data" testflying-api:latest
```

本地开发继续保留最简单的 Python 虚拟环境路径。

- [ ] **步骤 6：验证 Docker 构建和健康检查**

运行：

```bash
docker compose build
docker compose up -d
curl http://localhost:8000/health
docker compose down
```

预期：`curl` 返回 `{"status":"ok"}`。

- [ ] **步骤 7：提交**

```bash
git add Dockerfile .dockerignore docker-compose.yml docker-compose.local.yml README.md
git commit -m "feat: add Docker full stack deployment"
```

## 阶段一：项目基础

### 任务 1：配置和应用工厂

**文件：**

- 新建：`src/testflying_api/config.py`
- 新建：`src/testflying_api/app.py`
- 修改：`src/testflying_api/main.py`
- 新建：`src/testflying_api/routes/health.py`
- 测试：`tests/test_health.py`

- [ ] **步骤 1：先写失败测试**

创建 `tests/test_health.py`：

```python
from fastapi.testclient import TestClient

from testflying_api.app import create_app


def test_health_returns_ok() -> None:
    client = TestClient(create_app())

    response = client.get("/health")

    assert response.status_code == 200
    assert response.json() == {"status": "ok"}
```

- [ ] **步骤 2：运行测试并确认失败**

运行：

```bash
pytest tests/test_health.py -v
```

预期：失败，原因是 `testflying_api.app` 还不存在。

- [ ] **步骤 3：实现配置和应用工厂**

实现 `Settings.from_environment()`，读取：

- `TESTFLYING_DATABASE_URL`
- `TESTFLYING_PUBLIC_BASE_URL`
- `TESTFLYING_STORAGE_ROOT`
- `TESTFLYING_STATIC_TOKEN`

实现 `create_app()`，注册 health route。`main.py` 只导出：

```python
from __future__ import annotations

from testflying_api.app import create_app

app = create_app()
```

- [ ] **步骤 4：运行测试并确认通过**

运行：

```bash
pytest tests/test_health.py -v
```

预期：通过。

- [ ] **步骤 5：提交**

```bash
git add src/testflying_api/config.py src/testflying_api/app.py src/testflying_api/main.py src/testflying_api/routes/health.py tests/test_health.py
git commit -m "feat: add API app foundation"
```

### 任务 2：统一错误响应

**文件：**

- 新建：`src/testflying_api/errors.py`
- 修改：`src/testflying_api/app.py`
- 测试：`tests/test_errors.py`

- [ ] **步骤 1：先写失败测试**

```python
from fastapi.testclient import TestClient

from testflying_api.app import create_app
from testflying_api.errors import ApiError


def test_api_errors_use_client_contract_shape() -> None:
    app = create_app()

    @app.get("/boom")
    def boom() -> None:
        raise ApiError("build_not_found", "构建不存在", status_code=404)

    client = TestClient(app)
    response = client.get("/boom")

    assert response.status_code == 404
    assert response.json() == {
        "code": "build_not_found",
        "message": "构建不存在",
        "retryable": False,
    }
```

- [ ] **步骤 2：运行测试并确认失败**

```bash
pytest tests/test_errors.py -v
```

预期：失败，原因是 `ApiError` 还没有实现。

- [ ] **步骤 3：实现错误处理**

`ApiError` 携带：

- `code`
- `message`
- `status_code`
- `retryable`

在 `create_app()` 中注册异常处理器，响应形状和客户端契约一致：

```json
{
  "code": "build_not_found",
  "message": "构建不存在",
  "retryable": false
}
```

- [ ] **步骤 4：运行测试并确认通过**

```bash
pytest tests/test_errors.py -v
```

预期：通过。

- [ ] **步骤 5：提交**

```bash
git add src/testflying_api/errors.py src/testflying_api/app.py tests/test_errors.py
git commit -m "feat: add API error contract"
```

## 阶段二：只保存分发事实的持久化模型

### 任务 3：数据库和数据表

**文件：**

- 修改：`pyproject.toml`
- 新建：`src/testflying_api/database.py`
- 新建：`src/testflying_api/schema.py`
- 新建：`alembic.ini`
- 新建：`alembic/env.py`
- 新建：`alembic/versions/20260616_0001_initial_catalog.py`
- 测试：`tests/test_schema.py`

- [ ] **步骤 1：增加依赖**

在 `pyproject.toml` 中增加：

```toml
"sqlalchemy>=2.0,<3.0",
"alembic>=1.14,<2.0",
"psycopg[binary]>=3.2,<4.0",
"boto3>=1.35,<2.0",
"python-multipart>=0.0.20,<1.0",
```

- [ ] **步骤 2：先写数据表边界测试**

```python
from sqlalchemy import inspect

from testflying_api.database import create_engine_for_url
from testflying_api.schema import Base


def test_catalog_schema_contains_no_user_state_tables() -> None:
    engine = create_engine_for_url("sqlite:///:memory:")
    Base.metadata.create_all(engine)

    table_names = set(inspect(engine).get_table_names())

    assert {
        "apps",
        "builds",
        "artifacts",
        "devices",
        "developer_accounts",
        "notifications",
        "device_build_visibility",
    }.issubset(table_names)
    assert "install_tasks" not in table_names
    assert "sort_orders" not in table_names
    assert "notification_reads" not in table_names
```

- [ ] **步骤 3：运行测试并确认失败**

```bash
pytest tests/test_schema.py -v
```

预期：失败，原因是数据库模块和 schema 还不存在。

- [ ] **步骤 4：实现数据表**

需要的数据表：

- `apps`：应用事实。
- `builds`：构建事实。
- `artifacts`：IPA/APK 制品事实。
- `devices`：设备登记事实。
- `device_build_visibility`：设备和构建的可见性关系。
- `developer_accounts`：开发者账号续费事实。
- `developer_account_apps`：开发者账号和应用的关联。
- `notifications`：服务端产生的通知 feed。

禁止新增这些表：

- `install_tasks`
- `sort_orders`
- `notification_reads`

- [ ] **步骤 5：运行测试并确认通过**

```bash
pytest tests/test_schema.py -v
```

预期：通过。

- [ ] **步骤 6：提交**

```bash
git add pyproject.toml src/testflying_api/database.py src/testflying_api/schema.py alembic.ini alembic tests/test_schema.py
git commit -m "feat: add catalog persistence schema"
```

### 任务 4：演示数据

**文件：**

- 新建：`src/testflying_api/seed.py`
- 修改：`tests/conftest.py`
- 测试：`tests/test_seed.py`

- [ ] **步骤 1：先写失败测试**

```python
from sqlalchemy.orm import Session

from testflying_api.schema import App, Build
from testflying_api.seed import seed_demo_catalog


def test_seed_demo_catalog_creates_apps_and_builds(db_session: Session) -> None:
    seed_demo_catalog(db_session)

    assert db_session.query(App).count() >= 1
    assert db_session.query(Build).count() >= 1
```

- [ ] **步骤 2：运行测试并确认失败**

```bash
pytest tests/test_seed.py -v
```

预期：失败，原因是 seed helper 还不存在。

- [ ] **步骤 3：实现演示数据**

演示数据要和当前移动端 UI 接近：

- `Aurora Mobile`
- `Insight Desk`
- `DataFlow`
- 开发环境和线上环境。
- 至少一个 iOS build，包含 manifest URL。
- 至少一个 Android build，包含 APK 下载 URL。

- [ ] **步骤 4：运行测试并确认通过**

```bash
pytest tests/test_seed.py -v
```

预期：通过。

- [ ] **步骤 5：提交**

```bash
git add src/testflying_api/seed.py tests/conftest.py tests/test_seed.py
git commit -m "feat: add demo catalog seed data"
```

## 阶段三：工作台目录接口

### 任务 5：目录仓储和工作台组合服务

**文件：**

- 新建：`src/testflying_api/domain.py`
- 新建：`src/testflying_api/catalog_repository.py`
- 新建：`src/testflying_api/catalog_service.py`
- 测试：`tests/test_catalog_service.py`

- [ ] **步骤 1：先写失败测试**

```python
from sqlalchemy.orm import Session

from testflying_api.catalog_repository import CatalogRepository
from testflying_api.catalog_service import CatalogService
from testflying_api.seed import seed_demo_catalog


def test_workspace_contains_distribution_facts_only(db_session: Session) -> None:
    seed_demo_catalog(db_session)
    workspace = CatalogService(CatalogRepository(db_session)).workspace_for_device(
        device_id="device-001",
        platform="ios",
    )

    assert workspace.apps
    assert workspace.builds
    assert workspace.install_tasks == []
    assert workspace.sort_order.build_ids == []
```

- [ ] **步骤 2：运行测试并确认失败**

```bash
pytest tests/test_catalog_service.py -v
```

预期：失败，原因是仓储和服务还不存在。

- [ ] **步骤 3：实现仓储和服务**

规则：

- 只返回当前设备和平台可见的 build。
- `installTasks` 永远返回空数组。
- `sortOrder.buildIds` 永远返回空数组。
- 通知不包含已读/未读状态。
- 开发者账号只返回和可见应用相关的账号事实。

- [ ] **步骤 4：运行测试并确认通过**

```bash
pytest tests/test_catalog_service.py -v
```

预期：通过。

- [ ] **步骤 5：提交**

```bash
git add src/testflying_api/domain.py src/testflying_api/catalog_repository.py src/testflying_api/catalog_service.py tests/test_catalog_service.py
git commit -m "feat: compose workspace catalog"
```

### 任务 6：工作台路由契约

**文件：**

- 新建：`src/testflying_api/routes/workspace.py`
- 修改：`src/testflying_api/app.py`
- 测试：`tests/test_workspace.py`

- [ ] **步骤 1：改写 workspace 测试**

测试必须断言：

- 响应包含 `apps`、`builds`、`devices`、`developerAccounts`、`notifications`、`installTasks`、`sortOrder`、`profile`。
- `installTasks` 永远是 `[]`。
- `sortOrder.buildIds` 永远是 `[]`。
- 响应不能出现 `isRead`、`readAt`、`installedAt`、`installState`、`progress`。

- [ ] **步骤 2：运行测试并确认失败**

```bash
pytest tests/test_workspace.py -v
```

预期：失败，直到 route 接入 catalog service。

- [ ] **步骤 3：实现路由**

接口：

```http
GET /v1/test-distribution/workspace
```

请求头：

```http
Authorization: Bearer <token>
X-Device-ID: <device-id>
X-Client-Platform: ios
```

- [ ] **步骤 4：运行测试并确认通过**

```bash
pytest tests/test_workspace.py -v
```

预期：通过。

- [ ] **步骤 5：提交**

```bash
git add src/testflying_api/routes/workspace.py src/testflying_api/app.py tests/test_workspace.py
git commit -m "feat: expose workspace catalog route"
```

## 阶段四：上传和制品分发

### 任务 7：S3/MinIO 和本地兼容制品存储

**文件：**

- 新建：`src/testflying_api/storage.py`
- 测试：`tests/test_storage.py`

- [ ] **步骤 1：先写失败测试**

```python
from testflying_api.storage import LocalArtifactStorage


def test_storage_writes_file_and_returns_public_url(tmp_path) -> None:
    storage = LocalArtifactStorage(root=tmp_path, public_base_url="https://dist.example.test")

    saved = storage.save("build-1", "app.ipa", b"ipa-bytes")

    assert saved.storage_path.exists()
    assert saved.download_url == "https://dist.example.test/artifacts/build-1/app.ipa"
```

再补一个 S3/MinIO 客户端构造测试，使用 fake client，不连接真实 MinIO：

```python
from testflying_api.storage import S3ArtifactStorage


def test_s3_storage_uses_configured_bucket_and_public_base_url(fake_s3_client) -> None:
    storage = S3ArtifactStorage(
        client=fake_s3_client,
        bucket="testflying",
        public_base_url="https://objects.example.test/testflying",
    )

    saved = storage.save("build-1", "app.ipa", b"ipa-bytes")

    assert saved.download_url == "https://objects.example.test/testflying/build-1/app.ipa"
    assert fake_s3_client.put_objects[0]["Bucket"] == "testflying"
```

- [ ] **步骤 2：运行测试并确认失败**

```bash
pytest tests/test_storage.py -v
```

预期：失败，原因是 storage 模块还不存在。

- [ ] **步骤 3：实现本地存储和 S3/MinIO 存储**

文件保存到：

```text
data/artifacts/{build_id}/{file_name}
```

默认部署使用 `S3ArtifactStorage`，连接 MinIO/S3。轻量本地测试使用 `LocalArtifactStorage`。

存储必须通过抽象封装，业务层只依赖统一的 artifact storage 接口，不直接知道具体存储后端。

- [ ] **步骤 4：运行测试并确认通过**

```bash
pytest tests/test_storage.py -v
```

预期：通过。

- [ ] **步骤 5：提交**

```bash
git add src/testflying_api/storage.py tests/test_storage.py
git commit -m "feat: add local artifact storage"
```

### 任务 8：IPA 解析和安装清单生成

**文件：**

- 新建：`src/testflying_api/package_parser.py`
- 新建：`src/testflying_api/manifest.py`
- 测试：`tests/test_package_parser.py`
- 测试：`tests/test_manifest.py`

- [ ] **步骤 1：先写失败测试**

IPA 测试：

- 构造一个包含 `Payload/Test.app/Info.plist` 的 zip。
- 断言能解析 bundle id、应用名、版本号、build number。

APK 第一版测试：

- Android 暂时不解析二进制 APK。
- 上传时必须传入 package name、app name、version、build number。
- 缺少必填 metadata 时返回错误。

- [ ] **步骤 2：运行测试并确认失败**

```bash
pytest tests/test_package_parser.py tests/test_manifest.py -v
```

预期：失败，原因是模块还不存在。

- [ ] **步骤 3：实现解析和 manifest 生成**

IPA 解析：

- 使用 `zipfile`。
- 找到 `Payload/*.app/Info.plist`。
- 使用 `plistlib` 解析。

安装清单生成：

- 生成合法 plist。
- 包含 `software-package`。
- 使用制品公开下载 URL。
- 包含 bundle identifier、version、title。

- [ ] **步骤 4：运行测试并确认通过**

```bash
pytest tests/test_package_parser.py tests/test_manifest.py -v
```

预期：通过。

- [ ] **步骤 5：提交**

```bash
git add src/testflying_api/package_parser.py src/testflying_api/manifest.py tests/test_package_parser.py tests/test_manifest.py
git commit -m "feat: parse packages and generate manifests"
```

### 任务 9：上传接口

**文件：**

- 新建：`src/testflying_api/routes/uploads.py`
- 修改：`src/testflying_api/app.py`
- 测试：`tests/test_uploads.py`

- [ ] **步骤 1：先写失败测试**

测试场景：

- 上传 IPA，字段包含 `environment=development` 和更新说明。
- 服务端创建 app 和 build。
- 响应包含 iOS `itms-services://?...manifest.plist`。
- 上传完成后 workspace 能看到新 build。
- 不创建安装任务。
- 不创建任何用户态安装记录。

- [ ] **步骤 2：运行测试并确认失败**

```bash
pytest tests/test_uploads.py -v
```

预期：失败，原因是上传路由还不存在。

- [ ] **步骤 3：实现上传路由**

接口：

```http
POST /v1/test-distribution/uploads
```

表单字段：

- `file`：IPA 或 APK。
- `platform`：`ios` 或 `android`。
- `environment`：`development` 或 `production`。
- `changelog`：可选。
- Android 专用 metadata：`packageName`、`appName`、`version`、`buildNumber`。

规则：

- 根据 bundle id / package name + platform upsert app。
- 创建 build。
- 保存制品。
- iOS 生成 manifest。
- 生成 `build` 类型通知 feed。
- 不创建 install task。
- 不创建设备安装状态。
- 不创建用户排序或通知已读状态。

- [ ] **步骤 4：运行测试并确认通过**

```bash
pytest tests/test_uploads.py -v
```

预期：通过。

- [ ] **步骤 5：提交**

```bash
git add src/testflying_api/routes/uploads.py src/testflying_api/app.py tests/test_uploads.py
git commit -m "feat: add package upload endpoint"
```

## 阶段五：设备可见性和账号事实

### 任务 10：设备目录和可见性

**文件：**

- 新建：`src/testflying_api/routes/devices.py`
- 修改：`src/testflying_api/app.py`
- 测试：`tests/test_devices.py`

- [ ] **步骤 1：先写失败测试**

断言：

- `GET /v1/test-distribution/devices/current` 根据 `X-Device-ID` 返回设备事实。
- 未登记设备返回 `device_not_registered`。
- workspace 排除当前设备不可见的 build。
- 不记录“这个设备是否安装过某个 build”。

- [ ] **步骤 2：运行测试并确认失败**

```bash
pytest tests/test_devices.py -v
```

预期：失败，直到设备路由和可见性逻辑实现。

- [ ] **步骤 3：实现设备路由**

接口：

```http
GET /v1/test-distribution/devices/current
GET /v1/test-distribution/devices
POST /v1/test-distribution/devices/registration-link
```

`registration-link` 只生成登记请求或登记链接，不自动审批设备。

- [ ] **步骤 4：运行测试并确认通过**

```bash
pytest tests/test_devices.py -v
```

预期：通过。

- [ ] **步骤 5：提交**

```bash
git add src/testflying_api/routes/devices.py src/testflying_api/app.py tests/test_devices.py
git commit -m "feat: add device visibility endpoints"
```

### 任务 11：开发者账号事实

**文件：**

- 新建：`src/testflying_api/routes/accounts.py`
- 修改：`src/testflying_api/app.py`
- 测试：`tests/test_accounts.py`

- [ ] **步骤 1：先写失败测试**

断言：

- 账号响应包含 `expiresAt`、`status` 和关联 app id。
- workspace 包含当前可见应用相关的账号续费事实。
- 不保存客户端关闭提醒、已读、已处理状态。

- [ ] **步骤 2：运行测试并确认失败**

```bash
pytest tests/test_accounts.py -v
```

预期：失败，直到账号路由实现。

- [ ] **步骤 3：实现账号路由**

接口：

```http
GET /v1/test-distribution/developer-accounts
GET /v1/test-distribution/developer-accounts/renewals
```

- [ ] **步骤 4：运行测试并确认通过**

```bash
pytest tests/test_accounts.py -v
```

预期：通过。

- [ ] **步骤 5：提交**

```bash
git add src/testflying_api/routes/accounts.py src/testflying_api/app.py tests/test_accounts.py
git commit -m "feat: add developer account renewal facts"
```

## 阶段六：无已读状态的通知流

### 任务 12：通知流

**文件：**

- 新建：`src/testflying_api/routes/notifications.py`
- 修改：`src/testflying_api/app.py`
- 测试：`tests/test_notifications.py`

- [ ] **步骤 1：先写失败测试**

断言：

- `GET /v1/test-distribution/notifications` 返回 build/account/device 类型通知。
- 支持 `type=build|account|device` 筛选。
- 响应不包含 `isRead` 或 `readAt`。
- 不存在 mark-read 接口。

- [ ] **步骤 2：运行测试并确认失败**

```bash
pytest tests/test_notifications.py -v
```

预期：失败，直到通知路由实现。

- [ ] **步骤 3：实现通知路由**

接口：

```http
GET /v1/test-distribution/notifications
```

不要实现任何已读写入接口。

- [ ] **步骤 4：运行测试并确认通过**

```bash
pytest tests/test_notifications.py -v
```

预期：通过。

- [ ] **步骤 5：提交**

```bash
git add src/testflying_api/routes/notifications.py src/testflying_api/app.py tests/test_notifications.py
git commit -m "feat: add notification feed"
```

## 阶段七：文档和客户端契约清理

### 任务 13：更新服务端接口文档

**文件：**

- 修改：`README.md`
- 新建：`docs/api-contract.md`

- [ ] **步骤 1：写清服务端拥有的事实**

文档必须覆盖：

- 工作台响应结构。
- 上传请求。
- 安装清单生成。
- 设备可见性。
- 开发者账号续费事实。
- 通知 feed。

- [ ] **步骤 2：写清客户端拥有的状态**

明确说明服务端不保存：

- 安装状态。
- 下载进度。
- 暂停/继续状态。
- 用户排序。
- 通知已读。

- [ ] **步骤 3：提交**

```bash
git add README.md docs/api-contract.md
git commit -m "docs: document stateless catalog contract"
```

### 任务 14：客户端集成说明

**文件：**

- 新建：`docs/client-integration.md`

- [ ] **步骤 1：写客户端集成说明**

说明远端客户端应该这样组合 UI 数据：

```text
服务端 catalog/workspace
+ 客户端本地安装状态
+ 客户端本地暂停/下载进度
+ 客户端本地排序
+ 客户端本地通知已读
= UI workspace
```

远端客户端不能再调用已删除的 install-task、sort-order、mark-read 接口。

- [ ] **步骤 2：提交**

```bash
git add docs/client-integration.md
git commit -m "docs: add client integration boundary"
```

## 最终验证

运行：

```bash
pytest
ruff check src tests
python3.11 -m compileall -q src tests
docker compose build
```

预期：

- 所有测试通过。
- Ruff 没有问题。
- `compileall` 退出码为 0。
- Docker 镜像构建成功。

推送：

```bash
git push
```

## 推荐执行顺序

1. 阶段零：第一版 Docker 部署入口。
2. 阶段一：项目基础。
3. 阶段二：只保存分发事实的持久化模型。
4. 阶段三：工作台目录接口。
5. 阶段四：上传和制品分发。
6. 阶段五：设备可见性和账号事实。
7. 阶段六：无已读状态的通知 feed。
8. 阶段七：文档和客户端契约清理。

在阶段三通过前，不开始客户端远端集成；在只读目录稳定前，不开始上传能力。
