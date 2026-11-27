//! Worker-side admission and authenticated local handoff to the independent update manager.
mod manager;
use super::*;
use axum::extract::State as AxumState;
pub use manager::{run_manager, validate_trust_files};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Mutex,
    },
};
use store_update::{SignedCommand, Trust};

pub struct State {
    trust: Mutex<Option<Trust>>,
    request: Mutex<Option<SignedCommand>>,
    draining: AtomicBool,
    active: AtomicUsize,
    frozen: AtomicBool,
    maintenance: Mutex<Option<PathBuf>>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            trust: Mutex::new(None),
            request: Mutex::new(None),
            draining: AtomicBool::new(false),
            active: AtomicUsize::new(0),
            frozen: AtomicBool::new(false),
            maintenance: Mutex::new(None),
        }
    }
}
pub struct Active<'a>(&'a AtomicUsize);
impl Drop for Active<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
impl State {
    pub fn configure(&self, path: &Path) {
        let Ok(root) = super::upgrade_os::installed_root() else {
            return;
        };
        if path.canonicalize().ok() != root.join("data/connector.json").canonicalize().ok() {
            return;
        }
        let executable = std::env::current_exe()
            .ok()
            .and_then(|p| p.canonicalize().ok());
        if executable != root.join("program").join(binary()).canonicalize().ok() {
            return;
        }
        let Ok(trust) = validate_trust_files(path, &root.join("updater/trust.json")) else {
            return;
        };
        if trust.target != store_update::target() || !root.join("updater").join(binary()).is_file()
        {
            return;
        }
        let maintenance = root.join("updater/maintenance.json");
        if std::fs::symlink_metadata(&maintenance)
            .ok()
            .is_some_and(|m| m.is_file() && !m.file_type().is_symlink() && m.len() < 128 * 1024)
        {
            if let Some(command) = std::fs::read(&maintenance)
                .ok()
                .and_then(|b| serde_json::from_slice::<SignedCommand>(&b).ok())
            {
                if store_update::verify(
                    &command,
                    &trust,
                    command.request.expires_at.saturating_sub(1),
                )
                .is_ok()
                {
                    *self.request.lock().unwrap() = Some(command);
                    self.draining.store(true, Ordering::SeqCst);
                    self.frozen.store(true, Ordering::SeqCst);
                }
            }
        }
        *self.maintenance.lock().unwrap() = Some(maintenance);
        *self.trust.lock().unwrap() = Some(trust);
    }
    pub fn capability(&self) -> Value {
        json!({"protocol":1,"target":store_update::target(),"installed":self.trust.lock().unwrap().is_some(),"bootstrapVersion":1})
    }
    pub(super) fn active(&self) -> Active<'_> {
        self.active.fetch_add(1, Ordering::SeqCst);
        Active(&self.active)
    }
    pub(super) fn admit(&self, r: &StoreRequest) -> bool {
        if self.frozen.load(Ordering::SeqCst)
            && self
                .maintenance
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|p| !p.exists())
        {
            let id = self
                .request
                .lock()
                .unwrap()
                .as_ref()
                .map(|r| r.request.request_id.clone());
            if let Some(id) = id {
                self.cancel(&id);
            }
        }
        self.expire();
        if !self.draining.load(Ordering::SeqCst) {
            return true;
        }
        if matches!(
            r.operation,
            Operation::StoreImageSyncStatus
                | Operation::UploadChunk
                | Operation::UploadFinish
                | Operation::UploadAbort
        ) {
            return true;
        }
        matches!(
            r.operation,
            Operation::Sync | Operation::StartStoreImageSync
        ) && !store_provider::assets::asset_ids(&r.payload).is_empty()
    }
    fn expire(&self) {
        let mut current = self.request.lock().unwrap();
        if !self.frozen.load(Ordering::SeqCst)
            && current
                .as_ref()
                .is_some_and(|c| c.request.expires_at <= store_update::now())
        {
            *current = None;
            self.draining.store(false, Ordering::SeqCst);
        }
    }
    pub fn begin(&self, command: SignedCommand) -> std::result::Result<Value, String> {
        self.expire();
        let trust = self
            .trust
            .lock()
            .unwrap()
            .clone()
            .ok_or("upgrade_not_installed")?;
        store_update::verify(&command, &trust, store_update::now())?;
        if store_update::compare_versions(&command.request.version, CONNECTOR_VERSION)?
            != std::cmp::Ordering::Greater
        {
            return Ok(
                json!({"requestId":command.request.request_id,"nodeId":trust.node_id,"state":"notified","errorCode":null}),
            );
        }
        let mut current = self.request.lock().unwrap();
        if current.as_ref().is_some_and(|c| c != &command) {
            return Err("upgrade_in_progress".into());
        }
        let result = json!({"requestId":command.request.request_id,"nodeId":trust.node_id,"state":"draining","errorCode":null});
        *current = Some(command);
        self.draining.store(true, Ordering::SeqCst);
        Ok(result)
    }
    fn cancel(&self, id: &str) {
        let mut current = self.request.lock().unwrap();
        if current.as_ref().is_some_and(|c| c.request.request_id == id) {
            *current = None;
            self.draining.store(false, Ordering::SeqCst);
            self.frozen.store(false, Ordering::SeqCst);
        }
    }
    fn freeze(&self, executor: &Executor, id: &str) -> Value {
        if self.ready(executor, id)["ready"] != true {
            return json!({"ready":false});
        }
        let current = self.request.lock().unwrap();
        if current.as_ref().is_some_and(|c| {
            c.request.request_id == id && c.request.expires_at > store_update::now()
        }) {
            self.frozen.store(true, Ordering::SeqCst);
            json!({"ready":true})
        } else {
            json!({"ready":false})
        }
    }
    fn ready(&self, executor: &Executor, id: &str) -> Value {
        self.expire();
        let matches = self
            .request
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|c| c.request.request_id == id);
        let (jobs_idle, ids) = executor.image_jobs.upgrade_snapshot();
        let assets = executor.assets.pending_count();
        let ready = matches
            && self.draining.load(Ordering::SeqCst)
            && self.active.load(Ordering::SeqCst) == 0
            && executor.capacity.available_permits() == 1
            && jobs_idle
            && assets == 0;
        json!({"ready":ready,"completedJobIds":ids,"pendingAssets":assets})
    }
}
pub fn binary() -> &'static str {
    if cfg!(windows) {
        "store-connector.exe"
    } else {
        "store-connector"
    }
}
pub fn instance_id() -> &'static str {
    static INSTANCE: std::sync::LazyLock<String> =
        std::sync::LazyLock::new(|| uuid::Uuid::new_v4().to_string());
    &INSTANCE
}
fn authorized(executor: &Executor, headers: &HeaderMap) -> bool {
    bool::from(
        executor
            .config
            .token
            .as_bytes()
            .ct_eq(bearer(headers).as_bytes()),
    )
}
fn error(code: &str) -> Response {
    let code = if [
        "upgrade_not_installed",
        "upgrade_in_progress",
        "invalid_update_signature",
        "invalid_update_command",
        "invalid_update_trust",
        "invalid_version",
    ]
    .contains(&code)
    {
        code
    } else {
        "upgrade_invalid"
    };
    (
        StatusCode::CONFLICT,
        Json(StoreError::new(409, code, "Store update request could not be accepted").body()),
    )
        .into_response()
}
pub(super) async fn http_drain(
    AxumState(executor): AxumState<Arc<Executor>>,
    headers: HeaderMap,
    Json(command): Json<SignedCommand>,
) -> Response {
    if !authorized(&executor, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match executor.upgrade.begin(command) {
        Ok(v) => Json(v).into_response(),
        Err(code) => error(&code),
    }
}
pub(super) async fn http_ready(
    AxumState(executor): AxumState<Arc<Executor>>,
    headers: HeaderMap,
    Json(v): Json<Value>,
) -> Response {
    if !authorized(&executor, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(
        executor
            .upgrade
            .ready(&executor, v["requestId"].as_str().unwrap_or("")),
    )
    .into_response()
}
pub(super) async fn http_cancel(
    AxumState(executor): AxumState<Arc<Executor>>,
    headers: HeaderMap,
    Json(v): Json<Value>,
) -> Response {
    if !authorized(&executor, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    executor
        .upgrade
        .cancel(v["requestId"].as_str().unwrap_or(""));
    Json(json!({"canceled":true})).into_response()
}
pub(super) async fn http_freeze(
    AxumState(executor): AxumState<Arc<Executor>>,
    headers: HeaderMap,
    Json(v): Json<Value>,
) -> Response {
    if !authorized(&executor, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(
        executor
            .upgrade
            .freeze(&executor, v["requestId"].as_str().unwrap_or("")),
    )
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn freeze_holds_admission_after_expiry_until_manager_releases_maintenance() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("maintenance.json");
        std::fs::write(&marker, b"fixture").unwrap();
        let config=serde_json::from_value(json!({"connectorId":"fixture","token":"fixture-token-0123456789","serverUrl":"https://fixture.invalid","messageKey":"01".repeat(32),"uploadKey":"02".repeat(32),"provider":{"mode":"mock"}})).unwrap();
        let executor = Executor::new(config).unwrap();
        let state = &executor.upgrade;
        let node = "00000000-0000-4000-8000-000000000001";
        *state.trust.lock().unwrap() = Some(Trust {
            protocol: 1,
            node_id: node.into(),
            server_url: "https://fixture.invalid".into(),
            public_key: store_update::public_key(b"fixture").unwrap(),
            target: store_update::target().into(),
        });
        *state.maintenance.lock().unwrap() = Some(marker.clone());
        let command = store_update::sign(
            store_update::UpgradeRequest {
                protocol: 1,
                node_id: node.into(),
                request_id: "00000000-0000-4000-8000-000000000002".into(),
                version: "9.0.0".into(),
                target: store_update::target().into(),
                sha256: "11".repeat(32),
                size_bytes: 99,
                expires_at: store_update::now() + 3600,
                bootstrap_version: 1,
            },
            b"fixture",
        )
        .unwrap();
        let id = command.request.request_id.clone();
        state.begin(command).unwrap();
        assert_eq!(state.freeze(&executor, &id)["ready"], true);
        state
            .request
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .request
            .expires_at = store_update::now() - 1;
        let request = StoreRequest {
            request_id: "fixture".into(),
            platform: store_protocol::Platform::Ios,
            bundle_identifier: "com.example.app".into(),
            operation: Operation::Sync,
            payload: json!({"operation":"update_app_metadata","locale":"en-US","metadata":{"title":"Copy"}}),
        };
        assert!(!state.admit(&request));
        assert!(state.frozen.load(Ordering::SeqCst));
        std::fs::remove_file(marker).unwrap();
        assert!(state.admit(&request));
    }
    #[test]
    fn busy_admission_preserves_old_uploads_and_cancellation_cannot_release_other_request() {
        let state = State::default();
        let node = "00000000-0000-4000-8000-000000000001";
        let trust = Trust {
            protocol: 1,
            node_id: node.into(),
            server_url: "https://fixture.invalid".into(),
            public_key: store_update::public_key(b"fake").unwrap(),
            target: store_update::target().into(),
        };
        *state.trust.lock().unwrap() = Some(trust);
        let command = store_update::sign(
            store_update::UpgradeRequest {
                protocol: 1,
                node_id: node.into(),
                request_id: "00000000-0000-4000-8000-000000000002".into(),
                version: "9.0.0".into(),
                target: store_update::target().into(),
                sha256: "11".repeat(32),
                size_bytes: 99,
                expires_at: store_update::now() + 3600,
                bootstrap_version: 1,
            },
            b"fake",
        )
        .unwrap();
        state.begin(command.clone()).unwrap();
        state.begin(command).unwrap();
        let mut r = StoreRequest {
            request_id: "fixture".into(),
            platform: store_protocol::Platform::Ios,
            bundle_identifier: "com.example.app".into(),
            operation: Operation::Sync,
            payload: json!({"operation":"update_app_metadata","locale":"en-US","metadata":{"title":"Copy"}}),
        };
        assert!(!state.admit(&r));
        r.operation = Operation::UploadStart;
        assert!(!state.admit(&r));
        r.operation = Operation::UploadFinish;
        assert!(state.admit(&r));
        r.operation = Operation::StoreImageSyncStatus;
        assert!(state.admit(&r));
        state.cancel("other");
        r.operation = Operation::Sync;
        assert!(!state.admit(&r));
        state.cancel("00000000-0000-4000-8000-000000000002");
        assert!(state.admit(&r));
    }
}
