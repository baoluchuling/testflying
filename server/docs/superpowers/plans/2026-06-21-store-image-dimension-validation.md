# Store Image Dimension Validation Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make store screenshot previews use store-specific aspect ratios and reject uploaded images whose dimensions do not meet Apple or Google requirements.

**Architecture:** Add a focused Python validator for server-side upload checks, expose the same rule metadata to the admin template, and mirror the validation in the existing admin JavaScript for immediate feedback. Keep storage and metadata draft persistence unchanged.

**Tech Stack:** FastAPI, Jinja templates, vanilla JavaScript, CSS, pytest.

---

## Chunk 1: Rule Model And Server Validation

### Task 1: Add store image requirement rules

**Files:**
- Create: `src/testflying_api/store_image_requirements.py`
- Modify: `src/testflying_api/admin/routes.py`
- Test: `tests/test_store_image_requirements.py`

- [x] Define platform-aware rules for Apple and Google store image slots.
- [x] Validate uploaded image dimensions using Pillow-free stdlib image header parsing where practical, or lightweight PNG/JPEG parsers.
- [x] Return friendly reasons for invalid files.
- [x] Block invalid uploaded store images before saving.
- [x] Add tests for valid Apple iPhone/iPad screenshots, invalid Apple dimensions, valid Google screenshots, invalid Google ratio, and Google feature graphic size.

## Chunk 2: Admin UI Feedback And Proportional Preview

### Task 2: Surface rules in the admin page

**Files:**
- Modify: `src/testflying_api/admin/view_models.py`
- Modify: `src/testflying_api/templates/admin/store_metadata.html`
- Modify: `src/testflying_api/templates/admin/base.html`
- Modify: `src/testflying_api/static/admin/admin.css`
- Test: `tests/test_admin.py`

- [x] Add platform and slot requirement metadata to the store image slot view model.
- [x] Render requirement metadata into `data-*` attributes on upload zones and file inputs.
- [x] Read image dimensions in the browser before adding files.
- [x] Reject invalid local images with a visible status message.
- [x] Render thumbnails and lightbox cards with CSS aspect ratios derived from each image when available, falling back to the slot requirement ratio.
- [x] Show per-image resolution and validation status.

## Chunk 3: Verification And Deployment

### Task 3: Extend verification and ship

**Files:**
- Modify: `scripts/verify_admin_store_metadata_ui.py`
- Run: `.venv/bin/python3 -m pytest -q`
- Run: `.venv/bin/python3 scripts/verify_admin_store_metadata_ui.py --url <local-url>`

- [x] Verify the page contains rule metadata, validation functions, status UI, and proportional preview hooks.
- [x] Run full tests and local UI verification.
- [ ] Commit, push, and update the remote Docker deployment.
