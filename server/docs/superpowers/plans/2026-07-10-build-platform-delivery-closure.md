# Build Platform Delivery Closure Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Complete TestFlying's build delivery loop with durable DingTalk failure notifications, a self-contained macOS Runner package, authenticated automatic updates, and real Flutter build acceptance.

**Architecture:** Terminal build transitions create in-app notifications and durable webhook outbox rows in the same transaction. A FastAPI lifespan worker signs and retries DingTalk deliveries. macOS release tooling packages the Go Runner and a PyInstaller `package-agent`; the Runner checks authenticated release manifests and atomically replaces both binaries. Real acceptance runs the packaged Agent against an unchanged project and verifies the required artifacts and Git boundaries.

**Tech Stack:** Python 3.11, FastAPI, SQLAlchemy 2, Alembic, React 18, TypeScript, Go, Bash, PyInstaller, macOS `pkgbuild`, Flutter/FVM.

## Global Constraints

- DingTalk is the only external notification channel in this phase.
- Read `TESTFLYING_DINGTALK_WEBHOOK_URL` and `TESTFLYING_DINGTALK_SECRET` only from the server environment; never persist or return their values.
- Notify only agent builds entering `failed` or `needs_human`; deduplicate by `build_id + lifecycle_status + channel`.
- DingTalk delivery failure must never change the build result.
- Retry deliveries at 0 seconds, 1 minute, 5 minutes, 30 minutes, and 2 hours; mark the fifth failed attempt `dead`.
- The Notifications page displays configuration status and setup instructions without secret inputs or masked credential values.
- The macOS package must contain both the Go Runner and a PyInstaller `--onefile` `package-agent` and reject incomplete node configuration.
- Automatic updates must authenticate with the existing Runner token, verify SHA-256, reject unsafe archives, and atomically replace both binaries.
- The Agent may run `git fetch`, `git pull`, inspection commands, and local Git operations in its managed checkout; it must block `git commit` and `git push`.
- The Agent must not modify project source files or project build scripts to make a build pass.
- Do not install or overwrite Codex CLI, Claude CLI, or `llm-runtime`; keep automatic discovery order `codex -> claude -> llm-runtime`.
- Do not push Git commits.

## File Map

- `src/testflying_api/dingtalk.py`: signing and one-request DingTalk transport.
- `src/testflying_api/webhook_delivery.py`: outbox enqueueing, retry scheduling, dispatcher loop, and delivery counts.
- `src/testflying_api/build_notifications.py`: terminal build message and in-app notification construction.
- `src/testflying_api/runner_releases.py`: release manifest validation and contained bundle lookup.
- `build-runner/internal/runner/update.go`: update check, archive validation, checksum, atomic replacement, and rollback.
- `build-runner/internal/runner/loop.go`: continuous polling and update cadence; keep `RunOnce` for focused tests.
- `scripts/build_runner_installer.sh`: build both binaries, release bundle, checksum, and per-node `.pkg`.
- `build-runner/packaging/postinstall`: install LaunchAgent for the console user.
- `scripts/verify_real_build.sh`: immutable-worktree real build acceptance.

---

### Task 1: Persist DingTalk Configuration and Delivery Outbox

**Files:**
- Create: `alembic/versions/20260710_0011_webhook_deliveries.py`
- Modify: `src/testflying_api/config.py`
- Modify: `src/testflying_api/schema.py`
- Modify: `tests/conftest.py`
- Modify: `tests/test_config.py`
- Modify: `tests/test_schema.py`

**Interfaces:**
- Produces: `Settings.dingtalk_configured: bool`.
- Produces: `WebhookDelivery` with unique `event_key` and statuses `pending`, `delivered`, `dead`.
- Consumed by: Tasks 2 and 3.

- [ ] **Step 1: Write failing settings and schema tests**

```python
def test_settings_reads_dingtalk_and_runner_release_configuration(monkeypatch, tmp_path):
    monkeypatch.setenv("TESTFLYING_DINGTALK_WEBHOOK_URL", " https://oapi.dingtalk.test/robot/send ")
    monkeypatch.setenv("TESTFLYING_DINGTALK_SECRET", " SEC-test ")
    monkeypatch.setenv("TESTFLYING_DINGTALK_TIMEOUT_SECONDS", "7")
    monkeypatch.setenv("TESTFLYING_DINGTALK_DISPATCH_INTERVAL_SECONDS", "12")
    monkeypatch.setenv("TESTFLYING_RUNNER_RELEASE_ROOT", str(tmp_path / "releases"))

    settings = Settings.from_environment()

    assert settings.dingtalk_webhook_url == "https://oapi.dingtalk.test/robot/send"
    assert settings.dingtalk_secret == "SEC-test"
    assert settings.dingtalk_timeout_seconds == 7.0
    assert settings.dingtalk_dispatch_interval_seconds == 12.0
    assert settings.dingtalk_configured is True
    assert settings.runner_release_root == tmp_path / "releases"


def test_webhook_delivery_event_key_is_unique():
    engine = create_engine_for_url("sqlite:///:memory:")
    Base.metadata.create_all(engine)
    constraints = inspect(engine).get_unique_constraints("webhook_deliveries")
    assert any(item["column_names"] == ["event_key"] for item in constraints)
```

