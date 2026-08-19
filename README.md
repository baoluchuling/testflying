# testflying monorepo

testflying 是一套"内部测试包分发 + 类 TestFlight"系统。本仓库由原 `testflying`（Flutter 客户端）和 `testflying-api`（服务端）两个仓库于 2026-08 合并而来，双方完整提交历史均保留在本仓库中。

## 仓库布局

```text
├── client/   # Flutter 客户端（iOS / Android / Web / macOS），详见 client/README.md
├── server/   # FastAPI 中心后台 + Go connector + admin-web 管理后台 + build-runner，详见 server/README.md
├── docs/     # 客户端与服务端共享的契约文档（单一真相源）
└── .github/workflows/  # client.yml 与 server.yml 按路径独立触发
```

## 共享文档

- [docs/api-contract.md](docs/api-contract.md)：接口契约，客户端与服务端共同遵守，修改需同步评审。
- [docs/client-integration.md](docs/client-integration.md)：客户端集成边界（服务端事实 vs 客户端本地状态的拼装公式）。

服务端专属文档（商店同步、构建交付、Runner 节点等）保留在 `server/docs/`；客户端 UI 基线保留在 `client/docs/`。

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
