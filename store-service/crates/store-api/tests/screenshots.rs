use axum::{
    body::{Body, Bytes},
    http::{header, HeaderMap, Request, StatusCode},
    response::Response,
    routing::post,
    Json, Router,
};
use futures_util::{SinkExt, StreamExt};
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
use store_protocol::crypto;
use tokio::sync::mpsc;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
    MaybeTlsStream, WebSocketStream,
};
use tower::ServiceExt;

const ADMIN: &str = "test-screenshot-admin-token";
const BUSINESS: &str = "test-screenshot-business-token";

fn state() -> Arc<ApiState> {
    state_with_releases(PathBuf::from("unused-releases"))
}

fn state_with_releases(release_root: PathBuf) -> Arc<ApiState> {
    state_with_database(release_root, ":memory:")
}

fn state_with_database(release_root: PathBuf, database: &str) -> Arc<ApiState> {
    ApiState::new(
        Registry::open(database, "test-screenshot-signing-key-0123456789").unwrap(),
        ADMIN.into(),
        BUSINESS.into(),
        "https://center.example.invalid/store-api".into(),
        release_root,
        Duration::from_secs(1),
    )
    .unwrap()
}

type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

async fn screenshot_socket(
    addr: std::net::SocketAddr,
    id: &str,
    token: &str,
    key: &str,
) -> (Socket, crypto::Channel) {
    let mut request = format!("ws://{addr}/v1/connectors/{id}/ws")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    request
        .headers_mut()
        .insert("x-forwarded-proto", "https".parse().unwrap());
    request.headers_mut().insert(
        "sec-websocket-protocol",
        crypto::SUBPROTOCOL.parse().unwrap(),
    );
    let mut socket = connect_async(request).await.unwrap().0;
    let mut handshake = crypto::handshake(key, id, true).unwrap();
    let mut buffer = vec![0; crypto::HANDSHAKE_BYTES];
    let registration = json!({"connectorId":id}).to_string();
    let size = handshake
        .write_message(registration.as_bytes(), &mut buffer)
        .unwrap();
    socket
        .send(Message::Binary(buffer[..size].to_vec().into()))
        .await
        .unwrap();
    let Message::Binary(reply) = socket.next().await.unwrap().unwrap() else {
        panic!("missing handshake")
    };
    handshake.read_message(&reply, &mut buffer).unwrap();
    let mut channel = crypto::Channel::new(handshake).unwrap();
    for record in channel.encrypt(b"confirm").unwrap() {
        socket.send(Message::Binary(record.into())).await.unwrap();
    }
    let Message::Binary(reply) = socket.next().await.unwrap().unwrap() else {
        panic!("missing ready")
    };
    assert_eq!(
        channel.decrypt(&reply).unwrap().as_deref(),
        Some(b"ready".as_slice())
    );
    (socket, channel)
}