- [ ] **Step 2: Run tests and confirm the new fields/table are missing**

Run: `PYTHONPATH=src pytest tests/test_config.py tests/test_schema.py -q`

Expected: FAIL because the DingTalk settings and `webhook_deliveries` do not exist.

- [ ] **Step 3: Add typed settings and the outbox model**

Add these `Settings` fields and property, using `_normalize_optional` and a positive-float parser that raises `ValueError` for zero, negative, or malformed values:

```python
dingtalk_webhook_url: str | None
dingtalk_secret: str | None
dingtalk_timeout_seconds: float
dingtalk_dispatch_interval_seconds: float
runner_release_root: Path

@property
def dingtalk_configured(self) -> bool:
    return bool(self.dingtalk_webhook_url and self.dingtalk_secret)
```

Map environment defaults to timeout `5`, interval `10`, and release root `./data/runner-releases`. Add this model to `schema.py`:

```python
class WebhookDelivery(Base):
    __tablename__ = "webhook_deliveries"
    __table_args__ = (UniqueConstraint("event_key", name="uq_webhook_deliveries_event_key"),)

    id: Mapped[str] = mapped_column(String(80), primary_key=True)
    channel: Mapped[str] = mapped_column(String(30), nullable=False)
    event_key: Mapped[str] = mapped_column(String(240), nullable=False)
    status: Mapped[str] = mapped_column(String(30), nullable=False, default="pending")
    payload_json: Mapped[dict] = mapped_column(JSON, nullable=False, default=dict)
    attempt_count: Mapped[int] = mapped_column(Integer, nullable=False, default=0)
    next_attempt_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, default=utc_now
    )
    last_error: Mapped[str | None] = mapped_column(Text)
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, default=utc_now
    )
    delivered_at: Mapped[datetime | None] = mapped_column(DateTime(timezone=True))
```

Create the Alembic table with the same columns, unique constraint, and indexes on `(status, next_attempt_at)` and `channel`. Downgrade drops indexes before the table. Update the `test_settings` fixture with `None`, default timing values, and `tmp_path / "runner-releases"`.

- [ ] **Step 4: Run focused tests and migration round trip**

Run:

```bash
PYTHONPATH=src pytest tests/test_config.py tests/test_schema.py -q
TMP_DB="$(mktemp -t testflying-webhook.XXXXXX.db)"
TESTFLYING_DATABASE_URL="sqlite:///$TMP_DB" PYTHONPATH=src alembic upgrade head
TESTFLYING_DATABASE_URL="sqlite:///$TMP_DB" PYTHONPATH=src alembic downgrade 20260710_0010
rm -f "$TMP_DB"
```

Expected: tests pass; migration upgrades to `20260710_0011` and downgrades to `20260710_0010`.

- [ ] **Step 5: Commit Task 1**

```bash
git add alembic/versions/20260710_0011_webhook_deliveries.py src/testflying_api/config.py src/testflying_api/schema.py tests/conftest.py tests/test_config.py tests/test_schema.py
git commit -m "feat(notifications): add webhook delivery outbox"
```

### Task 2: Sign, Send, and Retry DingTalk Deliveries

**Files:**
- Create: `src/testflying_api/dingtalk.py`
- Create: `src/testflying_api/webhook_delivery.py`
- Create: `tests/test_dingtalk.py`
- Create: `tests/test_webhook_delivery.py`
- Modify: `src/testflying_api/app.py`
- Modify: `docker-compose.yml`
- Modify: `README.md`

**Interfaces:**
- Consumes: `Settings` and `WebhookDelivery` from Task 1.
- Produces: `signed_webhook_url(url, secret, timestamp_ms) -> str`.
- Produces: `send_dingtalk_markdown(...) -> None`, raising `DingTalkDeliveryError` on HTTP or nonzero `errcode` failure.
- Produces: `dispatch_due_deliveries(session_factory, settings, now=None) -> int` and `run_delivery_loop(...)`.

- [ ] **Step 1: Write failing signature and transport tests**

Use a fixed timestamp and independently calculated HMAC in the assertion. Cover successful `{"errcode": 0}`, HTTP 500, timeout, invalid JSON, and nonzero `errcode` without asserting secret-bearing URLs in errors.

```python
def test_signed_webhook_url_uses_dingtalk_hmac():
    signed = signed_webhook_url(
        "https://oapi.dingtalk.test/robot/send?access_token=abc",
        "SEC-test",
        1700000000123,
    )
    query = parse_qs(urlsplit(signed).query)
    assert query["timestamp"] == ["1700000000123"]
    assert query["access_token"] == ["abc"]
    assert query["sign"] == [expected_signature]
```

- [ ] **Step 2: Run the DingTalk tests and confirm imports fail**

Run: `PYTHONPATH=src pytest tests/test_dingtalk.py -q`

Expected: FAIL because `testflying_api.dingtalk` is missing.

- [ ] **Step 3: Implement the DingTalk transport**

Use only the Python standard library. The public implementation must have this shape:

