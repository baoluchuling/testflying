# Admin App Rebuild Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the current Jinja-driven `/admin` experience with an independent React/Vite Admin App, starting with a no-refresh shell and store review analysis page while preserving the legacy admin as a rollback path.

**Architecture:** FastAPI remains the backend and business boundary. The new Admin App is built from `admin-web/`, copied into the Python package, and served from `/admin`; old Jinja pages move under `/admin-legacy`. New UI data is loaded through `/admin/api/*` JSON endpoints that call existing services.

**Tech Stack:** FastAPI, SQLAlchemy, Pydantic, Vite, React, TypeScript, Vitest, Playwright, pytest, ruff, Docker.

## Global Constraints

- Do not duplicate store sync, upload, review, or connector business logic in the frontend.
- Keep old Jinja admin pages available through `/admin-legacy` until the new Admin App covers the feature.
- New page and tab navigation must not trigger a browser document reload.
- Review page app switching, rating filtering, review fetch, and LLM analysis must update the current route locally.
- JSON API errors must be structured and user-readable.
- Docker and CI must build both backend and frontend.

---

## File Structure

- Create `admin-web/`: Vite + React + TypeScript frontend.
- Create `src/testflying_api/admin_spa.py`: serve built Admin App and legacy redirects.
- Create `src/testflying_api/admin_api/`: JSON endpoints for the new Admin App.
- Modify `src/testflying_api/app.py`: mount built frontend assets and register new admin API routes.
- Modify `src/testflying_api/admin/routes.py`: move old admin HTML under `/admin-legacy`.
- Modify `pyproject.toml`: include built Admin App package data.
- Modify `Dockerfile`: build `admin-web` before Python wheel.
- Modify `.github/workflows/ci.yml`: run frontend lint/test/build.
- Add `tests/test_admin_spa.py` and `tests/test_admin_api_reviews.py`.

---

## Task 1: New Admin App Shell And Legacy Route

**Files:**
- Create: `admin-web/package.json`
- Create: `admin-web/index.html`
- Create: `admin-web/tsconfig.json`
- Create: `admin-web/vite.config.ts`
- Create: `admin-web/src/main.tsx`
- Create: `admin-web/src/app/AdminApp.tsx`
- Create: `admin-web/src/app/routes.tsx`
- Create: `admin-web/src/app/apiClient.ts`
- Create: `admin-web/src/styles/admin.css`
- Create: `src/testflying_api/admin_spa.py`
- Modify: `src/testflying_api/app.py`
- Modify: `src/testflying_api/admin/routes.py`
- Modify: `pyproject.toml`
- Test: `tests/test_admin_spa.py`

**Interfaces:**
- Produces `GET /admin` serving the React shell.
- Produces `GET /admin-legacy` serving the old Jinja dashboard.
- Produces `GET /admin/assets/*` serving built Vite assets.

- [ ] Write tests proving `/admin` returns the new shell marker `data-admin-app-root`.
- [ ] Write tests proving `/admin-legacy` still returns the old Jinja dashboard content.
- [ ] Scaffold Vite React TypeScript with scripts: `lint`, `test`, `build`.
- [ ] Implement `AdminApp` with top navigation, health action, migrated route views, and legacy-backed route views that show a clear “打开旧版后台” action.
- [ ] Implement FastAPI SPA fallback that serves `index.html` for `/admin` subroutes.
- [ ] Move the old admin router prefix to `/admin-legacy` while keeping Basic auth.
- [ ] Run `python -m pytest tests/test_admin_spa.py -q`.
- [ ] Run `npm --prefix admin-web run build`.

## Task 2: Admin JSON API Foundation

**Files:**
- Create: `src/testflying_api/admin_api/__init__.py`
- Create: `src/testflying_api/admin_api/errors.py`
- Create: `src/testflying_api/admin_api/routes.py`
- Create: `src/testflying_api/admin_api/schemas.py`
- Modify: `src/testflying_api/app.py`
- Test: `tests/test_admin_api.py`

**Interfaces:**
- Produces `GET /admin/api/bootstrap`.
- Produces JSON error shape: `{ "error": { "code": string, "message": string, "detail": object|null } }`.
- Reuses existing admin Basic auth.

- [ ] Write tests for `/admin/api/bootstrap` returning nav items and health state.
- [ ] Write tests for an unknown app/account returning structured JSON error.
- [ ] Implement shared `AdminApiError`.
- [ ] Register JSON routes under `/admin/api`.
- [ ] Return nav metadata matching the new shell labels: 总览, 上传, 商店管理, 构建, 设备, App 日志, 通知.
- [ ] Run `python -m pytest tests/test_admin_api.py -q`.

## Task 3: Store Review JSON API

**Files:**
- Modify: `src/testflying_api/admin_api/routes.py`
- Modify: `src/testflying_api/admin_api/schemas.py`
- Test: `tests/test_admin_api_reviews.py`

**Interfaces:**
- Produces `GET /admin/api/store-reviews`.
- Produces `POST /admin/api/store-reviews/fetch`.
- Produces `POST /admin/api/store-reviews/analyze`.
- Calls existing `store_reviews_context`, `fetch_store_reviews_incremental`, and `analyze_store_reviews`.

- [ ] Write tests for review page state with seeded demo apps.
- [ ] Write tests for app switching by `accountId` and `appId`.
- [ ] Write tests for rating filtering.
- [ ] Write tests for fetch returning inserted/fetched/stopped counts.
- [ ] Write tests for disabled LLM returning a friendly structured error.
- [ ] Implement serializers for review apps, reviews, stats, latest fetch run, and latest analysis run.
- [ ] Run `python -m pytest tests/test_admin_api_reviews.py -q`.