async fn wait_for_screenshot_wake(
    socket: &mut Socket,
    channel: &mut crypto::Channel,
    seconds: u64,
) {
    tokio::time::timeout(Duration::from_secs(seconds), async {
        loop {
            match socket.next().await.unwrap().unwrap() {
                Message::Binary(bytes) => {
                    assert_eq!(
                        channel.decrypt(&bytes).unwrap().as_deref(),
                        Some(b"screenshot_job_available".as_slice())
                    );
                    break;
                }
                Message::Ping(bytes) => socket.send(Message::Pong(bytes)).await.unwrap(),
                other => panic!("Unexpected screenshot frame: {other:?}"),
            }
        }
    })
    .await
    .unwrap();
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

async fn bind_app(app: &Router, token: &str, node: &str) {
    let response = send(
        app,
        "PUT",
        "/v1/apps/ios/com.example.app/screenshot-connector",
        token,
        json!({"connectorId":node}),
    )
    .await;
    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn screenshot_discovery_and_app_bindings_route_precisely_without_touching_store_bindings() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("routing.sqlite");
    let state = state_with_database(PathBuf::from("unused-releases"), database.to_str().unwrap());
    let app = store_api::router(state.clone());
    let (mac_a, token_a) = node(&app, "Mac A", "screenshot").await;
    let (mac_b, _) = node(&app, "Mac B", "screenshot").await;
    let (disabled, _) = node(&app, "Disabled Mac", "screenshot").await;
    let (store, _) = node(&app, "Store", "store").await;
    let token = integration(&app, "routing", &[]).await;
    state.registry.set_enabled(&disabled, false).unwrap();
    assert_eq!(
        send(
            &app,
            "GET",
            "/v1/screenshot-connectors",
            BUSINESS,
            Value::Null
        )
        .await
        .status(),
        403
    );
    let listed = value(
        send(
            &app,
            "GET",
            "/v1/screenshot-connectors",
            &token,
            Value::Null,
        )
        .await,
    )
    .await;
    let nodes = listed["connectors"].as_array().unwrap();
    assert_eq!(nodes.len(), 2);
    assert!(nodes
        .iter()
        .all(|n| n["kind"] == "screenshot" && n["enabled"] == true));
    assert!(!listed.to_string().contains(&token_a));
    let ios = "/v1/apps/ios/com.example.App/screenshot-connector";
    let android = "/v1/apps/android/com.example.App/screenshot-connector";
    let payload = json!({"platform":"ios","bundleIdentifier":"com.example.App","prompt":"  完整截图要求\n  "});
    assert_eq!(
        send(&app, "POST", "/v1/screenshot-jobs", &token, payload.clone())
            .await
            .status(),
        404
    );
    for (id, status) in [(&store, 422), (&disabled, 409)] {
        assert_eq!(
            send(&app, "PUT", ios, &token, json!({"connectorId":id}))
                .await
                .status(),
            status
        );
    }
    assert_eq!(
        send(&app, "PUT", ios, BUSINESS, json!({"connectorId":mac_a}))
            .await
            .status(),
        401
    );
    assert_eq!(
        send(&app, "PUT", ios, &token, json!({"connectorId":mac_a}))
            .await
            .status(),
        200
    );
    assert_eq!(
        send(&app, "PUT", android, &token, json!({"connectorId":mac_b}))
            .await
            .status(),
        200
    );
    assert_eq!(
        send(
            &app,
            "PUT",
            "/v1/apps/ios/com.example.App/connector",
            ADMIN,
            json!({"connectorId":store})
        )
        .await
        .status(),
        200
    );
    let store_binding = value(
        send(
            &app,
            "GET",
            "/v1/apps/ios/com.example.App/connector",
            ADMIN,
            Value::Null,
        )
        .await,
    )
    .await;
    assert_eq!(store_binding["connectorId"], store);
    assert_eq!(
        value(send(&app, "GET", ios, &token, Value::Null).await).await["connector"]["connectorId"],
        mac_a
    );
    assert_eq!(
        send(
            &app,
            "GET",
            "/v1/apps/ios/com.example.app/screenshot-connector",
            &token,
            Value::Null
        )
        .await
        .status(),
        404
    );
    let first =
        value(send(&app, "POST", "/v1/screenshot-jobs", &token, payload.clone()).await).await;
    assert_eq!(first["connectorId"], mac_a);
    assert_eq!(first["platform"], "ios");
    let mut other = payload.clone();
    other["platform"] = json!("android");
    assert_eq!(
        value(send(&app, "POST", "/v1/screenshot-jobs", &token, other).await).await["connectorId"],
        mac_b
    );
    assert_eq!(
        send(&app, "PUT", ios, &token, json!({"connectorId":mac_b}))
            .await
            .status(),
        200
    );
    assert_eq!(
        value(send(&app, "POST", "/v1/screenshot-jobs", &token, payload.clone()).await).await
            ["connectorId"],
        mac_b
    );
    let persisted =
        state_with_database(PathBuf::from("unused-releases"), database.to_str().unwrap());
    let persisted = store_api::router(persisted);
    assert_eq!(
        value(send(&persisted, "GET", ios, &token, Value::Null).await).await["connector"]
            ["connectorId"],
        mac_b
    );
    let claim = value(
        send(
            &app,
            "POST",
            &format!("/v1/connectors/{mac_a}/screenshot-jobs/claim"),
            &token_a,
            Value::Null,
        )
        .await,
    )
    .await;
    assert_eq!(claim["jobId"], first["jobId"]);
    assert!(claim["prompt"]
        .as_str()
        .unwrap()
        .ends_with(payload["prompt"].as_str().unwrap()));
    assert!(claim["prompt"]
        .as_str()
        .unwrap()
        .contains("com.example.App"));
    assert_eq!(
        send(&app, "DELETE", ios, &token, Value::Null)
            .await
            .status(),
        204
    );
    assert_eq!(
        send(&app, "GET", ios, &token, Value::Null).await.status(),
        404
    );
    assert_eq!(
        send(&app, "POST", "/v1/screenshot-jobs", &token, payload.clone())
            .await
            .status(),
        404
    );
    assert_eq!(
        send(
            &app,
            "POST",
            "/v1/screenshot-jobs",
            &token,
            json!({"connectorId":mac_a,"prompt":"legacy"})
        )
        .await
        .status(),
        422
    );
    assert_eq!(
        send(&app, "PUT", ios, ADMIN, json!({"connectorId":mac_a}))
            .await
            .status(),
        200
    );
    state.registry.set_enabled(&mac_a, false).unwrap();
    assert_eq!(
        send(&app, "POST", "/v1/screenshot-jobs", &token, payload)
            .await
            .status(),
        503
    );
}

#[tokio::test]
async fn old_screenshot_jobs_survive_schema_migration_with_their_original_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("old.sqlite");
    rusqlite::Connection::open(&database).unwrap().execute_batch("CREATE TABLE screenshot_jobs(id TEXT PRIMARY KEY,connector_id TEXT NOT NULL,integration_id TEXT NOT NULL,profile TEXT NOT NULL,prompt TEXT NOT NULL,status TEXT NOT NULL,created_at INTEGER NOT NULL,deadline INTEGER NOT NULL,lease TEXT,result TEXT,error TEXT);").unwrap();
    let state = state_with_database(PathBuf::from("unused-releases"), database.to_str().unwrap());
    let app = store_api::router(state);
    let (mac, mac_token) = node(&app, "Legacy Mac", "screenshot").await;
    let token = integration(&app, "legacy", &[]).await;
    let integrations = value(send(&app, "GET", "/v1/integrations", ADMIN, Value::Null).await).await;
    let id = "00000000-0000-4000-8000-000000000001";
    let now = store_protocol::upload::now().unwrap() as i64;
    rusqlite::Connection::open(&database).unwrap().execute("INSERT INTO screenshot_jobs(id,connector_id,integration_id,profile,prompt,status,created_at,deadline) VALUES(?1,?2,?3,'','old prompt','queued',?4,?5)",rusqlite::params![id,mac,integrations["integrations"][0]["id"].as_str().unwrap(),now,now+60]).unwrap();
    let claimed = value(
        send(
            &app,
            "POST",
            &format!("/v1/connectors/{mac}/screenshot-jobs/claim"),
            &mac_token,
            Value::Null,
        )
        .await,
    )
    .await;
    assert_eq!(claimed["prompt"], "old prompt");
    assert_eq!(claimed["platform"], Value::Null);
    assert_eq!(
        value(
            send(
                &app,
                "GET",
                &format!("/v1/screenshot-jobs/{id}"),
                &token,
                Value::Null
            )
            .await
        )
        .await["status"],
        "running"
    );
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
    let database = tempfile::tempdir().unwrap();
    let database_path = database.path().join("registry.sqlite3");
    let state = state_with_database(
        PathBuf::from("unused-releases"),
        database_path.to_str().unwrap(),
    );
    let app = store_api::router_with_screenshots(
        state.clone(),
        Some(ScreenshotUploader::new(&url, Some("upstream-only-test-token")).unwrap()),
    );
    let (mac, mac_token) = node(&app, "Screenshot Mac", "screenshot").await;
    let installation = value(
        send(
            &app,
            "GET",
            &format!("/v1/connectors/{mac}/installation"),
            ADMIN,
            Value::Null,
        )
        .await,
    )
    .await;
    let message_key = installation["messageKey"].as_str().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let center_app = app.clone();
    let center = tokio::spawn(async move { axum::serve(listener, center_app).await.unwrap() });
    let (mut socket, mut channel) = screenshot_socket(addr, &mac, &mac_token, message_key).await;
    assert!(state.transport.connected(&mac));
    let (store, store_token) = node(&app, "Store Connector", "store").await;
    let client = integration(&app, "screenshots", &[&store]).await;
    bind_app(&app, &client, &mac).await;
    let peer = integration(&app, "same Mac, different integration", &[&mac]).await;
    let submit = "/v1/screenshot-jobs";
    let prompt = "  请打开设置并保留当前页面。\n  ";
    let payload = json!({"platform":"ios","bundleIdentifier":"com.example.app","profile":"ios-iphone","prompt":prompt});
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
    wait_for_screenshot_wake(&mut socket, &mut channel, 2).await;
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
    assert!(claim_reply["prompt"].as_str().unwrap().ends_with(prompt));
    assert!(claim_reply["prompt"]
        .as_str()
        .unwrap()
        .contains("com.example.app"));
    assert_eq!(claim_reply["platform"], "ios");
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

    socket.close(None).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while state.transport.connected(&mac) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let failed_job = value(send(&app, "POST", submit, &client, payload.clone()).await).await;
    let (mut socket, mut channel) = screenshot_socket(addr, &mac, &mac_token, message_key).await;
    wait_for_screenshot_wake(&mut socket, &mut channel, 2).await;
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

    let abandoned = value(send(&app, "POST", submit, &client, payload.clone()).await).await;
    wait_for_screenshot_wake(&mut socket, &mut channel, 2).await;
    let claimed = value(send(&app, "POST", &claim, &mac_token, Value::Null).await).await;
    assert_eq!(claimed["jobId"], abandoned["jobId"]);
    let next = send(
        &app,
        "POST",
        submit,
        &client,
        json!({"platform":"ios","bundleIdentifier":"com.example.app","prompt":prompt}),
    )
    .await;
    assert_eq!(next.status(), 202);
    let next = value(next).await;
    let next_status = format!("{submit}/{}", next["jobId"].as_str().unwrap());
    let pending = value(send(&app, "GET", &next_status, &client, Value::Null).await).await;
    assert_eq!(pending.get("profile"), Some(&Value::Null));
    wait_for_screenshot_wake(&mut socket, &mut channel, 2).await;
    assert_eq!(
        send(&app, "POST", &claim, &mac_token, Value::Null)
            .await
            .status(),
        204
    );
    rusqlite::Connection::open(database_path)
        .unwrap()
        .execute(
            "UPDATE screenshot_jobs SET deadline=0 WHERE id=?1",
            [abandoned["jobId"].as_str().unwrap()],
        )
        .unwrap();
    wait_for_screenshot_wake(&mut socket, &mut channel, 12).await;
    let claimed = value(send(&app, "POST", &claim, &mac_token, Value::Null).await).await;
    assert_eq!(claimed["jobId"], next["jobId"]);
    assert_eq!(claimed.get("profile"), Some(&Value::Null));
    assert!(claimed["prompt"].as_str().unwrap().ends_with(prompt));

    assert_eq!(
        send(&app, "GET", "/v1/screenshot-artifacts", ADMIN, Value::Null)
            .await
            .status(),
        404
    );
    server.abort();
    let _ = server.await;
    center.abort();
    let _ = center.await;
}

