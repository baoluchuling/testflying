use axum::{body::Body, http::Request};
use http_body_util::BodyExt;
use serde_json::Value;
use std::{fs, io::Cursor, path::PathBuf, time::Duration};
use store_api::{registry::Registry, ApiState};
use tower::ServiceExt;

fn app() -> axum::Router {
    app_with_release_root(PathBuf::from("unused-releases"))
}

fn app_with_release_root(release_root: PathBuf) -> axum::Router {
    store_api::router(
        ApiState::new(
            Registry::open(":memory:", "setup-signing-test-01234567890123456789").unwrap(),
            "setup-admin-test-0123456789".into(),
            "setup-client-test-0123456789".into(),
            "https://store.example.invalid/store-api".into(),
            release_root,
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
    assert!(!text.contains("adminToken"));
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
async fn setup_api_creates_and_downloads_nodes_without_exposing_other_management_operations() {
    let releases = tempfile::tempdir().unwrap();
    fs::create_dir(releases.path().join("windows-amd64")).unwrap();
    fs::write(
        releases.path().join("windows-amd64/store-connector.exe"),
        b"test executable",
    )
    .unwrap();
    let app = app_with_release_root(releases.path().to_path_buf());
    let send = |method: &'static str, path: String, body: &'static str| {
        app.clone().oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
    };
    let setup = "/connector-setup/api/connectors";
    let response = send("POST", setup.into(), r#"{"name":"Setup test node"}"#)
        .await
        .unwrap();
    assert_eq!(response.status(), 201);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let created: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let id = created["connector"]["connectorId"].as_str().unwrap();

    let response = send("GET", setup.into(), "").await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let listed: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(listed["connectors"][0]["connectorId"], id);

    let installer = format!("{id}/installer/windows-amd64");
    let response = send("GET", format!("{setup}/{installer}"), "")
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let mut zip = zip::ZipArchive::new(Cursor::new(
        response.into_body().collect().await.unwrap().to_bytes(),
    ))
    .unwrap();
    let config: Value = serde_json::from_reader(zip.by_name("connector.json").unwrap()).unwrap();
    assert_eq!(config["connectorId"], id);
    assert_eq!(config["token"], created["installation"]["token"]);

    for (method, path, expected) in [
        ("POST", "/v1/connectors".to_owned(), 401),
        ("GET", format!("/v1/connectors/{installer}"), 401),
        ("PATCH", format!("/v1/connectors/{id}"), 401),
        ("PATCH", format!("{setup}/{id}"), 404),
    ] {
        assert_eq!(send(method, path, "").await.unwrap().status(), expected);
    }
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
        ("install.cmd", "text/plain"),
        ("install.ps1", "text/plain"),
        ("install.sh", "text/plain"),
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
