# testflying-server

`testflying-server` 是 `testflying` 内部测试包管理分发和商店同步的服务端项目。测试包管理分发仍然是主能力；商店同步是新增的账号级扩展能力。

当前仓库下分两个项目：

- 根目录：中心化后台 `testflying-server`，负责测试包上传、分发目录、设备、开发者账号、商店同步草稿、构建任务、Runner 节点、预检查缓存和同步记录。
- `connector/`：账号级子项目 `testflying-connector`，每个开发者账号单独部署一份，负责持有该账号商店凭证并执行商店 API 调用。

中心后台只保存 connector 地址和调用 token，不保存 Apple `.p8` 或 Google service account 凭据。服务端只维护分发事实，不保存安装状态、下载进度、用户排序、通知已读等客户端状态。

## 相关文档

- [客户端集成边界](docs/client-integration.md)
- [接口契约](docs/api-contract.md)
- [商店同步设计](docs/store-sync.md)
- [对外商店管理接口](docs/store-management-api.md)
- [构建交付与 Runner 节点](docs/build-delivery.md)

## 当前能力

- `GET /health`：服务健康检查。
- `GET /v1/test-distribution/workspace`：返回客户端首屏需要的 workspace 快照结构。
- `POST /v1/test-distribution/uploads`：上传 IPA/APK，自动解析包信息，创建应用、构建、制品和构建通知；可选绑定开发者账号并写入商店标识。
- `GET /v1/test-distribution/devices/current`：读取当前设备登记事实。
- `GET /v1/test-distribution/devices`：读取设备列表。
- `POST /v1/test-distribution/devices/registration-link`：生成设备登记请求链接，不自动审批设备。
- `GET /v1/test-distribution/developer-accounts`：读取开发者账号续费事实。
- `GET /v1/test-distribution/developer-accounts/renewals`：读取需要续费提醒的账号。
- `GET /v1/test-distribution/notifications`：读取服务端通知 feed，支持 `type=build|account|device`。
- `POST /v1/store-management/developer-accounts/{accountId}/apps/{appId}/metadata-content-sets`：通过接口导入默认商店页文案和截图草稿，只保存到 testflying，不同步到真实商店后台。
- `POST /v1/store-management/developer-accounts/{accountId}/apps/{appId}/store-versions/{version}/draft`：通过接口导入某个商店版本的文案元数据和版本说明，只保存到 testflying 草稿，不同步到真实商店后台。
- `POST /v1/store-management/developer-accounts/{accountId}/apps/{appId}/marketing-pages`：通过接口创建自定义产品页面并导入文案和截图草稿，只保存到 testflying，不同步到真实商店后台。
- `GET /admin`：内置单页管理后台，用于上传包、商店管理、构建、设备、App 日志、通知、系统设置和接口文档。
- 管理后台支持新增/编辑开发者账号、上传时绑定账号、绑定/解绑账号下 App、维护 App 商店标识、配置账号 connector，以及同步版本说明和商店元数据。商店内容按 App 维护当前草稿，版本说明按版本维护；商店图支持图片上传和预览。
- 应用可以为开发环境和线上环境分别保存 Git 构建配置，并创建 Agent 构建任务。macOS Runner 按标签和平台领取任务，回传构建事件、包、符号文件、日志和报告。
- 构建工作区统一使用 `/admin/builds/apps`、`/admin/builds/history` 和 `/admin/builds/runners`，分别负责已接入应用和构建触发、构建记录与制品、Runner 预配和状态。
- 设置工作区统一使用 `/admin/settings/general`、`/admin/settings/notifications`、`/admin/settings/llm` 和 `/admin/settings/runtime`，分别负责 Connector 默认模板、钉钉通知、LLM 模型与功能绑定、只读运行环境。
- 后台可写业务设置采用“数据库优先、环境变量兜底”；管理后台不会改写 `.env`。运行环境页只展示脱敏后的配置状态，不返回密钥原文。
- `GET /admin/api/build-runners`：读取 Runner 节点状态、能力、版本、心跳与当前构建；节点安装、自动更新和运行边界见 `docs/build-delivery.md`。
- Agent 构建进入 `failed` 或 `needs_human` 时会生成站内通知；配置钉钉 Webhook 与加签密钥后，会通过可重试的投递队列发送群通知。
- `GET /admin/app-logs`：电脑端 App 日志查看页，展示连接二维码、在线设备、客户端异常和实时日志流。
- `WS /push`：手机端主动连接的日志 WebSocket 入口，query 中的 `token` 会作为设备唯一标识；缺失时允许连接并标记为未知设备。
- 商店同步页进入时会自动预检查，5 分钟内相同账号、App、平台、版本、语言和操作返回同一个预检查状态。
- 请求上下文预留 `Authorization`、`X-Device-ID`、`X-Client-Platform`。
- Docker Compose 默认启动中心后台、connector、PostgreSQL、MinIO。

