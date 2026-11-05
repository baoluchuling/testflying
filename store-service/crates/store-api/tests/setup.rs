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
            "tf_int_legacy-client-test-0123456789".into(),
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
        ("integrations.mjs", "text/javascript"),
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
async fn integration_token_lists_granted_nodes_and_checks_bindings_business_and_revocation() {
    let app = app();
    let send = |method: &'static str, path: &str, token: &str, scheme: &str, body: Value| {
        app.clone().oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("authorization", format!("Bearer {token}"))
                .header("x-forwarded-proto", scheme)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
    };
    async fn payload(response: axum::response::Response) -> Value {
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
    }

    let admin = "setup-admin-test-0123456789";
    let client = "tf_int_legacy-client-test-0123456789";
    let mut ids = Vec::new();
    for name in ["Allowed one", "Allowed two", "Other"] {
        let node = payload(
            send(
                "POST",
                "/v1/connectors",
                admin,
                "https",
                serde_json::json!({"name":name}),
            )
            .await
            .unwrap(),
        )
        .await;
        ids.push(
            node["connector"]["connectorId"]
                .as_str()
                .unwrap()
                .to_owned(),
        );
    }
    let endpoint = "/v1/integrations";
    let creation = serde_json::json!({"name":"Internal system","connectorIds":[ids[0],ids[1]]});
    for (token, scheme, status) in [
        (client, "https", 401),
        ("", "https", 401),
        (admin, "http", 403),
    ] {
        let response = send("POST", endpoint, token, scheme, creation.clone())
            .await
            .unwrap();
        assert_eq!(response.status(), status);
    }
    let issued = send("POST", endpoint, admin, "https", creation)
        .await
        .unwrap();
    assert_eq!(issued.status(), 201);
    assert_eq!(issued.headers()["cache-control"], "no-store");
    let issued = payload(issued).await;
    let token = issued["token"].as_str().unwrap();
    let integration = format!(
        "{endpoint}/{}",
        issued["integration"]["id"].as_str().unwrap()
    );
    let reveal = format!("{integration}/token");
    let listed = payload(
        send("GET", endpoint, admin, "https", Value::Null)
            .await
            .unwrap(),
    )
    .await;
    assert!(listed["integrations"][0].get("token").is_none());
    assert_eq!(
        payload(
            send("GET", &reveal, admin, "https", Value::Null)
                .await
                .unwrap()
        )
        .await["token"],
        token
    );
    assert_eq!(
        send("GET", &reveal, token, "https", Value::Null)
            .await
            .unwrap()
            .status(),
        401
    );

    let list = "/v1/store-management/connectors";
    assert_eq!(
        send("GET", list, client, "http", Value::Null)
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        send("GET", list, token, "http", Value::Null)
            .await
            .unwrap()
            .status(),
        403
    );
    let nodes = payload(
        send("GET", list, token, "https", Value::Null)
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(nodes["connectors"].as_array().unwrap().len(), 2);
    assert_eq!(nodes["connectors"][0]["connectorId"], ids[0]);
    assert!(nodes["connectors"][0].get("baseUrl").is_none());

    let own = "/v1/apps/ios/com.example.allowed/connector";
    let other = "/v1/apps/ios/com.example.other/connector";
    let node = |index: usize| serde_json::json!({"connectorId":ids[index]});
    for (path, key, index, expected) in [
        (other, admin, 2, 200),
        (other, token, 0, 403),
        (own, token, 2, 403),
        (own, token, 0, 200),
        (own, token, 1, 200),
    ] {
        assert_eq!(
            send("PUT", path, key, "https", node(index))
                .await
                .unwrap()
                .status(),
            expected
        );
    }
    let binding = payload(
        send("GET", other, admin, "https", Value::Null)
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(binding["connectorId"], ids[2]);
    assert_eq!(
        send("DELETE", own, token, "https", Value::Null)
            .await
            .unwrap()
            .status(),
        401
    );
    for (app_id, status) in [("com.example.other", 403), ("com.example.allowed", 503)] {
        let path = format!("/v1/store-management/apps/ios/{app_id}/store-images");
        assert_eq!(
            send("GET", &path, token, "https", Value::Null)
                .await
                .unwrap()
                .status(),
            status
        );
    }
    let uploads = "/v1/store-management/apps/ios/com.example.other/uploads";
    let upload = serde_json::json!({"fileName":"test.png","contentType":"image/png","sizeBytes":10,"sha256":"a".repeat(64)});
    assert_eq!(
        send("POST", uploads, token, "https", upload)
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        send("DELETE", &integration, admin, "https", Value::Null)
            .await
            .unwrap()
            .status(),
        204
    );
    assert_eq!(
        send("GET", list, token, "https", Value::Null)
            .await
            .unwrap()
            .status(),
        401
    );
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
