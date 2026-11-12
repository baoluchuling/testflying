use super::*;
use futures_util::{stream::FuturesUnordered, StreamExt};
use registry::store_jobs::{now, StoreJob};

struct DispatchGuard<'a>(&'a ApiState, String);
impl Drop for DispatchGuard<'_> {
    fn drop(&mut self) {
        self.0.store_job_dispatches.lock().unwrap().remove(&self.1);
    }
}

fn snapshot(id: &str, status: &str) -> Value {
    json!({"jobId":id,"status":status,"result":null,"httpStatus":null,"errorCode":null,"outcomeUnknown":status=="unknown"})
}

pub(super) async fn start(
    state: &Arc<ApiState>,
    node: &Connector,
    owner: Option<String>,
    mut request: StoreRequest,
) -> Result<Response> {
    request.operation = Operation::StartStoreImageSync;
    store_provider::validate_request(&request)?;
    let id = request.request_id.clone();
    state
        .store_job_dispatches
        .lock()
        .unwrap()
        .insert(id.clone());
    let _guard = DispatchGuard(state, id.clone());
    let job = StoreJob {
        id: id.clone(),
        owner,
        platform: request.platform,
        app: request.bundle_identifier.clone(),
        connector: node.connector_id.clone(),
        created: now(),
        snapshot: snapshot(&id, "dispatching"),
    };
    state.registry.insert_store_job(&job)?;
    let reply = state
        .transport
        .dispatch(node, &state.registry.token(node), request)
        .await;
    match reply {
        Ok(reply) if reply.status >= 400 => {
            state.registry.delete_store_job(&id)?;
            let mut body = reply.body;
            let summary = logs::error_summary(
                body["error"]["code"].as_str().unwrap_or("request_failed"),
                body["error"]["message"].as_str().unwrap_or(""),
            );
            if let Some(error) = body.get_mut("error").and_then(Value::as_object_mut) {
                error.insert("requestId".into(), json!(id));
            }
            let mut response = (
                StatusCode::from_u16(reply.status).unwrap_or(StatusCode::BAD_GATEWAY),
                Json(body),
            )
                .into_response();
            response.extensions_mut().insert(summary);
            Ok(response)
        }
        Ok(reply) => {
            if reply.body["jobId"].as_str() != Some(&id)
                || reply.body["executionStatus"] != "queued"
                || reply.body["status"] != "succeeded"
            {
                state
                    .registry
                    .update_store_job(&id, &snapshot(&id, "unknown"))?;
                return Err(StoreError::new(
                    502,
                    "invalid_job_acknowledgement",
                    "Image sync acceptance could not be confirmed; query the original request ID",
                )
                .into());
            }
            state
                .registry
                .update_store_job(&id, &snapshot(&id, "queued"))?;
            event(state, &job, "store_images.accepted", "queued");
            Ok((StatusCode::OK, Json(reply.body)).into_response())
        }
        Err(error) => {
            state
                .registry
                .update_store_job(&id, &snapshot(&id, "unknown"))?;
            Err(error.into())
        }
    }
}

pub(super) async fn read(
    State(state): State<Arc<ApiState>>,
    owner: Option<Extension<registry::Integration>>,
    Path((platform, app, id)): Path<(Platform, String, String)>,
) -> Result<Json<Value>> {
    let mut value =
        state
            .registry
            .store_job(&id, owner.as_ref().map(|v| v.0.id.as_str()), platform, &app)?;
    if value["status"] == "dispatching" {
        value["status"] = json!("unknown");
        value["outcomeUnknown"] = json!(true);
    }
    Ok(Json(value))
}

fn event(state: &ApiState, job: &StoreJob, name: &'static str, status: &str) {
    state.registry.logs.record(logs::Event { name, request_id:Some(job.id.clone()),connector_id:Some(job.connector.clone()),platform:Some(job.platform),identifier:Some(job.app.clone()),details:json!({"jobId":job.id,"status":status,"actorKind":if job.owner.is_some(){"integration"}else{"business"},"actorId":job.owner}),..Default::default() });
}

