//! Real encrypted transport with synthetic workers only; never invokes a Store provider.
use axum::{body::Body, http::Request};
use futures_util::{SinkExt, StreamExt};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
use store_api::{
    registry::{Connector, ConnectorInput, Registry},
    ApiState,
};
use store_protocol::{crypto, Operation, Platform, StoreReply, StoreRequest};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
    MaybeTlsStream, WebSocketStream,
};
use tower::ServiceExt;

const KEY: &str = "encrypted-upgrade-fixture-key-0123456789";
const ADMIN: &str = "encrypted-upgrade-admin-token-12345";
const CLIENT: &str = "encrypted-upgrade-client-token-12345";
struct Worker {
    socket: WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
    channel: crypto::Channel,
}
impl Worker {
    async fn receive(&mut self) -> Vec<u8> {
        loop {
            match self.socket.next().await.unwrap().unwrap() {
                Message::Binary(bytes) => {
                    if let Some(plain) = self.channel.decrypt(&bytes).unwrap() {
                        return plain;
                    }
                }
                Message::Ping(bytes) => {
                    self.socket.send(Message::Pong(bytes)).await.unwrap();
                }
                other => panic!("unexpected fixture frame: {other:?}"),
            }
        }
    }
    async fn send(&mut self, value: Value) {
        for record in self
            .channel
            .encrypt(&serde_json::to_vec(&value).unwrap())
            .unwrap()
        {
            self.socket
                .send(Message::Binary(record.into()))
                .await
                .unwrap();
        }
    }
}
async fn worker(state: &ApiState, base: &str, node: &Connector, capable: bool) -> Worker {
    let mut request = format!("{base}/v1/connectors/{}/ws", node.connector_id)
        .replace("http://", "ws://")
        .into_client_request()
        .unwrap();
    request.headers_mut().insert(
        "authorization",
        format!("Bearer {}", state.registry.token(node))
            .parse()
            .unwrap(),
    );
    request.headers_mut().insert(
        "sec-websocket-protocol",
        crypto::SUBPROTOCOL.parse().unwrap(),
    );
    let (mut socket, _) = connect_async(request).await.unwrap();
    let key = state.registry.purpose_key(node, "message");
    let mut handshake = crypto::handshake(&key, &node.connector_id, true).unwrap();
    let mut registration =
        json!({"connectorId":node.connector_id,"baseUrl":null,"connectorVersion":"1.0.0"});
    if capable {
        registration["storeUpgrade"] =
            json!({"protocol":1,"target":"darwin-arm64","installed":true,"bootstrapVersion":1});
    }
    let mut bytes = vec![0; crypto::HANDSHAKE_BYTES];
    let size = handshake
        .write_message(&serde_json::to_vec(&registration).unwrap(), &mut bytes)
        .unwrap();
    socket
        .send(Message::Binary(bytes[..size].to_vec().into()))
        .await
        .unwrap();
    let Message::Binary(reply) = socket.next().await.unwrap().unwrap() else {
        panic!("handshake expected")
    };
    handshake.read_message(&reply, &mut bytes).unwrap();
    let mut channel = crypto::Channel::new(handshake).unwrap();
    for record in channel.encrypt(b"confirm").unwrap() {
        socket.send(Message::Binary(record.into())).await.unwrap();
    }
    let mut worker = Worker { socket, channel };
    assert_eq!(worker.receive().await, b"ready");
    worker
}
async fn admin(state: Arc<ApiState>) -> Value {
    let response = store_api::router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/store-node-upgrades")
                .header("authorization", format!("Bearer {ADMIN}"))
                .header("x-forwarded-proto", "https")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}
