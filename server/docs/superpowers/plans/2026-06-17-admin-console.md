# testflying-api 管理后台实现计划

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 `testflying-api` 内置第一版 `/admin` 管理后台，用于上传 IPA/APK、查看应用/构建/设备/账号/通知和复制安装资源链接。

**Architecture:** 管理后台作为 FastAPI server-side HTML 模块实现，复用现有 SQLAlchemy schema、上传逻辑和存储配置。页面使用 Jinja2 模板、少量原生 JavaScript 和静态 CSS，不引入独立前端工程。

**Tech Stack:** FastAPI, SQLAlchemy, Jinja2, python-multipart, pytest, ruff.

---

## 文件结构

- `src/testflying_api/admin/`: 新增后台模块。
- `src/testflying_api/admin/routes.py`: `/admin` 页面路由、上传表单动作和资源查询。
- `src/testflying_api/admin/security.py`: HTTP Basic 管理入口保护。
- `src/testflying_api/admin/view_models.py`: 后台页面展示数据组装。
- `src/testflying_api/templates/admin/*.html`: 后台模板。
- `src/testflying_api/static/admin/admin.css`: 后台样式。
- `src/testflying_api/app.py`: 注册后台路由和静态资源。
- `pyproject.toml`: 增加 Jinja2 依赖。
- `tests/test_admin.py`: 覆盖后台鉴权、页面、上传和列表展示。

## Task 1: 后台路由骨架和鉴权

**Files:**
- Create: `src/testflying_api/admin/security.py`
- Create: `src/testflying_api/admin/routes.py`
- Create: `src/testflying_api/admin/__init__.py`
- Modify: `src/testflying_api/app.py`
- Test: `tests/test_admin.py`

- [ ] 写失败测试：未认证访问 `/admin` 返回 401，使用 `admin:dev-token` 可访问。
- [ ] 实现 HTTP Basic 鉴权，用户名默认 `TESTFLYING_ADMIN_USERNAME=admin`，密码默认复用 `TESTFLYING_STATIC_TOKEN`。
- [ ] 注册 `/admin` 路由。
- [ ] 运行 `pytest tests/test_admin.py -q`。

## Task 2: 管理后台页面和数据展示

**Files:**
- Create: `src/testflying_api/admin/view_models.py`
- Create: `src/testflying_api/templates/admin/base.html`
- Create: `src/testflying_api/templates/admin/dashboard.html`
- Create: `src/testflying_api/templates/admin/apps.html`
- Create: `src/testflying_api/templates/admin/builds.html`
- Create: `src/testflying_api/templates/admin/upload.html`
- Create: `src/testflying_api/templates/admin/devices.html`
- Create: `src/testflying_api/templates/admin/accounts.html`
- Create: `src/testflying_api/templates/admin/notifications.html`
- Create: `src/testflying_api/static/admin/admin.css`
- Modify: `src/testflying_api/app.py`

- [ ] 写测试：种子数据下 `/admin`、`/admin/apps`、`/admin/builds` 页面返回 200 并包含核心文本。
- [ ] 实现 dashboard 统计卡片、最近构建和最近通知。
- [ ] 实现应用、构建、设备、开发者账号、通知表格页。
- [ ] 注册静态资源目录。
- [ ] 运行 `pytest tests/test_admin.py -q`。

## Task 3: 上传页面

**Files:**
- Modify: `src/testflying_api/admin/routes.py`
- Modify: `src/testflying_api/templates/admin/upload.html`
- Test: `tests/test_admin.py`

- [ ] 写测试：通过 `/admin/uploads` 上传 Android 包 metadata 后跳转到构建列表，并能看到新构建。
- [ ] 后台上传动作复用现有上传服务路径，保持和 API 上传一致的创建应用、构建、制品、通知逻辑。
- [ ] 上传成功页展示 install/download/manifest 链接和复制按钮。
- [ ] 上传失败在页面内显示错误信息。
- [ ] 运行上传相关测试。

## Task 4: 验证和文档

**Files:**
- Modify: `README.md`
- Modify: `docs/api-contract.md`

- [ ] 补充 `/admin` 入口、默认账号和上传说明。
- [ ] 运行 `ruff check src tests alembic`。
- [ ] 运行 `pytest -q`。
- [ ] 运行 `python -m compileall -q src tests`。
