use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    Router,
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::{
    io::{Cursor, Write},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use store_api::{
    registry::{ConnectorInput, ConnectorKind, Registry},
    ApiState,
};
use tower::ServiceExt;
use zip::{write::SimpleFileOptions, ZipWriter};
const ADMIN: &str = "skill-administrator-fixture-token";
const BUSINESS: &str = "skill-business-fixture-token";
fn fixture() -> (Router, Arc<ApiState>) {
    let state = ApiState::new(
        Registry::open(":memory:", "skill-fixture-signing-key-0123456789").unwrap(),
        ADMIN.into(),
        BUSINESS.into(),
        "https://center.example.invalid".into(),
        PathBuf::from("unused"),
        Duration::from_secs(1),
    )
    .unwrap();
    (store_api::router(state.clone()), state)
}
fn package() -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file("capture/SKILL.md", SimpleFileOptions::default())
        .unwrap();
    zip.write_all(b"---\nname: capture\ndescription: Capture\n---\nUse the current task.")
        .unwrap();
    zip.finish().unwrap().into_inner()
}
async fn send(
    app: &Router,
    method: &str,
    path: &str,
    token: &str,
    body: Vec<u8>,
    content_type: &str,
    options: (bool, Option<&str>, bool),
) -> axum::response::Response {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::CONTENT_TYPE, content_type);
    if options.0 {
        request = request.header("x-forwarded-proto", "https");
    }
    if !token.is_empty() {
        request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    if let Some(cookie) = options.1 {
        request = request.header(header::COOKIE, cookie);
    }
    if options.2 {
        request = request.header("x-testflying-setup", "1");
    }
    app.clone()
        .oneshot(request.body(Body::from(body)).unwrap())
        .await
        .unwrap()
}
async fn value(response: axum::response::Response) -> Value {
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}
#[tokio::test]
async fn admin_and_session_publish_rollback_and_worker_download_enforce_separate_authorities() {
    let (app, state) = fixture();
    for token in ["", BUSINESS] {
        assert_eq!(
            send(
                &app,
                "POST",
                "/v1/screenshot-skill",
                token,
                package(),
                "application/zip",
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
            "/v1/screenshot-skill",
            ADMIN,
            vec![],
            "application/json",
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
            "/v1/screenshot-skill",
            ADMIN,
            vec![],
            "application/json",
            (true, None, false),
        )
        .await,
    )
    .await;
    assert_eq!(initial, json!({"active":null,"versions":[]}));
    let login = send(
        &app,
        "POST",
        "/connector-setup/api/session",
        ADMIN,
        vec![],
        "application/json",
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
    assert_eq!(
        send(
            &app,
            "POST",
            "/connector-setup/api/screenshot-skill",
            "",
            package(),
            "application/zip",
            (true, Some(&cookie), false)
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    let published = send(
        &app,
        "POST",
        "/connector-setup/api/screenshot-skill",
        "",
        package(),
        "application/zip",
        (true, Some(&cookie), true),
    )
    .await;
    assert_eq!(published.status(), StatusCode::CREATED);
    assert_eq!(published.headers()[header::CACHE_CONTROL], "no-store");
    let published = value(published).await;
    let version = published["active"]["version"].as_str().unwrap();
    let repeat = value(
        send(
            &app,
            "POST",
            "/v1/screenshot-skill",
            ADMIN,
            package(),
            "application/zip",
            (true, None, false),
        )
        .await,
    )
    .await;
    assert_eq!(repeat, published);
    let selected = value(
        send(
            &app,
            "PUT",
            "/connector-setup/api/screenshot-skill",
            "",
            json!({"version":version}).to_string().into_bytes(),
            "application/json",
            (true, Some(&cookie), true),
        )
        .await,
    )
    .await;
    assert_eq!(selected, published);
    assert_eq!(
        send(
            &app,
            "PUT",
            "/v1/screenshot-skill",
            ADMIN,
            json!({"version":"0".repeat(64)}).to_string().into_bytes(),
            "application/json",
            (true, None, false)
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(
            &app,
            "POST",
            "/v1/screenshot-skill",
            ADMIN,
            package(),
            "application/json",
            (true, None, false)
        )
        .await
        .status(),
        StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
    assert_eq!(
        send(
            &app,
            "POST",
            "/v1/screenshot-skill",
            ADMIN,
            vec![0; 2 * 1024 * 1024 + 1],
            "application/zip",
            (true, None, false)
        )
        .await
        .status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
    let screenshot = state
        .registry
        .create_with_kind(
            ConnectorInput {
                name: "Skill worker".into(),
            },
            ConnectorKind::Screenshot,
        )
        .unwrap();
    let token = state.registry.token(&screenshot);
    let path = format!(
        "/v1/connectors/{}/screenshot-skill/{version}",
        screenshot.connector_id
    );
    for denied in [ADMIN, BUSINESS, ""] {
        assert_eq!(
            send(
                &app,
                "GET",
                &path,
                denied,
                vec![],
                "application/json",
                (true, None, false)
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    let integration = value(
        send(
            &app,
            "POST",
            "/v1/integrations",
            ADMIN,
            json!({"name":"Denied skill integration"})
                .to_string()
                .into_bytes(),
            "application/json",
            (true, None, false),
        )
        .await,
    )
    .await;
    assert_eq!(
        send(
            &app,
            "GET",
            &path,
            integration["token"].as_str().unwrap(),
            vec![],
            "application/json",
            (true, None, false)
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    let downloaded = send(
        &app,
        "GET",
        &path,
        &token,
        vec![],
        "application/json",
        (true, None, false),
    )
    .await;
    assert_eq!(downloaded.status(), StatusCode::OK);
    assert_eq!(
        downloaded.headers()[header::CONTENT_TYPE],
        "application/json"
    );
    let bytes = downloaded.into_body().collect().await.unwrap().to_bytes();
    let manifest: store_protocol::skill::SkillManifest =
        serde_json::from_value(published["active"].clone()).unwrap();
    assert_eq!(manifest.verify(&bytes).unwrap().len(), 1);
    let store = state
        .registry
        .create(ConnectorInput {
            name: "Store worker".into(),
        })
        .unwrap();
    let store_path = format!(
        "/v1/connectors/{}/screenshot-skill/{version}",
        store.connector_id
    );
    assert_eq!(
        send(
            &app,
            "GET",
            &store_path,
            &state.registry.token(&store),
            vec![],
            "application/json",
            (true, None, false)
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    state
        .registry
        .set_enabled(&screenshot.connector_id, false)
        .unwrap();
    assert_eq!(
        send(
            &app,
            "GET",
            &path,
            &token,
            vec![],
            "application/json",
            (true, None, false)
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    let asset = send(
        &app,
        "GET",
        "/connector-setup/skills.mjs",
        "",
        vec![],
        "application/json",
        (true, None, false),
    )
    .await;
    assert_eq!(asset.status(), StatusCode::OK);
}
