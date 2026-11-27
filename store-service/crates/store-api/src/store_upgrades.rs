//! Explicit administrator-authorized Store Connector upgrades.
use super::*;
use serde::Serialize;
use std::{
    cmp::Ordering,
    io::{Cursor, Read, Write},
    path::Path as FsPath,
};
use store_update::{SignedCommand, Trust};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct UpgradeCapability {
    pub protocol: u32,
    pub target: String,
    pub installed: bool,
    pub bootstrap_version: u32,
}
impl UpgradeCapability {
    pub(crate) fn valid(&self) -> bool {
        self.protocol == 1
            && self.bootstrap_version == 1
            && store_update::TARGETS.contains(&self.target.as_str())
    }
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpgradeNode {
    pub connector_id: String,
    pub name: String,
    pub request_id: Option<String>,
    pub version: Option<String>,
    pub target: Option<String>,
    pub state: String,
    pub reason: Option<String>,
    pub error_code: Option<String>,
    pub updated_at: u64,
    #[serde(skip)]
    pub command: Option<SignedCommand>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Batch {
    pub batch_id: String,
    pub version: String,
    pub created_at: u64,
    pub expires_at: u64,
    pub state: String,
    pub nodes: Vec<UpgradeNode>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct UpgradeStatus {
    pub request_id: String,
    pub node_id: String,
    pub state: String,
    #[serde(default)]
    pub error_code: Option<String>,
}
impl UpgradeStatus {
    pub(crate) fn valid(&self) -> bool {
        matches!(
            self.state.as_str(),
            "notified"
                | "downloading"
                | "draining"
                | "installing"
                | "awaiting_confirmation"
                | "succeeded"
                | "rolled_back"
                | "failed"
                | "cleanup_pending"
        ) && self.error_code.as_ref().is_none_or(|code| {
            !code.is_empty()
                && code.len() <= 64
                && code
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        })
    }
}
pub(super) fn management_routes(path: &str) -> Router<Arc<ApiState>> {
    Router::new()
        .route(path, get(overview).post(create))
        .route(&format!("{path}/{{batchId}}"), get(batch))
}
pub(super) fn routes(state: Arc<ApiState>) -> Router<Arc<ApiState>> {
    let admin = management_routes("/v1/store-node-upgrades")
        .route_layer(middleware::from_fn_with_state(state, authorize_admin))
        .route_layer(middleware::from_fn(integrations::authorize_https));
    admin.merge(
        Router::new()
            .route("/v1/connectors/{id}/store-upgrades/pending", get(pending))
            .route(
                "/v1/connectors/{id}/store-upgrades/{request}/package",
                get(package),
            )
            .route(
                "/v1/connectors/{id}/store-upgrades/{request}/status",
                post(status),
            )
            .route(
                "/v1/connectors/{id}/store-upgrades/{request}/ready",
                post(ready),
            )
            .route(
                "/v1/connectors/{id}/store-upgrades/{request}/confirm",
                get(confirm),
            )
            .route_layer(middleware::from_fn(integrations::authorize_https))
            .route_layer(middleware::map_response(|response| async {
                integrations::no_store(response)
            })),
    )
}
fn reason(
    node: &Connector,
    cap: Option<&UpgradeCapability>,
    latest: Option<&str>,
) -> Option<&'static str> {
    let Some(current) = node.connector_version.as_deref() else {
        return Some("unknown_version");
    };
    let Some(latest) = latest else {
        return Some("release_unavailable");
    };
    match store_update::compare_versions(current, latest) {
        Ok(Ordering::Equal) => return Some("current"),
        Ok(Ordering::Greater) => return Some("newer"),
        Err(_) => return Some("unknown_version"),
        _ => {}
    }
    match cap {
        Some(c) if c.valid() && c.installed && c.target != "linux-amd64" => None,
        Some(c) if c.target == "linux-amd64" => Some("unsupported_installation"),
        _ => Some("manual_install_required"),
    }
}
async fn overview(State(state): State<Arc<ApiState>>) -> Result<Json<Value>> {
    let latest = releases::node_version(&state.release_root, ConnectorKind::Store);
    let nodes:Vec<_>=state.registry.list()?.into_iter().filter(|n|n.kind==ConnectorKind::Store).map(|n|{
        let cap=state.transport.store_upgrade_capability(&n.connector_id);
        let why=reason(&n,cap.as_ref(),latest.as_deref());
        json!({"connectorId":n.connector_id,"name":n.name,"kind":"store","online":state.transport.connected(&n.connector_id),"enabled":n.enabled,"version":n.connector_version,"target":cap.as_ref().map(|c|&c.target),"supported":cap.as_ref().is_some_and(|c|c.valid()&&c.installed&&c.target!="linux-amd64"),"reason":why})
    }).collect();
    Ok(Json(
        json!({"latestVersion":latest,"nodes":nodes,"latestBatch":state.registry.latest_upgrade_batch()?}),
    ))
}
async fn batch(State(state): State<Arc<ApiState>>, Path(id): Path<String>) -> Result<Json<Batch>> {
    Ok(Json(state.registry.upgrade_batch(&id)?))
}
fn release_snapshot(root: &FsPath) -> std::result::Result<(PathBuf, String), StoreError> {
    // Resolve production's movable `current` symlink once, before reading any release
    // metadata. The version, binaries and launchers must all use this immutable directory.
    let root = std::fs::canonicalize(root)
        .map_err(|_| StoreError::new(503, "release_unavailable", "Store release unavailable"))?;
    let version = releases::node_version(&root, ConnectorKind::Store)
        .ok_or_else(|| StoreError::new(503, "release_unavailable", "Store release unavailable"))?;
    Ok((root, version))
}
async fn create(State(state): State<Arc<ApiState>>) -> Result<Json<Batch>> {
    if let Some(batch) = state.registry.active_upgrade_batch()? {
        return Ok(Json(batch));
    }
    let (root, version) = release_snapshot(&state.release_root)?;
    let nodes: Vec<_> = state
        .registry
        .list()?
        .into_iter()
        .filter(|n| {
            n.kind == ConnectorKind::Store
                && n.enabled
                && state.transport.connected(&n.connector_id)
        })
        .map(|n| {
            let cap = state.transport.store_upgrade_capability(&n.connector_id);
            let why = reason(&n, cap.as_ref(), Some(&version));
            UpgradeNode {
                connector_id: n.connector_id,
                name: n.name,
                request_id: None,
                version: n.connector_version,
                target: cap.map(|c| c.target),
                state: if why.is_some() { "skipped" } else { "notified" }.into(),
                reason: why.map(str::to_owned),
                error_code: None,
                updated_at: store_update::now(),
                command: None,
            }
        })
        .collect();
    if nodes.len() > 256 {
        return Err(StoreError::new(
            409,
            "upgrade_node_limit",
            "At most 256 online Store Connectors per batch",
        )
        .into());
    }
    let targets: std::collections::BTreeSet<_> = nodes
        .iter()
        .filter(|n| n.state != "skipped")
        .filter_map(|n| n.target.clone())
        .collect();
    let packages = tokio::task::spawn_blocking(move || {
        targets
            .into_iter()
            .filter_map(|target| {
                build_package(&root, &target)
                    .ok()
                    .map(|bytes| (target, bytes))
            })
            .collect::<Vec<_>>()
    })
    .await
    .map_err(|_| {
        StoreError::new(
            500,
            "upgrade_package_failed",
            "Cannot prepare Store release",
        )
    })?;
    let batch = state
        .registry
        .create_upgrade_batch(&version, nodes, packages)?;
    for node in &batch.nodes {
        if let Some(command) = state.registry.pending_upgrade(&node.connector_id)? {
            state
                .transport
                .send_store_upgrade(&node.connector_id, command);
        }
    }
    Ok(Json(batch))
}
fn authenticate(state: &ApiState, id: &str, headers: &HeaderMap) -> Result<Connector> {
    let node = state.registry.authenticate(id, bearer(headers))?;
    if node.kind != ConnectorKind::Store || !node.enabled {
        return Err(StoreError::new(
            403,
            "store_upgrade_forbidden",
            "Enabled Store Connector required",
        )
        .into());
    }
    Ok(node)
}
async fn pending(
    State(state): State<Arc<ApiState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    authenticate(&state, &id, &headers)?;
    Ok(Json(
        json!({"command":state.registry.pending_upgrade(&id)?}),
    ))
}
async fn package(
    State(state): State<Arc<ApiState>>,
    Path((id, request)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    authenticate(&state, &id, &headers)?;
    let bytes = state.registry.upgrade_package(&id, &request)?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/zip"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        bytes,
    )
        .into_response())
}
pub(crate) fn accept_status(
    state: &ApiState,
    id: &str,
    report: &UpgradeStatus,
) -> std::result::Result<(), StoreError> {
    if state.registry.upgrade_status_acknowledged(id, report)? {
        return Ok(());
    }
    if report.state == "succeeded" {
        let command = state.registry.upgrade_command(id, &report.request_id)?;
        let node = state.registry.get(id)?;
        if !state.transport.connected(id)
            || node.connector_version.as_deref() != Some(command.request.version.as_str())
            || !state
                .transport
                .store_upgrade_capability(id)
                .is_some_and(|c| c.valid() && c.installed && c.target == command.request.target)
        {
            return Err(StoreError::new(
                409,
                "upgrade_unconfirmed",
                "Target connection is not online",
            ));
        }
    }
    state.registry.upgrade_status(id, report)
}
async fn status(
    State(state): State<Arc<ApiState>>,
    Path((id, request)): Path<(String, String)>,
    headers: HeaderMap,
    Json(report): Json<UpgradeStatus>,
) -> Result<Json<Value>> {
    authenticate(&state, &id, &headers)?;
    if report.request_id != request {
        return Err(
            StoreError::new(422, "invalid_upgrade_status", "Request identity mismatch").into(),
        );
    }
    accept_status(&state, &id, &report)?;
    // Receipt ACK permits the manager to release its durable outbox. Active reports
    // have been persisted; obsolete terminal reports are intentionally discarded
    // under retention and do not change the retained batch outcome.
    Ok(Json(json!({"accepted":true})))
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Ready {
    completed_job_ids: Vec<String>,
}
async fn ready(
    State(state): State<Arc<ApiState>>,
    Path((id, request)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<Ready>,
) -> Result<Json<Value>> {
    authenticate(&state, &id, &headers)?;
    let active = state
        .registry
        .pending_upgrade(&id)?
        .is_some_and(|c| c.request.request_id == request);
    let ready = active
        && state.transport.connected(&id)
        && state.transport.pending_count(&id) == 0
        && state
            .registry
            .upgrade_jobs_ready(&id, &body.completed_job_ids)?
        && state.registry.authorize_upgrade_ready(&id, &request)?;
    Ok(Json(json!({"ready":ready})))
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfirmationQuery {
    version: Option<String>,
}
async fn confirm(
    State(state): State<Arc<ApiState>>,
    Path((id, request)): Path<(String, String)>,
    headers: HeaderMap,
    Query(query): Query<ConfirmationQuery>,
) -> Result<Json<Value>> {
    let node = authenticate(&state, &id, &headers)?;
    if query
        .version
        .as_ref()
        .is_some_and(|version| store_update::compare_versions(version, version).is_err())
    {
        return Err(
            StoreError::new(422, "invalid_upgrade_version", "Invalid target version").into(),
        );
    }
    let command = state.registry.upgrade_confirmation_command(&id, &request)?;
    let version = match (&command, query.version) {
        (Some(command), Some(version)) if version != command.request.version => {
            return Err(StoreError::new(
                409,
                "upgrade_version_mismatch",
                "Target version differs from the retained request",
            )
            .into());
        }
        (Some(command), _) => command.request.version.clone(),
        (None, Some(version)) => version,
        (None, None) => {
            return Err(StoreError::new(
                404,
                "store_upgrade_not_found",
                "Target version is required after upgrade history expires",
            )
            .into())
        }
    };
    let online = state.transport.connected(&id);
    let cap = state.transport.store_upgrade_capability(&id);
    let confirmed = online
        && node.connector_version.as_deref() == Some(version.as_str())
        && cap.as_ref().is_some_and(|c| {
            c.valid()
                && c.installed
                && matches!(c.target.as_str(), "darwin-arm64" | "windows-amd64")
                && command
                    .as_ref()
                    .is_none_or(|command| c.target == command.request.target)
        });
    if command.is_some() {
        state.registry.confirm_upgrade(&id, &request, confirmed)?;
    }
    Ok(Json(
        json!({"confirmed":confirmed,"online":online,"version":node.connector_version,"targetVersion":version}),
    ))
}
pub(crate) fn trust(
    state: &ApiState,
    node: &Connector,
    target: &str,
) -> std::result::Result<Vec<u8>, StoreError> {
    let mut server = reqwest::Url::parse(&state.public_url)
        .map_err(|_| StoreError::new(500, "invalid_public_url", "Invalid public center address"))?;
    server
        .set_scheme("https")
        .map_err(|_| StoreError::new(500, "invalid_public_url", "Invalid public center address"))?;
    serde_json::to_vec_pretty(&Trust {
        protocol: 1,
        node_id: node.connector_id.clone(),
        server_url: server.as_str().trim_end_matches('/').into(),
        public_key: state.registry.update_public_key()?,
        target: target.into(),
    })
    .map_err(|_| StoreError::new(500, "upgrade_trust_failed", "Cannot package upgrade trust"))
}
fn bounded_read(path: &FsPath, limit: u64) -> std::result::Result<Vec<u8>, StoreError> {
    let unavailable = || StoreError::new(503, "release_unavailable", "Store release unavailable");
    let metadata = std::fs::symlink_metadata(path).map_err(|_| unavailable())?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > limit {
        return Err(unavailable());
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|_| unavailable())?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| unavailable())?;
    if bytes.len() as u64 > limit {
        return Err(unavailable());
    }
    Ok(bytes)
}
fn build_package(root: &FsPath, target: &str) -> std::result::Result<Vec<u8>, StoreError> {
    if !matches!(target, "windows-amd64" | "darwin-arm64") {
        return Err(StoreError::new(
            422,
            "unsupported_installation",
            "Unsupported installation",
        ));
    }
    let windows = target == "windows-amd64";
    let binary = if windows {
        "store-connector.exe"
    } else {
        "store-connector"
    };
    let script = if windows { "start.cmd" } else { "start.sh" };
    let binary_bytes = bounded_read(&root.join(target).join(binary), store_update::MAX_PACKAGE)?;
    let script_bytes = installer_assets::read(root, ConnectorKind::Store, script)?;
    let ca = include_bytes!("../../../deploy/testflying-prod-ca.crt");
    let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default())
        .unix_permissions(0o755);
    for (name, bytes) in [
        (binary, binary_bytes.as_slice()),
        (script, script_bytes.as_slice()),
        ("testflying-prod-ca.crt", ca.as_slice()),
    ] {
        archive.start_file(name, options).map_err(|_| {
            StoreError::new(
                500,
                "upgrade_package_failed",
                "Cannot package Store release",
            )
        })?;
        archive.write_all(bytes).map_err(|_| {
            StoreError::new(
                500,
                "upgrade_package_failed",
                "Cannot package Store release",
            )
        })?;
    }
    let bytes = archive
        .finish()
        .map_err(|_| StoreError::new(500, "upgrade_package_failed", "Cannot finish Store release"))?
        .into_inner();
    if bytes.len() as u64 > store_update::MAX_PACKAGE {
        return Err(StoreError::new(
            503,
            "upgrade_package_limit",
            "Store upgrade package exceeded limit",
        ));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    const ADMIN: &str = "upgrade-fixture-admin-token-12345";
    const CLIENT: &str = "upgrade-fixture-business-token-12345";
    const KEY: &str = "upgrade-fixture-signing-key-0123456789";
    fn capability(target: &str) -> UpgradeCapability {
        UpgradeCapability {
            protocol: 1,
            target: target.into(),
            installed: true,
            bootstrap_version: 1,
        }
    }
    fn state(root: &FsPath) -> Arc<ApiState> {
        ApiState::new(
            Registry::open(":memory:", KEY).unwrap(),
            ADMIN.into(),
            CLIENT.into(),
            "http://fixture.invalid".into(),
            root.into(),
            Duration::from_secs(1),
        )
        .unwrap()
    }
    async fn call(
        state: Arc<ApiState>,
        method: &str,
        path: &str,
        token: &str,
        https: bool,
        body: Value,
    ) -> (StatusCode, Value) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json");
        if https {
            request = request.header("x-forwarded-proto", "https");
        }
        let response = router(state)
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap_or_default())
    }
    fn node(state: &ApiState, name: &str, kind: ConnectorKind, version: &str) -> Connector {
        let node = state
            .registry
            .create_with_kind(ConnectorInput { name: name.into() }, kind)
            .unwrap();
        state
            .registry
            .register_connection(
                &node.connector_id,
                &state.registry.token(&node),
                None,
                Some(version),
            )
            .unwrap();
        state.registry.get(&node.connector_id).unwrap()
    }
    fn release(root: &FsPath) {
        std::fs::create_dir_all(root.join("darwin-arm64")).unwrap();
        std::fs::write(
            root.join("darwin-arm64/store-connector"),
            b"fixture-store-binary",
        )
        .unwrap();
        std::fs::write(
            root.join("connector-versions.json"),
            br#"{"store":"1.0.1","screenshot":"1.0.1"}"#,
        )
        .unwrap();
    }
    #[tokio::test]
    async fn admin_https_and_exact_online_store_snapshot_duplicate_clicks_and_immutable_package() {
        let root = tempfile::tempdir().unwrap();
        release(root.path());
        let state = state(root.path());
        let eligible = node(&state, "installed", ConnectorKind::Store, "1.0.0");
        let mut control = state
            .transport
            .fixture_upgrade_connection(&eligible.connector_id, Some(capability("darwin-arm64")));
        let legacy = node(&state, "legacy", ConnectorKind::Store, "1.0.0");
        let legacy_control = state
            .transport
            .fixture_upgrade_connection(&legacy.connector_id, None);
        let current = node(&state, "current", ConnectorKind::Store, "1.0.1");
        state
            .transport
            .fixture_upgrade_connection(&current.connector_id, Some(capability("darwin-arm64")));
        let linux = node(&state, "linux", ConnectorKind::Store, "1.0.0");
        state
            .transport
            .fixture_upgrade_connection(&linux.connector_id, Some(capability("linux-amd64")));
        let screenshot = node(&state, "screenshot", ConnectorKind::Screenshot, "1.0.0");
        state
            .transport
            .fixture_upgrade_connection(&screenshot.connector_id, None);
        let offline = node(&state, "offline", ConnectorKind::Store, "1.0.0");
        let disabled = node(&state, "disabled", ConnectorKind::Store, "1.0.0");
        state
            .registry
            .set_enabled(&disabled.connector_id, false)
            .unwrap();
        state
            .transport
            .fixture_upgrade_connection(&disabled.connector_id, Some(capability("darwin-arm64")));
        for (token, https, status) in [
            (ADMIN, false, StatusCode::FORBIDDEN),
            (CLIENT, true, StatusCode::UNAUTHORIZED),
        ] {
            assert_eq!(
                call(
                    state.clone(),
                    "POST",
                    "/v1/store-node-upgrades",
                    token,
                    https,
                    json!({})
                )
                .await
                .0,
                status
            );
        }
        let (a, b) = tokio::join!(
            call(
                state.clone(),
                "POST",
                "/v1/store-node-upgrades",
                ADMIN,
                true,
                json!({})
            ),
            call(
                state.clone(),
                "POST",
                "/v1/store-node-upgrades",
                ADMIN,
                true,
                json!({})
            )
        );
        assert_eq!(a.0, StatusCode::OK);
        assert_eq!(b.0, StatusCode::OK);
        assert_eq!(a.1["batchId"], b.1["batchId"]);
        let nodes = a.1["nodes"].as_array().unwrap();
        assert_eq!(nodes.len(), 4);
        for excluded in [&screenshot, &offline, &disabled] {
            assert!(!nodes
                .iter()
                .any(|n| n["connectorId"] == excluded.connector_id));
        }
        assert!(nodes
            .iter()
            .any(|n| n["connectorId"] == legacy.connector_id
                && n["reason"] == "manual_install_required"));
        assert!(nodes
            .iter()
            .any(|n| n["connectorId"] == linux.connector_id
                && n["reason"] == "unsupported_installation"));
        assert!(nodes
            .iter()
            .any(|n| n["connectorId"] == current.connector_id && n["reason"] == "current"));
        assert!(!legacy_control.has_changed().unwrap());
        assert!(control.has_changed().unwrap());
        let command = control.borrow_and_update().clone().unwrap();
        assert_eq!(state.transport.pending_count(&eligible.connector_id), 0);
        let trust: Trust =
            serde_json::from_slice(&trust(&state, &eligible, "darwin-arm64").unwrap()).unwrap();
        assert_eq!(trust.server_url, "https://fixture.invalid");
        store_update::verify(&command, &trust, store_update::now()).unwrap();
        let original = state
            .registry
            .upgrade_package(&eligible.connector_id, &command.request.request_id)
            .unwrap();
        std::fs::write(
            root.path().join("darwin-arm64/store-connector"),
            b"subsequent-release",
        )
        .unwrap();
        assert_eq!(
            state
                .registry
                .upgrade_package(&eligible.connector_id, &command.request.request_id)
                .unwrap(),
            original
        );
        assert!(state
            .registry
            .pending_upgrade(&offline.connector_id)
            .unwrap()
            .is_none());
        let new_online = node(&state, "joined-after-click", ConnectorKind::Store, "1.0.0");
        state
            .transport
            .fixture_upgrade_connection(&new_online.connector_id, Some(capability("darwin-arm64")));
        assert!(state
            .registry
            .pending_upgrade(&new_online.connector_id)
            .unwrap()
            .is_none());
        let token = state.registry.token(&eligible);
        let path = format!(
            "/v1/connectors/{}/store-upgrades/pending",
            eligible.connector_id
        );
        assert_eq!(
            call(state.clone(), "GET", &path, CLIENT, true, json!({}))
                .await
                .0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            call(state.clone(), "GET", &path, &token, false, json!({}))
                .await
                .0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            call(state.clone(), "GET", &path, &token, true, json!({}))
                .await
                .1["command"]["request"]["requestId"],
            command.request.request_id
        );
        let screenshot_path = format!(
            "/v1/connectors/{}/store-upgrades/pending",
            screenshot.connector_id
        );
        assert_eq!(
            call(
                state.clone(),
                "GET",
                &screenshot_path,
                &state.registry.token(&screenshot),
                true,
                json!({})
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
        let text = serde_json::to_string(&a.1).unwrap();
        assert!(!text.contains("signature"));
        assert!(!text.contains(KEY));
    }
    #[test]
    fn pure_package_is_deterministic_and_cannot_overwrite_trust_or_business_data() {
        let root = tempfile::tempdir().unwrap();
        release(root.path());
        let a = build_package(root.path(), "darwin-arm64").unwrap();
        assert_eq!(a, build_package(root.path(), "darwin-arm64").unwrap());
        let zip = zip::ZipArchive::new(Cursor::new(a)).unwrap();
        assert_eq!(
            zip.file_names().collect::<Vec<_>>(),
            vec!["store-connector", "start.sh", "testflying-prod-ca.crt"]
        );
        assert!(build_package(root.path(), "../darwin-arm64").is_err());
        assert!(build_package(root.path(), "linux-amd64").is_err());
        #[cfg(unix)]
        {
            let outside = root.path().join("outside");
            std::fs::write(&outside, b"untrusted").unwrap();
            std::fs::remove_file(root.path().join("darwin-arm64/store-connector")).unwrap();
            std::os::unix::fs::symlink(outside, root.path().join("darwin-arm64/store-connector"))
                .unwrap();
            assert!(build_package(root.path(), "darwin-arm64").is_err());
        }
    }
    #[cfg(unix)]
    #[test]
    fn deployment_symlink_switch_keeps_version_and_every_target_resource_in_one_snapshot() {
        let root = tempfile::tempdir().unwrap();
        let old = root.path().join("release-old");
        let new = root.path().join("release-new");
        for (directory, version, marker) in [(&old, "1.0.1", "old"), (&new, "1.0.2", "new")] {
            std::fs::create_dir_all(directory.join("installer-assets/store")).unwrap();
            std::fs::write(
                directory.join("connector-versions.json"),
                json!({"store":version,"screenshot":"1.0.0"}).to_string(),
            )
            .unwrap();
            for (target, binary, launcher) in [
                ("darwin-arm64", "store-connector", "start.sh"),
                ("windows-amd64", "store-connector.exe", "start.cmd"),
            ] {
                std::fs::create_dir_all(directory.join(target)).unwrap();
                std::fs::write(
                    directory.join(target).join(binary),
                    format!("{marker}-{target}-binary"),
                )
                .unwrap();
                std::fs::write(
                    directory.join("installer-assets/store").join(launcher),
                    format!("{marker}-{target}-launcher"),
                )
                .unwrap();
            }
        }
        let current = root.path().join("current");
        std::os::unix::fs::symlink(&old, &current).unwrap();
        let (snapshot, version) = release_snapshot(&current).unwrap();
        // Match production's atomic replacement while package preparation is deferred.
        let next = root.path().join("next");
        std::os::unix::fs::symlink(&new, &next).unwrap();
        std::fs::rename(next, &current).unwrap();
        assert_eq!(
            releases::node_version(&current, ConnectorKind::Store).as_deref(),
            Some("1.0.2")
        );
        assert_eq!(version, "1.0.1");
        assert_eq!(snapshot, std::fs::canonicalize(&old).unwrap());
        for (target, binary, launcher) in [
            ("darwin-arm64", "store-connector", "start.sh"),
            ("windows-amd64", "store-connector.exe", "start.cmd"),
        ] {
            let bytes = build_package(&snapshot, target).unwrap();
            let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
            for (name, expected) in [
                (binary, format!("old-{target}-binary")),
                (launcher, format!("old-{target}-launcher")),
            ] {
                let mut actual = String::new();
                zip.by_name(name)
                    .unwrap()
                    .read_to_string(&mut actual)
                    .unwrap();
                assert_eq!(actual, expected);
            }
        }
    }
    #[tokio::test]
    async fn only_manager_confirmed_online_target_can_finish_and_backwards_phases_do_not_regress() {
        let root = tempfile::tempdir().unwrap();
        release(root.path());
        let state = state(root.path());
        let node = node(&state, "installed", ConnectorKind::Store, "1.0.0");
        let mut control = state
            .transport
            .fixture_upgrade_connection(&node.connector_id, Some(capability("darwin-arm64")));
        let batch = call(
            state.clone(),
            "POST",
            "/v1/store-node-upgrades",
            ADMIN,
            true,
            json!({}),
        )
        .await
        .1;
        let command = control.borrow_and_update().clone().unwrap();
        let request = &command.request.request_id;
        let base = format!(
            "/v1/connectors/{}/store-upgrades/{request}",
            node.connector_id
        );
        let token = state.registry.token(&node);
        assert_eq!(
            call(
                state.clone(),
                "POST",
                &format!("{base}/ready"),
                &token,
                true,
                json!({"completedJobIds":[]})
            )
            .await
            .1["ready"],
            true
        );
        for phase in [
            "draining",
            "downloading",
            "notified",
            "installing",
            "awaiting_confirmation",
        ] {
            assert_eq!(
                call(
                    state.clone(),
                    "POST",
                    &format!("{base}/status"),
                    &token,
                    true,
                    json!({"requestId":request,"nodeId":node.connector_id,"state":phase})
                )
                .await
                .0,
                StatusCode::OK
            );
        }
        let status = json!({"requestId":request,"nodeId":node.connector_id,"state":"succeeded"});
        assert_eq!(
            call(
                state.clone(),
                "POST",
                &format!("{base}/status"),
                &token,
                true,
                status.clone()
            )
            .await
            .0,
            StatusCode::CONFLICT
        );
        assert_eq!(
            call(
                state.clone(),
                "GET",
                &format!("{base}/confirm"),
                &token,
                true,
                json!({})
            )
            .await
            .1["confirmed"],
            false
        );
        state
            .registry
            .register_connection(&node.connector_id, &token, None, Some("1.0.1"))
            .unwrap();
        assert_eq!(
            call(
                state.clone(),
                "GET",
                &format!("{base}/confirm"),
                &token,
                true,
                json!({})
            )
            .await
            .1["confirmed"],
            true
        );
        assert_eq!(
            call(
                state.clone(),
                "POST",
                &format!("{base}/status"),
                &token,
                true,
                status
            )
            .await
            .0,
            StatusCode::OK
        );
        assert!(state
            .registry
            .pending_upgrade(&node.connector_id)
            .unwrap()
            .is_none());
        assert_eq!(
            state
                .registry
                .upgrade_batch(batch["batchId"].as_str().unwrap())
                .unwrap()
                .state,
            "completed"
        );
        assert_eq!(call(state.clone(),"POST",&format!("{base}/status"),&token,true,json!({"requestId":request,"nodeId":node.connector_id,"state":"failed","errorCode":"unsafe token text"})).await.0,StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            state
                .registry
                .upgrade_batch(batch["batchId"].as_str().unwrap())
                .unwrap()
                .nodes[0]
                .state,
            "succeeded"
        );
        // A lost terminal ACK must be retryable after the confirmed worker goes offline.
        state.transport.disconnect(&node.connector_id);
        assert_eq!(
            call(
                state.clone(),
                "POST",
                &format!("{base}/status"),
                &token,
                true,
                json!({"requestId":request,"nodeId":node.connector_id,"state":"succeeded"})
            )
            .await
            .0,
            StatusCode::OK
        );
    }
    #[tokio::test]
    async fn authenticated_obsolete_terminal_http_receipts_are_acknowledged_without_recreating_commands(
    ) {
        let root = tempfile::tempdir().unwrap();
        release(root.path());
        let state = state(root.path());
        let node = node(&state, "installed", ConnectorKind::Store, "1.0.0");
        let mut control = state
            .transport
            .fixture_upgrade_connection(&node.connector_id, Some(capability("darwin-arm64")));
        let batch = call(
            state.clone(),
            "POST",
            "/v1/store-node-upgrades",
            ADMIN,
            true,
            json!({}),
        )
        .await
        .1;
        let command = control.borrow_and_update().clone().unwrap();
        let request = &command.request.request_id;
        let token = state.registry.token(&node);
        let path = format!(
            "/v1/connectors/{}/store-upgrades/{request}/status",
            node.connector_id
        );
        let batch_id = batch["batchId"].as_str().unwrap();
        state
            .registry
            .db
            .lock()
            .unwrap()
            .execute(
                "UPDATE store_upgrade_batches SET expires=0 WHERE id=?1",
                [batch_id],
            )
            .unwrap();
        // No target version, ready or confirmation exists: these are receipt ACKs only.
        for phase in ["failed", "rolled_back", "succeeded"] {
            let report = json!({"requestId":request,"nodeId":node.connector_id,"state":phase});
            assert_eq!(
                call(state.clone(), "POST", &path, CLIENT, true, report.clone())
                    .await
                    .0,
                StatusCode::UNAUTHORIZED
            );
            let (status, receipt) = call(state.clone(), "POST", &path, &token, true, report).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(receipt, json!({"accepted":true}));
            assert_eq!(
                state.registry.upgrade_batch(batch_id).unwrap().nodes[0].state,
                "expired"
            );
        }
        state
            .registry
            .db
            .lock()
            .unwrap()
            .execute(
                "UPDATE store_upgrade_batches SET created=0 WHERE id=?1",
                [batch_id],
            )
            .unwrap();
        assert_eq!(
            call(
                state.clone(),
                "POST",
                &path,
                &token,
                true,
                json!({"requestId":request,"nodeId":node.connector_id,"state":"succeeded"})
            )
            .await
            .0,
            StatusCode::OK
        );
        let unknown = Uuid::new_v4().to_string();
        let path = format!(
            "/v1/connectors/{}/store-upgrades/{unknown}/status",
            node.connector_id
        );
        for phase in ["failed", "rolled_back", "succeeded"] {
            assert_eq!(
                call(
                    state.clone(),
                    "POST",
                    &path,
                    &token,
                    true,
                    json!({"requestId":unknown,"nodeId":node.connector_id,"state":phase})
                )
                .await
                .0,
                StatusCode::OK
            );
        }
        assert_eq!(
            call(
                state.clone(),
                "POST",
                &path,
                &token,
                true,
                json!({"requestId":unknown,"nodeId":node.connector_id,"state":"draining"})
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            call(
                state.clone(),
                "POST",
                &path,
                &token,
                true,
                json!({"requestId":unknown,"nodeId":Uuid::new_v4().to_string(),"state":"failed"})
            )
            .await
            .0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
        let counts: (i64,i64) = state.registry.db.lock().unwrap().query_row(
            "SELECT (SELECT COUNT(*) FROM store_upgrade_batches),(SELECT COUNT(*) FROM store_upgrade_commands)",
            [],|r| Ok((r.get(0)?,r.get(1)?)),
        ).unwrap();
        assert_eq!(counts, (0, 0));
    }
    #[tokio::test]
    async fn missing_confirmation_history_only_observes_validated_version_and_installed_connection() {
        let root = tempfile::tempdir().unwrap(); release(root.path()); let state = state(root.path());
        let node = node(&state,"installed",ConnectorKind::Store,"1.0.1");
        state.transport.fixture_upgrade_connection(&node.connector_id,Some(capability("darwin-arm64")));
        let token = state.registry.token(&node); let request = Uuid::new_v4().to_string();
        let base = format!("/v1/connectors/{}/store-upgrades/{request}/confirm",node.connector_id);
        let path = format!("{base}?version=1.0.1");
        let (status,confirmation) = call(state.clone(),"GET",&path,&token,true,json!({})).await;
        assert_eq!(status,StatusCode::OK);
        assert_eq!(confirmation,json!({"confirmed":true,"online":true,"version":"1.0.1","targetVersion":"1.0.1"}));
        let package = format!("/v1/connectors/{}/store-upgrades/{request}/package",node.connector_id);
        assert_eq!(call(state.clone(),"GET",&package,&token,true,json!({})).await.0,StatusCode::NOT_FOUND);
        let pending = format!("/v1/connectors/{}/store-upgrades/pending",node.connector_id);
        assert_eq!(call(state.clone(),"GET",&pending,&token,true,json!({})).await.1,json!({"command":null}));
        assert_eq!(call(state.clone(),"GET",&base,&token,true,json!({})).await.0,StatusCode::NOT_FOUND);
        for version in ["1.0","1.0.01","1.0.1-pre","invalid"] {
            assert_eq!(call(state.clone(),"GET",&format!("{base}?version={version}"),&token,true,json!({})).await.0,StatusCode::UNPROCESSABLE_ENTITY);
        }
        let invalid_id = format!("/v1/connectors/{}/store-upgrades/not-a-uuid/confirm?version=1.0.1",node.connector_id);
        assert_eq!(call(state.clone(),"GET",&invalid_id,&token,true,json!({})).await.0,StatusCode::UNPROCESSABLE_ENTITY);
        state.transport.fixture_upgrade_connection(&node.connector_id,None);
        assert_eq!(call(state.clone(),"GET",&path,&token,true,json!({})).await.1["confirmed"],false);
        let mut unsupported = capability("darwin-arm64"); unsupported.installed = false;
        state.transport.fixture_upgrade_connection(&node.connector_id,Some(unsupported));
        assert_eq!(call(state.clone(),"GET",&path,&token,true,json!({})).await.1["confirmed"],false);
        state.transport.disconnect(&node.connector_id);
        let offline = call(state.clone(),"GET",&path,&token,true,json!({})).await.1;
        assert_eq!(offline["confirmed"],false); assert_eq!(offline["online"],false);
        let count: i64 = state.registry.db.lock().unwrap().query_row("SELECT COUNT(*) FROM store_upgrade_commands",[],|r|r.get(0)).unwrap();
        assert_eq!(count,0);
    }
    #[tokio::test]
    async fn retained_confirmation_is_authoritative_and_pruned_confirmation_remains_read_only() {
        let root = tempfile::tempdir().unwrap(); release(root.path()); let state = state(root.path());
        let owner = node(&state,"installed",ConnectorKind::Store,"1.0.0");
        let mut control = state.transport.fixture_upgrade_connection(&owner.connector_id,Some(capability("darwin-arm64")));
        let batch = call(state.clone(),"POST","/v1/store-node-upgrades",ADMIN,true,json!({})).await.1;
        let command = control.borrow_and_update().clone().unwrap(); let request = &command.request.request_id;
        let token = state.registry.token(&owner);
        let base = format!("/v1/connectors/{}/store-upgrades/{request}/confirm",owner.connector_id);
        assert_eq!(call(state.clone(),"GET",&format!("{base}?version=1.0.2"),&token,true,json!({})).await.0,StatusCode::CONFLICT);
        assert_eq!(call(state.clone(),"GET",&format!("{base}?version=1.0.1"),&token,true,json!({})).await.1["confirmed"],false);
        state.registry.register_connection(&owner.connector_id,&token,None,Some("1.0.1")).unwrap();
        // Optional version preserves the existing retained-command API.
        assert_eq!(call(state.clone(),"GET",&base,&token,true,json!({})).await.1["confirmed"],true);
        let foreign = node(&state,"other installed",ConnectorKind::Store,"1.0.1");
        state.transport.fixture_upgrade_connection(&foreign.connector_id,Some(capability("darwin-arm64")));
        let foreign_path = format!("/v1/connectors/{}/store-upgrades/{request}/confirm?version=1.0.1",foreign.connector_id);
        assert_eq!(call(state.clone(),"GET",&foreign_path,&state.registry.token(&foreign),true,json!({})).await.0,StatusCode::NOT_FOUND);
        state.registry.db.lock().unwrap().execute("UPDATE store_upgrade_batches SET created=0 WHERE id=?1",[batch["batchId"].as_str().unwrap()]).unwrap();
        let confirmation = call(state.clone(),"GET",&format!("{base}?version=1.0.1"),&token,true,json!({})).await;
        assert_eq!(confirmation.0,StatusCode::OK); assert_eq!(confirmation.1["confirmed"],true);
        let counts: (i64,i64) = state.registry.db.lock().unwrap().query_row(
            "SELECT (SELECT COUNT(*) FROM store_upgrade_batches),(SELECT COUNT(*) FROM store_upgrade_commands)",
            [],|r|Ok((r.get(0)?,r.get(1)?)),
        ).unwrap();
        assert_eq!(counts,(0,0));
    }
}
