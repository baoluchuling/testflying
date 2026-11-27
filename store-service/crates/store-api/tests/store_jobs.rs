mod support;
use axum::{
    body::Body,
    http::Request,
    routing::{get, post},
    Router,
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use store_api::{
    registry::{ConnectorInput, Registry},
    ApiState,
};
use store_protocol::Platform;
use tower::ServiceExt;

const TOKEN: &str = "jobs-business-token-0123456789";
const SIGNING: &str = "jobs-node-signing-key-01234567890123456789";
const APP: &str = "com.example.jobs";

#[tokio::test]
async fn platform_batch_requires_new_node_then_preserves_payload_and_uses_existing_job_query() {
    for platform in [Platform::Ios, Platform::Android] {
        let received = Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
        let sink = received.clone();
        let fake=Router::new().route("/health",get(||async{axum::Json(json!({"asyncStoreImagesSupported":true}))}))
        .route("/v1/execute",post(move|axum::Json(v):axum::Json<Value>|{let sink=sink.clone();async move{
            sink.lock().unwrap().push(v.clone());
            axum::Json(json!({"requestId":v["requestId"],"status":200,"body":{"status":"succeeded","jobId":v["requestId"],"executionStatus":"queued"}}))
        }}));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, fake).await.unwrap() });
        let registry = Registry::open(":memory:", SIGNING).unwrap();
        let node = registry
            .create(ConnectorInput {
                name: "batch".into(),
            })
            .unwrap();
        support::catalog(&registry, &node.connector_id, platform, &[APP]);
        registry.bind(platform, APP, &node.connector_id).unwrap();
        registry
            .register_connection(
                &node.connector_id,
                &registry.token(&node),
                Some(&url),
                Some("0.2.8"),
            )
            .unwrap();
        let state = ApiState::new(
            registry,
            "jobs-admin-token-0123456789".into(),
            TOKEN.into(),
            "http://127.0.0.1:8200".into(),
            PathBuf::from("unused"),
            Duration::from_secs(2),
        )
        .unwrap();
        let app = store_api::router(state.clone());
        let path = format!("/v1/store-management/apps/{platform}/{APP}/sync");
        let payload = json!({"operation":"update_app_metadata","localizations":[{"locale":"en-US","metadata":{"title":"English"}},{"locale":"ja","metadata":{"title":"Japanese"}}]});
        let (status, body) = call(app.clone(), "POST", &path, payload.clone()).await;
        assert_eq!(status, 409);
        assert_eq!(body["error"]["code"], "store_batch_upgrade_required");
        assert!(received.lock().unwrap().is_empty());
        for resource in ["sync", "preflight"] {
            for removed in [
                json!({"operation":"update_release_notes","locale":"en-US","releaseNotes":"Removed"}),
                json!({"operation":"update_app_metadata","locale":"en-US","releaseNotes":"Wrong level","metadata":{"description":"Copy"}}),
            ] {
                let (status, error) = call(
                    app.clone(),
                    "POST",
                    &path.replace("/sync", &format!("/{resource}")),
                    removed,
                )
                .await;
                assert_eq!(status, 422);
                assert_eq!(error["error"]["code"], "invalid_store_payload");
            }
        }
        assert!(received.lock().unwrap().is_empty());
        state
            .registry
            .register_connection(
                &node.connector_id,
                &state.registry.token(&node),
                Some(&url),
                Some("0.2.9"),
            )
            .unwrap();
        let mut invalid = payload.clone();
        let mut notes = payload.clone();
        notes["localizations"][0]["metadata"]["releaseNotes"] = json!("Notes");
        for candidate in [
            notes.clone(),
            json!({"operation":"update_app_metadata","locale":"en-US","metadata":{"releaseNotes":"Notes"}}),
        ] {
            let (status, error) = call(app.clone(), "POST", &path, candidate).await;
            assert_eq!(status, 409);
            assert_eq!(
                error["error"]["code"],
                "store_release_notes_upgrade_required"
            );
        }
        let mut invalid_notes = notes.clone();
        invalid_notes["localizations"][0]["metadata"]["releaseNotes"] = json!(123);
        assert_eq!(call(app.clone(), "POST", &path, invalid_notes).await.0, 422);
        invalid["localizations"][1]["locale"] = json!("EN-us");
        assert_eq!(call(app.clone(), "POST", &path, invalid).await.0, 422);
        assert!(received.lock().unwrap().is_empty());
        let (status, body) = call(app.clone(), "POST", &path, payload.clone()).await;
        assert_eq!(status, 200);
        assert_eq!(body["executionStatus"], "queued");
        let requests = received.lock().unwrap().clone();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0]["operation"], "start_store_image_sync");
        assert_eq!(requests[0]["payload"], payload);
        let job = body["jobId"].as_str().unwrap();
        let query = format!("/v1/store-management/apps/{platform}/{APP}/sync-jobs/{job}");
        let (status, snapshot) = call(app.clone(), "GET", &query, Value::Null).await;
        assert_eq!(status, 200);
        assert_eq!(snapshot["jobId"], job);
        assert_eq!(snapshot["status"], "queued");
        state
            .registry
            .register_connection(
                &node.connector_id,
                &state.registry.token(&node),
                Some(&url),
                Some("0.2.10"),
            )
            .unwrap();
        let (status, result) = call(app.clone(), "POST", &path, notes.clone()).await;
        assert_eq!(status, 200, "{result}");
        assert_eq!(result["executionStatus"], "queued");
        assert_eq!(received.lock().unwrap().last().unwrap()["payload"], notes);
        let single = json!({"operation":"update_app_metadata","locale":"en-US","metadata":{"releaseNotes":"Single notes"}});
        assert_eq!(call(app, "POST", &path, single.clone()).await.0, 200);
        assert_eq!(received.lock().unwrap().last().unwrap()["payload"], single);
        server.abort();
    }
}

