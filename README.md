# testflying monorepo

testflying 是一套"内部测试包分发 + 类 TestFlight"系统。本仓库由原 `testflying`（Flutter 客户端）和 `testflying-api`（服务端）两个仓库于 2026-08 合并而来。2026-11-05 起以当前代码快照重新开始，旧提交历史已移除。

## 仓库布局

```text
├── client/   # Flutter 客户端（iOS / Android / Web / macOS），详见 client/README.md
├── server/   # FastAPI 中心后台 + Go connector + admin-web 管理后台 + build-runner，详见 server/README.md
├── store-service/  # 独立 Rust 商店 API + 账号 Connector，详见 store-service/README.md
├── docs/     # 客户端与服务端共享的契约文档（单一真相源）
└── .github/workflows/  # client.yml、server.yml、store.yml 按路径独立触发
```

## 共享文档

- [docs/api-contract.md](docs/api-contract.md)：接口契约，客户端与服务端共同遵守，修改需同步评审。
- [docs/client-integration.md](docs/client-integration.md)：客户端集成边界（服务端事实 vs 客户端本地状态的拼装公式）。

服务端专属文档（商店同步、构建交付、Runner 节点等）保留在 `server/docs/`；客户端 UI 基线保留在 `client/docs/`。

独立商店服务的 [商店业务与图片上传接口](store-service/docs/store-api.md)、[含节点管理的完整接口](store-service/docs/api-reference.md)和[部署／验证说明](store-service/README.md)位于 `store-service/`。新 Rust Connector 为 `store-service/crates/store-connector/`，旧 Go Connector 保持在 `server/connector/`；现有后台尚未切换到新服务。

独立商店服务已部署，Connector 通用安装包见 [Store Connector v0.1.0](https://github.com/baoluchuling/testflying/releases/tag/store-connector-v0.1.0)，支持 Linux x64、macOS Apple Silicon 和 Windows x64。接入商店 API 从 [store-service/README.md](store-service/README.md) 开始；新服务使用独立生成的凭据，旧服务 Token 不适用于新服务。通用安装包只含配置模板，节点身份配置通过管理员接口单独获取。

公开文档中的部署地址均为占位示例，真实地址通过私密配置提供。业务接口支持 HTTP 与 HTTPS，HTTP 调用不需要证书参数。旧后台的预编译 Release 已停用，当前安装包使用上方 Rust Connector Release。

## 常用命令

客户端（在 `client/` 内执行，Flutter 3.41.2）：

```bash
flutter pub get
flutter analyze
flutter test
```

服务端（在 `server/` 内执行）：

```bash
python3.11 -m venv .venv && source .venv/bin/activate
pip install -e ".[dev]"
pytest
ruff check src tests alembic
cd connector && go test ./...
```

注意：服务端测试（如 `test_docker_runtime.py`）和 `alembic`、`docker compose` 均以 `server/` 为工作目录，请勿从仓库根执行。

## 本地联调

```bash
# server/ 内启动轻量后端（SQLite + 本地 artifacts）
docker compose -f docker-compose.local.yml up --build

# client/ 内以远端模式启动 Web 客户端
fvm flutter run -d chrome \
  --dart-define=TESTFLYING_SERVICE=remote \
  --dart-define=TESTFLYING_API_ENV=development \
  --dart-define=TESTFLYING_API_BASE_URL=http://127.0.0.1:8000 \
  --dart-define=TESTFLYING_ACCESS_TOKEN=dev-token \
  --dart-define=TESTFLYING_DEVICE_ID=device-001 \
  --dart-define=TESTFLYING_CLIENT_PLATFORM=ios
```

部署、Docker Compose、Runner 安装包等完整说明见 [server/README.md](server/README.md)。

## 提交规范

沿用原服务端仓库的 Conventional Commits：`feat(server): ...`、`fix(client): ...`、`docs: ...` 等，scope 使用 `client` / `server` / `connector` / 根目录改动不带 scope。