## Docker 部署

第一版部署默认使用 Docker Compose，并直接包含：

- `api`：中心化后台 FastAPI 服务。
- `connector`：账号级商店同步 connector 示例服务。
- `postgres`：PostgreSQL，保存应用、构建、设备、账号、通知等分发事实。
- `minio`：S3 兼容对象存储，保存 IPA/APK 和 iOS manifest。
- `minio-init`：启动时自动创建 `testflying` bucket。

运行镜像使用多阶段 Docker 构建：builder 阶段先生成 wheel 产物，runtime 阶段只安装 wheel，不以源码目录方式运行服务。中心后台镜像同时包含 `alembic/` 和 `alembic.ini`。API 容器默认启动时会先执行 `alembic upgrade head`，迁移成功后再启动 FastAPI 服务。

如果在服务器本机直接构建镜像：

```bash
git checkout main
git pull --ff-only origin main
docker compose build api connector
docker compose up -d postgres minio minio-init
docker compose up -d api connector
```

后续只更新镜像时直接重启 `api` 即可，容器会自动检查并执行未应用的 Alembic 迁移。单机 Docker Compose 默认开启自动迁移；如果以后改成多副本部署，可以设置 `TESTFLYING_AUTO_MIGRATE=0` 关闭容器启动迁移，改由单独迁移任务执行。

## GitHub Actions 产物部署

`.github/workflows/ci.yml` 会在 CI 里生成这些产物：

- `testflying_server-*.whl`
- `testflying-server-<commit>.tar.gz`
- `testflying-connector-<commit>.tar.gz`：Connector Docker 镜像包。
- `testflying-connector-windows-amd64-<commit>.zip`：Windows Connector 单二进制。
- `testflying-connector-linux-amd64-<commit>.tar.gz`：Linux Connector 单二进制。
- `testflying-connector-darwin-arm64-<commit>.tar.gz` / `testflying-connector-darwin-amd64-<commit>.tar.gz`：macOS Connector 单二进制。

如果不想在服务器上从源码构建，部署时可以下载 GitHub Actions 的 artifact，然后在服务器加载镜像：

```bash
gunzip -c testflying-server-<commit>.tar.gz | docker load
gunzip -c testflying-connector-<commit>.tar.gz | docker load
```

镜像加载后，用加载出来的 tag 更新 Compose，或者重新打成本地固定 tag：

```bash
docker tag testflying-server:<commit> testflying-server:latest
docker tag testflying-connector:<commit> testflying-connector:latest
```

随后启动服务，API 容器会自动执行迁移：

```bash
docker compose up -d api connector
```

验证：

```bash
curl http://localhost:8000/health
curl http://localhost:8100/health
```

预期响应：

```json
{"status":"ok"}
```

如果 connector 部署在 Windows 机器上，不需要 Docker。下载 Release 里的 `testflying-connector-windows-amd64-<commit>.zip`，解压后用 PowerShell 设置 `TESTFLYING_CONNECTOR_*` 环境变量并运行 exe；需要长期运行时用任务计划程序创建开机启动任务。完整命令在开发者账号详情页的 `Connector 部署说明` 中可以直接复制。

MinIO 控制台：

```text
http://localhost:9001
```

如果服务器上 `9001` 已被占用，可以通过 `.env` 覆盖控制台端口：

```bash
MINIO_CONSOLE_PORT=8082
```

本地默认账号仅用于开发和内网试部署：

```text
username: testflying
password: testflying-secret
```