```python
class DingTalkDeliveryError(RuntimeError):
    pass


def signed_webhook_url(url: str, secret: str, timestamp_ms: int) -> str:
    string_to_sign = f"{timestamp_ms}\n{secret}".encode()
    digest = hmac.new(secret.encode(), string_to_sign, sha256).digest()
    sign = b64encode(digest).decode()
    parts = urlsplit(url)
    query = parse_qsl(parts.query, keep_blank_values=True)
    query.extend((("timestamp", str(timestamp_ms)), ("sign", sign)))
    return urlunsplit((parts.scheme, parts.netloc, parts.path, urlencode(query), parts.fragment))


def send_dingtalk_markdown(
    *, webhook_url: str, secret: str, title: str, markdown: str,
    timeout_seconds: float, timestamp_ms: int | None = None,
    opener: Callable[..., Any] = urlopen,
) -> None:
    timestamp = timestamp_ms or int(time.time() * 1000)
    payload = json.dumps({"msgtype": "markdown", "markdown": {
        "title": title, "text": markdown,
    }}).encode()
    request = Request(
        signed_webhook_url(webhook_url, secret, timestamp),
        data=payload,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        with opener(request, timeout=timeout_seconds) as response:
            result = json.loads(response.read(65536))
    except (OSError, ValueError, HTTPError, URLError) as error:
        raise DingTalkDeliveryError(redact_text(str(error))[:500]) from error
    if result.get("errcode") != 0:
        raise DingTalkDeliveryError(redact_text(str(result.get("errmsg") or "unknown"))[:500])
```

- [ ] **Step 4: Write failing dispatcher tests**

Create pending deliveries in SQLite and verify: due rows deliver; future rows remain pending; attempts schedule `1m`, `5m`, `30m`, `2h`; fifth failure becomes `dead`; successful rows become `delivered`; errors are redacted and truncated; a second dispatcher invocation does not resend delivered rows.

- [ ] **Step 5: Implement dispatcher and lifespan loop**

Use this fixed retry schedule:

```python
RETRY_DELAYS = (
    timedelta(0), timedelta(minutes=1), timedelta(minutes=5),
    timedelta(minutes=30), timedelta(hours=2),
)
```

`dispatch_due_deliveries` must open its own session, select `pending` rows with `next_attempt_at <= now`, send one by one, update attempt count, and commit after each row so one failure cannot roll back another. `run_delivery_loop` must use `asyncio.to_thread`, wait with `asyncio.wait_for(stop_event.wait(), timeout=interval)`, and stop cleanly on cancellation.

Create a FastAPI lifespan context in `app.py`, storing `app.state.delivery_stop_event` and `app.state.delivery_task`. Start the task only when `settings.dingtalk_configured`; always cancel/await it during shutdown. Specific lifespan tests must use `with TestClient(app)`.

- [ ] **Step 6: Document environment variables and Compose wiring**

Add pass-through entries without defaults containing secrets:

```yaml
TESTFLYING_DINGTALK_WEBHOOK_URL: ${TESTFLYING_DINGTALK_WEBHOOK_URL:-}
TESTFLYING_DINGTALK_SECRET: ${TESTFLYING_DINGTALK_SECRET:-}
TESTFLYING_DINGTALK_TIMEOUT_SECONDS: ${TESTFLYING_DINGTALK_TIMEOUT_SECONDS:-5}
TESTFLYING_DINGTALK_DISPATCH_INTERVAL_SECONDS: ${TESTFLYING_DINGTALK_DISPATCH_INTERVAL_SECONDS:-10}
TESTFLYING_RUNNER_RELEASE_ROOT: /app/data/runner-releases
```

README must state that both DingTalk values are required and are never shown by the admin API.

- [ ] **Step 7: Run Task 2 tests and commit**

Run: `PYTHONPATH=src pytest tests/test_dingtalk.py tests/test_webhook_delivery.py -q`

```bash
git add src/testflying_api/dingtalk.py src/testflying_api/webhook_delivery.py src/testflying_api/app.py tests/test_dingtalk.py tests/test_webhook_delivery.py docker-compose.yml README.md
git commit -m "feat(notifications): deliver signed dingtalk messages"
```

### Task 3: Enqueue Terminal Build Notifications and Add the Admin Tutorial

**Files:**
- Create: `src/testflying_api/build_notifications.py`
- Create: `tests/test_build_notifications.py`
- Modify: `src/testflying_api/build_platform.py`
- Modify: `src/testflying_api/admin_api/routes.py`
- Modify: `src/testflying_api/admin_api/schemas.py`
- Modify: `tests/test_build_runner_api.py`
- Modify: `tests/test_admin_api_pages.py`
- Modify: `admin-web/src/app/apiClient.ts`
- Modify: `admin-web/src/pages/NotificationsPage.tsx`
- Modify: `admin-web/src/pages/OrdinaryPages.test.tsx`
- Modify: `admin-web/src/styles/admin.css`

**Interfaces:**
- Consumes: `WebhookDelivery` and dispatcher data from Tasks 1-2.
- Produces: `enqueue_terminal_build_notifications(session, build, *, dingtalk_enabled, public_base_url) -> None`.
- Produces: `DingTalkConfigState` in `NotificationsState`.

- [ ] **Step 1: Write failing terminal notification tests**

