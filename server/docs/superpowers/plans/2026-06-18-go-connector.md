# Go Connector Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将 `testflying-connector` 从 Python/FastAPI 替换为轻量 Go 单二进制服务，同时保持中心后台协议兼容。

**Architecture:** `connector/` 成为独立 Go module，使用标准库 `net/http`、`encoding/json`、`crypto` 和 `httptest`。实现层拆成配置、模型、限流、商店凭据、路由处理和测试，避免引入数据库或重型框架。

**Tech Stack:** Go 1.20、Docker multi-stage build、Python/FastAPI 中心后台保持不变。

---

## Chunk 1: Go Connector Core

### Task 1: 建立 Go module 和核心模型

**Files:**
- Create: `connector/go.mod`
- Create: `connector/cmd/testflying-connector/main.go`
- Create: `connector/internal/connector/models.go`
- Create: `connector/internal/connector/config.go`
- Delete: `connector/pyproject.toml`
- Delete: `connector/src/testflying_connector/*`

- [ ] **Step 1: 写 Go 配置和请求/响应模型**

实现环境变量读取、平台模式、Apple / Google 凭据路径和现有 JSON 字段映射。

- [ ] **Step 2: 运行 Go 测试确认 module 可编译**

Run: `cd connector && go test ./...`

### Task 2: 实现 HTTP 路由和鉴权

**Files:**
- Create: `connector/internal/connector/server.go`
- Create: `connector/internal/connector/server_test.go`

- [ ] **Step 1: 写鉴权和账号校验测试**

覆盖缺 token、错误账号、健康检查。

- [ ] **Step 2: 实现 `/health`、`/v1/preflight`、`/v1/apps/{app_id}/supported-locales`、`/v1/sync-runs`**

保持当前中心后台字段兼容。

- [ ] **Step 3: 运行 Go 测试**

Run: `cd connector && go test ./...`

### Task 3: 实现限流和凭据校验

**Files:**
- Create: `connector/internal/connector/ratelimit.go`
- Create: `connector/internal/connector/credentials.go`
- Modify: `connector/internal/connector/server.go`
- Modify: `connector/internal/connector/server_test.go`

- [ ] **Step 1: 写 Google 200 次/分钟和 Apple 安全比例测试**
- [ ] **Step 2: 实现滑动窗口限流**
- [ ] **Step 3: 实现 Apple `.p8` 解析、JWT 生成和 Google service account JSON 解析**
- [ ] **Step 4: 运行 Go 测试**

Run: `cd connector && go test ./...`

## Chunk 2: Packaging And Docs

### Task 4: 更新 Docker、Compose 和 CI

**Files:**
- Modify: `connector/Dockerfile`
- Modify: `connector/.dockerignore`
- Modify: `docker-compose.yml`
- Modify: `.github/workflows/ci.yml`

- [ ] **Step 1: Dockerfile 改为 Go multi-stage build**
- [ ] **Step 2: Compose 增加 connector store mode 和凭据挂载示例变量**
- [ ] **Step 3: CI 增加 Go 测试，移除 connector Python wheel**
- [ ] **Step 4: 验证 Docker Compose 配置**

Run: `docker compose -f docker-compose.yml config`

### Task 5: 更新中文文档

**Files:**
- Modify: `connector/README.md`
- Modify: `docs/store-sync.md`
- Modify: `docs/api-contract.md`

- [ ] **Step 1: 写 Go connector 本地启动命令**
- [ ] **Step 2: 写 Apple 和 Google 凭据生成与挂载说明**
- [ ] **Step 3: 写 mock/live 模式区别和生产部署命令**

## Chunk 3: Verification And Delivery

### Task 6: 全量验证、提交、推送、远程更新

**Files:**
- All changed files

- [ ] **Step 1: 运行 Go connector 测试**

Run: `cd connector && go test ./...`

- [ ] **Step 2: 运行 Python 后端测试和 lint**

Run: `.venv/bin/python -m pytest`

Run: `.venv/bin/python -m ruff check src tests alembic`

- [ ] **Step 3: 运行 Docker 构建检查**

Run: `docker build -t testflying-connector:local connector`

- [ ] **Step 4: 提交并推送 main**

Run: `git add ... && git commit -m "feat(connector): rewrite connector in go" && git push origin main`

- [ ] **Step 5: 更新远程 API 服务**

只重建 `api`，不在远程服务器重新创建 connector。
