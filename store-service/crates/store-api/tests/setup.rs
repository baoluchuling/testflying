use axum::{body::Body, http::Request};
use http_body_util::BodyExt;
use std::{path::PathBuf, time::Duration};
use store_api::{registry::Registry, ApiState};
use tower::ServiceExt;

fn app() -> axum::Router {
    store_api::router(
        ApiState::new(
            Registry::open(":memory:", "setup-signing-test-01234567890123456789").unwrap(),
            "setup-admin-test-0123456789".into(),
            "setup-client-test-0123456789".into(),
            "https://store.example.invalid/store-api".into(),
            PathBuf::from("unused-releases"),
            Duration::from_secs(1),
        )
        .unwrap(),
    )
}

#[tokio::test]
async fn setup_page_is_public_without_exposing_authentication_or_external_scripts() {
    let response = app()
        .oneshot(
            Request::builder()
                .uri("/connector-setup/")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let csp = response.headers()["content-security-policy"]
        .to_str()
        .unwrap();
    assert!(csp.contains("script-src 'self'"));
    assert!(csp.contains("frame-ancestors 'none'"));
    assert!(!csp.contains("unsafe-inline"));
    assert!(!csp.contains("unsafe-eval"));
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(text.contains("appleEnabled") && text.contains("googleEnabled"));
    assert!(!text.contains("setup-admin-test-0123456789"));
    assert!(!text.contains("store.example.invalid"));
    assert!(!text.contains("src=\"http"));
    let private = app()
        .oneshot(
            Request::builder()
                .uri("/v1/connectors")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(private.status(), 401);
}

#[tokio::test]
async fn setup_assets_have_explicit_types_and_cannot_read_arbitrary_files() {
    for (file, kind) in [
        ("app.mjs", "text/javascript"),
        ("packager.mjs", "text/javascript"),
        ("jszip.min.js", "text/javascript"),
        ("style.css", "text/css"),
        ("ca.crt", "application/x-pem-file"),
        ("start.cmd", "text/plain"),
        ("start.sh", "text/plain"),
        ("license.txt", "text/plain"),
    ] {
        let response = app()
            .oneshot(
                Request::builder()
                    .uri(format!("/connector-setup/{file}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200, "{file}");
        assert!(response.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with(kind));
        assert_eq!(response.headers()["x-content-type-options"], "nosniff");
        assert_eq!(response.headers()["referrer-policy"], "no-referrer");
        assert_eq!(response.headers()["cache-control"], "no-store");
    }
    for file in [
        "connector.json",
        "service.env",
        "%2e%2e%2fsecret",
        "unknown.js",
    ] {
        let response = app()
            .oneshot(
                Request::builder()
                    .uri(format!("/connector-setup/{file}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 404, "{file}");
    }
}

#[tokio::test]
async fn setup_redirect_keeps_the_proxy_prefix_relative() {
    let response = app()
        .oneshot(
            Request::builder()
                .uri("/connector-setup")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 307);
    assert_eq!(response.headers()["location"], "./connector-setup/");
    assert_eq!(response.headers()["cache-control"], "no-store");
}