Cover `failed`, `needs_human`, `succeeded`, `cancelled`, duplicate enqueue, secret redaction, and retry-cap transition. Assert failed/needs-human create exactly one `Notification`; configured DingTalk also creates one `WebhookDelivery`; other statuses create neither delivery nor build-failure notification.

```python
enqueue_terminal_build_notifications(
    db_session,
    build,
    dingtalk_enabled=True,
    public_base_url="https://dist.example.test",
)
assert db_session.scalar(select(func.count(WebhookDelivery.id))) == 1
assert db_session.scalar(select(func.count(Notification.id))) == 1
```

- [ ] **Step 2: Run tests and confirm the helper is missing**

Run: `PYTHONPATH=src pytest tests/test_build_notifications.py -q`

Expected: FAIL importing `build_notifications`.

- [ ] **Step 3: Implement terminal message construction and enqueueing**

Build a redacted payload with keys `title` and `markdown`. Use event key
`build:{build.id}:{build.lifecycle_status}:dingtalk`. The app URL must be
`{public_base_url.rstrip('/')}/admin/apps/{quote(build.app_id, safe='')}`.

Create notification IDs with `notice-build-{uuid4().hex[:12]}`, type `build`, section `构建`, icon `alert-triangle`, tag `需处理`, and a red tag color. Before adding either record, query by `build_id`/terminal marker and `event_key` respectively to make the helper idempotent.

- [ ] **Step 4: Call the helper from every agent terminal path**

Extend `complete_runner_build`, `poll_runner_build`, `_recover_expired_assignments`, and `_mark_retry_cap_needs_human` with keyword-only `dingtalk_enabled: bool = False` and `public_base_url: str = ""`. Call the helper before the existing commit in `complete_runner_build` and before the recovery commit when retry cap is reached.

Pass `request.app.state.settings.dingtalk_configured` and `public_base_url` from `runner_build_complete` and `runner_poll`. Keep uploaded/manual builds unchanged.

- [ ] **Step 5: Write failing admin API and React tutorial tests**

Backend response contract:

```python
class DingTalkConfigState(AdminApiModel):
    configured: bool
    webhook_configured: bool
    secret_configured: bool
    triggers: list[str]
    pending_delivery_count: int
    dead_delivery_count: int

class NotificationsState(AdminApiModel):
    notifications: list[NotificationItem]
    type_counts: list[NotificationTypeCount]
    active_type: str
    total: int
    dingtalk: DingTalkConfigState
```

Assert serialized JSON contains booleans/counts and does not contain either configured secret value. React test must find `钉钉机器人配置`, `TESTFLYING_DINGTALK_WEBHOOK_URL`, `TESTFLYING_DINGTALK_SECRET`, `加签`, `failed`, and `needs_human`.

- [ ] **Step 6: Implement the Notifications page tutorial**

Add the matching TypeScript type. Render one unframed configuration section before the notification list with a status badge, pending/dead counts, an ordered five-step setup guide, and this exact Compose snippet:

```yaml
TESTFLYING_DINGTALK_WEBHOOK_URL: ${TESTFLYING_DINGTALK_WEBHOOK_URL}
TESTFLYING_DINGTALK_SECRET: ${TESTFLYING_DINGTALK_SECRET}
```

Do not add inputs or display API values for URL/secret. Keep the existing filter behavior and responsive layout.

- [ ] **Step 7: Run Task 3 verification and commit**

Run:

```bash
PYTHONPATH=src pytest tests/test_build_notifications.py tests/test_build_runner_api.py tests/test_admin_api_pages.py -q
cd admin-web
npm run lint
npm test -- --run src/pages/OrdinaryPages.test.tsx
```

```bash
git add src/testflying_api/build_notifications.py src/testflying_api/build_platform.py src/testflying_api/admin_api/routes.py src/testflying_api/admin_api/schemas.py tests/test_build_notifications.py tests/test_build_runner_api.py tests/test_admin_api_pages.py admin-web/src/app/apiClient.ts admin-web/src/pages/NotificationsPage.tsx admin-web/src/pages/OrdinaryPages.test.tsx admin-web/src/styles/admin.css
git commit -m "feat(admin): add dingtalk build notification guide"
```

### Task 4: Serve Authenticated Runner Release Manifests and Bundles

**Files:**
- Create: `src/testflying_api/runner_releases.py`
- Create: `tests/test_runner_releases.py`
- Modify: `src/testflying_api/build_platform.py`
- Modify: `src/testflying_api/admin_api/routes.py`
- Modify: `src/testflying_api/admin_api/schemas.py`
- Modify: `tests/test_build_runner_api.py`

**Interfaces:**
- Produces: `RunnerReleaseManifest.load(root, platform, arch) -> RunnerReleaseManifest`.
- Produces: authenticated `POST /admin/api/build-runners/{runner_id}/updates/check`.
- Produces: authenticated `GET /admin/api/build-runners/{runner_id}/updates/{platform}/{arch}/{version}/bundle`.
- Consumed by: Task 5 Go client.

- [ ] **Step 1: Write failing manifest containment and endpoint tests**