/// Reconciles persisted identities only. It never holds or replays a store write request.
pub async fn run(state: Arc<ApiState>) {
    let mut interval = tokio::time::interval(Duration::from_secs(5));
    let mut pending = FuturesUnordered::new();
    let mut active = std::collections::HashSet::new();
    let mut attempts = HashMap::new();
    let mut sequence = 0;
    loop {
        tokio::select! {
            Some(connector) = pending.next(), if !pending.is_empty() => { active.remove(&connector); }
            _ = interval.tick() => {
                // Deadline/retention maintenance runs on its own clock, even while reads wait.
                let Ok(jobs) = state.registry.pending_store_jobs() else {continue};
                let dispatching=state.store_job_dispatches.lock().unwrap().clone();
                for job in schedule(jobs,&mut active,&dispatching,&mut attempts,&mut sequence,32-pending.len()) {
                    let state=state.clone();
                    pending.push(async move {
                        observe(&state,&job).await;
                        job.connector
                    });
                }
            }
        }
    }
}

/// Oldest attempted job first, independent of result or database row order.
/// Reserve one slot per connector; jobs skipped for an active connector retain
/// their priority until that connector is available. Failed or disabled reads
/// still consume an attempt, preventing them from monopolizing the next tick.
fn schedule(
    mut jobs: Vec<StoreJob>,
    active: &mut std::collections::HashSet<String>,
    dispatching: &std::collections::HashSet<String>,
    attempts: &mut HashMap<String, u64>,
    sequence: &mut u64,
    slots: usize,
) -> Vec<StoreJob> {
    let ids: std::collections::HashSet<_> = jobs.iter().map(|j| j.id.as_str()).collect();
    attempts.retain(|id, _| ids.contains(id.as_str()));
    jobs.sort_by(|a, b| {
        attempts
            .get(&a.id)
            .cmp(&attempts.get(&b.id))
            .then_with(|| a.id.cmp(&b.id))
    });
    let mut selected = Vec::new();
    for job in jobs {
        if selected.len() >= slots {
            break;
        }
        if active.contains(&job.connector) || dispatching.contains(&job.id) {
            continue;
        }
        *sequence += 1;
        attempts.insert(job.id.clone(), *sequence);
        active.insert(job.connector.clone());
        selected.push(job);
    }
    selected
}

async fn observe(state: &ApiState, job: &StoreJob) {
    if state.store_job_dispatches.lock().unwrap().contains(&job.id) {
        return;
    }
    if now() - job.created >= 86400 {
        return;
    }
    let Ok(node) = state.registry.get(&job.connector) else {
        return;
    };
    if !node.enabled {
        return;
    }
    let request = StoreRequest {
        request_id: Uuid::new_v4().to_string(),
        platform: job.platform,
        bundle_identifier: job.app.clone(),
        operation: Operation::StoreImageSyncStatus,
        payload: json!({"jobId":job.id}),
    };
    let result = state
        .transport
        .dispatch(&node, &state.registry.token(&node), request)
        .await;
    let value = match result {
        Ok(reply) if reply.status == 404 => {
            let value = snapshot(&job.id, "unknown");
            // A status read can overtake an unconfirmed Start. Keep querying its
            // original identity until the deadline; never resend the write.
            let admitted = matches!(job.snapshot["status"].as_str(), Some("queued" | "running"));
            if state
                .registry
                .update_store_job(&job.id, &value)
                .unwrap_or(false)
                && admitted
            {
                let _ = state.registry.stop_store_job(&job.id);
                event(state, job, "store_images.completed", "unknown");
            }
            return;
        }
        Ok(reply) if reply.status == 200 => {
            let mut value = reply.body;
            if !valid_snapshot(&value, &job.id) {
                return;
            }
            if value["status"] == "succeeded"
                && (value["result"]["status"] == "failed"
                    || value["httpStatus"].as_u64().is_some_and(|s| s >= 400))
            {
                value["status"] = json!("failed");
            }
            value
        }
        _ => return,
    };
    let terminal = matches!(
        value["status"].as_str(),
        Some("succeeded" | "failed" | "unknown")
    );
    if value == job.snapshot && !terminal {
        return;
    }
    if state
        .registry
        .update_store_job(&job.id, &value)
        .unwrap_or(false)
        && terminal
    {
        let _ = state.registry.stop_store_job(&job.id);
        event(
            state,
            job,
            "store_images.completed",
            value["status"].as_str().unwrap(),
        );
    }
}

