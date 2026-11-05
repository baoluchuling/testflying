# Admin Full SPA Migration Correction Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Finish the admin migration so every visible/admin-enterable page runs inside `/admin-next` React without falling back to old `/admin/...` Jinja pages.

**Architecture:** Keep FastAPI business services as the source of truth. Add JSON API adapters under `src/testflying_api/admin_api/`, then build React pages under `admin-web/src/pages/` that call those APIs. Legacy `/admin/...` routes may remain for compatibility, but new React UI must not link to them.

**Tech Stack:** FastAPI, SQLAlchemy, Pydantic, React, TypeScript, Vite, Vitest, pytest.

## Global Constraints

- No visible React navigation may use `/admin/...` page links, except file/download endpoints that are intentionally non-SPA resources.
- Any newly added page must support browser back/forward via `history.pushState` and `popstate`.
- Admin forms must use JSON or multipart API endpoints and show success/error inline without full document navigation.
- Store metadata, marketing page, account, connector, and release-note behavior must reuse existing service functions rather than duplicate business rules in React.
- Verification must include an automated guard that fails if React-rendered admin UI exposes legacy `/admin/developer-accounts` or `/admin/.../store` page links.
- Final completion requires local tests, build, commit, push, deploy, and remote smoke checks.

---

### Task 1: Add Route Coverage Guard

**Files:**
- Modify: `admin-web/src/pages/StoreAppsPage.test.tsx`
- Modify: `admin-web/src/app/AdminApp.test.tsx`
- Create or modify: `tests/test_admin_spa_routes.py`

**Interfaces:**
- Consumes: current `/admin-next` route list and React page output.
- Produces: failing tests that expose legacy links and missing SPA routes.

- [ ] Add a React test that renders `StoreAppsPage` with a selected bound app and asserts no anchor points to `/admin/developer-accounts`.
- [ ] Add a FastAPI test that confirms `/admin-next/*` returns the React shell for deep paths such as `/admin-next/accounts`, `/admin-next/accounts/{id}`, `/admin-next/apps/{appId}/store`, `/admin-next/apps/{appId}/marketing`, `/admin-next/apps/{appId}/connection`, and `/admin-next/apps/{appId}/release-notes`.
- [ ] Run the focused tests and confirm the current code fails on at least one legacy-link assertion.

### Task 2: Developer Accounts API

**Files:**
- Modify: `src/testflying_api/admin_api/schemas.py`
- Modify: `src/testflying_api/admin_api/routes.py`
- Add tests: `tests/test_admin_api_accounts.py`

**Interfaces:**
- Produces:
  - `GET /admin/api/developer-accounts`
  - `GET /admin/api/developer-accounts/{accountId}`
  - `POST /admin/api/developer-accounts`
  - `PATCH /admin/api/developer-accounts/{accountId}`
  - `POST /admin/api/developer-accounts/{accountId}/connector`
  - `POST /admin/api/developer-accounts/{accountId}/connector/check`
  - `POST /admin/api/developer-accounts/{accountId}/apps`
  - `PATCH /admin/api/developer-accounts/{accountId}/apps/{appId}/settings`
  - `DELETE /admin/api/developer-accounts/{accountId}/apps/{appId}`

- [ ] Add Pydantic response/request models for account list, account detail, connector config, app binding, and account form errors.
- [ ] Wrap existing service functions: `list_accounts`, `account_detail_context`, `save_developer_account`, `save_connector`, `check_connector_health`, `bind_app_to_account`, `update_bound_app_store_settings`, `unbind_app_from_account`.
- [ ] Preserve current behavior: one connector per account, app binding uses iOS app ID or Android package name, connector checks return friendly status.
- [ ] Add pytest coverage for list, create/update validation, connector check success/failure shape, bind/update/unbind.

### Task 3: Developer Accounts React Pages

**Files:**
- Modify: `admin-web/src/app/routes.tsx`
- Modify: `admin-web/src/app/AdminApp.tsx`
- Modify: `admin-web/src/app/apiClient.ts`
- Create: `admin-web/src/pages/DeveloperAccountsPage.tsx`
- Test: `admin-web/src/pages/DeveloperAccountsPage.test.tsx`

**Interfaces:**
- Consumes Task 2 APIs.
- Produces React routes:
  - `/admin-next/accounts`
  - `/admin-next/accounts/new`
  - `/admin-next/accounts/:accountId`
  - `/admin-next/accounts/:accountId/edit`

- [ ] Add route key(s) and route matching for accounts.
- [ ] Implement account list distinct from store management: account cards/table focused on credentials, connector, bound app count, renewal status.
- [ ] Implement inline create/edit account form.
- [ ] Implement account detail sections: connector config/package instructions, bound apps, app binding form, store identifier edit, unbind.
- [ ] Convert existing “账号与连接” actions in store management to `/admin-next/accounts`.
- [ ] Add tests for list navigation, open account, save connector, bind app, and no legacy `/admin/developer-accounts` anchors.

### Task 4: Store Workspace API

**Files:**
- Modify: `src/testflying_api/admin_api/schemas.py`
- Modify: `src/testflying_api/admin_api/routes.py`
- Add tests: `tests/test_admin_api_store_workspace.py`