Use a temporary release tree `darwin/arm64/release.json` and bundle. Cover valid manifest, missing manifest, malformed SHA, platform/arch allowlist, bundle path traversal, unknown runner, wrong token, no-update response, update response, and authenticated download bytes.

Manifest fixture:

```json
{
  "version": "0.2.0",
  "runnerVersion": "0.2.0",
  "packageAgentVersion": "0.2.0",
  "platform": "darwin",
  "arch": "arm64",
  "bundleFile": "testflying-runner-0.2.0-darwin-arm64.zip",
  "sha256": "<64 lowercase hex characters>"
}
```

- [ ] **Step 2: Run tests and confirm release APIs are absent**

Run: `PYTHONPATH=src pytest tests/test_runner_releases.py -q`

Expected: FAIL importing `runner_releases`.

- [ ] **Step 3: Implement manifest parsing and containment**

`RunnerReleaseManifest` is a frozen dataclass. Accept only platform `darwin`, arch `arm64`/`amd64`, semantic versions matching `^[0-9]+\.[0-9]+\.[0-9]+$`, lowercase 64-character SHA-256, and a basename-only `.zip` bundle filename. Resolve both manifest and bundle and require `candidate.relative_to(release_root.resolve())`.

Expose a public `authenticate_runner(...) -> BuildRunner` wrapper in `build_platform.py`; do not import private `_runner_or_401` from routes.

- [ ] **Step 4: Implement request/response schemas and routes**

```python
class RunnerUpdateCheckRequest(AdminApiModel):
    platform: str
    arch: str
    runner_version: str
    package_agent_version: str

class RunnerUpdateCheckResponse(AdminApiModel):
    update_available: bool
    version: str = ""
    runner_version: str = ""
    package_agent_version: str = ""
    bundle_url: str = ""
    sha256: str = ""
```

Authenticate path `runner_id` with the Bearer token and static token pepper. Return `updateAvailable=false` when both current versions equal the manifest. Use `FileResponse` for downloads and verify requested version equals the active manifest version.

- [ ] **Step 5: Run Task 4 tests and commit**

Run: `PYTHONPATH=src pytest tests/test_runner_releases.py tests/test_build_runner_api.py -q`

```bash
git add src/testflying_api/runner_releases.py src/testflying_api/build_platform.py src/testflying_api/admin_api/routes.py src/testflying_api/admin_api/schemas.py tests/test_runner_releases.py tests/test_build_runner_api.py
git commit -m "feat(build-runner): expose authenticated releases"
```

### Task 5: Add Safe Automatic Updates and a Long-Running Runner Loop

**Files:**
- Create: `build-runner/internal/runner/update.go`
- Create: `build-runner/internal/runner/update_test.go`
- Modify: `build-runner/internal/runner/config.go`
- Modify: `build-runner/internal/runner/client.go`
- Modify: `build-runner/internal/runner/loop.go`
- Modify: `build-runner/internal/runner/runner_test.go`
- Modify: `build-runner/cmd/testflying-build-runner/main.go`

**Interfaces:**
- Consumes: Task 4 check/download API.
- Produces: `Client.CheckUpdate(ctx, runnerID, UpdateCheckRequest) (*UpdateManifest, error)`.
- Produces: `InstallUpdate(ctx, client, cfg, manifest) error`.
- Produces: sentinel `ErrUpdateInstalled`.

- [ ] **Step 1: Write failing Go updater tests**

Cover: no update, authenticated check JSON, checksum mismatch, extra archive entry, absolute/`..` path, symlink, missing binary, successful replacement, second-file replacement failure with rollback, and semantic version comparison. Use temporary executable files and `httptest.Server`; do not replace the test process binary.

- [ ] **Step 2: Run tests and confirm updater symbols are missing**

Run: `cd build-runner && CGO_ENABLED=0 go test ./internal/runner -run 'Test(Update|Install|Version)' -count=1`

Expected: compile failure for missing updater APIs.

- [ ] **Step 3: Extend Runner configuration**

```go
type Config struct {
    // existing fields remain
    PollInterval   time.Duration
    UpdateInterval time.Duration
    InstallDir     string
    Platform       string
    Arch           string
}
```

Defaults in `main.go`: poll `5s`, update `30m`, install dir `filepath.Dir(os.Executable())`, platform `runtime.GOOS`, arch `runtime.GOARCH`. Allow positive duration overrides through `TESTFLYING_BUILD_RUNNER_POLL_INTERVAL` and `TESTFLYING_BUILD_RUNNER_UPDATE_INTERVAL`; validation rejects unsupported platform/arch and nonpositive intervals.

- [ ] **Step 4: Implement check, safe extraction, and atomic replacement**

The bundle may contain exactly `testflying-build-runner` and `package-agent`, both regular files with executable mode. Stream download through `io.TeeReader` into a temp file and SHA-256 hasher; compare with `subtle.ConstantTimeCompare`. Extract to a temp directory under `InstallDir`.

Before downloading, require `isNewerVersion(manifest.RunnerVersion, cfg.Version)` or
`isNewerVersion(manifest.PackageAgentVersion, cfg.PackageAgentVersion)`. Valid semantic versions
must compare numerically by major/minor/patch; `dev` may upgrade to any valid release, while equal
or older manifests are ignored so the server cannot accidentally downgrade a node.