async fn call(app: Router, method: &str, path: &str, body: Value) -> (u16, Value) {
    let response = app
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("authorization", format!("Bearer {TOKEN}"))
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status().as_u16();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&body).unwrap())
}

#[tokio::test]
async fn accepts_then_observes_business_failure_and_isolates_app_and_platform() {
    let starts = Arc::new(AtomicUsize::new(0));
    let queries = Arc::new(AtomicUsize::new(0));
    let (s, q) = (starts.clone(), queries.clone());
    let fake=Router::new().route("/health",get(||async{axum::Json(json!({"asyncStoreImagesSupported":true}))})).route("/v1/execute",post(move|axum::Json(v):axum::Json<Value>|{
        let (s,q)=(s.clone(),q.clone());
        async move {
            let body=if v["operation"]=="start_store_image_sync" {
                s.fetch_add(1,Ordering::SeqCst);
                json!({"status":"succeeded","jobId":v["requestId"],"executionStatus":"queued","message":"Accepted for background execution","errorCode":null,"errorSummary":null,"storeState":null})
            } else {
                q.fetch_add(1,Ordering::SeqCst);
                json!({"jobId":v["payload"]["jobId"],"status":"succeeded","result":{"status":"failed","errorCode":"upstream_failure"},"httpStatus":200,"errorCode":"upstream_failure","outcomeUnknown":false})
            };
            axum::Json(json!({"requestId":v["requestId"],"status":200,"body":body}))
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, fake).await.unwrap() });
    let registry = Registry::open(":memory:", SIGNING).unwrap();
    let node = registry
        .create(ConnectorInput {
            name: "jobs".into(),
        })
        .unwrap();
    support::catalog(&registry, &node.connector_id, Platform::Ios, &[APP]);
    registry
        .bind(Platform::Ios, APP, &node.connector_id)
        .unwrap();
    registry
        .register_endpoint(&node.connector_id, &registry.token(&node), Some(&url))
        .unwrap();
    let state = ApiState::new(
        registry,
        "jobs-admin-token-0123456789".into(),
        TOKEN.into(),
        "http://127.0.0.1:8200".into(),
        PathBuf::from("unused"),
        Duration::from_secs(2),
    )
    .unwrap();
    let app = store_api::router(state.clone());
    let (code,accepted)=call(app.clone(),"POST",&format!("/v1/store-management/apps/ios/{APP}/sync"),json!({"operation":"update_app_metadata","locale":"en-US","syncScopes":["store_images"],"metadata":{"storeImages":{"phone_screenshots":{"urls":["https://example.com/screen.png"]}}}})).await;
    assert_eq!(code, 200, "{accepted}");
    assert_eq!(accepted["executionStatus"], "queued");
    let id = accepted["jobId"].as_str().unwrap();
    let path = format!("/v1/store-management/apps/ios/{APP}/sync-jobs/{id}");
    assert_eq!(
        call(app.clone(), "GET", &path, Value::Null).await.1["status"],
        "queued"
    );
    assert_eq!(
        call(
            app.clone(),
            "GET",
            &path.replace(APP, "com.example.other"),
            Value::Null
        )
        .await
        .0,
        404
    );
    assert_eq!(
        call(
            app.clone(),
            "GET",
            &path.replace("/ios/", "/android/"),
            Value::Null
        )
        .await
        .0,
        404
    );
    // Starting an observer after acceptance models a center restart: only a status read is sent.
    let observer = tokio::spawn(store_api::run_store_image_jobs(state));
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let (_, body) = call(app.clone(), "GET", &path, Value::Null).await;
            if body["status"] == "failed" {
                assert_eq!(body["httpStatus"], 200);
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(starts.load(Ordering::SeqCst), 1);
    assert_eq!(queries.load(Ordering::SeqCst), 1);
    observer.abort();
    let _ = observer.await;
    server.abort();
    let _ = server.await;
}
