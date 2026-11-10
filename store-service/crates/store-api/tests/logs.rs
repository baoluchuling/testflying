use axum::{body::Body, http::Request, routing::post, Json, Router};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::{path::PathBuf, sync::Arc, time::Duration};
use store_api::{
    logs::LogQuery,
    registry::{ConnectorInput, Registry},
    ApiState,
};
use store_protocol::{Platform, StoreReply, StoreRequest};
use tower::ServiceExt;

const ADMIN: &str = "log-test-admin-0123456789";
const CLIENT: &str = "log-test-client-0123456789";
fn state() -> Arc<ApiState> {
    ApiState::new(
        Registry::open(":memory:", "log-test-signing-key-01234567890123456789").unwrap(),
        ADMIN.into(),
        CLIENT.into(),
        "https://store.example.invalid".into(),
        PathBuf::from("nonexistent"),
        Duration::from_millis(100),
    )
    .unwrap()
}
async fn call(
    state: Arc<ApiState>,
    method: &str,
    path: &str,
    token: &str,
    body: Value,
) -> (u16, String, Value) {
    let response = store_api::router(state)
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .header("x-testflying-request-id", "untrusted-caller-id")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status().as_u16();
    let id = response.headers()["x-testflying-request-id"]
        .to_str()
        .unwrap()
        .to_owned();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        id,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
async fn node(state: &ApiState, app: Router) -> tokio::task::JoinHandle<()> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let node = state
        .registry
        .create(ConnectorInput {
            name: "log fixture".into(),
        })
        .unwrap();
    state
        .registry
        .register_endpoint(
            &node.connector_id,
            &state.registry.token(&node),
            Some(&format!("http://{}", listener.local_addr().unwrap())),
        )
        .unwrap();
    state
        .registry
        .bind(Platform::Ios, "com.cfs.amberia", &node.connector_id)
        .unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    })
}

#[tokio::test]
async fn failed_store_request_has_one_trace_and_safe_request_metadata() {
    let state = state();
    let fake=Router::new().route("/v1/execute",post(|Json(request):Json<StoreRequest>| async move {
        Json(StoreReply{request_id:request.request_id,status:502,body:json!({"error":{"code":"store_upstream_error","message":"Store upstream returned HTTP 400","private":"node-secret"}})})
    }));
    let server = node(&state, fake).await;
    let (status,id,body)=call(state.clone(),"POST","/v1/store-management/apps/ios/com.cfs.amberia/sync",CLIENT,
        json!({"operation":"update_app_metadata","version":"1.2.0","locale":"en-US","syncScopes":["title","description"],
            "metadata":{"title":"copy-secret","description":"description-secret"},"token":"request-secret"})).await;
    assert_eq!(status, 502);
    assert_ne!(id, "untrusted-caller-id");
    assert_eq!(body["error"]["requestId"], id);
    let result = state
        .registry
        .logs
        .query(&LogQuery {
            request_id: Some(id),
            ..Default::default()
        })
        .unwrap();
    let entries = result["entries"].as_array().unwrap();
    for expected in [
        "http.started",
        "store.request",
        "connector.dispatch",
        "connector.result",
        "http.completed",
    ] {
        assert!(
            entries.iter().any(|entry| entry["event"] == expected),
            "Missing {expected}"
        );
    }
    let submitted = entries
        .iter()
        .find(|entry| entry["event"] == "store.request")
        .unwrap();
    assert_eq!(submitted["details"]["request"]["locale"], "en-US");
    assert_eq!(
        submitted["details"]["request"]["syncScopes"],
        json!(["title", "description"])
    );
    assert_eq!(
        submitted["details"]["request"]["textFields"]["title"]["characters"],
        11
    );
    let returned = entries
        .iter()
        .find(|entry| entry["event"] == "connector.result")
        .unwrap();
    assert_eq!(returned["details"]["error"]["upstreamStatus"], 400);
    for secret in [
        "copy-secret",
        "description-secret",
        "request-secret",
        "node-secret",
        ADMIN,
        CLIENT,
    ] {
        assert!(!result.to_string().contains(secret));
    }
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn logs_require_admin_and_record_rejections_and_bad_json() {
    let state = state();
    let (status, id, body) = call(state.clone(), "GET", "/v1/logs", CLIENT, Value::Null).await;
    assert_eq!(status, 401);
    assert_eq!(body["error"]["requestId"], id);
    let (status, _, body) = call(
        state.clone(),
        "GET",
        "/v1/logs?errorsOnly=true",
        ADMIN,
        Value::Null,
    )
    .await;
    assert_eq!(status, 200);
    assert!(body["entries"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["status"] == 401));
    assert_eq!(
        call(
            state.clone(),
            "GET",
            "/v1/logs?limit=201",
            ADMIN,
            Value::Null
        )
        .await
        .0,
        422
    );
    assert_eq!(
        call(
            state.clone(),
            "GET",
            "/v1/logs?unknown=secret",
            ADMIN,
            Value::Null
        )
        .await
        .0,
        400
    );
    let (status, id, _) = call(
        state.clone(),
        "POST",
        "/v1/store-management/apps/ios/com.cfs.amberia/sync",
        CLIENT,
        json!("not-an-object"),
    )
    .await;
    assert_eq!(status, 404); // valid JSON reaches missing binding, and is still traceable
    assert!(state
        .registry
        .logs
        .query(&LogQuery {
            request_id: Some(id),
            ..Default::default()
        })
        .unwrap()["entries"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["event"] == "http.completed"));
    assert_eq!(
        call(
            state,
            "GET",
            "/connector-setup/api/logs",
            ADMIN,
            Value::Null
        )
        .await
        .0,
        403
    ); // HTTPS remains mandatory for setup
}

#[tokio::test]
async fn successful_and_timed_out_node_calls_are_both_logged_without_retry() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let state = state();
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let fake = Router::new().route(
        "/v1/execute",
        post(move |Json(request): Json<StoreRequest>| {
            let count = count.clone();
            async move {
                count.fetch_add(1, Ordering::Relaxed);
                if request.payload["version"] == "slow" {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                Json(StoreReply {
                    request_id: request.request_id,
                    status: 200,
                    body: json!({"canSync":false,"warnings":["sensitive-warning"]}),
                })
            }
        }),
    );
    let server = node(&state, fake).await;
    let (status, id, _) = call(
        state.clone(),
        "POST",
        "/v1/store-management/apps/ios/com.cfs.amberia/preflight",
        CLIENT,
        json!({}),
    )
    .await;
    assert_eq!(status, 200);
    let events = state
        .registry
        .logs
        .query(&LogQuery {
            request_id: Some(id),
            ..Default::default()
        })
        .unwrap();
    let result = events["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["event"] == "connector.result")
        .unwrap();
    assert_eq!(result["details"]["response"]["canSync"], false);
    assert!(!events.to_string().contains("sensitive-warning"));
    let (status, id, _) = call(
        state.clone(),
        "POST",
        "/v1/store-management/apps/ios/com.cfs.amberia/sync",
        CLIENT,
        json!({"version":"slow"}),
    )
    .await;
    assert_eq!(status, 504);
    let events = state
        .registry
        .logs
        .query(&LogQuery {
            request_id: Some(id),
            ..Default::default()
        })
        .unwrap();
    assert!(events["entries"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["details"]["error"]["code"] == "connector_timeout"));
    assert_eq!(calls.load(Ordering::Relaxed), 2);
    server.abort();
    let _ = server.await;
}