Replacement order must preserve backups for both files. If any rename or chmod fails, remove new files and restore both backups before returning the error. On success remove backups and return `ErrUpdateInstalled`.

- [ ] **Step 5: Convert `Run` into a controlled loop while preserving `RunOnce`**

```go
func Run(ctx context.Context, cfg Config) error {
    if err := cfg.Validate(); err != nil { return err }
    client := NewClient(cfg.ServerURL, cfg.Token, http.DefaultClient)
    nextUpdate := time.Time{}
    ticker := time.NewTicker(cfg.PollInterval)
    defer ticker.Stop()
    for {
        if time.Now().After(nextUpdate) {
            manifest, err := client.CheckUpdate(ctx, cfg.RunnerID, updateRequestFromConfig(cfg))
            if err != nil { log.Printf("update check failed: %s", RedactText(err.Error())) }
            if manifest != nil {
                if err := InstallUpdate(ctx, client, cfg, *manifest); err != nil {
                    if errors.Is(err, ErrUpdateInstalled) { return err }
                    log.Printf("update install failed: %s", RedactText(err.Error()))
                }
            }
            nextUpdate = time.Now().Add(cfg.UpdateInterval)
        }
        if err := RunOnce(ctx, cfg, client.httpClient); err != nil {
            log.Printf("runner poll failed: %s", RedactText(err.Error()))
        }
        select {
        case <-ctx.Done(): return nil
        case <-ticker.C:
        }
    }
}
```

In `main`, treat `ErrUpdateInstalled` as a normal zero exit so LaunchAgent `KeepAlive` restarts the new binary. Other errors remain fatal.

- [ ] **Step 6: Run Go tests and commit**

Run: `cd build-runner && CGO_ENABLED=0 go test ./... -count=1`

```bash
git add build-runner/internal/runner/update.go build-runner/internal/runner/update_test.go build-runner/internal/runner/config.go build-runner/internal/runner/client.go build-runner/internal/runner/loop.go build-runner/internal/runner/runner_test.go build-runner/cmd/testflying-build-runner/main.go
git commit -m "feat(build-runner): add atomic automatic updates"
```

### Task 6: Correct Git Policy and Produce a Standalone Package Agent

**Files:**
- Modify: `package-agent/src/package_agent/policy.py`
- Modify: `package-agent/src/package_agent/cli.py`
- Modify: `package-agent/tests/test_policy.py`
- Modify: `package-agent/tests/test_cli.py`
- Create: `package-agent/src/package_agent/__main__.py`

**Interfaces:**
- Produces: only `git commit` and `git push` blocked by `_contains_blocked_git_operation`.
- Produces: `package-agent build --input ... --output ... --config /absolute/config.json`.
- Consumed by: Tasks 7-8 and PyInstaller.

- [ ] **Step 1: Replace the incorrect policy expectations with failing boundary tests**

```python
@pytest.mark.parametrize("command", [
    ["git", "pull", "--ff-only"],
    ["git", "fetch", "origin"],
    ["git", "tag", "local-checkpoint"],
    ["git", "reset", "--hard", "HEAD"],
])
def test_policy_allows_permitted_git_operations(command):
    assert evaluate_action(Action(kind="inspect", command=command)).allowed is True

@pytest.mark.parametrize("command", [
    ["git", "commit", "-m", "blocked"],
    ["git", "push", "origin", "main"],
    ["bash", "-lc", "git commit -m blocked"],
    ["zsh", "-lc", "git push origin main"],
])
def test_policy_blocks_only_publishing_git_operations(command):
    assert evaluate_action(Action(kind="inspect", command=command)).reason == "blocked_git_operation"
```

- [ ] **Step 2: Run tests and confirm pull/tag still fail**

Run: `cd package-agent && python3.11 -m pytest tests/test_policy.py -q`

Expected: pull and tag cases fail because they are currently blocked.

- [ ] **Step 3: Narrow the blocked Git set**

Set `BLOCKED_GIT_COMMANDS = {"commit", "push"}`. Keep protected source/build file checks unchanged. Add a comment that checkout cleanup is permitted only inside the Runner-managed checkout; the Runner remains responsible for workspace containment.

- [ ] **Step 4: Add an external acceptance config option**

Add optional `--config`; pass `Path(args.config).resolve()` into `_build_report`. If provided, require a readable regular JSON file and use it instead of `<projectDir>/testflying-package-agent.json`. This does not relax command policy.

```python
build_parser.add_argument("--config")
report = _build_report(
    Path(args.input),
    output_dir=output_dir,
    config_override=Path(args.config).resolve() if args.config else None,
)
```

Add `__main__.py`:

```python
from package_agent.cli import main

raise SystemExit(main())
```

Test missing override, invalid JSON, policy-blocked override, and successful override without creating a config in the project.

- [ ] **Step 5: Run package-agent tests and a PyInstaller smoke build**

Run:

```bash
cd package-agent
python3.11 -m pytest -q
python3.11 -m PyInstaller --clean --onefile --name package-agent --paths src src/package_agent/__main__.py
./dist/package-agent --help
```

