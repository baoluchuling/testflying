# Store Management Direct Sync API Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add two machine-facing APIs that let third-party computers trigger store sync through the center backend.

**Architecture:** Keep center backend as the only public API surface. The new endpoints read existing drafts, run existing preflight and validation, create `store_sync_runs`, and dispatch through the configured Connector. Add in-process rate limiting, idempotency, and per-account serialization for the current single-instance deployment.

**Tech Stack:** FastAPI, Pydantic, SQLAlchemy, existing `store_sync.py` functions, pytest.

## Global Constraints

- Do not let third-party callers hit Connector directly.
- Do not save or mutate draft content during direct sync; callers must import drafts first.
- Default store page endpoint supports `metadata`, `release_notes`, and `store_images`.
- Marketing page endpoint supports `marketing_text` and `store_images`.
- Return per-locale run results synchronously.
- Reject over-limit calls with `429` and a `retryAfterSeconds` value.
- Reuse `TESTFLYING_STATIC_TOKEN` authorization.

---

### Task 1: API models and request guards

**Files:**
- Modify: `/Users/admin/ai_project/apps/testflying-server/src/testflying_api/routes/store_management.py`
- Test: `/Users/admin/ai_project/apps/testflying-server/tests/test_store_management.py`

**Interfaces:**
- Produces: `StoreDirectSyncRequest`, `MarketingDirectSyncRequest`, `DirectSyncResponse`
- Produces: `_direct_sync_guard(...)` context manager for rate limiting, idempotency, and account lock

- [ ] Add Pydantic request/response models.
- [ ] Add process-local rate limiter: 10 requests/minute and 100 requests/hour per token/account/app.
- [ ] Add idempotency cache keyed by token/account/app/endpoint/idempotencyKey.
- [ ] Add per-account non-blocking lock; return `409 account_sync_in_progress` when busy.

### Task 2: Default store page direct sync endpoint

**Files:**
- Modify: `/Users/admin/ai_project/apps/testflying-server/src/testflying_api/routes/store_management.py`
- Test: `/Users/admin/ai_project/apps/testflying-server/tests/test_store_management.py`

**Interfaces:**
- Route: `POST /v1/store-management/developer-accounts/{accountId}/apps/{appId}/sync-runs`

- [ ] Resolve scoped app and requested locales.
- [ ] For `metadata`/`store_images`, read current metadata draft per locale and call `sync_current_app_metadata`.
- [ ] For `release_notes`, read release-note draft per locale and call `sync_release_notes`.
- [ ] Commit successful runs and return per-locale operation results.
- [ ] Roll back and return normal `ApiError` responses on missing drafts, failed preflight, or validation errors.

### Task 3: Marketing page direct sync endpoint

**Files:**
- Modify: `/Users/admin/ai_project/apps/testflying-server/src/testflying_api/routes/store_management.py`
- Test: `/Users/admin/ai_project/apps/testflying-server/tests/test_store_management.py`

**Interfaces:**
- Route: `POST /v1/store-management/developer-accounts/{accountId}/apps/{appId}/marketing-pages/{pageId}/sync-runs`

- [ ] Resolve marketing page and requested locales.
- [ ] Call existing `sync_marketing_page` for each locale.
- [ ] Return per-locale operation results.
- [ ] Preserve platform validation: marketing pages are iOS only.

### Task 4: Documentation and verification

**Files:**
- Modify: `/Users/admin/ai_project/apps/testflying-server/docs/store-management-api.md`
- Modify: `/Users/admin/ai_project/apps/testflying-server/docs/api-contract.md`

- [ ] Add curl examples for both direct sync endpoints.
- [ ] Run targeted tests.
- [ ] Run full test suite or at least the project’s standard pytest check before commit.
