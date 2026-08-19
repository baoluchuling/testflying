# Android APK Metadata And Upload Progress Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让后台和上传 API 能从 APK 自动解析包名、应用名称、版本号和构建号，并在后台上传大包时显示进度。

**Architecture:** iOS 继续使用 `Info.plist` 解析；Android 使用 `pyaxmlparser` 解析 APK 中的 binary manifest 和资源表。后台上传页使用 XHR 接管表单提交以显示上传进度，服务端仍返回原 HTML 成功/失败页面。

**Tech Stack:** FastAPI、Jinja2、pyaxmlparser、pytest、原生 XMLHttpRequest。

---

## Chunk 1: Android APK Metadata

**Files:**
- Modify: `src/testflying_api/package_parser.py`
- Modify: `src/testflying_api/upload_service.py`
- Modify: `src/testflying_api/routes/uploads.py`
- Modify: `src/testflying_api/admin/routes.py`
- Modify: `pyproject.toml`
- Test: `tests/test_package_parser.py`
- Test: `tests/test_uploads.py`
- Test: `tests/test_admin.py`
- Create: `tests/fixtures.py`

- [x] **Step 1: Add failing parser tests**
  - 用真实最小 APK fixture 覆盖 `package`、`versionName`、`versionCode` 和 `application label`。

- [x] **Step 2: Add pyaxmlparser dependency**
  - 将 `pyaxmlparser>=0.3.31,<0.4.0` 加入项目依赖。

- [x] **Step 3: Implement APK parsing**
  - 新增 `parse_apk_metadata()`，将上传 bytes 写入临时 `.apk` 文件后交给 `pyaxmlparser.APK`。
  - 解析失败统一转成 `PackageParseError("APK 包结构不正确")`。

- [x] **Step 4: Wire upload service**
  - Android 上传改为自动解析 APK。
  - 只保留 `appName` 作为可选名称覆盖。

## Chunk 2: Admin Upload Progress

**Files:**
- Modify: `src/testflying_api/templates/admin/upload.html`
- Modify: `src/testflying_api/static/admin/admin.css`
- Test: `tests/test_admin.py`

- [x] **Step 1: Simplify upload form**
  - 移除 `packageName`、`version`、`buildNumber` 输入。
  - 保留可选“应用名称覆盖”。

- [x] **Step 2: Add progress UI**
  - 添加上传进度状态、百分比和进度条。

- [x] **Step 3: Add XHR submit**
  - 使用 `XMLHttpRequest.upload.onprogress` 展示真实上传百分比。
  - 上传完成后用服务端返回 HTML 替换当前页面。
  - JS 不可用时保留原生表单提交。

## Chunk 3: Verification

- [x] **Step 1: Run focused tests**
  - `PYTHONPATH=src .venv/bin/pytest tests/test_package_parser.py tests/test_uploads.py tests/test_admin.py -q`

- [x] **Step 2: Run full verification**
  - `PYTHONPATH=src .venv/bin/ruff check src tests alembic`
  - `PYTHONPATH=src .venv/bin/pytest -q`
  - `PYTHONPATH=src .venv/bin/python -m compileall -q src tests`
