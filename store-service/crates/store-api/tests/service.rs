use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use futures_util::{SinkExt, StreamExt};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::{path::PathBuf, sync::Arc, time::Duration};
use store_api::{
    registry::{ConnectorInput, Registry},
    ApiState,
};
use store_connector::{ConnectorConfig, Executor};
use store_protocol::{crypto, Operation, Platform, StoreReply, StoreRequest};
use tokio::net::TcpListener;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
};
use tower::ServiceExt;

const ADMIN: &str = "admin-test-token-0123456789";
const CLIENT: &str = "client-test-token-0123456789";
const SIGNING: &str = "signing-key-for-local-test-only-0123456789";

fn state(timeout: Duration) -> Arc<ApiState> {
    ApiState::new(
        Registry::open(":memory:", SIGNING).unwrap(),
        ADMIN.into(),
        CLIENT.into(),
        "http://127.0.0.1:8200".into(),
        PathBuf::from("nonexistent-test-releases"),
        timeout,
    )
    .unwrap()
}
async fn call(
    app: Router,
    method: &str,
    path: &str,
    token: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = app
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
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
async fn serve(app: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (base, handle)
}
fn input() -> ConnectorInput {
    ConnectorInput {
        name: "Account A".into(),
    }
}
fn command(id: &str) -> StoreRequest {
    StoreRequest {
        request_id: id.into(),
        platform: Platform::Ios,
        bundle_identifier: "com.example.app".into(),
        operation: Operation::SupportedLocales,
        payload: json!({}),
    }
}

#[test]
fn registry_persists_many_apps_exact_platform_and_rotated_identity() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("registry.sqlite");
    let registry = Registry::open(&path, SIGNING).unwrap();
    let first = registry.create(input()).unwrap();
    let second = registry.create(input()).unwrap();
    let token = registry.token(&first);
    registry
        .bind(Platform::Ios, "com.example.app", &first.connector_id)
        .unwrap();
    registry
        .bind(Platform::Ios, "com.example.second", &first.connector_id)
        .unwrap();
    registry
        .bind(Platform::Android, "com.example.app", &second.connector_id)
        .unwrap();
    assert_eq!(registry.get(&first.connector_id).unwrap().app_count, 2);
    assert!(registry.resolve(Platform::Ios, "com.Example.app").is_err());
    assert_eq!(
        registry
            .resolve(Platform::Android, "com.example.app")
            .unwrap()
            .connector_id,
        second.connector_id
    );
    assert!(!serde_json::to_string(&registry.list().unwrap())
        .unwrap()
        .contains(&token));
    drop(registry);
    let registry = Registry::open(&path, SIGNING).unwrap();
    assert_eq!(
        registry
            .resolve(Platform::Ios, "com.example.second")
            .unwrap()
            .connector_id,
        first.connector_id
    );
    assert!(registry.authenticate(&first.connector_id, &token).is_ok());
    registry.rotate(&first.connector_id).unwrap();
    assert!(registry.authenticate(&first.connector_id, &token).is_err());
    registry.set_enabled(&first.connector_id, false).unwrap();
    assert_eq!(
        registry
            .resolve(Platform::Ios, "com.example.app")
            .unwrap_err()
            .code,
        "connector_disabled"
    );
    registry.unbind(Platform::Ios, "com.example.app").unwrap();
    assert_eq!(
        registry
            .resolve(Platform::Ios, "com.example.app")
            .unwrap_err()
            .code,
        "app_not_found"
    );
}