Expected: all tests pass and the standalone binary prints the CLI help without Python import errors.

- [ ] **Step 6: Commit Task 6**

```bash
git add package-agent/src/package_agent/policy.py package-agent/src/package_agent/cli.py package-agent/src/package_agent/__main__.py package-agent/tests/test_policy.py package-agent/tests/test_cli.py
git commit -m "fix(package-agent): enforce exact git boundary"
```

### Task 7: Build the Complete Per-Node macOS Package and Update Bundle

**Files:**
- Modify: `scripts/build_runner_installer.sh`
- Modify: `build-runner/packaging/install.command`
- Create: `build-runner/packaging/postinstall`
- Create: `tests/test_build_runner_installer.py`
- Modify: `.gitignore`
- Modify: `README.md`

**Interfaces:**
- Consumes: Task 5 Go Runner and Task 6 PyInstaller entrypoint.
- Produces: `TestFlyingBuildRunner-<version>-darwin-<arch>.pkg`.
- Produces: `darwin/<arch>/release.json` plus the two-binary update ZIP and `.sha256`.

- [ ] **Step 1: Write failing installer contract tests**

Run the script with a temporary config and fake tool executables injected through `PATH`. Assert it rejects missing/blank `runnerId`, `token`, `serverUrl`, `rootDir`, and labels/platforms; invokes Go, PyInstaller, `ditto`, and `pkgbuild`; stages both binaries; rewrites `packageAgentBin` to the installed binary; and writes a manifest whose checksum matches the ZIP.

- [ ] **Step 2: Run the focused test and confirm current installer is incomplete**

Run: `PYTHONPATH=src pytest tests/test_build_runner_installer.py -q`

Expected: FAIL because no package-agent, release manifest, checksum, or `.pkg` exists.

- [ ] **Step 3: Implement strict arguments and two-binary build**

Use CLI `build_runner_installer.sh CONFIG_SOURCE OUTPUT_DIR VERSION`. Validate semantic version and JSON before creating the final output directory. Build to a temporary staging root and move into place only after every command succeeds.

Commands must be equivalent to:

```bash
go build -trimpath -ldflags "-s -w" -o "$BIN_DIR/testflying-build-runner" ./cmd/testflying-build-runner
python3.11 -m PyInstaller --clean --onefile --name package-agent \
  --distpath "$BIN_DIR" --workpath "$TMP_ROOT/pyinstaller-work" \
  --specpath "$TMP_ROOT" --paths "$REPO_ROOT/package-agent/src" \
  "$REPO_ROOT/package-agent/src/package_agent/__main__.py"
```

The config validator must parse JSON structurally, require capacity `1`, and set `packageAgentBin` to `/Library/Application Support/TestFlying/build-runner/package-agent` before writing the package payload.

- [ ] **Step 4: Build update ZIP, checksum, and release manifest**

Use `ditto -c -k --keepParent` only on a directory containing the two expected executable basenames. Compute `shasum -a 256`. Write `release.json` atomically with version, runner/package-agent versions, platform `darwin`, detected `uname -m` normalized to `arm64` or `amd64`, bundle basename, and lowercase checksum.

- [ ] **Step 5: Build the `.pkg` and console-user LaunchAgent postinstall**

Use `pkgbuild --root PAYLOAD --scripts SCRIPTS --identifier com.testflying.build-runner --version VERSION --install-location / OUTPUT.pkg`.

`postinstall` must find the console user with `stat -f '%Su' /dev/console`, reject `root`/empty, resolve UID/home with `id` and `dscl`, set config ownership and `0600`, write `~/Library/LaunchAgents/com.testflying.build-runner.plist`, and call:

```bash
launchctl bootout "gui/$CONSOLE_UID" "$PLIST" >/dev/null 2>&1 || true
launchctl bootstrap "gui/$CONSOLE_UID" "$PLIST"
launchctl kickstart -k "gui/$CONSOLE_UID/com.testflying.build-runner"
```

The wrapper exports config fields and executes the installed Runner as the console user. Include `ThrottleInterval=10`, `RunAtLoad=true`, and `KeepAlive=true`. Update `install.command` as a supported unpacked fallback and require the bundled `package-agent`.

- [ ] **Step 6: Run tests and build a real unsigned package**

Run:

```bash
PYTHONPATH=src pytest tests/test_build_runner_installer.py -q
bash -n scripts/build_runner_installer.sh
bash -n build-runner/packaging/install.command
bash -n build-runner/packaging/postinstall
scripts/build_runner_installer.sh /absolute/path/to/validated-runner-config.json outputs/build-runner-installer 0.2.0
find outputs/build-runner-installer -name 'TestFlyingBuildRunner-0.2.0-darwin-*.pkg' -exec pkgutil --payload-files {} \;
```

Expected: package and release bundle exist; `pkgutil` may report unsigned but must parse the package successfully. Do not install the package over an active user Runner during automated tests.

- [ ] **Step 7: Commit Task 7**

```bash
git add scripts/build_runner_installer.sh build-runner/packaging/install.command build-runner/packaging/postinstall tests/test_build_runner_installer.py .gitignore README.md
git commit -m "feat(build-runner): package complete macos installer"
```