#[tokio::test]
async fn installer_is_an_https_admin_only_mac_package_for_the_selected_node() {
    let releases = tempfile::tempdir().unwrap();
    let mac_release = releases.path().join("darwin-arm64");
    std::fs::create_dir(&mac_release).unwrap();
    let executable = mac_release.join("screenshot-connector");
    let app = store_api::router(state_with_releases(releases.path().into()));
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
    for unsupported in [
        b"\xcf\xfa\xed\xfe\x07\0\0\x01".as_slice(),
        b"\xca\xfe\xba\xbe".as_slice(),
    ] {
        std::fs::write(&executable, unsupported).unwrap();
        assert_eq!(
            send(&app, "GET", &url, ADMIN, Value::Null).await.status(),
            503
        );
    }
    std::fs::write(&executable, b"\xcf\xfa\xed\xfe\x0c\0\0\x01fixture").unwrap();
    let response = send(&app, "GET", &url, ADMIN, Value::Null).await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/zip");
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    assert_eq!(zip.len(), 6);
    {
        let config = zip.by_name("connector.json").unwrap();
        assert_eq!(config.unix_mode().unwrap() & 0o777, 0o600);
        let config: Value = serde_json::from_reader(config).unwrap();
        assert_eq!(config["kind"], "screenshot");
        assert_eq!(config["connectorId"], mac);
        assert_eq!(config["token"], token);
        assert_eq!(config["messageKey"].as_str().unwrap().len(), 64);
        assert_eq!(
            config["serverUrl"],
            "https://center.example.invalid/store-api"
        );
        assert!(config.get("profiles").is_none() && config.get("projectRoots").is_none());
        assert!(config.get("codexPermissionProfile").is_none());
        assert!(config.get("codexPath").is_none() && config.get("codexHome").is_none());
    }
    for name in [
        "run.sh",
        "install.sh",
        "screenshot-connector",
        "README.md",
        "testflying-prod-ca.crt",
    ] {
        assert!(zip.by_name(name).is_ok(), "{name}");
    }
    assert_eq!(
        zip.by_name("run.sh").unwrap().unix_mode().unwrap() & 0o777,
        0o700
    );
}