async fn node_ready(state: Arc<ApiState>, node: &Connector, request: &str) -> bool {
    let token = state.registry.token(node);
    let response = store_api::router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/v1/connectors/{}/store-upgrades/{request}/ready",
                    node.connector_id
                ))
                .header("authorization", format!("Bearer {token}"))
                .header("x-forwarded-proto", "https")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"completedJobIds":[]}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let value: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    value["ready"].as_bool().unwrap()
}
#[tokio::test]
async fn encrypted_control_is_independent_of_business_pending_and_recovers_only_original_member() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("darwin-arm64")).unwrap();
    std::fs::write(
        root.path().join("darwin-arm64/store-connector"),
        b"fixture-binary",
    )
    .unwrap();
    std::fs::write(
        root.path().join("connector-versions.json"),
        br#"{"store":"1.0.1","screenshot":"1.0.0"}"#,
    )
    .unwrap();
    let state = ApiState::new(
        Registry::open(":memory:", KEY).unwrap(),
        ADMIN.into(),
        CLIENT.into(),
        "https://fixture.invalid".into(),
        root.path().into(),
        Duration::from_secs(3),
    )
    .unwrap();
    let node = state
        .registry
        .create(ConnectorInput {
            name: "upgradable".into(),
        })
        .unwrap();
    let legacy = state
        .registry
        .create(ConnectorInput {
            name: "legacy".into(),
        })
        .unwrap();
    let later = state
        .registry
        .create(ConnectorInput {
            name: "offline at click".into(),
        })
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server_state = state.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, store_api::router(server_state))
            .await
            .unwrap()
    });
    let mut current = worker(&state, &base, &node, true).await;
    let mut old = worker(&state, &base, &legacy, false).await;
    let batch = admin(state.clone()).await;
    assert_eq!(batch["nodes"].as_array().unwrap().len(), 2);
    let command: Value = serde_json::from_slice(
        &tokio::time::timeout(Duration::from_secs(2), current.receive())
            .await
            .unwrap(),
    )
    .unwrap();
    let request = command["storeUpgrade"]["request"]["requestId"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(admin(state.clone()).await["batchId"], batch["batchId"]);
    assert!(
        tokio::time::timeout(Duration::from_millis(50), old.receive())
            .await
            .is_err()
    );
    assert!(state.transport.connected(&legacy.connector_id));
    let dispatch_state = state.clone();
    let dispatch_node = node.clone();
    let dispatch = tokio::spawn(async move {
        dispatch_state
            .transport
            .dispatch(
                &dispatch_node,
                &dispatch_state.registry.token(&dispatch_node),
                StoreRequest {
                    request_id: uuid::Uuid::new_v4().to_string(),
                    platform: Platform::Ios,
                    bundle_identifier: "fixture.example".into(),
                    operation: Operation::SupportedLocales,
                    payload: json!({}),
                },
            )
            .await
    });
    let business: StoreRequest = serde_json::from_slice(&current.receive().await).unwrap();
    assert_eq!(state.transport.pending_count(&node.connector_id), 1);
    // A forged unsolicited status shares the normal request ID. It must not consume it.
    current.send(json!({"requestId":business.request_id,"status":200,"body":{"storeUpgradeStatus":{"nodeId":node.connector_id,"requestId":business.request_id,"state":"draining"}}})).await;
    assert!(!node_ready(state.clone(), &node, &request).await);
    current
        .send(
            serde_json::to_value(StoreReply::from_result(
                business.request_id,
                Ok(json!({"locales":["fixture"]})),
            ))
            .unwrap(),
        )
        .await;
    assert_eq!(
        dispatch.await.unwrap().unwrap().body,
        json!({"locales":["fixture"]})
    );
    assert!(node_ready(state.clone(), &node, &request).await);
    current.socket.close(None).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while state.transport.connected(&node.connector_id) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let mut reconnected = worker(&state, &base, &node, true).await;
    let recovered: Value = serde_json::from_slice(&reconnected.receive().await).unwrap();
    assert_eq!(recovered, command);
    let mut late = worker(&state, &base, &later, true).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(50), late.receive())
            .await
            .is_err()
    );
    assert_eq!(
        admin(state.clone()).await["nodes"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    state.transport.close_all();
    server.abort();
    let _ = server.await;
}