正式部署前必须修改 `docker-compose.yml` 里的数据库密码、MinIO 密码、`TESTFLYING_STATIC_TOKEN`、`TESTFLYING_CONNECTOR_TOKEN` 和公开访问域名。当前全栈 Compose 的公开访问域名默认指向测试服务器 `47.90.163.122`，也可以通过同名环境变量或 `.env` 覆盖。iOS OTA 安装真实使用时，`TESTFLYING_PUBLIC_BASE_URL` 和对象存储下载地址需要是设备可访问的 HTTPS 地址。

管理后台：

```text
http://localhost:8000/admin
```

后台使用 HTTP Basic 认证：

```text
username: admin
password: dev-token
```

`username` 来自 `TESTFLYING_ADMIN_USERNAME`，默认是 `admin`；`password` 复用 `TESTFLYING_STATIC_TOKEN`。测试环境可以沿用默认值，公网或正式环境必须替换 `TESTFLYING_STATIC_TOKEN`。

默认环境变量在 `docker-compose.yml` 中配置：

- `TESTFLYING_DATABASE_URL`：默认 `postgresql+psycopg://testflying:testflying@postgres:5432/testflying`
- `TESTFLYING_PUBLIC_BASE_URL`：默认 `http://47.90.163.122:8000`
- `TESTFLYING_STORAGE_BACKEND`：默认 `s3`
- `TESTFLYING_S3_ENDPOINT_URL`：默认 `http://minio:9000`
- `TESTFLYING_S3_PUBLIC_BASE_URL`：默认 `http://47.90.163.122:9000/testflying`
- `TESTFLYING_S3_BUCKET`：默认 `testflying`
- `TESTFLYING_STATIC_TOKEN`：默认 `dev-token`
- `TESTFLYING_ADMIN_USERNAME`：默认 `admin`
- `TESTFLYING_CORS_ALLOWED_ORIGINS`：默认允许 `http://localhost:8080,http://127.0.0.1:8080`，用于 Flutter Web 本地联调。
- `TESTFLYING_CONNECTOR_BASE_URL_TEMPLATE`：按开发者账号 ID 自动生成 connector 地址的模板。支持 `{account_id}` 占位符，例如 `http://connector-{account_id}:8100`。账号详情页手填地址时优先使用手填值；留空时使用该模板。
- `TESTFLYING_TRANSLATION_PROVIDER`：商店元数据多语言翻译服务，默认 `disabled`。设为 `openai` 后，后台“生成该项多语言”会调用 OpenAI-compatible Chat Completions 接口翻译源文案。
- `TESTFLYING_TRANSLATION_OPENAI_API_KEY`：翻译服务 API Key。未配置时不会复制源文案冒充翻译，会在页面提示翻译服务未配置。
- `TESTFLYING_TRANSLATION_OPENAI_BASE_URL`：默认 `https://api.openai.com/v1`，可替换成兼容 OpenAI Chat Completions 的内部网关地址。
- `TESTFLYING_TRANSLATION_OPENAI_MODEL`：默认 `gpt-4o-mini`，可按实际账号可用模型调整。
- `TESTFLYING_DINGTALK_WEBHOOK_URL`：钉钉自定义机器人的 Webhook URL。必须和加签密钥同时配置；服务端不会通过管理 API 返回该值。
- `TESTFLYING_DINGTALK_SECRET`：钉钉机器人安全设置中的加签密钥。只从服务端环境读取，不写入数据库或日志。
- `TESTFLYING_DINGTALK_TIMEOUT_SECONDS`：单次钉钉请求超时，默认 `5` 秒。
- `TESTFLYING_DINGTALK_DISPATCH_INTERVAL_SECONDS`：待发送通知扫描间隔，默认 `10` 秒。
- `TESTFLYING_RUNNER_RELEASE_ROOT`：Runner 自动更新清单和 bundle 目录，Compose 默认为 `/app/data/runner-releases`。
- `TESTFLYING_CONNECTOR_DEVELOPER_ACCOUNT_ID`：connector 绑定的开发者账号 ID。Docker Compose 会给 connector 增加 `connector-<账号 ID>` 网络别名。
- `TESTFLYING_CONNECTOR_TOKEN`：中心后台调用 connector 的 Bearer token。
- `TESTFLYING_CONNECTOR_GOOGLE_RATE_LIMIT_MAX_REQUESTS`：connector Google / Android 接口默认 `200` 次。
- `TESTFLYING_CONNECTOR_GOOGLE_RATE_LIMIT_WINDOW_SECONDS`：connector Google / Android 限流窗口默认 `60` 秒。
- `TESTFLYING_CONNECTOR_APPLE_RATE_LIMIT_FALLBACK_MAX_REQUESTS`：connector 未拿到 Apple `X-Rate-Limit` 前的 fallback，默认 `2880` 次。
- `TESTFLYING_CONNECTOR_APPLE_RATE_LIMIT_WINDOW_SECONDS`：connector Apple fallback 窗口默认 `3600` 秒。
- `TESTFLYING_CONNECTOR_APPLE_RATE_LIMIT_SAFETY_RATIO`：connector 读取 Apple `user-hour-lim` 后使用的安全比例，默认 `0.8`。

