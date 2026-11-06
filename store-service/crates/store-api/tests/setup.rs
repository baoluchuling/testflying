use axum::{body::Body, http::Request};
use hmac::{Hmac, Mac};
use http_body_util::BodyExt;
use serde_json::Value;
use std::{fs, io::Cursor, path::PathBuf, time::Duration};
use store_api::{registry::Registry, ApiState};
use tower::ServiceExt;

fn app() -> axum::Router {
    app_with_release_root(PathBuf::from("unused-releases"))
}

fn app_with_release_root(release_root: PathBuf) -> axum::Router {
    app_with_registry(
        Registry::open(":memory:", SIGNING_KEY).unwrap(),
        release_root,
    )
}

const SIGNING_KEY: &str = "setup-signing-test-01234567890123456789";

#[tokio::test(start_paused = true)]
async fn setup_session_is_scoped_to_https_and_can_be_revoked_or_expire() {
    let app = app();
    let session = "/connector-setup/api/session";
    let list = "/connector-setup/api/connectors";
    let admin = "setup-admin-test-0123456789";
    let send = |method: &str, uri: &str, scheme: &str, token: &str, cookie: &str, marked: bool| {
        let mut request = Request::builder()
            .method(method)
            .uri(uri)
            .header("x-forwarded-proto", scheme);
        if !token.is_empty() {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        if !cookie.is_empty() {
            request = request.header("cookie", cookie);
        }
        if marked {
            request = request.header("x-testflying-setup", "1");
        }
        app.clone().oneshot(request.body(Body::empty()).unwrap())
    };
    for (scheme, token, expected) in [
        ("http", admin, 403),
        ("https", "wrong-token", 401),
        ("https", "", 401),
    ] {
        let response = send("POST", session, scheme, token, "", true)
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        assert!(!response.headers().contains_key("set-cookie"));
    }
    let login = send("POST", session, "https", admin, "", true)
        .await
        .unwrap();
    assert_eq!(login.status(), 204);
    assert_eq!(login.headers()["cache-control"], "no-store");
    let set_cookie = login.headers()["set-cookie"].to_str().unwrap();
    assert!(set_cookie.ends_with("; Secure; HttpOnly; SameSite=Strict"));
    assert!(!set_cookie.contains(admin));
    let cookie = set_cookie.split(';').next().unwrap();
    for (route, scheme, marker, expected) in [
        (list, "https", true, 200),
        (list, "http", true, 403),
        (list, "https", false, 401),
        ("/connector-setup/api/integrations", "https", true, 200),
        ("/v1/integrations", "https", true, 401),
        ("/v1/connectors", "https", true, 401),
    ] {
        assert_eq!(
            send("GET", route, scheme, "", cookie, marker)
                .await
                .unwrap()
                .status(),
            expected
        );
    }
    assert_eq!(
        send("DELETE", session, "https", "", cookie, false)
            .await
            .unwrap()
            .status(),
        403
    );
    let logout = send("DELETE", session, "https", "", cookie, true)
        .await
        .unwrap();
    assert_eq!(logout.status(), 204);
    assert!(logout.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .ends_with("; Max-Age=0"));
    assert_eq!(
        send("GET", list, "https", "", cookie, true)
            .await
            .unwrap()
            .status(),
        401
    );

    let login = send("POST", session, "https", admin, "", true)
        .await
        .unwrap();
    let cookie = login.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap();
    tokio::time::advance(Duration::from_secs(8 * 60 * 60)).await;
    assert_eq!(
        send("GET", list, "https", "", cookie, true)
            .await
            .unwrap()
            .status(),
        401
    );
}

fn app_with_registry(registry: Registry, release_root: PathBuf) -> axum::Router {
    store_api::router(
        ApiState::new(
            registry,
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
async fn setup_api_requires_https_and_administrator_before_accessing_nodes() {
    let releases = tempfile::tempdir().unwrap();
    fs::create_dir(releases.path().join("windows-amd64")).unwrap();
    fs::write(
        releases.path().join("windows-amd64/store-connector.exe"),
        b"test executable",
    )
    .unwrap();
    let app = app_with_release_root(releases.path().to_path_buf());
    let request = |method: &'static str, path: String, body, scheme, token| {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json");
        if let Some(scheme) = scheme {
            request = request.header("x-forwarded-proto", scheme);
        }
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        app.clone().oneshot(request.body(Body::from(body)).unwrap())
    };
    let admin = "setup-admin-test-0123456789";
    let client = "tf_int_legacy-client-test-0123456789";
    let send = |method, path, body| request(method, path, body, Some("https"), Some(admin));
    let setup = "/connector-setup/api/connectors";
    let input = r#"{"name":"Setup test node"}"#;
    for (scheme, token, expected) in [
        (Some("https"), None, 401),
        (Some("https"), Some(client), 401),
        (Some("http"), Some(admin), 403),
        (None, Some(admin), 403),
    ] {
        for (method, body) in [("GET", ""), ("POST", input)] {
            let response = request(method, setup.into(), body, scheme, token)
                .await
                .unwrap();
            assert_eq!(response.status(), expected);
            assert_eq!(response.headers()["cache-control"], "no-store");
        }
    }
    let response = send("POST", setup.into(), input).await.unwrap();
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
    assert_eq!(listed["connectors"].as_array().unwrap().len(), 1);
    assert_eq!(listed["connectors"][0]["connectorId"], id);

    let installer = format!("{id}/installer/windows-amd64");
    let download = format!("{setup}/{installer}");
    for (scheme, token, expected) in [
        ("https", None, 401),
        ("https", Some(client), 401),
        ("http", Some(admin), 403),
    ] {
        let response = request("GET", download.clone(), "", Some(scheme), token)
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert_ne!(response.headers()["content-type"], "application/zip");
    }
    let response = send("GET", download, "").await.unwrap();
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
        assert_eq!(
            request(method, path, "", Some("https"), None)
                .await
                .unwrap()
                .status(),
            expected
        );
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
async fn new_and_existing_integration_tokens_access_all_nodes_without_reissuing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("registry.sqlite");
    let app = app_with_registry(
        Registry::open(&path, SIGNING_KEY).unwrap(),
        dir.path().into(),
    );
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
    for name in ["First", "Second", "Third"] {
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
    let legacy_id = "11111111-1111-4111-8111-111111111111";
    let db = rusqlite::Connection::open(path).unwrap();
    db.execute(
        "INSERT INTO integrations(id,name) VALUES(?1,'Previously issued')",
        [legacy_id],
    )
    .unwrap();
    db.execute(
        "INSERT INTO integration_connectors(integration_id,connector_id) VALUES(?1,?2)",
        rusqlite::params![legacy_id, ids[0]],
    )
    .unwrap();
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(SIGNING_KEY.as_bytes()).unwrap();
    mac.update(format!("testflying:integration:v1:{legacy_id}").as_bytes());
    let old_token = format!(
        "tf_int_{legacy_id}.{}",
        hex::encode(mac.finalize().into_bytes())
    );
    let endpoint = "/v1/integrations";
    let creation = serde_json::json!({"name":"Internal system"});
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
    assert_eq!(issued["integration"]["allConnectors"], true);
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
    assert!(listed["integrations"]
        .as_array()
        .unwrap()
        .iter()
        .all(|row| row.get("token").is_none() && row["allConnectors"] == true));
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
    let added = payload(
        send(
            "POST",
            "/v1/connectors",
            admin,
            "https",
            serde_json::json!({"name":"New after issuance"}),
        )
        .await
        .unwrap(),
    )
    .await;
    ids.push(added["connector"]["connectorId"].as_str().unwrap().into());
    for key in [token, &old_token] {
        let nodes = payload(send("GET", list, key, "https", Value::Null).await.unwrap()).await;
        let rows = nodes["connectors"].as_array().unwrap();
        assert_eq!(rows.len(), 4);
        for id in &ids {
            assert!(rows.iter().any(|row| row["connectorId"] == *id));
        }
        assert!(rows[0].get("baseUrl").is_none());
    }

    let own = "/v1/apps/ios/com.example.allowed/connector";
    let other = "/v1/apps/ios/com.example.other/connector";
    let node = |index: usize| serde_json::json!({"connectorId":ids[index]});
    for (path, key, index, expected) in [
        (other, admin, 2, 200),
        (other, &old_token, 3, 200),
        (own, token, 2, 200),
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
    assert_eq!(binding["connectorId"], ids[3]);
    assert_eq!(
        send("DELETE", own, token, "https", Value::Null)
            .await
            .unwrap()
            .status(),
        401
    );
    for (app_id, key) in [
        ("com.example.other", old_token.as_str()),
        ("com.example.allowed", token),
    ] {
        let path = format!("/v1/store-management/apps/ios/{app_id}/store-images");
        assert_eq!(
            send("GET", &path, key, "https", Value::Null)
                .await
                .unwrap()
                .status(),
            503
        );
    }
    let uploads = "/v1/store-management/apps/ios/com.example.other/uploads";
    let upload = serde_json::json!({"fileName":"test.png","contentType":"image/png","sizeBytes":10,"sha256":"a".repeat(64)});
    assert_eq!(
        send("POST", uploads, token, "https", upload)
            .await
            .unwrap()
            .status(),
        503
    );
    let disabled = format!("/v1/connectors/{}", ids[3]);
    assert_eq!(
        send(
            "PATCH",
            &disabled,
            admin,
            "https",
            serde_json::json!({"enabled":false})
        )
        .await
        .unwrap()
        .status(),
        200
    );
    assert_eq!(
        send("PUT", own, &old_token, "https", node(3))
            .await
            .unwrap()
            .status(),
        409
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
