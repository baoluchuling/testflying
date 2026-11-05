# Store Management IA B Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将默认商店页和营销页面拆成同级的商店管理入口，避免营销页面继续隐藏在商店元数据页面里。

**Architecture:** 保留现有商店元数据、营销页面保存和同步能力，只调整 Admin 页面信息架构。新增营销页面列表页作为独立工作区，营销详情页继续复用已有保存、上传、预检、同步逻辑。

**Tech Stack:** FastAPI Admin routes, Jinja2 templates, SQLAlchemy view models, existing admin CSS, pytest.

## Global Constraints

- 不改数据模型。
- 创建营销页面不自动同步到 App Store Connect。
- 营销页面和默认商店页是同级模块，页面上不再从商店元数据 tab 进入营销页面。
- 保留旧 `/store-metadata/marketing-pages` URL 的兼容能力，主入口使用 `/store/marketing`。

---

### Task 1: 独立营销页面列表

**Files:**
- Modify: `src/testflying_api/admin/view_models.py`
- Modify: `src/testflying_api/admin/routes.py`
- Create: `src/testflying_api/templates/admin/store_marketing.html`
- Test: `tests/test_admin.py`

**Interfaces:**
- Consumes: `_marketing_pages_for_app`, `account_connector`, `recent_sync_runs`.
- Produces: `store_marketing_context(session, account_id, app_id, locale)` and `GET /admin/developer-accounts/{account_id}/apps/{app_id}/store/marketing`.

- [ ] Add a context function that returns account, app, connector, supported locales, marketing pages, and recent marketing sync runs.
- [ ] Add a polished list page with create actions, page status, Apple page id display, language/image counts, and open links.
- [ ] Add tests proving the metadata page no longer contains the marketing console and the new marketing page lists created pages.

### Task 2: Route Migration And Compatibility

**Files:**
- Modify: `src/testflying_api/admin/routes.py`
- Modify: `src/testflying_api/templates/admin/marketing_page.html`
- Modify: `src/testflying_api/templates/admin/account_detail.html`
- Test: `tests/test_admin.py`

**Interfaces:**
- Primary routes use `/store/marketing-pages`.
- Old routes remain callable by reusing the same handler logic.

- [ ] Point account detail and marketing list links to the new store management URLs.
- [ ] Make create/delete/copy return the new marketing area.
- [ ] Keep old nested URLs working for existing bookmarks.

### Task 3: Detail Page Cleanup

**Files:**
- Modify: `src/testflying_api/templates/admin/marketing_page.html`
- Modify: `src/testflying_api/static/admin/admin.css`
- Test: `tests/test_admin.py`

**Interfaces:**
- Apple 页面 ID is read-only display only.
- Status copy uses `未同步` when the page has no Apple page id.

- [ ] Change back link to the marketing list page.
- [ ] Replace the large Apple 页面 ID block with compact metadata.
- [ ] Remove visible top save/sync controls from the marketing detail page.
- [ ] Keep bottom actions as the only save/sync/delete/copy action row.

### Task 4: Verification

**Files:**
- Test: `tests/test_admin.py`

- [ ] Run `python -m pytest tests/test_admin.py -q`.
- [ ] Run `python -m pytest -q`.
- [ ] Run `python -m ruff check src tests`.
- [ ] Run `git diff --check`.
- [ ] Capture or inspect rendered HTML enough to verify the new route is visible.