#[tokio::test]
async fn jobs_run_without_a_destination_and_only_image_delivery_fails() {
    let app = store_api::router(state());
    let (mac, mac_token) = node(&app, "Mac", "screenshot").await;
    let token = integration(&app, "external", &[&mac]).await;
    bind_app(&app, &token, &mac).await;
    let payload =
        json!({"platform":"ios","bundleIdentifier":"com.example.app","prompt":"Open settings"});
    let submit = "/v1/screenshot-jobs";
    let response = app
        .clone()
        .oneshot(
            Request::post(submit)
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

    let response = send(&app, "POST", submit, &token, payload.clone()).await;
    assert_eq!(response.status(), 202);
    let job = value(response).await;
    let id = job["jobId"].as_str().unwrap();
    let prefix = format!("/v1/connectors/{mac}/screenshot-jobs");
    let claim = format!("{prefix}/claim");
    let response = send(&app, "POST", &claim, &mac_token, Value::Null).await;
    assert_eq!(response.status(), 200);
    let claimed = value(response).await;
    assert_eq!(claimed["jobId"], id);
    assert!(claimed["prompt"]
        .as_str()
        .unwrap()
        .ends_with("Open settings"));
    let job_status = format!("{submit}/{id}");
    let mut png = Cursor::new(Vec::new());
    image::DynamicImage::new_rgba8(2, 2)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let png = png.into_inner();
    let upload = |lease: &str| {
        app.clone().oneshot(
            Request::put(format!("{prefix}/{id}/image"))
                .header("x-forwarded-proto", "https")
                .header(header::AUTHORIZATION, format!("Bearer {mac_token}"))
                .header(header::CONTENT_TYPE, "image/png")
                .header("x-screenshot-lease", lease)
                .body(Body::from(png.clone()))
                .unwrap(),
        )
    };
    assert_eq!(
        upload("99999999-9999-4999-8999-999999999999")
            .await
            .unwrap()
            .status(),
        409
    );
    let running = value(send(&app, "GET", &job_status, &token, Value::Null).await).await;
    assert_eq!(running["status"], "running");
    let lease = claimed["lease"].as_str().unwrap();
    let response = upload(lease).await.unwrap();
    assert_eq!(response.status(), 503);
    assert_eq!(
        value(response).await["error"]["code"],
        "screenshot_upload_unconfigured"
    );
    assert_eq!(upload(lease).await.unwrap().status(), 409);
    let failed = value(send(&app, "GET", &job_status, &token, Value::Null).await).await;
    assert_eq!(failed["status"], "failed");
    assert_eq!(failed["error"], "screenshot_upload_unconfigured");
    let next = value(send(&app, "POST", submit, &token, payload).await).await;
    let claimed = value(send(&app, "POST", &claim, &mac_token, Value::Null).await).await;
    assert_eq!(claimed["jobId"], next["jobId"]);
}