这些公开 URL 会在上传时写入构建制品记录。修改环境变量后，已经上传过的构建不会自动改 URL；需要重新上传包，或者用 SQL 替换 `artifacts.download_url`、`artifacts.manifest_url` 和 `artifacts.install_url` 里的旧域名。

## macOS 构建节点安装包

每个节点使用后台签发的独立 Runner 配置生成安装包。所有 macOS 构建节点固定同时支持 iOS 和 Android，不需要配置构建平台。配置至少需要非空的 `runnerId`、`name`、`token`、`serverUrl`、绝对路径 `rootDir`、`labels`，且 `capacity` 必须为 `1`。`runnerId` 必须以字母或数字开头，只能包含字母、数字、点、下划线和短横线，最长 64 个字符：

```json
{
  "runnerId": "runner-mac-1",
  "name": "Mac Runner 1",
  "token": "replace-with-runner-token",
  "serverUrl": "https://testflying.example.com",
  "rootDir": "/Users/Shared/TestFlyingRunner/runner-mac-1",
  "labels": ["ios-release", "flutter"],
  "llmAdapters": ["codex", "claude"],
  "capacity": 1
}
```

在发布机生成节点安装包和自动更新 bundle：

```bash
scripts/build_runner_installer.sh \
  /absolute/path/to/runner-config.json \
  outputs/build-runner-installer \
  0.2.0
```

输出包括：

- `TestFlyingBuildRunner-0.2.0-darwin-<arch>.pkg`：节点用户双击安装。安装脚本会识别当前登录用户、写入用户 LaunchAgent，并立即启动 Runner。
- `darwin/<arch>/release.json`、ZIP 和 `.sha256`：复制到 `TESTFLYING_RUNNER_RELEASE_ROOT` 后供 Runner 自动更新。
- `build-runner-macos/`：无法使用 `.pkg` 时的 fallback，双击其中的 `install.command` 安装。

安装包内已包含 Go Runner 和冻结后的 `package-agent`，节点不需要另装 Python 包。Runner 启动后只自动发现本机可用的 Codex CLI、Claude CLI 或 llm-runtime。自动更新同时替换两个二进制，LaunchAgent 会在更新进程正常退出后拉起新版本。

节点安装包包含该节点的 Runner token，应按密钥产物限制访问，不要提交到 Git 或上传到公开制品库。签名证书、Provisioning Profile 或必须修改项目源码/构建脚本的错误不会由 agent 绕过；构建会进入 `needs_human` 并触发已配置的站内和钉钉通知。

## App 日志调试

电脑端打开管理后台 `App 日志` 页面后，会显示二维码和连接参数。二维码内容格式为：

```text
http://<电脑IP>:<端口>/app-logs/connect?host=<电脑IP>&port=<端口>&name=Mac
```

手机扫码后会先打开一个 H5 连接页，页面按钮再用应用 scheme 打开 App。当前第一版先写死
AnyStories：

```text
anystories:///connect?host=<电脑IP>&port=<端口>&name=Mac
```

手机端扫码后主动连接：

```text
ws://<电脑IP>:<端口>/push?token=<设备ID>
```

电脑端不会访问手机。`token` 会作为设备唯一标识；同一设备重复连接时，会合并到同一个设备视图。日志保存在内存环形缓冲里，服务重启后清空。

