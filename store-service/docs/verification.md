# Verification and lifecycle record

The standalone service is maintained on `refactor/store-service-rust`. Existing Testflying Python/React/Go code was not changed in this extraction; earlier application-identity API edits remain separate in the working tree. The initial extraction checks were local; subsequent production releases are recorded below.

## Final checks

The current managed run and exact source hashes are recorded in [verification.json](verification.json). Earlier extraction and current isolation runs are listed below.

- `cargo fmt --all -- --check`: passed.
- Initial extraction: `cargo test --locked --workspace` passed 30 tests. The current upload-timeout revision passes **42 tests**, described below.
- `cargo clippy --locked --workspace --all-targets -- -D warnings`: passed.
- `cargo build --locked --release --package store-api`: passed independently on the local macOS host.
- `cargo build --locked --release --package store-connector`: passed independently on the local macOS host.
- Source hashes matched both before and after the checks and were independently compared with the current source.
- `git diff --check`: passed.
- OpenAPI, Compose and CI YAML parsed successfully with Ruby's YAML parser.
- Provider task review and final whole-service review passed after scoped corrections.

[verification.json](verification.json) contains the executed commands, test-result lines, exact source hashes, timestamp and managed run ID. [provider-capabilities.md](provider-capabilities.md) maps capabilities to real local HTTP fixtures and states remaining coverage limits.

The original source-verification runs did not exercise live Apple/Google accounts, Docker image builds, non-macOS builds or GitHub CI. Docker daemon was unavailable locally. Subsequent native releases and production deployment are recorded below; no vendor acceptance of all content/device dimensions is claimed.

## Managed lifecycle

Installed integration self-check returned `ACTIVE`; the design passed the installed artifact-design-gate. Cargo download/cache, target, fixtures, and captured logs were created under each supervisor-owned run root. Successful temporary outputs were deleted, retaining the declared Cargo lockfile and final verification report as formal deliverables.

| Run ID | Outcome | Cleanup | Retained evidence |
|---|---|---|---|
| 45d5cdaa7f79434da8890c8c25af165a | Initial compile caught WSS borrow error; fixed | Entire owned run deleted | Compact failed logs, 7 days |
| e96fbc07b3244a86bc4b2742c61f8f6f | Compile caught SQLite count conversion; fixed | Entire owned run deleted | Compact failed logs, 7 days |
| 64a1dd1dd5504f62bce1a920653f0b5d | Initial complete fmt/test/clippy pass, before final review fixes | Entire owned run deleted | No temporary evidence retained |
| c54cbe760b0a40c68117299df59fe1fc | 30 tests passed; one Clippy boolean simplification; fixed | Entire owned run deleted | Compact failed logs, 7 days |
| 0f881c4bc0b04c63b75166544f4912c4 | Final fmt/test/clippy pass against stable source | Entire owned run deleted | Formal verification.json only |
| 469624179eb44b288e61306f96ae3260 | Verification moved inside standalone directory; 30 tests, fmt and Clippy passed | Entire owned run deleted | Formal verification.json only |
| ce8ee7f2edca4b81868fc3130eab0262 | Final standalone checks plus separate native API and Connector release builds passed | Entire owned run deleted | Formal verification.json only |

Supervisor cleanup reports are in `~/Library/Caches/CodexArtifactLifecycle/reports/{runId}/cleanup-report.json`. Compact failed evidence is in `~/Library/Caches/CodexArtifactLifecycle/evidence/testflying-4705feaf0d9a/{runId}/`; each `.metadata.json` records its exact creation/expiration timestamp. These failures expire on 2026-11-06 UTC. No full failure contexts were retained.

The cleanup tool did not record released byte totals, so no released-byte number is claimed. There are no remaining processes, loopback servers, downloads, caches, mounts or external resources created by the completed runs listed above. No new quarantine findings were created.

## Code and packaging separation

- New API: `store-service/crates/store-api`, image target `store-api`, only executable `store-api`.
- New account Connector: `store-service/crates/store-connector`, image target `store-connector`, only executable `store-connector`.
- New provider/protocol modules, verification script and artifact declarations are all under `store-service/`. No legacy Python/Go source is needed by the build.
- Legacy Go Connector stays in `server/connector`; the old deployment configuration is untouched.
- The new Compose file selects only the API image; the new CI validates both image targets. The Docker configuration was checked statically; no local Docker image build is claimed.

