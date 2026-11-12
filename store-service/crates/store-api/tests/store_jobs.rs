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
