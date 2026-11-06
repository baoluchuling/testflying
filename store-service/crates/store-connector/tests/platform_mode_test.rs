use serde_json::{json, Value};
use store_connector::{ConnectorConfig, Executor};
use store_protocol::{Operation, Platform, StoreRequest};

fn config() -> Value {
    json!({
        "connectorId":"platform-mode-fixture",
        "token":"test-only-connector-token",
        "serverUrl":"http://127.0.0.1:8200",
        "messageKey":"11".repeat(32),
        "uploadKey":"22".repeat(32),
        "provider":{"mode":"mock"}
    })
}

#[tokio::test]
async fn platform_mode_limits_store_and_upload_operations() {
    for (mode, allow_ios, allow_android) in [
        (None, true, true),
        (Some("all"), true, true),
        (Some("ios"), true, false),
        (Some("android"), false, true),
    ] {
        let mut raw = config();
        if let Some(mode) = mode {
            raw["platformMode"] = json!(mode);
        }
        let executor = Executor::new(serde_json::from_value(raw).unwrap()).unwrap();
        for (platform, allowed) in [
            (Platform::Ios, allow_ios),
            (Platform::Android, allow_android),
        ] {
            for operation in [Operation::SupportedLocales, Operation::UploadStart] {
                let reply = executor
                    .clone()
                    .execute(StoreRequest {
                        request_id: "platform-check".into(),
                        platform,
                        bundle_identifier: "com.example.fixture".into(),
                        operation,
                        payload: json!({}),
                    })
                    .await;
                assert_eq!(reply.request_id, "platform-check");
                match (allowed, operation) {
                    (true, Operation::SupportedLocales) => assert_eq!(reply.status, 200),
                    (true, _) => assert_eq!(reply.body["error"]["code"], "invalid_asset_id"),
                    (false, _) => {
                        assert_eq!(reply.status, 422);
                        assert_eq!(reply.body["error"]["code"], "platform_not_supported");
                    }
                }
            }
        }
    }
}

#[test]
fn disabled_provider_configuration_is_ignored() {
    let mut raw = config();
    raw["provider"]["applePrivateKeyPath"] = json!("");
    raw["provider"]["appleBaseUrl"] = json!("invalid-url");
    raw["platformMode"] = json!("android");
    assert!(Executor::new(serde_json::from_value(raw.clone()).unwrap()).is_ok());
    raw["platformMode"] = json!("ios");
    assert!(Executor::new(serde_json::from_value(raw).unwrap()).is_err());

    let mut raw = config();
    raw["provider"]["googleServiceAccountPath"] = json!("");
    raw["provider"]["googleTokenUrl"] = json!("invalid-url");
    raw["platformMode"] = json!("ios");
    assert!(Executor::new(serde_json::from_value(raw.clone()).unwrap()).is_ok());
    raw["platformMode"] = json!("android");
    assert!(Executor::new(serde_json::from_value(raw.clone()).unwrap()).is_err());

    raw["platformMode"] = json!("windows");
    assert!(serde_json::from_value::<ConnectorConfig>(raw).is_err());
}