#[tokio::test]
async fn admin_api_provisions_authenticates_and_hides_tokens() {
    let state = state(Duration::from_secs(1));
    let app = store_api::router(state.clone());
    assert_eq!(
        call(
            app.clone(),
            "POST",
            "/v1/connectors",
            CLIENT,
            json!({"name":"A"})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let (status, created) = call(
        app.clone(),
        "POST",
        "/v1/connectors",
        ADMIN,
        json!({"name":"A"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = created["connector"]["connectorId"].as_str().unwrap();
    let token = created["installation"]["token"].as_str().unwrap();
    assert!(state.registry.authenticate(id, token).is_ok());
    let (_, listed) = call(app.clone(), "GET", "/v1/connectors", ADMIN, Value::Null).await;
    assert!(!listed.to_string().contains(token));
    assert!(!listed.to_string().contains("nonce"));
    let path = "/v1/apps/ios/com.example.app/connector";
    assert_eq!(
        call(app.clone(), "PUT", path, ADMIN, json!({"connectorId":id}))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        call(app.clone(), "GET", path, ADMIN, Value::Null).await.1["connectorId"],
        id
    );
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
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        call(
            app.clone(),
            "GET",
            "/v1/store-management/apps/ios/missing/supported-locales",
            "bad",
            Value::Null
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    for legacy in [
        "metadata-content-sets",
        "store-versions/1.0/draft",
        "marketing-pages",
    ] {
        let path = format!("/v1/store-management/apps/ios/com.example.app/{legacy}");
        assert_eq!(
            call(app.clone(), "POST", &path, CLIENT, json!({})).await.0,
            StatusCode::NOT_FOUND
        );
    }
    let (status, _) = call(
        app.clone(),
        "GET",
        &format!("/v1/connectors/{id}/installer/../../etc/passwd"),
        ADMIN,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, rotated) = call(
        app.clone(),
        "POST",
        &format!("/v1/connectors/{id}/rotate-token"),
        ADMIN,
        json!({}),
    )
    .await;
    assert_ne!(rotated["token"], token);
    assert!(state.registry.authenticate(id, token).is_err());
    assert_eq!(
        call(app, "DELETE", path, ADMIN, Value::Null).await.0,
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn http_connector_executes_shared_protocol_and_rejects_wrong_identity() {
    let config:ConnectorConfig=serde_json::from_value(json!({"connectorId":"node-http","token":"node-token-for-local-tests","serverUrl":"http://127.0.0.1:8200","messageKey":"11".repeat(32),"uploadKey":"22".repeat(32),"provider":{"mode":"mock"}})).unwrap();
    let executor = Executor::new(config).unwrap();
    let (base, server) = serve(store_connector::http_router(executor.clone())).await;
    let state = state(Duration::from_secs(1));
    let node = state.registry.create(input()).unwrap();
    state
        .registry
        .register_endpoint(
            &node.connector_id,
            &state.registry.token(&node),
            Some(&base),
        )
        .unwrap();
    let node = state.registry.get(&node.connector_id).unwrap();
    let reply = state
        .transport
        .dispatch(&node, "node-token-for-local-tests", command("http-1"))
        .await
        .unwrap();
    assert_eq!(reply.body["locales"][0], "en-US");
    assert_eq!(reply.request_id, "http-1");
    assert!(state
        .transport
        .dispatch(&node, "wrong", command("http-2"))
        .await
        .is_err());
    server.abort();
    let _ = server.await;
    executor.drain().await;
}

type RawSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
struct EncryptedSocket {
    raw: RawSocket,
    channel: crypto::Channel,
}
impl EncryptedSocket {
    async fn send(
        &mut self,
        message: Message,
    ) -> Result<(), tokio_tungstenite::tungstenite::Error> {
        if let Message::Text(text) = message {
            for record in self.channel.encrypt(text.as_bytes()).unwrap() {
                self.raw.send(Message::Binary(record.into())).await?;
            }
            Ok(())
        } else {
            self.raw.send(message).await
        }
    }
    async fn close(
        &mut self,
        frame: Option<tokio_tungstenite::tungstenite::protocol::CloseFrame>,
    ) -> Result<(), tokio_tungstenite::tungstenite::Error> {
        self.raw.close(frame).await
    }
}
async fn ws_connect(base: &str, id: &str, token: &str, key: &str) -> EncryptedSocket {
    let mut request = format!("{}/v1/connectors/{id}/ws", base.replacen("http", "ws", 1))
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    request.headers_mut().insert(
        "sec-websocket-protocol",
        crypto::SUBPROTOCOL.parse().unwrap(),
    );
    let mut raw = connect_async(request).await.unwrap().0;
    let mut handshake = crypto::handshake(key, id, true).unwrap();
    let mut buffer = vec![0; crypto::HANDSHAKE_BYTES];
    let size = handshake
        .write_message(
            &serde_json::to_vec(&json!({"connectorId":id,"baseUrl":null})).unwrap(),
            &mut buffer,
        )
        .unwrap();
    raw.send(Message::Binary(buffer[..size].to_vec().into()))
        .await
        .unwrap();
    let Message::Binary(reply) = raw.next().await.unwrap().unwrap() else {
        panic!("Expected handshake");
    };
    handshake.read_message(&reply, &mut buffer).unwrap();
    let mut channel = crypto::Channel::new(handshake).unwrap();
    for record in channel.encrypt(b"confirm").unwrap() {
        raw.send(Message::Binary(record.into())).await.unwrap();
    }
    let Message::Binary(reply) = raw.next().await.unwrap().unwrap() else {
        panic!("Expected confirmation");
    };
    assert_eq!(channel.decrypt(&reply).unwrap().unwrap(), b"ready");
    EncryptedSocket { raw, channel }
}
async fn next_request(socket: &mut EncryptedSocket) -> StoreRequest {
    loop {
        match tokio::time::timeout(Duration::from_secs(2), socket.raw.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
        {
            Message::Binary(bytes) => {
                if let Some(plain) = socket.channel.decrypt(&bytes).unwrap() {
                    return serde_json::from_slice(&plain).unwrap();
                }
            }
            Message::Ping(bytes) => socket.send(Message::Pong(bytes)).await.unwrap(),
            _ => {}
        }
    }
}
async fn wait_connected(state: &ApiState, id: &str) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while !state.transport.connected(id) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn websocket_routes_multiple_apps_and_rejects_duplicate_nodes() {
    let state = state(Duration::from_secs(1));
    let node = state.registry.create(input()).unwrap();
    let token = state.registry.token(&node);
    for name in ["com.example.app", "com.example.other"] {
        state
            .registry
            .bind(Platform::Ios, name, &node.connector_id)
            .unwrap();
    }
    let (base, server) = serve(store_api::router(state.clone())).await;
    let mut socket = ws_connect(
        &base,
        &node.connector_id,
        &token,
        &state.registry.purpose_key(&node, "message"),
    )
    .await;
    wait_connected(&state, &node.connector_id).await;
    let mut duplicate = format!(
        "{}/v1/connectors/{}/ws",
        base.replacen("http", "ws", 1),
        node.connector_id
    )
    .into_client_request()
    .unwrap();
    duplicate
        .headers_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    duplicate.headers_mut().insert(
        "sec-websocket-protocol",
        crypto::SUBPROTOCOL.parse().unwrap(),
    );
    let error = connect_async(duplicate).await.unwrap_err();
    assert!(
        matches!(error,tokio_tungstenite::tungstenite::Error::Http(response) if response.status()==409)
    );
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    for name in ["com.example.app", "com.example.other"] {
        let request = client
            .get(format!(
                "{base}/v1/store-management/apps/ios/{name}/supported-locales"
            ))
            .bearer_auth(CLIENT);
        let pending =
            tokio::spawn(
                async move { request.send().await.unwrap().json::<Value>().await.unwrap() },
            );
        let received = next_request(&mut socket).await;
        assert_eq!(received.bundle_identifier, name);
        socket
            .send(Message::Text(
                serde_json::to_string(&StoreReply {
                    request_id: received.request_id,
                    status: 200,
                    body: json!({"locales":["en-US"]}),
                })
                .unwrap()
                .into(),
            ))
            .await
            .unwrap();
        assert_eq!(pending.await.unwrap()["locales"][0], "en-US");
    }
    let (_, body) = call(
        store_api::router(state.clone()),
        "PATCH",
        &format!("/v1/connectors/{}", node.connector_id),
        ADMIN,
        json!({"enabled":false}),
    )
    .await;
    assert_eq!(body["enabled"], false);
    assert!(!state.transport.connected(&node.connector_id));
    state.transport.close_all();
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn timeout_busy_disconnect_and_reconnect_never_replay_commands() {
    let state = state(Duration::from_millis(150));
    let node = state.registry.create(input()).unwrap();
    let token = state.registry.token(&node);
    let (base, server) = serve(store_api::router(state.clone())).await;
    let mut socket = ws_connect(
        &base,
        &node.connector_id,
        &token,
        &state.registry.purpose_key(&node, "message"),
    )
    .await;
    wait_connected(&state, &node.connector_id).await;
    let (transport, copy, secret) = (state.transport.clone(), node.clone(), token.clone());
    let first = tokio::spawn(async move {
        transport
            .dispatch(&copy, &secret, command("old-request"))
            .await
    });
    assert_eq!(next_request(&mut socket).await.request_id, "old-request");
    assert_eq!(
        state
            .transport
            .dispatch(&node, &token, command("busy-request"))
            .await
            .unwrap_err()
            .code,
        "connector_busy"
    );
    assert_eq!(first.await.unwrap().unwrap_err().code, "connector_timeout");
    // Late results cannot satisfy another request.
    socket
        .send(Message::Text(
            serde_json::to_string(&StoreReply {
                request_id: "old-request".into(),
                status: 200,
                body: json!({"stale":true}),
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    let (transport, copy, secret) = (state.transport.clone(), node.clone(), token.clone());
    let pending = tokio::spawn(async move {
        transport
            .dispatch(&copy, &secret, command("disconnect-request"))
            .await
    });
    next_request(&mut socket).await;
    socket.close(None).await.unwrap();
    assert_eq!(
        pending.await.unwrap().unwrap_err().code,
        "connector_disconnected"
    );
    tokio::time::timeout(Duration::from_secs(2), async {
        while state.transport.connected(&node.connector_id) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let mut reconnected = ws_connect(
        &base,
        &node.connector_id,
        &token,
        &state.registry.purpose_key(&node, "message"),
    )
    .await;
    wait_connected(&state, &node.connector_id).await;
    let (transport, copy, secret) = (state.transport.clone(), node.clone(), token.clone());
    let fresh = tokio::spawn(async move {
        transport
            .dispatch(&copy, &secret, command("fresh-request"))
            .await
    });
    let received = next_request(&mut reconnected).await;
    assert_eq!(received.request_id, "fresh-request");
    reconnected
        .send(Message::Text(
            serde_json::to_string(&StoreReply {
                request_id: received.request_id,
                status: 200,
                body: json!({"fresh":true}),
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    assert_eq!(fresh.await.unwrap().unwrap().body, json!({"fresh":true}));
    state.transport.close_all();
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn real_connector_active_loop_and_api_round_trip() {
    let state = state(Duration::from_secs(2));
    let node = state.registry.create(input()).unwrap();
    let token = state.registry.token(&node);
    state
        .registry
        .bind(Platform::Ios, "com.example.app", &node.connector_id)
        .unwrap();
    let (base, server) = serve(store_api::router(state.clone())).await;
    let config:ConnectorConfig=serde_json::from_value(json!({"connectorId":node.connector_id,"token":token,"messageKey":state.registry.purpose_key(&node,"message"),"uploadKey":state.registry.purpose_key(&node,"upload"),"serverUrl":base,"provider":{"mode":"mock"}})).unwrap();
    let executor = Executor::new(config).unwrap();
    let (stop, stopped) = tokio::sync::watch::channel(false);
    let connector = tokio::spawn(store_connector::run_active(executor.clone(), stopped));
    wait_connected(&state, &node.connector_id).await;
    let response = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .get(format!(
            "{base}/v1/store-management/apps/ios/com.example.app/supported-locales"
        ))
        .bearer_auth(CLIENT)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.json::<Value>().await.unwrap()["locales"][0],
        "en-US"
    );
    stop.send(true).unwrap();
    tokio::time::timeout(Duration::from_secs(2), connector)
        .await
        .unwrap()
        .unwrap();
    executor.drain().await;
    state.transport.close_all();
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn installer_contains_published_binary_private_identity_and_launcher() {
    use std::io::{Cursor, Read};
    let directory = tempfile::tempdir().unwrap();
    let releases = directory.path().join("releases");
    std::fs::create_dir_all(releases.join("linux-amd64")).unwrap();
    std::fs::write(
        releases.join("linux-amd64/store-connector"),
        b"test-binary-fixture",
    )
    .unwrap();
    let state = ApiState::new(
        Registry::open(":memory:", SIGNING).unwrap(),
        ADMIN.into(),
        CLIENT.into(),
        "https://store.example.com".into(),
        releases,
        Duration::from_secs(1),
    )
    .unwrap();
    let node = state.registry.create(input()).unwrap();
    let app = store_api::router(state.clone());
    let path = format!("/v1/connectors/{}/installer/linux-amd64", node.connector_id);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&path)
                .header("authorization", format!("Bearer {ADMIN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    assert_eq!(archive.len(), 3);
    let mut binary = Vec::new();
    archive
        .by_name("store-connector")
        .unwrap()
        .read_to_end(&mut binary)
        .unwrap();
    assert_eq!(binary, b"test-binary-fixture");
    let mut raw = String::new();
    archive
        .by_name("connector.json")
        .unwrap()
        .read_to_string(&mut raw)
        .unwrap();
    let config: ConnectorConfig = serde_json::from_str(&raw).unwrap();
    assert_eq!(config.connector_id, node.connector_id);
    assert_eq!(config.server_url, "https://store.example.com");
    assert!(state
        .registry
        .authenticate(&node.connector_id, &config.token)
        .is_ok());
    assert!(archive.by_name("start.sh").is_ok());
    let missing = format!(
        "/v1/connectors/{}/installer/windows-amd64",
        node.connector_id
    );
    assert_eq!(
        call(app.clone(), "GET", &missing, ADMIN, Value::Null)
            .await
            .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        call(app.clone(), "GET", &path, CLIENT, Value::Null).await.0,
        StatusCode::UNAUTHORIZED
    );
    state
        .registry
        .set_enabled(&node.connector_id, false)
        .unwrap();
    assert_eq!(
        call(app, "GET", &path, ADMIN, Value::Null).await.0,
        StatusCode::CONFLICT
    );
}

#[test]
fn service_credentials_cannot_collapse_admin_and_client_roles() {
    assert!(ApiState::new(
        Registry::open(":memory:", SIGNING).unwrap(),
        CLIENT.into(),
        CLIENT.into(),
        "http://127.0.0.1:8200".into(),
        PathBuf::from("releases"),
        Duration::from_secs(1)
    )
    .is_err());
}