## Task 4: React Review Page

**Files:**
- Create: `admin-web/src/pages/StoreReviewsPage.tsx`
- Create: `admin-web/src/pages/StoreReviewsPage.test.tsx`
- Modify: `admin-web/src/app/routes.tsx`
- Modify: `admin-web/src/app/apiClient.ts`
- Modify: `admin-web/src/styles/admin.css`

**Interfaces:**
- Consumes `GET /admin/api/store-reviews`.
- Calls `POST /admin/api/store-reviews/fetch`.
- Calls `POST /admin/api/store-reviews/analyze`.

- [ ] Add a test proving clicking an app updates the selected app without calling `window.location.assign`.
- [ ] Add a test proving rating filter reloads only review state.
- [ ] Add a test proving fetch/analyze buttons show loading and then refresh state.
- [ ] Implement review app list, review list, rating filter, LLM analysis panel, and boundary notes.
- [ ] Use `history.pushState` for selected app/rating so refresh keeps context.
- [ ] Run `npm --prefix admin-web test -- --run`.
- [ ] Run `npm --prefix admin-web run build`.

## Task 5: Docker And CI Frontend Build

**Files:**
- Modify: `Dockerfile`
- Modify: `.github/workflows/ci.yml`
- Modify: `.dockerignore`
- Modify: `pyproject.toml`
- Test: `tests/test_docker_runtime.py`

**Interfaces:**
- Python wheel includes `static/admin-app/**`.
- Runtime image serves built frontend without requiring Node.

- [ ] Add a backend test that package data includes the Admin App build directory when present.
- [ ] Update Dockerfile with a Node builder stage that runs `npm ci` and `npm run build`.
- [ ] Copy `admin-web/dist` into `src/testflying_api/static/admin-app` before Python wheel build.
- [ ] Update CI to run `npm ci`, `npm run lint`, `npm test -- --run`, and `npm run build`.
- [ ] Run `docker build -t testflying-server:admin-app-test .` locally if Docker is available.

## Task 6: Store Management Migration

**Files:**
- Create: `admin-web/src/pages/StoreAppsPage.tsx`
- Create: `admin-web/src/pages/StoreManagementPage.tsx`
- Create: `admin-web/src/pages/MarketingPagesPage.tsx`
- Create: `admin-web/src/pages/StoreConnectionPage.tsx`
- Modify: `src/testflying_api/admin_api/routes.py`
- Modify: `src/testflying_api/admin_api/schemas.py`
- Test: `tests/test_admin_api_store_management.py`

**Interfaces:**
- Produces JSON APIs for app list, default store page state, marketing page state, connection state, save draft, preflight, and sync run.
- Reuses existing store sync service and database models.

- [ ] Add JSON endpoint tests for app list and app logo/icon data.
- [ ] Add JSON endpoint tests for default store page state.
- [ ] Add JSON endpoint tests for marketing pages.
- [ ] Add JSON endpoint tests for store connection checks.
- [ ] Implement React pages using the retained store management design baseline.
- [ ] Preserve sync confirmation modal before actual sync.
- [ ] Run backend and frontend tests.

## Task 7: Upload And App Logs Migration

**Files:**
- Create: `admin-web/src/pages/UploadPage.tsx`
- Create: `admin-web/src/pages/AppLogsPage.tsx`
- Modify: `src/testflying_api/admin_api/routes.py`
- Modify: `src/testflying_api/admin_api/schemas.py`
- Test: `tests/test_admin_api_uploads.py`
- Test: `tests/test_admin_api_app_logs.py`

**Interfaces:**
- Upload uses existing `/uploads` multipart API or an admin wrapper that calls the same service.
- App logs use existing `/admin-legacy/app-logs/events` behavior until a JSON route replaces it.

- [ ] Add upload progress handling in frontend global state.
- [ ] Add package parsing result display after upload.
- [ ] Add App Logs compact connected layout.
- [ ] Add log filter and device filter without page reload.
- [ ] Run Playwright smoke for upload route change and app logs route change.

## Task 8: Remaining Admin Pages And Legacy Cleanup

**Files:**
- Create: `admin-web/src/pages/DashboardPage.tsx`
- Create: `admin-web/src/pages/BuildsPage.tsx`
- Create: `admin-web/src/pages/DevicesPage.tsx`
- Create: `admin-web/src/pages/NotificationsPage.tsx`
- Modify: `src/testflying_api/admin_api/routes.py`
- Modify: `README.md`
- Modify: `docs/store-management-api.md`

**Interfaces:**
- Produces JSON endpoints for dashboard summary, builds, devices, notifications.

- [ ] Implement ordinary list pages.
- [ ] Add Playwright smoke covering all top-level nav items.
- [ ] Document `/admin` and `/admin-legacy`.
- [ ] Decide whether `/admin-legacy` stays hidden or is removed in a later release.
- [ ] Run full verification: `ruff`, `pytest`, frontend lint/test/build, Docker build.
- [ ] Commit, push, and deploy to `testflying-prod`.

---

## Plan Self-Review

- Spec coverage: Tasks 1-8 map to shell, JSON API, reviews, store management, upload, App Logs, ordinary pages, CI, Docker, and legacy rollback.
- Scope: This is a large migration, but each task produces a deployable slice and keeps legacy fallback.
- Known risk: Task 6 is the largest UI migration; it should only start after Task 1-5 are stable.
