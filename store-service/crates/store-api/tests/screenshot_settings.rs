use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    Router,
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::{path::PathBuf, time::Duration};
use store_api::{registry::Registry, ApiState};
use tower::ServiceExt;

const ADMIN: &str = "test-screenshot-settings-administrator";
const BUSINESS: &str = "test-screenshot-settings-business-token";

fn app() -> Router {
    store_api::router(
        ApiState::new(
            Registry::open(":memory:", "test-settings-encryption-key-0123456789").unwrap(),
            ADMIN.into(),
            BUSINESS.into(),
            "https://center.example.invalid/store-api".into(),
            PathBuf::from("unused"),
            Duration::from_secs(1),
        )
        .unwrap(),
    )
}
async fn send(
    app: &Router,
    method: &str,
    path: &str,
    token: &str,
    body: Value,
    options: (bool, Option<&str>, bool),
) -> axum::response::Response {
    let (https, cookie, csrf) = options;
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json");
    if https {
        req = req.header("x-forwarded-proto", "https");
    }
    if !token.is_empty() {
        req = req.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    if let Some(cookie) = cookie {
        req = req.header(header::COOKIE, cookie);
    }
    if csrf {
        req = req.header("x-testflying-setup", "1");
    }
    app.clone()
        .oneshot(req.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap()
}
async fn value(response: axum::response::Response) -> Value {
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}
#[tokio::test]
async fn destinations_are_https_admin_only_and_setup_session_keeps_secrets_write_only() {
    let app = app();
    for token in ["", BUSINESS] {
        assert_eq!(
            send(
                &app,
                "GET",
                "/v1/screenshot-settings",
                token,
                json!({}),
                (true, None, false)
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        send(
            &app,
            "GET",
            "/v1/screenshot-settings",
            ADMIN,
            json!({}),
            (false, None, false)
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    let initial = value(
        send(
            &app,
            "GET",
            "/v1/screenshot-settings",
            ADMIN,
            json!({}),
            (true, None, false),
        )
        .await,
    )
    .await;
    assert_eq!(initial["oss"]["enabled"], false);
    assert_eq!(initial["notification"]["status"], "not_configured");
    let login = send(
        &app,
        "POST",
        "/connector-setup/api/session",
        ADMIN,
        json!({}),
        (true, None, true),
    )
    .await;
    assert_eq!(login.status(), StatusCode::NO_CONTENT);
    let cookie = login.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let body = json!({"oss":{"enabled":true,"region":"cn-hangzhou","bucket":"fixture-screenshots","endpoint":"https://fixture-screenshots.oss-cn-hangzhou.aliyuncs.com","prefix":"screenshots","accessKeyId":"fixture-access-id","accessKeySecret":"fixture-private-oss-secret","securityToken":"fixture-private-sts-token"},"notification":{"url":"https://external.example.invalid/results","token":"fixture-private-callback-token"}});
    let saved = send(
        &app,
        "PUT",
        "/connector-setup/api/screenshot-settings",
        "",
        body,
        (true, Some(&cookie), true),
    )
    .await;
    assert_eq!(saved.status(), StatusCode::OK);
    let saved = value(saved).await;
    assert_eq!(saved["oss"]["accessKeySecretConfigured"], true);
    assert_eq!(saved["notification"]["status"], "configured");
    assert_eq!(saved["notification"]["tokenConfigured"], true);
    for secret in [
        "fixture-private-oss-secret",
        "fixture-private-sts-token",
        "fixture-private-callback-token",
    ] {
        assert!(!saved.to_string().contains(secret));
    }
    assert_eq!(
        send(
            &app,
            "GET",
            "/connector-setup/api/screenshot-settings",
            "",
            json!({}),
            (true, Some(&cookie), false)
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    let blank = send(
        &app,
        "PUT",
        "/v1/screenshot-settings",
        ADMIN,
        json!({"oss":{"accessKeySecret":"","securityToken":""},"notification":{"token":""}}),
        (true, None, false),
    )
    .await;
    assert_eq!(blank.status(), StatusCode::OK);
    let blank = value(blank).await;
    assert_eq!(blank["oss"]["accessKeySecretConfigured"], true);
    assert_eq!(blank["notification"]["tokenConfigured"], true);
    let changed = send(
        &app,
        "PUT",
        "/v1/screenshot-settings",
        ADMIN,
        json!({"oss":{"accessKeyId":"changed-without-secret"}}),
        (true, None, false),
    )
    .await;
    assert_eq!(changed.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let logs = value(
        send(
            &app,
            "GET",
            "/v1/logs",
            ADMIN,
            json!({}),
            (true, None, false),
        )
        .await,
    )
    .await;
    for secret in [
        "fixture-private-oss-secret",
        "fixture-private-sts-token",
        "fixture-private-callback-token",
    ] {
        assert!(!logs.to_string().contains(secret));
    }
    let asset = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/connector-setup/screenshot-settings.mjs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(asset.status(), StatusCode::OK);
    assert_eq!(asset.headers()[header::CACHE_CONTROL], "no-store");
}
