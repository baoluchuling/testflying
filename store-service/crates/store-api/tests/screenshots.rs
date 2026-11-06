use axum::{
    body::{Body, Bytes},
    http::{header, HeaderMap, Request, StatusCode},
    response::Response,
    routing::post,
    Json, Router,
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::{
    io::Cursor,
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use store_api::{registry::Registry, ApiState, ScreenshotUploader};
use tokio::sync::mpsc;
use tower::ServiceExt;

const ADMIN: &str = "test-screenshot-admin-token";
const BUSINESS: &str = "test-screenshot-business-token";

fn state() -> Arc<ApiState> {
    ApiState::new(
        Registry::open(":memory:", "test-screenshot-signing-key-0123456789").unwrap(),
        ADMIN.into(),
        BUSINESS.into(),
        "https://center.example.invalid/store-api".into(),
        PathBuf::from("unused-releases"),
        Duration::from_secs(1),
    )
    .unwrap()
}

async fn send(app: &Router, method: &str, path: &str, token: &str, body: Value) -> Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("x-forwarded-proto", "https")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn value(response: Response) -> Value {
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}

async fn node(app: &Router, name: &str, kind: &str) -> (String, String) {
    let response = send(
        app,
        "POST",
        "/v1/connectors",
        ADMIN,
        json!({"name":name,"kind":kind}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = value(response).await;
    assert_eq!(body["connector"]["kind"], kind);
    (
        body["connector"]["connectorId"].as_str().unwrap().into(),
        body["installation"]["token"].as_str().unwrap().into(),
    )
}

async fn integration(app: &Router, name: &str, nodes: &[&str]) -> String {
    let response = send(
        app,
        "POST",
        "/v1/integrations",
        ADMIN,
        json!({"name":name,"connectorIds":nodes}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    value(response).await["token"].as_str().unwrap().into()
}

#[tokio::test]
async fn jobs_are_claimed_once_and_their_png_is_forwarded_once() {
    let (messages, mut received) = mpsc::unbounded_channel();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/upload", listener.local_addr().unwrap());
    let attempts = Arc::new(AtomicUsize::new(0));
    let upstream = Router::new().route(
        "/upload",
        post(move |headers: HeaderMap, bytes: Bytes| {
            let messages = messages.clone();
            let attempts = attempts.clone();
            async move {
                messages.send((headers, bytes)).unwrap();
                if attempts.fetch_add(1, Ordering::Relaxed) == 0 {
                    (
                        StatusCode::OK,
                        Json(json!({"imageUrl":"https://cdn.example.invalid/screenshot.png","imageData":"never persisted"})),
                    )
                } else {
                    (StatusCode::SERVICE_UNAVAILABLE, Json(Value::Null))
                }
            }
        }),
    );
    let server = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
    let app = store_api::router_with_screenshots(
        state(),
        Some(ScreenshotUploader::new(&url, Some("upstream-only-test-token")).unwrap()),
    );
    let (mac, mac_token) = node(&app, "Screenshot Mac", "screenshot").await;
    let (store, store_token) = node(&app, "Store Connector", "store").await;
    let client = integration(&app, "screenshots", &[&store]).await;
    let peer = integration(&app, "same Mac, different integration", &[&mac]).await;
    let submit = "/v1/screenshot-jobs";
    let prompt = "  请打开设置并保留当前页面。\n  ";
    let payload = json!({"connectorId":mac,"profile":"ios-iphone","prompt":prompt});
    assert_eq!(
        send(&app, "POST", submit, BUSINESS, payload.clone())
            .await
            .status(),
        403
    );
    assert_eq!(
        send(
            &app,
            "POST",
            submit,
            &client,
            json!({"connectorId":store,"profile":"ios-iphone","prompt":prompt})
        )
        .await
        .status(),
        422
    );
    assert_eq!(
        send(
            &app,
            "PUT",
            "/v1/apps/ios/com.example.app/connector",
            ADMIN,
            json!({"connectorId":mac})
        )
        .await
        .status(),
        422
    );
    let store_list = value(
        send(
            &app,
            "GET",
            "/v1/store-management/connectors",
            &client,
            Value::Null,
        )
        .await,
    )
    .await;
    assert_eq!(store_list["connectors"].as_array().unwrap().len(), 1);
    let response = send(&app, "POST", submit, &client, payload.clone()).await;
    assert_eq!(response.status(), 202);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let job = value(response).await;
    let id = job["jobId"].as_str().unwrap();
    let status = format!("{submit}/{id}");
    assert_eq!(
        send(&app, "GET", &status, &peer, Value::Null)
            .await
            .status(),
        404
    );
    let prefix = format!("/v1/connectors/{mac}/screenshot-jobs");
    let claim = format!("{prefix}/claim");
    assert_eq!(
        send(&app, "POST", &claim, &store_token, Value::Null)
            .await
            .status(),
        401
    );
    let claim_reply = value(send(&app, "POST", &claim, &mac_token, Value::Null).await).await;
    assert_eq!(claim_reply["jobId"], id);
    assert_eq!(claim_reply["prompt"], prompt);
    assert_eq!(
        send(&app, "POST", &claim, &mac_token, Value::Null)
            .await
            .status(),
        204
    );
    assert_eq!(
        value(send(&app, "GET", &status, &client, Value::Null).await).await["status"],
        "running"
    );

    let mut png = Cursor::new(Vec::new());
    image::DynamicImage::new_rgba8(2, 2)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let png = png.into_inner();
    let upload = |job: &str, lease: &str| {
        app.clone().oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("{prefix}/{job}/image"))
                .header("x-forwarded-proto", "https")
                .header(header::AUTHORIZATION, format!("Bearer {mac_token}"))
                .header(header::CONTENT_TYPE, "image/png")
                .header("x-screenshot-lease", lease)
                .body(Body::from(png.clone()))
                .unwrap(),
        )
    };
    assert_eq!(
        upload(id, "99999999-9999-4999-8999-999999999999")
            .await
            .unwrap()
            .status(),
        409
    );
    let lease = claim_reply["lease"].as_str().unwrap();
    assert_eq!(upload(id, lease).await.unwrap().status(), 204);
    assert_eq!(upload(id, lease).await.unwrap().status(), 409);
    let (headers, body) = received.try_recv().unwrap();
    assert!(received.try_recv().is_err());
    assert_eq!(
        headers[header::AUTHORIZATION],
        "Bearer upstream-only-test-token"
    );
    assert_eq!(headers["idempotency-key"], id);
    assert!(headers[header::CONTENT_TYPE]
        .to_str()
        .unwrap()
        .starts_with("multipart/form-data;"));
    assert!(body.windows(png.len()).any(|window| window == png));
    let status = value(send(&app, "GET", &status, &client, Value::Null).await).await;
    assert_eq!(status["status"], "succeeded");
    assert_eq!(
        status["result"]["imageUrl"],
        "https://cdn.example.invalid/screenshot.png"
    );
    assert!(status.get("prompt").is_none() && status.get("lease").is_none());
    assert!(status["result"].get("imageData").is_none());

    let failed_job = value(send(&app, "POST", submit, &client, payload).await).await;
    let failed_id = failed_job["jobId"].as_str().unwrap();
    let claimed = value(send(&app, "POST", &claim, &mac_token, Value::Null).await).await;
    assert_eq!(
        upload(failed_id, claimed["lease"].as_str().unwrap())
            .await
            .unwrap()
            .status(),
        502
    );
    let failed_status = format!("{submit}/{failed_id}");
    let failed = value(send(&app, "GET", &failed_status, &client, Value::Null).await).await;
    assert_eq!(failed["status"], "failed");
    assert_eq!(failed["error"], "upstream_rejected");
    assert_eq!(
        send(&app, "GET", "/v1/screenshot-artifacts", ADMIN, Value::Null)
            .await
            .status(),
        404
    );
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn installer_is_an_https_admin_only_mac_package_for_the_selected_node() {
    let app = store_api::router(state());
    let (mac, token) = node(&app, "Mac", "screenshot").await;
    let (store, _) = node(&app, "Store", "store").await;
    let client = integration(&app, "external", &[&mac]).await;
    let url = format!("/v1/connectors/{mac}/screenshot-installer");
    for non_admin in ["", &token, &client] {
        assert_eq!(
            send(&app, "GET", &url, non_admin, Value::Null)
                .await
                .status(),
            401
        );
    }
    assert_eq!(
        send(
            &app,
            "GET",
            &format!("/v1/connectors/{store}/screenshot-installer"),
            ADMIN,
            Value::Null
        )
        .await
        .status(),
        422
    );
    let plain = app
        .clone()
        .oneshot(
            Request::get(&url)
                .header(header::AUTHORIZATION, format!("Bearer {ADMIN}"))
                .header("x-forwarded-proto", "http")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(plain.status(), 403);
    let response = send(&app, "GET", &url, ADMIN, Value::Null).await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/zip");
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    assert_eq!(zip.len(), 7);
    {
        let config = zip.by_name("connector.json").unwrap();
        assert_eq!(config.unix_mode().unwrap() & 0o777, 0o600);
        let config: Value = serde_json::from_reader(config).unwrap();
        assert_eq!(config["kind"], "screenshot");
        assert_eq!(config["connectorId"], mac);
        assert_eq!(config["token"], token);
        assert_eq!(
            config["serverUrl"],
            "https://center.example.invalid/store-api"
        );
        assert_eq!(config["profiles"], json!({}));
    }
    for name in [
        "run.sh",
        "README.md",
        ".codex/artifacts.toml",
        "testflying-prod-ca.crt",
        "scripts/screenshot_connector.py",
        "scripts/managed_workspace.py",
    ] {
        assert!(zip.by_name(name).is_ok(), "{name}");
    }
    assert_eq!(
        zip.by_name("run.sh").unwrap().unix_mode().unwrap() & 0o777,
        0o700
    );
}

#[tokio::test]
async fn jobs_require_https_and_a_configured_destination() {
    let app = store_api::router(state());
    let (mac, _) = node(&app, "Mac", "screenshot").await;
    let token = integration(&app, "external", &[&mac]).await;
    let payload = json!({"connectorId":mac,"profile":"android-tablet","prompt":"Open settings"});
    assert_eq!(
        send(&app, "POST", "/v1/screenshot-jobs", &token, payload.clone())
            .await
            .status(),
        503
    );
    let response = app
        .oneshot(
            Request::post("/v1/screenshot-jobs")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .header("x-forwarded-proto", "http")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 403);
    assert!(ScreenshotUploader::new("http://external.example.invalid/upload", None).is_err());
}
