use axum::{body::Body, http::Request, routing::post, Router};
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
const ADMIN: &str = "forward-admin-test-token-0123456789";
const CLIENT: &str = "forward-client-test-token-0123456789";
const SIGNING: &str = "forward-signing-key-test-0123456789";
#[tokio::test]
async fn pausing_rejects_new_requests_but_does_not_cancel_a_dispatched_request() {
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let (received, continue_request) = (started.clone(), release.clone());
    let fake = Router::new().route(
        "/v1/execute",
        post(move |axum::Json(v): axum::Json<Value>| {
            let (received, continue_request) = (received.clone(), continue_request.clone());
            async move {
                received.notify_one();
                continue_request.notified().await;
                axum::Json(
                    json!({"requestId":v["requestId"],"status":200,"body":{"locales":["en-US"]}}),
                )
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, fake).await.unwrap();
    });
    let registry = Registry::open(":memory:", SIGNING).unwrap();
    let node = registry
        .create(ConnectorInput {
            name: "inflight".into(),
        })
        .unwrap();
    registry
        .bind(Platform::Ios, "com.example.app", &node.connector_id)
        .unwrap();
    registry
        .register_endpoint(&node.connector_id, &registry.token(&node), Some(&url))
        .unwrap();
    let state = ApiState::new(
        registry,
        ADMIN.into(),
        CLIENT.into(),
        "http://127.0.0.1:8200".into(),
        PathBuf::from("no-releases"),
        Duration::from_secs(3),
    )
    .unwrap();
    let app = store_api::router(state);
    let path = "/v1/store-management/apps/ios/com.example.app/supported-locales";
    let pending = tokio::spawn(call(app.clone(), "GET", path, CLIENT, Value::Null));
    tokio::time::timeout(Duration::from_secs(2), started.notified())
        .await
        .unwrap();
    assert_eq!(
        call(
            app.clone(),
            "PUT",
            "/v1/forwarding/ios",
            ADMIN,
            json!({"enabled":false})
        )
        .await
        .0,
        200
    );
    assert_eq!(call(app, "GET", path, CLIENT, Value::Null).await.0, 503);
    release.notify_one();
    let (status, body) = pending.await.unwrap();
    assert_eq!(status, 200);
    assert_eq!(body["locales"][0], "en-US");
    server.abort();
    let _ = server.await;
}
async fn call(app: Router, method: &str, path: &str, token: &str, body: Value) -> (u16, Value) {
    let r = app
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = r.status().as_u16();
    let bytes = r.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
#[tokio::test]
async fn independent_platform_controls_persist_and_stop_all_new_store_requests_and_uploads() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("registry.sqlite");
    let registry = Registry::open(&db, SIGNING).unwrap();
    let node = registry
        .create(ConnectorInput {
            name: "fixture".into(),
        })
        .unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let observed = count.clone();
    let fake = Router::new().route(
        "/v1/execute",
        post(move |axum::Json(v): axum::Json<Value>| {
            let observed = observed.clone();
            async move {
                observed.fetch_add(1, Ordering::SeqCst);
                axum::Json(json!({"requestId":v["requestId"],"status":200,"body":{"locales":[]}}))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, fake).await.unwrap();
    });
    registry
        .register_endpoint(&node.connector_id, &registry.token(&node), Some(&url))
        .unwrap();
    for p in [Platform::Ios, Platform::Android] {
        registry
            .bind(p, "com.example.app", &node.connector_id)
            .unwrap();
    }
    let state = ApiState::new(
        registry,
        ADMIN.into(),
        CLIENT.into(),
        "http://127.0.0.1:8200".into(),
        PathBuf::from("no-releases"),
        Duration::from_secs(2),
    )
    .unwrap();
    let app = store_api::router(state.clone());
    assert_eq!(
        call(
            app.clone(),
            "PUT",
            "/v1/forwarding/android",
            CLIENT,
            json!({"enabled":false})
        )
        .await
        .0,
        401
    );
    assert_eq!(
        call(
            app.clone(),
            "PUT",
            "/v1/forwarding/android",
            ADMIN,
            json!({"enabled":"false"})
        )
        .await
        .0,
        422
    );
    let android = "/v1/store-management/apps/android/com.example.app";
    let (_,upload)=call(app.clone(),"POST",&format!("{android}/uploads"),CLIENT,json!({"fileName":"s.png","contentType":"image/png","sizeBytes":1,"sha256":"00".repeat(32)})).await;
    assert!(upload["uploadToken"].is_string(), "{upload}");
    let (status, flags) = call(
        app.clone(),
        "PUT",
        "/v1/forwarding/android",
        ADMIN,
        json!({"enabled":false}),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(flags, json!({"ios":true,"android":false}));
    assert_eq!(
        call(
            app.clone(),
            "GET",
            "/v1/store-management/apps/ios/com.example.app/supported-locales",
            CLIENT,
            Value::Null
        )
        .await
        .0,
        200
    );
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(
        call(
            app.clone(),
            "PUT",
            "/v1/forwarding/ios",
            ADMIN,
            json!({"enabled":false})
        )
        .await
        .0,
        200
    );
    for p in ["ios", "android"] {
        let base = format!("/v1/store-management/apps/{p}/com.example.app");
        for resource in [
            "supported-locales",
            "store-listings",
            "store-images",
            "store-releases",
            "store-reviews",
            "product-page-optimizations",
        ] {
            let (status, body) = call(
                app.clone(),
                "GET",
                &format!("{base}/{resource}"),
                CLIENT,
                Value::Null,
            )
            .await;
            assert_eq!(status, 503);
            assert_eq!(body["error"]["code"], "platform_forwarding_disabled");
        }
        for resource in [
            "sync",
            "preflight",
            "product-page-optimizations",
            "store-releases",
        ] {
            assert_eq!(
                call(
                    app.clone(),
                    "POST",
                    &format!("{base}/{resource}"),
                    CLIENT,
                    json!({"versionString":"2.0"})
                )
                .await
                .0,
                503
            );
        }
        assert_eq!(call(app.clone(),"POST",&format!("{base}/uploads"),CLIENT,json!({"fileName":"s.png","contentType":"image/png","sizeBytes":1,"sha256":"00".repeat(32)})).await.0,503);
    }
    let put = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!(
                    "{android}/uploads/{}",
                    upload["assetId"].as_str().unwrap()
                ))
                .header(
                    "authorization",
                    format!("Bearer {}", upload["uploadToken"].as_str().unwrap()),
                )
                .body(Body::from("x"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(put.status(), 503);
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(
        call(app.clone(), "GET", "/v1/connectors", ADMIN, Value::Null)
            .await
            .0,
        200
    );
    assert_eq!(
        call(app, "GET", "/v1/logs", ADMIN, Value::Null).await.0,
        200
    );
    let persisted = Registry::open(&db, SIGNING).unwrap().forwarding().unwrap();
    assert!(!persisted.ios && !persisted.android);
    server.abort();
    let _ = server.await;
}