**Interfaces:**
- Produces:
  - `GET /admin/api/store-workspace/{accountId}/{appId}`
  - `PUT /admin/api/store-workspace/{accountId}/{appId}/metadata`
  - `POST /admin/api/store-workspace/{accountId}/{appId}/metadata/preflight`
  - `POST /admin/api/store-workspace/{accountId}/{appId}/metadata/sync`
  - `DELETE /admin/api/store-workspace/{accountId}/{appId}/metadata/store-images`
  - `GET /admin/api/store-workspace/{accountId}/{appId}/connection`
  - `POST /admin/api/store-workspace/{accountId}/{appId}/connection/check`
  - `GET /admin/api/store-workspace/{accountId}/{appId}/release-notes`
  - `PUT /admin/api/store-workspace/{accountId}/{appId}/release-notes`
  - `POST /admin/api/store-workspace/{accountId}/{appId}/release-notes/sync`

- [ ] Adapt `store_metadata_context`, `release_notes_context`, and `store_metadata` form helpers into JSON/multipart-safe API handlers.
- [ ] Keep readonly keywords behavior.
- [ ] Keep store image upload/delete and validation.
- [ ] Keep sync confirmation inputs as explicit payload fields.
- [ ] Add pytest coverage for load, save text, delete image, preflight, sync-scope validation, and release-note save.

### Task 5: Store Workspace React Pages

**Files:**
- Modify: `admin-web/src/app/routes.tsx`
- Modify: `admin-web/src/app/AdminApp.tsx`
- Modify: `admin-web/src/pages/StoreAppsPage.tsx`
- Create: `admin-web/src/pages/StoreWorkspacePage.tsx`
- Test: `admin-web/src/pages/StoreWorkspacePage.test.tsx`

**Interfaces:**
- Consumes Task 4 APIs.
- Produces React routes:
  - `/admin-next/accounts/:accountId/apps/:appId/store`
  - `/admin-next/accounts/:accountId/apps/:appId/release-notes`
  - `/admin-next/accounts/:accountId/apps/:appId/connection`

- [ ] Replace “打开商店编辑” with SPA navigation into the new store workspace route.
- [ ] Implement default store page using the approved compact layout direction: item list, current language fields, expandable all-language rows, store image preview/upload/delete, sync confirmation modal.
- [ ] Implement release-notes route or tab without leaving React.
- [ ] Implement store connection route focused on connector state, app identifiers, supported locales, latest version, and real-time check.
- [ ] Add Vitest coverage for navigation, language expand, store image delete, sync confirmation, and no old `/admin/.../store` links.

### Task 6: Marketing Pages API and React Migration

**Files:**
- Modify: `src/testflying_api/admin_api/schemas.py`
- Modify: `src/testflying_api/admin_api/routes.py`
- Create: `admin-web/src/pages/MarketingPagesPage.tsx`
- Create: `admin-web/src/pages/MarketingPageDetailPage.tsx`
- Add tests: `tests/test_admin_api_marketing_pages.py`, `admin-web/src/pages/MarketingPagesPage.test.tsx`

**Interfaces:**
- Produces:
  - `GET /admin/api/store-workspace/{accountId}/{appId}/marketing`
  - `POST /admin/api/store-workspace/{accountId}/{appId}/marketing-pages`
  - `GET /admin/api/store-workspace/{accountId}/{appId}/marketing-pages/{pageId}`
  - `PUT /admin/api/store-workspace/{accountId}/{appId}/marketing-pages/{pageId}`
  - `POST /admin/api/store-workspace/{accountId}/{appId}/marketing-pages/{pageId}/copy`
  - `DELETE /admin/api/store-workspace/{accountId}/{appId}/marketing-pages/{pageId}`
  - `POST /admin/api/store-workspace/{accountId}/{appId}/marketing-pages/{pageId}/preflight`
  - `POST /admin/api/store-workspace/{accountId}/{appId}/marketing-pages/{pageId}/sync`
  - `DELETE /admin/api/store-workspace/{accountId}/{appId}/marketing-pages/{pageId}/store-images`

- [ ] Adapt `store_marketing_context`, `marketing_page_context`, and marketing page service functions into JSON/multipart API.
- [ ] Implement marketing list with clickable whole-row/card items.
- [ ] Implement create, detail edit, copy, delete, preflight, sync, image upload/delete.
- [ ] Add tests for iOS-only behavior, create with content, copy/delete, and React no legacy link guard.

### Task 7: Final Legacy-Link Sweep and Deployment

**Files:**
- Modify as needed: `admin-web/src/**/*`, `src/testflying_api/admin_api/**/*`, tests.

- [ ] Run `rg -n 'href=.*"/admin|href=.*\\'/admin|/admin/developer-accounts|/admin/.*/store' admin-web/src src/testflying_api/admin_api`.
- [ ] Allow only known file/download compatibility endpoints such as `/admin/api-docs/store-management.md` and `/admin/artifacts/...`.
- [ ] Run full verification: `npm --prefix admin-web run lint`, `npm --prefix admin-web test -- --run`, `npm --prefix admin-web run build`, `python -m ruff check src tests`, `python -m pytest -q`.
- [ ] Commit, push `main`, deploy on `testflying-prod` from `/root/testflight-server`, and smoke check `/admin-next` plus every new deep route.