### Task 8: Add Immutable Real-Build Acceptance and Run the Full Closure

**Files:**
- Create: `scripts/verify_real_build.sh`
- Create: `tests/test_real_build_acceptance.py`
- Create outside the project under ignored output: `outputs/real-build-acceptance/*.json`
- Modify: `README.md`

**Interfaces:**
- Consumes: standalone `package-agent` from Task 7.
- Produces: machine-readable acceptance JSON containing before/after Git state, Agent report, and artifact inventory.

- [ ] **Step 1: Write failing acceptance harness tests**

Use a temporary Git repository and fake package-agent. Cover clean success, pre-existing dirty state preservation, newly modified tracked file failure, newly created untracked source/config file failure, allowed generated `build/` output, missing report, non-success report, and missing package/symbol/log paths.

- [ ] **Step 2: Run tests and confirm harness is absent**

Run: `PYTHONPATH=src pytest tests/test_real_build_acceptance.py -q`

Expected: FAIL because `scripts/verify_real_build.sh` does not exist.

- [ ] **Step 3: Implement structural Git and artifact checks**

Script arguments:

```text
verify_real_build.sh PROJECT_DIR PACKAGE_AGENT_BIN CONFIG_JSON PLATFORM ENVIRONMENT ARTIFACT_TYPE OUTPUT_DIR
```

Use `git status --porcelain=v1 -z --untracked-files=all` before and after and compare NUL-delimited records structurally. Existing dirty entries are allowed only if unchanged. New records outside `.dart_tool/`, `build/`, `.gradle/`, `Pods/`, `DerivedData/`, and Agent output cause failure. Never run cleanup against the user project.

Generate `build-input.json`, invoke:

```bash
"$PACKAGE_AGENT_BIN" build --input "$INPUT" --output "$AGENT_OUTPUT" --config "$CONFIG_JSON"
```

Parse `report.json` with Python, require status `success`, at least one existing path in `packagePaths`, `symbolsPaths`, and `logPaths`, plus the report itself. Write acceptance JSON even on failure and preserve all logs.

- [ ] **Step 4: Create ignored deterministic configs and run real Android builds**

Use Flutter at `/Users/admin/ai_project/apps/testflying/.fvm/flutter_sdk/bin/flutter`. Generate configs under `outputs/real-build-acceptance/config/`, not in the Flutter project.

APK command must run `flutter pub get`, `flutter build apk --release --split-debug-info=build/testflying-symbols/apk`, then archive the symbol directory under `build/`. AAB command uses `flutter build appbundle --release --split-debug-info=build/testflying-symbols/aab`. Artifact globs must select the APK/AAB, symbol ZIP, and command log.

Run the harness once for `apk` and once for `aab`. Record exact produced files and verify the Flutter project Git status has no new protected changes.

- [ ] **Step 5: Run real iOS build and preserve boundary failure evidence**

Use a config that runs `flutter pub get` and `flutter build ipa --release`, then archives `build/ios/archive/*.xcarchive` and contained `*.dSYM` under `build/`. Artifact globs select IPA, xcarchive ZIP, dSYM ZIP, and log.

If existing signing succeeds, require all artifacts. If Xcode reports missing signing/certificate/provisioning or any project/source edit is required, do not edit the Flutter project. Preserve report/log, create a TestFlying `needs_human` build through the local API, dispatch against the configured DingTalk environment, and record that external signing configuration blocks successful iOS acceptance.

- [ ] **Step 6: Run complete automated verification**

Run:

```bash
PYTHONPATH=src pytest -q
cd admin-web && npm run lint && npm test -- --run && npm run build
cd ../package-agent && python3.11 -m pytest -q
cd ../build-runner && CGO_ENABLED=0 go test ./... -count=1
cd ..
bash -n scripts/build_runner_installer.sh
bash -n build-runner/packaging/install.command
bash -n build-runner/packaging/postinstall
git diff --check
TMP_DB="$(mktemp -t testflying-final.XXXXXX.db)"
TESTFLYING_DATABASE_URL="sqlite:///$TMP_DB" PYTHONPATH=src alembic upgrade head
TESTFLYING_DATABASE_URL="sqlite:///$TMP_DB" PYTHONPATH=src alembic downgrade base
rm -f "$TMP_DB"
```

Expected: all test/build commands exit 0, migration reaches `20260710_0011` and returns to base, and no unexpected Git changes appear.

- [ ] **Step 7: Verify a real DingTalk delivery when credentials are present**

Check presence without printing values:

```bash
test -n "${TESTFLYING_DINGTALK_WEBHOOK_URL:-}"
test -n "${TESTFLYING_DINGTALK_SECRET:-}"
```

When present, enqueue one labeled acceptance `needs_human` build, run one dispatcher pass, and verify the delivery row becomes `delivered`. When absent, report this acceptance item as externally blocked; do not invent credentials and do not mark real DingTalk delivery complete.

- [ ] **Step 8: Commit Task 8 without pushing**

```bash
git add scripts/verify_real_build.sh tests/test_real_build_acceptance.py README.md
git commit -m "test(build-platform): add real build acceptance"
git status --short
```

Expected: only ignored acceptance/package outputs remain outside Git, and no push occurs.
