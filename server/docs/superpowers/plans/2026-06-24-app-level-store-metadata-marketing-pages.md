# App-Level Store Metadata And Marketing Pages Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move store metadata editing to an App-level current draft, keep sync history snapshots by version and time, and add Apple marketing pages that can be created independently of App versions.

**Architecture:** Keep existing tables compatible by using a fixed current metadata scope for new drafts, while adding explicit sync snapshot columns and marketing-page tables. The admin UI becomes a three-area workspace: current store content, sync history, and Apple marketing pages. Sync submission must present selectable scopes before connector calls.

**Tech Stack:** FastAPI, SQLAlchemy ORM, Alembic, Jinja templates, vanilla admin JavaScript, pytest, ruff.

## Global Constraints

- Do not delete existing `store_image_suites` data in this change; stop exposing it in the main store metadata UI.
- New store metadata editing must not require selecting or passing an App Store / Google Play version until sync time.
- Sync history must preserve the synced version, time, scope, languages, status, and payload snapshot.
- Marketing pages are App-level, Apple-focused, independent of App versions, and support multiple pages per App.
- Sync confirmation must allow the operator to select which scopes are sent: copy metadata, release notes, store images, or combinations.

---

### Task 1: Data Model And Migration

**Files:**
- Modify: `/Users/admin/ai_project/apps/testflying-server/src/testflying_api/schema.py`
- Create: `/Users/admin/ai_project/apps/testflying-server/alembic/versions/20260624_0008_app_level_store_metadata.py`
- Test: `/Users/admin/ai_project/apps/testflying-server/tests/test_schema.py`

**Interfaces:**
- Produces: `StoreMarketingPage`, `StoreMarketingPageLocale`, `StoreSyncRun.sync_scopes_json`, `StoreSyncRun.payload_snapshot_json`.
- Produces: app-level metadata helpers can keep using `StoreAppMetadataDraft` with fixed version scope.

- [ ] Add ORM models and columns.
- [ ] Add Alembic migration with additive changes only.
- [ ] Add schema test coverage for new tables and columns.

### Task 2: Service And API Scope Changes

**Files:**
- Modify: `/Users/admin/ai_project/apps/testflying-server/src/testflying_api/store_sync.py`
- Modify: `/Users/admin/ai_project/apps/testflying-server/src/testflying_api/routes/store_management.py`
- Test: `/Users/admin/ai_project/apps/testflying-server/tests/test_admin.py`

**Interfaces:**
- Produces: `CURRENT_METADATA_VERSION`, `current_metadata_drafts_for_app`, `save_current_app_metadata_draft`, `sync_current_app_metadata`.
- API saves current store metadata without `version`; sync endpoint accepts `version` and selected `syncScopes`.

- [ ] Add current App-level metadata helper functions.
- [ ] Store sync scope and payload snapshot on every sync run.
- [ ] Update public import endpoint to accept version-less current metadata payloads while retaining backward compatibility.

### Task 3: Admin Store Workspace

**Files:**
- Modify: `/Users/admin/ai_project/apps/testflying-server/src/testflying_api/admin/view_models.py`
- Modify: `/Users/admin/ai_project/apps/testflying-server/src/testflying_api/admin/routes.py`
- Modify: `/Users/admin/ai_project/apps/testflying-server/src/testflying_api/templates/admin/store_metadata.html`
- Modify: `/Users/admin/ai_project/apps/testflying-server/src/testflying_api/templates/admin/base.html`
- Modify: `/Users/admin/ai_project/apps/testflying-server/src/testflying_api/static/admin/admin.css`
- Test: `/Users/admin/ai_project/apps/testflying-server/tests/test_admin.py`

**Interfaces:**
- Page areas: `current`, `history`, `marketing`.
- Sync form fields: `syncVersion`, `syncScopes`.
- Marketing page actions: create page, open page, save localized draft.

- [ ] Remove visible content-set and store-image-suite controls from the main flow.
- [ ] Add a sync confirmation checklist with scope checkboxes and target version.
- [ ] Add grouped sync history by version and time, with snapshot summary.
- [ ] Add Apple-only marketing page list/detail skeleton with create/save support.

### Task 4: Verification And Delivery

**Files:**
- Modify: `/Users/admin/ai_project/apps/testflying-server/docs/store-sync.md`

**Interfaces:**
- Verification commands: `ruff check src tests`, `pytest -q`.

- [ ] Update docs to describe App-level current metadata, sync history, and marketing pages.
- [ ] Run lint and full tests.
- [ ] Commit, push, deploy to `/root/testflight-server`, and verify the deployed page loads.