fn valid_snapshot(value: &Value, id: &str) -> bool {
    if value.to_string().len() > 64 * 1024 || value["jobId"].as_str() != Some(id) {
        return false;
    }
    let Some(status) = value["status"].as_str() else {
        return false;
    };
    if !matches!(
        status,
        "queued" | "running" | "succeeded" | "failed" | "unknown"
    ) || !value["outcomeUnknown"].is_boolean()
        || (status == "unknown" && value["outcomeUnknown"] != true)
    {
        return false;
    }
    if matches!(status, "succeeded" | "failed")
        || (status == "unknown" && !value["result"].is_null())
    {
        return value["result"].is_object()
            && (status != "unknown" || value["outcomeUnknown"] == true)
            && value["httpStatus"]
                .as_u64()
                .is_some_and(|s| (100..=599).contains(&s));
    }
    value["result"].is_null() && value["httpStatus"].is_null()
}

#[cfg(test)]
mod tests {
    use super::*;
    use store_protocol::StoreReply;

    fn image_request(id: &str) -> StoreRequest {
        StoreRequest {
            request_id: id.into(),
            platform: Platform::Ios,
            bundle_identifier: "com.example.job".into(),
            operation: Operation::Sync,
            payload: json!({"operation":"update_app_metadata","locale":"en-US","syncScopes":["store_images"],
                "metadata":{"storeImages":{"phone_screenshots":{"urls":["https://example.com/image.png"]}}}}),
        }
    }
    async fn image_fixture(
        fake: Router,
    ) -> (Arc<ApiState>, Connector, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, fake).await.unwrap() });
        let registry =
            Registry::open(":memory:", "image-jobs-signing-key-01234567890123456789").unwrap();
        let mut node = registry
            .create(registry::ConnectorInput {
                name: "image fixture".into(),
            })
            .unwrap();
        registry
            .register_endpoint(&node.connector_id, &registry.token(&node), Some(&url))
            .unwrap();
        node.base_url = Some(url);
        let state = ApiState::new(
            registry,
            "image-admin-token-0123456789".into(),
            "image-client-token-0123456789".into(),
            "http://127.0.0.1:8200".into(),
            PathBuf::from("unused"),
            Duration::from_secs(2),
        )
        .unwrap();
        (state, node, server)
    }
    #[tokio::test]
    async fn unconfirmed_status_404_remains_reconcilable_and_later_result_is_saved() {
        let calls = Arc::new(std::sync::Mutex::new(HashMap::<String, usize>::new()));
        let seen = calls.clone();
        let fake = Router::new().route("/v1/execute", post(move |Json(request): Json<StoreRequest>| {
            let seen = seen.clone();
            async move {
                assert_eq!(request.operation, Operation::StoreImageSyncStatus);
                let id = request.payload["jobId"].as_str().unwrap().to_owned();
                let mut calls = seen.lock().unwrap();
                let count = calls.entry(id.clone()).or_default();
                *count += 1;
                Json(if *count == 1 {
                    StoreReply::from_result(request.request_id, Err(StoreError::new(404, "store_image_job_not_found", "Not yet admitted")))
                } else {
                    StoreReply::from_result(request.request_id, Ok(json!({"jobId":id,"status":"succeeded",
                        "result":{"status":"succeeded","message":"actual completion"},"httpStatus":200,"errorCode":null,"outcomeUnknown":false})))
                })
            }
        }));
        let (state, node, server) = image_fixture(fake).await;
        for initial in ["dispatching", "unknown", "queued", "running"] {
            let id = Uuid::new_v4().to_string();
            let job = StoreJob {
                id: id.clone(),
                owner: None,
                platform: Platform::Ios,
                app: "com.example.job".into(),
                connector: node.connector_id.clone(),
                created: now(),
                snapshot: snapshot(&id, initial),
            };
            state.registry.insert_store_job(&job).unwrap();
            // insert_store_job always starts dispatching; advance confirmed fixtures explicitly.
            state.registry.update_store_job(&id, &job.snapshot).unwrap();
            observe(&state, &job).await;
            assert_eq!(
                state
                    .registry
                    .store_job(&id, None, Platform::Ios, &job.app)
                    .unwrap()["status"],
                "unknown"
            );
            let pending = state.registry.pending_store_jobs().unwrap();
            let next = pending.into_iter().find(|j| j.id == id);
            if matches!(initial, "dispatching" | "unknown") {
                observe(&state, &next.unwrap()).await;
                let result = state
                    .registry
                    .store_job(&id, None, Platform::Ios, &job.app)
                    .unwrap();
                assert_eq!(result["status"], "succeeded");
                assert_eq!(result["result"]["message"], "actual completion");
            } else {
                assert!(next.is_none());
            }
        }
        server.abort();
        let _ = server.await;
    }
    #[tokio::test]
    async fn admission_rejection_retains_original_request_id_and_error_summary() {
        let fake = Router::new().route(
            "/v1/execute",
            post(|Json(request): Json<StoreRequest>| async move {
                Json(StoreReply::from_result(
                    request.request_id,
                    Err(StoreError::new(
                        404,
                        "asset_not_found",
                        "Image asset is unavailable",
                    )),
                ))
            }),
        );
        let (state, node, server) = image_fixture(fake).await;
        let id = Uuid::new_v4().to_string();
        let response = start(&state, &node, None, image_request(&id))
            .await
            .ok()
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let summary = response.extensions().get::<logs::ErrorSummary>().unwrap();
        assert_eq!(
            serde_json::to_value(summary).unwrap(),
            serde_json::to_value(logs::error_summary(
                "asset_not_found",
                "Image asset is unavailable"
            ))
            .unwrap()
        );
        let body: Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), 65536)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(body["error"]["requestId"], id);
        assert_eq!(body["error"]["code"], "asset_not_found");
        assert!(state.registry.pending_store_jobs().unwrap().is_empty());
        server.abort();
        let _ = server.await;
    }
    #[tokio::test]
    async fn contradictory_acknowledgement_keeps_original_identity_for_status_reconciliation() {
        let fake = Router::new().route(
            "/v1/execute",
            post(|Json(request): Json<StoreRequest>| async move {
                Json(StoreReply::from_result(
                    request.request_id.clone(),
                    Ok(json!({
                "jobId":request.request_id,"executionStatus":"queued","status":"failed"})),
                ))
            }),
        );
        let (state, node, server) = image_fixture(fake).await;
        let id = Uuid::new_v4().to_string();
        assert!(start(&state, &node, None, image_request(&id))
            .await
            .is_err());
        let pending = state.registry.pending_store_jobs().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, id);
        assert_eq!(pending[0].snapshot["status"], "unknown");
        server.abort();
        let _ = server.await;
    }
    #[test]
    fn scheduling_rotates_more_than_32_nonterminal_connectors_and_jobs_on_same_node() {
        let mut jobs: Vec<_> = (0..40)
            .map(|n| StoreJob {
                id: format!("{n:03}"),
                owner: None,
                platform: Platform::Ios,
                app: "com.example.job".into(),
                connector: format!("node-{n}"),
                created: now(),
                snapshot: snapshot(&format!("{n:03}"), "running"),
            })
            .collect();
        let mut sibling = jobs[0].clone();
        sibling.id = "040".into();
        sibling.snapshot = snapshot("040", "queued");
        jobs.push(sibling);
        let mut active = std::collections::HashSet::new();
        let dispatching = std::collections::HashSet::new();
        let mut attempts = HashMap::new();
        let mut sequence = 0;
        let first = schedule(
            jobs.clone(),
            &mut active,
            &dispatching,
            &mut attempts,
            &mut sequence,
            32,
        );
        assert_eq!(first.len(), 32);
        assert_eq!(first.iter().filter(|j| j.connector == "node-0").count(), 1);
        // All reads promptly return running, queued, or an error. No record is
        // terminal/deleted, and the database returns the same rows again.
        active.clear();
        jobs.reverse();
        let second = schedule(
            jobs.clone(),
            &mut active,
            &dispatching,
            &mut attempts,
            &mut sequence,
            32,
        );
        let seen: std::collections::HashSet<_> = first
            .iter()
            .chain(second.iter())
            .map(|j| j.id.as_str())
            .collect();
        assert_eq!(seen.len(), 41);
        assert!(second.iter().any(|j| j.id == "040"));
        assert_eq!(second.iter().filter(|j| j.connector == "node-0").count(), 1);
        // A still-active connector is skipped without losing the older job's
        // priority, while a failed/skipped read already selected above rotates.
        active.clear();
        active.insert("node-0".into());
        let third = schedule(
            jobs.clone(),
            &mut active,
            &dispatching,
            &mut attempts,
            &mut sequence,
            31,
        );
        assert!(third.iter().all(|j| j.connector != "node-0"));
        active.clear();
        let fourth = schedule(
            jobs,
            &mut active,
            &dispatching,
            &mut attempts,
            &mut sequence,
            32,
        );
        assert!(fourth.iter().any(|j| j.id == "000"));
    }
    #[tokio::test]
    async fn legacy_http_node_receives_original_sync_and_no_job_is_created() {
        let fake=Router::new().route("/health",get(||async{Json(json!({"status":"ok"}))})).route("/v1/execute",post(|Json(v):Json<Value>|async move {
            assert_eq!(v["operation"],"sync");
            Json(json!({"requestId":v["requestId"],"status":200,"body":{"status":"succeeded","message":"legacy complete"}}))
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, fake).await.unwrap() });
        let registry =
            Registry::open(":memory:", "legacy-test-signing-key-01234567890123456789").unwrap();
        let node = registry
            .create(registry::ConnectorInput {
                name: "legacy".into(),
            })
            .unwrap();
        registry
            .register_endpoint(&node.connector_id, &registry.token(&node), Some(&url))
            .unwrap();
        registry
            .bind(Platform::Ios, "com.example.legacy", &node.connector_id)
            .unwrap();
        let state = ApiState::new(
            registry,
            "legacy-admin-token-0123456789".into(),
            "legacy-client-token-0123456789".into(),
            "http://127.0.0.1:8200".into(),
            PathBuf::from("unused"),
            Duration::from_secs(2),
        )
        .unwrap();
        let id = Uuid::new_v4().to_string();
        let response=crate::write_store(State(state.clone()),Extension(logs::RequestContext(id.clone())),None,Path((Platform::Ios,"com.example.legacy".into(),"sync".into())),Json(json!({"operation":"update_app_metadata","locale":"en-US","syncScopes":["store_images"],"metadata":{"storeImages":{"phone_screenshots":{"urls":["https://example.com/screen.png"]}}}}))).await.ok().unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(state
            .registry
            .store_job(&id, None, Platform::Ios, "com.example.legacy")
            .is_err());
        server.abort();
        let _ = server.await;
    }
    #[tokio::test]
    async fn reopened_jobs_query_only_and_healthy_node_finishes_while_other_node_waits() {
        use axum::routing::post;
        let stalled = Arc::new(tokio::sync::Notify::new());
        let started = Arc::new(tokio::sync::Notify::new());
        let (wait, signal) = (stalled.clone(), started.clone());
        let slow = Router::new().route(
            "/v1/execute",
            post(move |Json(v): Json<Value>| {
                let (wait, signal) = (wait.clone(), signal.clone());
                async move {
                    assert_eq!(v["operation"], "store_image_sync_status");
                    signal.notify_one();
                    wait.notified().await;
                    Json(json!({}))
                }
            }),
        );
        let healthy=Router::new().route("/v1/execute",post(|Json(v):Json<Value>|async move {
            assert_eq!(v["operation"],"store_image_sync_status");
            Json(json!({"requestId":v["requestId"],"status":200,"body":{"jobId":v["payload"]["jobId"],"status":"unknown","result":{"error":{"code":"node_task_cancelled"}},"httpStatus":503,"errorCode":"node_task_cancelled","outcomeUnknown":true}}))
        }));
        let slow_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let good_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let slow_url = format!("http://{}", slow_listener.local_addr().unwrap());
        let good_url = format!("http://{}", good_listener.local_addr().unwrap());
        let slow_server =
            tokio::spawn(async move { axum::serve(slow_listener, slow).await.unwrap() });
        let good_server =
            tokio::spawn(async move { axum::serve(good_listener, healthy).await.unwrap() });
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jobs.sqlite");
        let key = "center-regression-signing-key-01234567890123456789";
        let good_id = Uuid::new_v4().to_string();
        let slow_id = Uuid::new_v4().to_string();
        {
            let registry = Registry::open(&path, key).unwrap();
            for (id, url) in [(&slow_id, &slow_url), (&good_id, &good_url)] {
                let node = registry
                    .create(registry::ConnectorInput { name: id.clone() })
                    .unwrap();
                registry
                    .register_endpoint(&node.connector_id, &registry.token(&node), Some(url))
                    .unwrap();
                registry
                    .insert_store_job(&StoreJob {
                        id: id.clone(),
                        owner: None,
                        platform: Platform::Ios,
                        app: "com.example.job".into(),
                        connector: node.connector_id,
                        created: now(),
                        snapshot: snapshot(id, "dispatching"),
                    })
                    .unwrap();
            }
        }
        let state = ApiState::new(
            Registry::open(&path, key).unwrap(),
            "regression-admin-token-0123456789".into(),
            "regression-client-token-0123456789".into(),
            "http://127.0.0.1:8200".into(),
            PathBuf::from("unused"),
            Duration::from_secs(60),
        )
        .unwrap();
        let public = read(
            State(state.clone()),
            None,
            Path((Platform::Ios, "com.example.job".into(), good_id.clone())),
        )
        .await
        .ok()
        .unwrap()
        .0;
        assert_eq!(public["status"], "unknown");
        assert_eq!(public["outcomeUnknown"], true);
        assert_eq!(
            state
                .registry
                .store_job(&good_id, None, Platform::Ios, "com.example.job")
                .unwrap()["status"],
            "dispatching"
        );
        let observer = tokio::spawn(run(state.clone()));
        tokio::time::timeout(Duration::from_secs(2), async {
            started.notified().await;
            loop {
                let result = state
                    .registry
                    .store_job(&good_id, None, Platform::Ios, "com.example.job")
                    .unwrap();
                if result["errorCode"] == "node_task_cancelled" {
                    assert_eq!(result["httpStatus"], 503);
                    assert_eq!(result["outcomeUnknown"], true);
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            state
                .registry
                .store_job(&slow_id, None, Platform::Ios, "com.example.job")
                .unwrap()["status"],
            "dispatching"
        );
        // The deadline sweep must proceed while the slow network read is pending.
        state
            .registry
            .db
            .lock()
            .unwrap()
            .execute(
                "UPDATE store_image_jobs SET created=?1 WHERE id=?2",
                rusqlite::params![now() - 86401, slow_id],
            )
            .unwrap();
        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(5)).await;
        tokio::task::yield_now().await;
        assert_eq!(
            state
                .registry
                .store_job(&slow_id, None, Platform::Ios, "com.example.job")
                .unwrap()["status"],
            "unknown"
        );
        tokio::time::resume();
        observer.abort();
        let _ = observer.await;
        stalled.notify_waiters();
        slow_server.abort();
        good_server.abort();
        let _ = slow_server.await;
        let _ = good_server.await;
    }
    #[test]
    fn validates_identity_state_and_bounded_terminal_result() {
        assert!(valid_snapshot(&snapshot("id", "queued"), "id"));
        assert!(!valid_snapshot(&snapshot("another", "queued"), "id"));
        assert!(!valid_snapshot(&snapshot("id", "arbitrary"), "id"));
        let mut uncertain = snapshot("id", "unknown");
        assert!(valid_snapshot(&uncertain, "id"));
        uncertain["outcomeUnknown"] = json!(false);
        assert!(!valid_snapshot(&uncertain, "id"));
        let mut value = snapshot("id", "succeeded");
        assert!(!valid_snapshot(&value, "id"));
        value["result"] = json!({"status":"succeeded"});
        value["httpStatus"] = json!(200);
        assert!(valid_snapshot(&value, "id"));
        value["result"] = json!({"message":"a".repeat(65536)});
        assert!(!valid_snapshot(&value, "id"));
    }
    #[test]
    fn accepts_node_uncertainty_with_original_result_and_status() {
        let mut value = json!({"jobId":"id","status":"unknown","result":{"error":{"code":"node_task_cancelled","message":"Outcome unknown"}},"httpStatus":503,"errorCode":"node_task_cancelled","outcomeUnknown":true});
        assert!(valid_snapshot(&value, "id"));
        value["outcomeUnknown"] = json!(false);
        assert!(!valid_snapshot(&value, "id"));
        value["outcomeUnknown"] = json!(true);
        value["httpStatus"] = json!(1000);
        assert!(!valid_snapshot(&value, "id"));
    }
}