如果希望按示例使用 `18080` 端口，本地可以这样启动：

```bash
docker run -d \
  --name testflying-server \
  -p 18080:8000 \
  -e TESTFLYING_DATABASE_URL=sqlite:////app/data/testflying.db \
  -e TESTFLYING_PUBLIC_BASE_URL=http://localhost:18080 \
  -v "$(pwd)/data:/app/data" \
  testflying-server:latest
```

## 轻量本地测试

如果本地测试环境没有 PostgreSQL 或 MinIO，可以使用 SQLite 和本地 `./data` 目录：

```bash
docker compose -f docker-compose.local.yml up --build
```

如果部署环境没有 Compose 插件，也可以直接使用 Docker 跑轻量模式：

```bash
docker build -t testflying-server:latest .
docker run -d \
  --name testflying-server \
  -p 8000:8000 \
  -e TESTFLYING_DATABASE_URL=sqlite:////app/data/testflying.db \
  -e TESTFLYING_PUBLIC_BASE_URL=http://localhost:8000 \
  -e TESTFLYING_STORAGE_ROOT=/app/data/artifacts \
  -e TESTFLYING_STATIC_TOKEN=dev-token \
  -e TESTFLYING_CORS_ALLOWED_ORIGINS=http://localhost:8080,http://127.0.0.1:8080 \
  -v "$(pwd)/data:/app/data" \
  testflying-server:latest
```

## 本地开发

```bash
python3.11 -m venv .venv
source .venv/bin/activate
pip install -e ".[dev]"
uvicorn testflying_api.main:app --reload
```

本地启动默认使用 SQLite 和 `./data/artifacts`。应用启动时会根据 SQLAlchemy schema 自动建表，后续正式迁移路径保留在 `alembic/`。

访问管理后台：

```bash
open http://localhost:8000/admin
```

第一版后台支持上传 IPA/APK、查看应用/构建/设备/开发者账号/通知，以及复制 `installUrl`、`manifestUrl` 和 `downloadUrl`。上传页会显示上传进度；IPA/APK 的包名、应用名、版本号和构建号由服务端自动解析，必要时可以在后台覆盖应用名称。上传时可以选择开发者账号，并按平台填写 App Store Connect App ID 或 Google Play package name 等商店标识。设备审批和构建删除会放到后续管理能力里。

商店同步当前流程：

1. 从顶部“商店管理”直接选择应用，进入默认商店页、营销页面或商店连接。
2. 开发者账号页只维护账号、App 绑定和 connector 配置；connector 地址可以手填，也可以由 `TESTFLYING_CONNECTOR_BASE_URL_TEMPLATE` 按账号 ID 自动生成。
3. iOS App 维护 App Store Connect App ID，Android App 维护 Google Play package name。
4. 页面通过 connector 获取商店实际支持的语言；当前 App 商店草稿维护宣传文本、描述和商店图，版本说明按商店版本维护。
5. 商店图支持拖拽图片、选择图片或拖入语言文件夹上传。非源语言可预览源语言回退图片，但只能删除自己上传的图片。
6. 同步前选择本次同步范围；中心后台复用预检查缓存，并在提交前再次校验。

详细的账号隔离、多语言、图片继承和同步规则见 `docs/store-sync.md`。

运行测试：

```bash
pytest
cd connector && go test ./...
ruff check src tests
```

## 接口边界

服务端拥有这些事实：

- 应用、构建、制品和 iOS `manifest.plist` 地址。
- IPA/APK 自动解析出的包名、应用名、版本号和构建号。
- 构建环境分类：`development` 或 `production`。
- 设备登记事实和设备对构建的可见性。
- 开发者账号续费事实。
- 开发者账号与 App 的绑定关系、App 商店标识。
- 商店版本说明草稿、文字类商店元数据草稿、预检查缓存、同步记录和审计日志。
- 服务端产生的通知 feed。

服务端明确不做：

- 安装状态。
- 下载进度。
- 暂停/继续状态。
- 用户排序。
- 通知已读。
- Apple `.p8`、Google service account 凭据等商店私钥。
- 跨开发者账号批量商店同步。

这些客户端状态不会落库，也没有对应写接口。详细契约见 `docs/api-contract.md` 和 `docs/client-integration.md`。
