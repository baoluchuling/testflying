# Product Page Optimization API Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Add API-only support for querying and creating Apple App Store Connect Product Page Optimization experiments.

**Architecture:** Keep testflying-server as the public API surface and connector as the store credential boundary. The center API validates token/account/app, then calls connector; connector mock mode returns deterministic data and live mode calls Apple `AppStoreVersionExperimentsV2` APIs.

**Tech Stack:** FastAPI, Pydantic v2, SQLAlchemy, pytest, Go net/http, Go tests, Apple App Store Connect API.

## Global Constraints

- No UI changes in this slice.
- Apple only; Android returns unsupported.
- Creation does not start an experiment.
- Store credentials remain connector-local.
- Existing direct-sync rate limits can be reused for center-facing mutation calls.

---

### Task 1: Connector PPO Contracts And Routes

**Files:**
- Modify: `connector/internal/connector/models.go`
- Modify: `connector/internal/connector/server.go`
- Modify: `connector/internal/connector/store.go`
- Test: `connector/internal/connector/server_test.go`

**Interfaces:**
- Produces `GET /v1/apps/{appId}/product-page-optimizations`
- Produces `POST /v1/apps/{appId}/product-page-optimizations`

- [x] Add request/response structs for Product Page Optimization.
- [x] Add routes guarded by connector token.
- [x] Implement mock and live gateway methods.
- [x] Add Go tests for list/create in mock mode.

### Task 2: Center API Proxy

**Files:**
- Modify: `src/testflying_api/store_sync.py`
- Modify: `src/testflying_api/routes/store_management.py`
- Test: `tests/test_store_management.py`

**Interfaces:**
- Produces `GET /v1/store-management/developer-accounts/{accountId}/apps/{appId}/product-page-optimizations`
- Produces `POST /v1/store-management/developer-accounts/{accountId}/apps/{appId}/product-page-optimizations`

- [x] Add Pydantic models for list/create.
- [x] Add `StoreConnectorClient` proxy methods.
- [x] Validate iOS app and connector presence.
- [x] Reuse direct-sync rate limiting/idempotency for create.
- [x] Add pytest coverage for list/create/auth/platform validation.

### Task 3: Docs And Verification

**Files:**
- Modify: `docs/store-management-api.md`
- Modify: `docs/api-contract.md`

- [x] Document curl examples for list and create.
- [x] Run Python lint/tests.
- [x] Run Go tests.
- [x] Commit, push, and deploy if verification passes.