## Pre-existing audit findings

Final project audit returned these five existing states, which were also present before the extraction:

| Run ID | State |
|---|---|
| 1db3215ed081473b86a856c423cd083f | retaining_evidence |
| 40e1db9ff2e54677b5ee4bff65412593 | retaining_evidence |
| 54f2240fc93741beaf04b5844736f2f6 | retaining_evidence |
| 8bb74c58fb7f431299fce19753d8f672 | cleaning / quarantined |
| 9bff00c45b9440e9b18bde4cec846364 | retaining_evidence |

The old `8bb...` backend test run remains quarantined because a fixture contains a protected `.git` marker. Its previously reported approximately 3.36 MiB was not manually removed or reclassified. Those historical findings are not an assertion that all workspace artifacts have been cleaned.

## Production deployment and Windows release, 2026-11-04

The standalone API was deployed to `https://store-api.example.com/store-api`, with a systemd service listening only on `127.0.0.1:8200`. The existing TLS proxy gained one prefix; removing that prefix block reproduces the backed-up original configuration byte-for-byte. Existing Python/Go/PostgreSQL/MinIO containers retained their original start times and the old API health check passed.

[deployment-build.json](deployment-build.json) records Linux static API/Connector and Apple Silicon Connector hashes. [deployment-verification.json](deployment-verification.json) records 17 successful external checks, including HTTPS certificate verification, administrator/client credential separation, node creation, installer identity, real local Connector WSS connection through the proxy, app routing, and a read reaching the node's missing-credentials response without vendor traffic. The service is enabled at boot; secrets are root-only, registry files are private to the dedicated service user. The verification node was disabled and its app mapping removed.

