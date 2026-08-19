# Active Connector Windows Package Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 Windows Connector 通过主动轮询中心后台执行商店同步任务，并由后台生成包含账号凭据的一次性 Windows 安装包。

**Architecture:** 中心后台保留现有 `StoreConnectorClient` 调用边界，新增 `active://<account_id>` 连接模式，把请求投递到内存任务队列；Connector 新增 active agent，轮询中心、复用本地 HTTP handler 执行任务并回传结果。后台生成 zip 包时只把 Apple/Google 凭据写进安装包，不长期保存凭据。

**Tech Stack:** FastAPI, SQLAlchemy, Jinja2, Go net/http, PowerShell scheduled task.

---

### Task 1: 中心后台主动 Connector 通道

**Files:**
- Create: `src/testflying_api/active_connector.py`
- Create: `src/testflying_api/routes/connector_agent.py`
- Modify: `src/testflying_api/app.py`
- Modify: `src/testflying_api/store_sync.py`
- Test: `tests/test_active_connector.py`

- [ ] 新增内存任务队列，支持 dispatch、poll、complete、last_seen。
- [ ] 新增 `/connector-agent/v1/poll` 和 `/connector-agent/v1/results`，使用账号 connector token 鉴权。
- [ ] `StoreConnectorClient` 识别 `active://`，把 health、preflight、sync、supported-locales 转为 active dispatch。
- [ ] 覆盖 dispatch/poll/result 和 active health 调用测试。

### Task 2: Go Connector active agent

**Files:**
- Modify: `connector/internal/connector/config.go`
- Create: `connector/internal/connector/agent.go`
- Modify: `connector/cmd/testflying-connector/main.go`
- Test: `connector/internal/connector/config_test.go`

- [ ] 支持 `TESTFLYING_CONNECTOR_CONFIG_PATH` JSON 配置。
- [ ] 新增 `TESTFLYING_CONNECTOR_CENTER_URL`，存在时进入 active agent 模式。
- [ ] active agent 轮询中心任务，使用 `httptest` 调用现有 server handler，回传状态码和响应体。
- [ ] 覆盖 JSON 配置加载测试。

### Task 3: Windows 一次性安装包

**Files:**
- Modify: `src/testflying_api/admin/routes.py`
- Modify: `src/testflying_api/templates/admin/account_detail.html`
- Test: `tests/test_admin.py`

- [ ] 新增后台表单，按账号平台上传 Apple `.p8` 和/或 Google service-account JSON。
- [ ] 生成 zip：`install.ps1`、`config.json`、`README.txt`、`secrets/...`。
- [ ] 生成包时自动创建/更新 `active://<account_id>` connector 和随机 token。
- [ ] 安装脚本把文件复制到 `C:\ProgramData\TestFlying\connectors\<account_id>`，注册计划任务，并自动从 GitHub Release 下载 Windows connector exe。
- [ ] 覆盖 zip 内容和 connector 配置测试。

### Task 4: 文档与验证

**Files:**
- Modify: `connector/README.md`
- Modify: `docs/store-sync.md`

- [ ] 文档说明 active 模式、Windows 安装包、凭据放置和重启方式。
- [ ] 运行 Python 相关测试。
- [ ] 运行 Go connector 测试。
- [ ] 提交、推送，并按项目约定更新远程部署。
