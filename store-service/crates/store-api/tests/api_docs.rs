use axum::{
    body::Body,
    http::{Request, StatusCode},
    response::Response,
    Router,
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::{path::PathBuf, sync::Arc, time::Duration};
use store_api::{registry::Registry, ApiState};
use tower::ServiceExt;

const CLIENT: &str = "docs-client-test-token-0123456789";
const ADMIN: &str = "docs-admin-test-token-0123456789";
const PATH: &str = "/v1/store-management/api-docs";

fn state() -> Arc<ApiState> {
    ApiState::new(
        Registry::open(":memory:", "docs-test-signing-key-0123456789").unwrap(),
        ADMIN.into(),
        CLIENT.into(),
        "https://example.test/store-api".into(),
        PathBuf::from("nonexistent-test-releases"),
        Duration::from_secs(1),
    )
    .unwrap()
}

async fn call(
    app: Router,
    method: &str,
    path: &str,
    token: &str,
    https: bool,
    body: Value,
) -> Response {
    app.oneshot(
        Request::builder()
            .method(method)
            .uri(path)
            .header("authorization", format!("Bearer {token}"))
            .header("x-forwarded-proto", if https { "https" } else { "http" })
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
    )
    .await
    .unwrap()
}

async fn body(response: Response) -> String {
    String::from_utf8(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap()
}

#[tokio::test]
async fn api_document_contract_is_complete_without_nodes_and_formats_match() {
    let state = state();
    let app = store_api::router(state.clone());
    let response = call(app.clone(), "GET", PATH, CLIENT, false, Value::Null).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(response.headers()["content-type"], "application/json");
    let raw = body(response).await;
    assert!(!raw.contains(CLIENT));
    assert!(!raw.contains(ADMIN));
    let catalog: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(catalog["schemaVersion"], 1);
    assert_eq!(catalog["contractVersion"], "2026-11-11");
    let endpoints = catalog["endpoints"].as_array().unwrap();
    assert_eq!(endpoints.len(), 11);
    assert_eq!(
        endpoints
            .iter()
            .filter(|endpoint| endpoint["group"] == "store")
            .count(),
        9
    );
    assert_eq!(
        endpoints
            .iter()
            .filter(|endpoint| endpoint["group"] == "node")
            .count(),
        2
    );
    let unique: std::collections::HashSet<_> = endpoints
        .iter()
        .map(|endpoint| {
            (
                endpoint["method"].as_str().unwrap(),
                endpoint["path"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(unique.len(), 11);
    for (method, resource) in [
        ("GET", "supported-locales"),
        ("GET", "store-listings"),
        ("GET", "store-images"),
        ("GET", "store-releases"),
        ("GET", "store-reviews"),
        ("GET", "product-page-optimizations"),
        ("POST", "product-page-optimizations"),
        ("POST", "preflight"),
        ("POST", "sync"),
    ] {
        assert!(unique.contains(&(
            method,
            format!("/v1/store-management/apps/{{platform}}/{{bundleIdentifier}}/{resource}")
                .as_str()
        )));
    }
    assert!(unique.contains(&("GET", "/v1/store-management/connectors")));
    assert!(unique.contains(&("PUT", "/v1/apps/{platform}/{bundleIdentifier}/connector")));
    assert_eq!(catalog["uploadEndpoints"].as_array().unwrap().len(), 2);
    assert_eq!(
        catalog["sync"]["androidCommit"],
        json!({
            "changesNotSentForReview":"omitted","changesInReviewBehavior":"ERROR_IF_IN_REVIEW",
            "maySendForReview":true,"draftOnly":false,
        })
    );
    let query = &catalog["imageQuery"];
    assert_eq!(
        query["parameters"][1]["enum"],
        json!(["phone", "tablet", "foldable"])
    );
    assert_eq!(
        query["ios"]["devices"]["foldable"],
        json!(["foldable_screenshots"])
    );
    assert_eq!(
        query["android"]["devices"]["tablet"],
        json!(["seven_inch_screenshots", "ten_inch_screenshots"])
    );
    assert!(query["android"]["devices"].get("foldable").is_none());
    assert_eq!(
        query["android"]["unsupportedDevice"]["foldable"]["code"],
        "unsupported_image_device"
    );
    for (format, content_type, expected) in [
        (
            "markdown",
            "text/markdown; charset=utf-8",
            include_str!("../../../docs/store-api.md"),
        ),
        (
            "openapi",
            "application/yaml; charset=utf-8",
            include_str!("../../../docs/openapi.yaml"),
        ),
    ] {
        let response = call(
            app.clone(),
            "GET",
            &format!("{PATH}?format={format}"),
            CLIENT,
            false,
            Value::Null,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-type"], content_type);
        assert_eq!(response.headers()["cache-control"], "no-store");
        let returned = body(response).await;
        assert_eq!(returned, expected);
        if format == "markdown" {
            assert_eq!(catalog["documentation"]["markdown"], returned);
        }
    }
    let response = call(
        app,
        "GET",
        &format!("{PATH}?format=html"),
        CLIENT,
        false,
        Value::Null,
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        serde_json::from_str::<Value>(&body(response).await).unwrap()["error"]["code"],
        "invalid_document_format"
    );
    assert!(state.registry.list().unwrap().is_empty());
}

#[tokio::test]
async fn api_documents_enforce_existing_token_transport_and_revocation_rules() {
    let state = state();
    let app = store_api::router(state.clone());
    for format in ["json", "markdown", "openapi"] {
        for token in ["", "invalid", ADMIN] {
            assert_eq!(
                call(
                    app.clone(),
                    "GET",
                    &format!("{PATH}?format={format}"),
                    token,
                    true,
                    Value::Null
                )
                .await
                .status(),
                StatusCode::UNAUTHORIZED
            );
        }
    }
    let response = call(
        app.clone(),
        "POST",
        "/v1/integrations",
        ADMIN,
        true,
        json!({"name":"Documentation consumer"}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let integration: Value = serde_json::from_str(&body(response).await).unwrap();
    let token = integration["token"].as_str().unwrap();
    assert_eq!(
        call(app.clone(), "GET", PATH, token, true, Value::Null)
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        call(app.clone(), "GET", PATH, CLIENT, true, Value::Null)
            .await
            .status(),
        StatusCode::OK
    );
    let response = call(app.clone(), "GET", PATH, token, false, Value::Null).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        serde_json::from_str::<Value>(&body(response).await).unwrap()["error"]["code"],
        "https_required"
    );
    assert_eq!(
        call(
            app.clone(),
            "DELETE",
            &format!(
                "/v1/integrations/{}",
                integration["integration"]["id"].as_str().unwrap()
            ),
            ADMIN,
            true,
            Value::Null
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(app, "GET", PATH, token, true, Value::Null)
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert!(state.registry.list().unwrap().is_empty());
}