Windows uses the same Connector source with an `x86_64-pc-windows-gnu` release. [windows-release-build.json](windows-release-build.json) records the PE x64 format, SHA-256 and imported system DLLs. The initial DLL allowlist missed `BCryptPrimitives.dll`; this was corrected after checking [Microsoft's ProcessPrng documentation](https://learn.microsoft.com/en-us/windows/win32/seccng/processprng). No application source changed. [windows-release-verification.json](windows-release-verification.json) records the subsequent production download check and user delivery identity. Windows execution on a real machine has not been tested.

Node IDs and Tokens are runtime JSON configuration. All node-specific ZIPs reuse the identical published executable; adding an account/node does not invoke a compiler. The separately delivered private Windows ZIP includes an enabled node waiting for account credentials/app mapping, the public CA, and a launcher that selects that CA. It is Git-ignored and mode 0600; only the secret-free delivery metadata enters Git.

| Run ID | Outcome | Cleanup | Retained formal files / evidence |
|---|---|---|---|
| 1b1f45b39ba8401ea55d9acd42080d99 | Linux amd64 API/Connector and macOS arm64 Connector releases passed | Entire owned run deleted | Release binaries and deployment-build.json, no expiration as formal deliveries |
| 7234ada4d1f742178e949e42703725da | Production installation and 17 HTTPS/WSS/download/routing checks passed | Entire owned run deleted; local Connector stopped | Production service/config/registry/binaries/proxy backup; public CA and deployment-verification.json, no expiration as formal deliveries |
| 35d260964351481ab25d62aa55ea693c | Windows compilation passed; DLL classification rejected a legitimate system DLL | Entire owned run deleted | Sanitized compact logs expire 2026-11-11T03:17:36Z; no full context |
| e880bd6e7cca4e9abab84fd9c6f172ef | Windows release and corrected system-DLL check passed | Entire owned run deleted | Windows EXE and windows-release-build.json, no expiration as formal deliveries |
| 277befd903d94357b41b07d7a2cf952d | Production Windows ZIP download, binary hash, node identity and launcher passed; private user ZIP produced | Entire owned run deleted; API PID unchanged | Production EXE/alias, private user ZIP, enabled delivery node and windows-release-verification.json, no expiration as formal deliveries; verification node disabled |

Cleanup reports remain at `~/Library/Caches/CodexArtifactLifecycle/reports/{runId}/cleanup-report.json`. The failed Windows run's compact evidence is `~/Library/Caches/CodexArtifactLifecycle/evidence/store-service-a59b33aca240/35d260964351481ab25d62aa55ea693c/`; its metadata owns the exact expiration timestamp. No released-byte total was recorded by the supervisor. No new quarantine findings were introduced; the five historical audit states above were unchanged. Source and production formal files are outside cleanup ownership.

## Encrypted transport and central uploads, 2026-11-04

The final source run `e3ff612030b64152806514d99fb410dd` passed all **40 tests**, formatting, all-target Clippy with warnings denied, and independent native release builds of the API and Connector. Source hashes were stable before and after verification. Python script syntax, deployment shell syntax, OpenAPI parsing and all local schema references also passed; the subsequently added external upload example and rollout script passed a separate AST syntax check.

Coverage includes encrypted handshake authentication, wrong keys, tamper/replay/reordering rejection, fragmentation, upload credential expiry and node binding, HTTP and WSS transfers of a valid 3 MiB PNG through the center, simultaneous node HTTP/WSS operation, interruption/hash failure cleanup, duplicate-session ownership, per-app isolation, quotas/expiration, preflight preservation, sync consumption, and a Google multipart fixture receiving the uploaded local bytes. Production vendor accounts and execution on real Windows hardware remain unverified.

| Run ID | Outcome | Cleanup | Retained evidence |
|---|---|---|---|
| bcb98d6fc0be4d32b846f32501411d6a | Initial check found a moved executor; corrected | Entire owned run deleted | Compact secret-free logs expire 2026-11-11T04:17:09Z |
| b78d59c5d21841da93917d81d7e5d49c | 38 tests passed; Clippy found a needless borrow; corrected | Entire owned run deleted | Compact secret-free logs expire 2026-11-11T04:23:02Z |
| e3ff612030b64152806514d99fb410dd | Final 40 tests, fmt, Clippy and both native release builds passed | Entire owned run deleted | Formal verification.json only, no expiration |
| d9dd166a5cd94f29a662f39769b091d9 | Linux static API/Connector and macOS arm64 Connector release builds passed | Entire owned run deleted; no remaining processes or external resources | Formal binaries and deployment-build.json only, no expiration |
| ce4f76f5d1bc40e582d761896dbab035 | Windows x64 release, executable format and system-DLL import checks passed | Entire owned run deleted; no remaining processes or external resources | Formal EXE and windows-release-build.json only, no expiration |
| 055da85e90a442ef996807f500b8d8a8 | Production upgrade and 23 HTTPS, encrypted-WSS, upload, sync, package and cleanup checks passed | Entire owned run deleted; local Connector stopped; verification mapping removed and node disabled | Production release/config/registry, private v2 Windows ZIP and upload-deployment-verification.json, no expiration |

The failed compact logs are under `~/Library/Caches/CodexArtifactLifecycle/evidence/store-service-a59b33aca240/{runId}/`; exact expiration is owned by each metadata file. No released-byte total was recorded. Local test processes, listeners, anonymous upload files and caches were released. No new quarantine was introduced; historical audit findings above remain separate.

[image-upload-api.md](image-upload-api.md) documents the external all-central upload flow. Node IDs/Tokens and app mappings are retained, but v2 requires updated Connector executables and runtime configuration with separate message/upload keys. HTTP endpoint registration accompanies authenticated WSS registration; no creation-time mode is needed.

Production deployment commit is `fb9353c3573ba3a931c491c188f18932597b3eba`, with binaries built from `99dea6110882c83124dc03b5c87ba220a06cb012`; the commits differ only in build/verification records. [upload-deployment-verification.json](upload-deployment-verification.json) records the actual 3,147,060-byte PNG and SHA-256, transfer through the public HTTPS center to an encrypted WSS Connector, mock sync consumption, subsequent 404, installer checks and original node identity preservation. No live store vendor was contacted. The proxy now accepts 64 MiB streaming requests without request buffering; business JSON remains limited to 2 MiB.

The new private `releases/windows-amd64/connector-windows-v2.zip` preserves the existing delivery node ID/Token and provider settings, contains the new protocol keys and public CA, is mode 0600 and Git-ignored; the old ZIP remains untouched. The final artifact audit still reports only the same five historical states listed above, including the protected `.git` fixture quarantine. No new quarantines or verification resources remain. Supervisor cleanup reports do not contain released-byte totals. This documentation-only update does not change the deployed executable or tested Rust sources.

## Thirty-minute upload windows, 2026-11-04

Upload credentials now allow 1800 seconds from issuance; the central transfer deadline and the node's incomplete-upload deadline independently allow 1800 seconds from transfer start. JWT expiry is checked when authenticating a new upload, while an authenticated session uses its own transfer deadline for subsequent chunks and finalization. Completed images still expire 900 seconds after completion. Nginx and the external upload example allow 1860 seconds, leaving response headroom. Node identity, keys and configuration format are unchanged, but the Connector executable must be updated to accept the longer credentials.

The final source run passed 42 workspace tests, formatting, all-target Clippy with warnings denied, and separate native release builds of both executables. Source hashes remained stable. Added coverage proves that credentials issued over ten minutes ago remain usable, credentials exceeding thirty minutes are rejected, an incomplete upload survives the old fifteen-minute boundary and expires after thirty, and an authenticated transfer can complete after its ticket expires while retaining the completed-image fifteen-minute cleanup. API integration fixtures verify `exp - iat == 1800`. Python syntax, deployment shell syntax and OpenAPI YAML parsing also passed.

| Run ID | Outcome | Cleanup | Retention |
|---|---|---|---|
| b6f3cfb3424c4e93b8b60d59566f6dad | New test used an empty chunk and hit input validation before the missing-asset check; fixture corrected | Entire owned run deleted; no remaining processes or resources | Compact sanitized logs expire 2026-11-11T07:39:30Z |
| b127afe60b0542b19d79dafed0989ba7 | All 42 tests, fmt, Clippy and both native release builds passed | Entire owned run deleted; no remaining processes or resources | Formal verification.json only, no expiration |
| f41fc70ecda44578afb366faeac704c0 | Official Rust toolchain download failed with TLS EOF | Entire owned run deleted; no remaining processes or resources | Compact sanitized logs expire 2026-11-11T07:45:36Z |
| 2af2e0a5996d4eb28e893757f9e5acf7 | Retried release download; default backend again failed with TLS EOF | Entire owned run deleted; no remaining processes or resources | Compact sanitized logs expire 2026-11-11T07:47:14Z |
| a633ed6dd6bb42c597998316f066e6ce | Three bounded in-run toolchain retries exhausted after TLS EOF | Entire owned run deleted; no remaining processes or resources | Compact sanitized logs expire 2026-11-11T07:49:09Z |
| 10460f29b4764523ad97a4a48b755ed8 | Linux static API/Connector and macOS arm64 Connector releases passed using the temporary official curl download backend | Entire owned run deleted; no remaining processes or resources | Formal binaries and deployment-build.json, no expiration |
| e5593a295538421d8dbf88aaab808af4 | Windows x64 release, architecture and system-DLL checks passed with the temporary official curl download backend | Entire owned run deleted; no remaining processes or resources | Formal EXE and windows-release-build.json, no expiration |
| fac930a7fe22479684896ddffe5c7cd8 | Candidate started and installers/WSS passed; HTTPS handshake failed during verification mapping, followed by SSH interruption during rollback | Owned local run and Connector deleted; verification node disabled; remote recovery verified separately below | Compact sanitized logs, 7 days; candidate release retained for review |
| 74cb8f2108244f41b100d45aeb4d0c5f | Previous proxy and service restored; status query used a Python keyword unsupported by the server's Python 3.6; final SSH retry interrupted | Entire owned local run deleted; no remaining local processes or resources | Compact sanitized logs, 7 days |
| 4eb2093a20d440c68f446ee195086428 | Independent read-only assertions confirmed the old current symlink, running binary, original proxy bytes, API health and legacy API health | Entire owned run deleted; no remaining processes or resources | No temporary evidence retained |
| 34e834df5f0d4977993c9f4ff6799431 | Final production deployment and all 26 HTTPS, installer, encrypted-WSS, thirty-minute credential, image transfer, sync and cleanup checks passed | Entire owned run deleted; Connector and foreground SSH master stopped; mapping removed and verification node disabled | Production release, v3 private Windows package and upload-deployment-verification.json, no expiration |

Failed logs are under `~/Library/Caches/CodexArtifactLifecycle/evidence/store-service-a59b33aca240/{runId}/`; metadata owns exact expiration. Cleanup reports remain under the per-run reports directory described above; released-byte totals were not recorded. No new quarantine was introduced. Regression tests simulate elapsed deadlines without waiting thirty real minutes; no thirty-minute slow-network test is claimed.

The release helper now retries only toolchain installation downloads, at most three times inside the same owned cache. Compilation and business operations are never retried. The default backend suffered repeated TLS EOF; the successful release run set `RUSTUP_USE_CURL=1` only for that supervised process, retaining official HTTPS URLs, certificate verification and Rustup component checksums, without changing system network settings or global caches.

The next deployment uses an installed Python 3.14 interpreter, reuses its authenticated HTTPS connection, and uses a foreground supervisor-owned SSH ControlMaster with its socket inside the exact run tmp directory. No global SSH setting or persistent background tunnel is created; the wrapper terminates and waits for the master in finally. TLS/hostname validation remains enabled, and business requests are not automatically retried. Rust sources and verified release binaries are unchanged by these deployment-only transport adjustments.

Final deployment succeeded at commit `9fd000757e22a8b0fdc18b6a6ba068a5551be521`, using binaries built from `703305180e49cf330d13d3ae507581a2d2226c48`. [upload-deployment-verification.json](upload-deployment-verification.json) records 26 passed checks, actual signed credential lifetime 1800 seconds, transfer deadline 1800 seconds, proxy read timeout 1860 seconds, and a 3,147,060-byte PNG uploaded through the central HTTPS endpoint and encrypted WSS to a mock-only Connector. Its SHA-256 matched, sync consumed it, and reuse returned 404; no live store vendor was contacted.

The new private `releases/windows-amd64/connector-windows-v3.zip` has the existing delivery identity and keys, a verified common executable and public CA. Its mode is 0600 and it is Git-ignored; v1/v2 private ZIPs remain intact. For an existing v2 node, replace the executable and keep its current connector.json, including local store credentials. Real Windows execution remains unverified. Final audit reported only the same five historical states listed above; no new quarantines or owned processes/resources remain. All successful temporary output was deleted, failed evidence expires after seven days, and formal deliveries are retained without expiry. Released-byte totals were not recorded by the supervisor. The final documentation-only commit does not require another executable rollout.

## GitHub Connector release, 2026-11-04

[Store Connector v0.1.0](https://github.com/baoluchuling/testflying/releases/tag/store-connector-v0.1.0) was published from commit `c4627b97140891f1091e51363b61963ba8865bee`. The existing Linux x64, macOS arm64 and Windows x64 binaries retain their verified build hashes and Rust source commit `703305180e49cf330d13d3ae507581a2d2226c48`; no recompilation or server deployment was performed for this packaging task. The previous 42-test source verification and 26-check deployment still match the Rust source hashes.

Three credential-free archives and their SHA256SUMS were uploaded to a draft, downloaded again, checked byte-for-byte against SHA-256, and then published. GitHub's reported asset digests also matched. The archive inventory permits only the common executable, placeholder Apple/Google configuration templates, launcher, README and public CA. No real connector.json, node Token/message/upload keys, account credentials or node-specific private ZIP was read or uploaded. Existing nodes should keep their real configuration and replace only the executable. The old backend's default Latest Release was independently confirmed unchanged.

[github-release-verification.json](github-release-verification.json) records the release ID/tag/commit, exact binary and archive hashes, four download URLs, GitHub asset IDs/digests, and archive file lists. macOS code signing/notarization and real Windows execution remain unverified; the release notes state those limits.

| Run ID | Outcome | Cleanup | Retention |
|---|---|---|---|
| 7e9605bffe2147918d0d2e4d32fe6c27 | Git SSH remote-ref preflight disconnected; switched the read-only check to the GitHub API | Entire owned run deleted; no remaining processes/resources; no release created | Sanitized compact logs expire 2026-11-11T08:44:22Z |
| e9f17823c8704ba4ad02c78c0a248034 | Three archive checks passed and draft created; public tag endpoint did not expose the unpublished draft; lookup corrected to use its Release ID | Entire owned run deleted; formal archives and empty draft retained; draft subsequently completed | Sanitized compact logs expire 2026-11-11T08:45:30Z |
| c1692429a0ae478b834370953e3bc63f | All three uploads, checksum upload, four round-trip hashes, GitHub digests, public status, tag target and legacy Latest checks passed | Entire owned run deleted; no remaining processes/resources or unfinished draft | Formal local archives/checksums, verification report and GitHub Release retained without expiration |

Failure evidence is under `~/Library/Caches/CodexArtifactLifecycle/evidence/store-service-a59b33aca240/{runId}/`, with exact expiration owned by each metadata file. Cleanup reports use the per-run report paths above. Released-byte totals were not recorded. Final artifact audit returned only the same five historical states already documented, including the protected .git fixture quarantine; this task introduced no new quarantine. The final documentation-only commit does not change the published tag or binaries.

## HTTP ingress and public-address redaction, 2026-11-05

The HTTP ingress now serves only `/store-api/` on port 80, alongside the unchanged HTTPS server. Its JSON filter adapts only the generated `uploadUrl` field for HTTP callers; HTTPS uploads and newly issued HTTPS/WSS node configuration retain their original transport. No Rust executable or persistent credential was changed or restarted. HTTP sends external API tokens and request content without TLS; HTTPS certificate verification remains enabled.

The final run passed **18 production checks**: public HTTP health, required and accepted authentication, mock-node creation and app mapping, business reads through encrypted WSS, upload credentials, HTTP upload URLs, streaming a valid 3,147,060-byte PNG through HTTP and encrypted WSS, mock sync consumption, preserved HTTPS upload URLs and health, the other HTTPS application, and unchanged API PID/environment. No real Apple/Google account was contacted. Verification mappings were removed, test nodes disabled and local Connector processes stopped.

| Run ID | Outcome | Cleanup / retention |
|---|---|---|
| 5b34b3a7552c456982e470f400e04cf7 | Initial IP/history/certificate inventory completed | Owned temporary files and processes removed; private local inventory only |
| add4e4da31274f0686570da88f30c96c | All 133 Release records and 135 selected assets inspected; 40 old wheel assets contained the address; new Rust archives and decoded CA certificates did not | Owned temporary files removed; private metadata report only |
| bdfcdad4f8c74f0085502b7ee2591e9c | Production proxy/port inspection completed | Owned temporary files and processes removed |
| 5a5efa5e777a4a6487787af3c16078d5 | HTTP authentication passed; verification expected a WSS URL where runtime configuration intentionally stores its HTTPS base; check corrected | Proxy restored, node disabled, mapping removed; compact sanitized evidence expires after 7 days |
| 7fc2d93fc9da46b0a826c920fa6875d0 | HTTP upload and mock sync passed; later HTTPS verification reused an idle connection; connection handling corrected | Proxy restored, node disabled, mapping removed; compact sanitized evidence expires after 7 days |
| 1deb6f19a96d40cfa0c161eaeb1a7767 | All 18 HTTP/HTTPS/WSS checks passed | Entire owned run removed, no local processes; HTTP configuration and permission-restricted server backup retained as formal production data |
| 8729f071e904489b83881560ef6d97e9 | Original Git refs, bundle and pending source recovery point preserved privately; current tracked deployment addresses replaced with examples | Entire owned run removed; private Git recovery source and mode-0600 ignored backup files retained without expiration |
| ab0eb21177c84b598e2bbf16caeecfc0 | Python/shell syntax, public report redaction, 21 JSON examples, all 11 endpoint sections, local document links, unchanged verified Rust source hashes, preserved pending user source and all 516 staged source blobs passed | Entire owned run removed; private summary only |
| a5b8d8b40dda4e09839c9ba44aa95466 | Removed 131 obsolete backend Releases, 877 assets, 131 build tags and one empty draft; retained the four Rust Release assets and selected that Release as Latest | Entire owned run removed; private deletion inventory and recovery source retained |

Public addresses in older deployment reports are now redacted and explicitly marked `addressRedacted`; example addresses are not live verification destinations. Deployment tools read the real `STORE_API_PUBLIC_URL` from private configuration and redact it when producing public verification reports. Exact private reports, recovery data and real node configuration are never committed or uploaded. Supervisor reports do not record released-byte totals. Successful runs retain no temporary evidence; failed compact logs remain under the per-run evidence paths described above and are limited to seven days.

The repository starts again from a single parentless current-code snapshot on 2026-11-05. Main, the existing refactor branch and the retained Rust tag point to that snapshot; historical commit identifiers in earlier verification records describe pre-reset executions. Verified binary and Rust source SHA-256 values remain unchanged. Old history survives only in private local recovery data; GitHub read-only pull-request references and cached unreachable objects require separate platform cleanup and are not covered by a branch/tag force push.
