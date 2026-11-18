mod support;
use axum::{
    body::Body,
    http::{header, Request},
    Router,
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::{path::PathBuf, sync::Arc, time::Duration};
use store_api::{
    registry::{AccountPatch, Connector, ConnectorInput, ConnectorKind, Registry},
    ApiState,
};
use store_protocol::Platform;
use tower::ServiceExt;

const KEY: &str = "account-screenshot-group-fixture-key-0123456789";
const ADMIN: &str = "account-screenshot-admin-fixture";
const BUSINESS: &str = "account-screenshot-business-fixture";
const APP: &str = "com.example.group";
fn registry() -> Registry {
    Registry::open(":memory:", KEY).unwrap()
}
fn node(registry: &Registry, name: &str, kind: ConnectorKind) -> Connector {
    registry
        .create_with_kind(ConnectorInput { name: name.into() }, kind)
        .unwrap()
}
fn patch(value: Value) -> AccountPatch {
    serde_json::from_value(value).unwrap()
}
fn group(registry: &Registry, store: &Connector) {
    for platform in [Platform::Ios, Platform::Android] {
        support::catalog(registry, &store.connector_id, platform, &[APP]);
    }
}
fn binding_values(registry: &Registry, store: &str) -> Vec<Option<String>> {
    registry
        .accounts()
        .unwrap()
        .into_iter()
        .filter(|account| account.store_connector_id == store)
        .map(|account| account.screenshot_connector_id)
        .collect()
}

#[test]
fn group_updates_and_clear_are_atomic_allow_shared_offline_mac_and_paused_store() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("registry.sqlite");
    let registry = Registry::open(&path, KEY).unwrap();
    let store = node(&registry, "Both platforms", ConnectorKind::Store);
    let peer = node(&registry, "Other account", ConnectorKind::Store);
    let mac = node(&registry, "Offline Mac", ConnectorKind::Screenshot);
    let disabled = node(&registry, "Disabled Mac", ConnectorKind::Screenshot);
    group(&registry, &store);
    support::catalog(
        &registry,
        &peer.connector_id,
        Platform::Ios,
        &["com.example.peer"],
    );
    let ios = registry
        .accounts()
        .unwrap()
        .into_iter()
        .find(|account| {
            account.store_connector_id == store.connector_id && account.platform == Platform::Ios
        })
        .unwrap();
    registry
        .set_app_enabled(&ios.id, Platform::Ios, APP, false)
        .unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute("INSERT INTO screenshot_jobs(id,connector_id,integration_id,profile,prompt,status,created_at,deadline,skill_manifest) VALUES('historical-shot',?1,'historical-integration','','original','running',1,2,'{\"version\":\"unchanged\"}')",[&mac.connector_id]).unwrap();
    registry.set_enabled(&store.connector_id, false).unwrap();
    let updated = registry
        .set_store_account_screenshot(&store.connector_id, Some(&mac.connector_id))
        .unwrap();
    assert_eq!(
        updated,
        json!({"storeConnectorId":store.connector_id,"connectorId":mac.connector_id,"updatedAccounts":2})
    );
    assert_eq!(
        binding_values(&registry, &store.connector_id),
        vec![Some(mac.connector_id.clone()); 2]
    );
    registry
        .set_store_account_screenshot(&peer.connector_id, Some(&mac.connector_id))
        .unwrap();
    registry.set_enabled(&disabled.connector_id, false).unwrap();
    let before = registry.store_account_screenshot_bindings().unwrap();
    for invalid in [&disabled.connector_id, &peer.connector_id, "missing"] {
        assert!(registry
            .set_store_account_screenshot(&store.connector_id, Some(invalid))
            .is_err());
        assert_eq!(
            registry.store_account_screenshot_bindings().unwrap(),
            before
        );
        assert_eq!(
            binding_values(&registry, &store.connector_id),
            vec![Some(mac.connector_id.clone()); 2]
        );
    }
    for invalid_store in [&mac.connector_id, "missing"] {
        assert!(registry
            .set_store_account_screenshot(invalid_store, Some(&mac.connector_id))
            .is_err());
        assert_eq!(
            registry.store_account_screenshot_bindings().unwrap(),
            before
        );
    }
    registry
        .set_store_account_screenshot(&store.connector_id, None)
        .unwrap();
    assert_eq!(
        binding_values(&registry, &store.connector_id),
        vec![None, None]
    );
    assert_eq!(
        binding_values(&registry, &peer.connector_id),
        vec![Some(mac.connector_id.clone())]
    );
    assert_eq!(registry.account_apps(&ios.id).unwrap()[0]["enabled"], false);
    assert_eq!(
        db.query_row(
            "SELECT connector_id,skill_manifest FROM screenshot_jobs WHERE id='historical-shot'",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        )
        .unwrap(),
        (mac.connector_id, "{\"version\":\"unchanged\"}".into())
    );
    drop(registry);
    let reopened = Registry::open(&path, KEY).unwrap();
    assert_eq!(
        binding_values(&reopened, &store.connector_id),
        vec![None, None]
    );
    assert!(reopened
        .store_account_screenshot_bindings()
        .unwrap()
        .iter()
        .any(|binding| binding["storeConnectorId"] == store.connector_id
            && binding["connectorId"].is_null()));
}

#[test]
fn future_platforms_inherit_explicit_node_or_null_and_refresh_retains_disabled_target() {
    let registry = registry();
    let mac = node(&registry, "Saved Mac", ConnectorKind::Screenshot);
    for clear in [false, true] {
        let store = node(
            &registry,
            if clear {
                "Explicit null"
            } else {
                "Explicit Mac"
            },
            ConnectorKind::Store,
        );
        let selected = (!clear).then_some(mac.connector_id.as_str());
        assert_eq!(
            registry
                .set_store_account_screenshot(&store.connector_id, selected)
                .unwrap()["updatedAccounts"],
            0
        );
        support::catalog(&registry, &store.connector_id, Platform::Ios, &[APP]);
        registry.set_enabled(&mac.connector_id, false).unwrap();
        support::catalog(&registry, &store.connector_id, Platform::Android, &[APP]);
        support::catalog(&registry, &store.connector_id, Platform::Ios, &[APP]);
        assert_eq!(
            binding_values(&registry, &store.connector_id),
            vec![selected.map(str::to_owned); 2]
        );
        registry.set_enabled(&mac.connector_id, true).unwrap();
    }
}

#[test]
fn legacy_unique_platform_inherits_but_catalog_arrival_does_not_resolve_conflicts() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("registry.sqlite");
    let registry = Registry::open(&path, KEY).unwrap();
    let mac = node(&registry, "Legacy Mac", ConnectorKind::Screenshot);
    let store = node(&registry, "Legacy account", ConnectorKind::Store);
    support::catalog(&registry, &store.connector_id, Platform::Ios, &[APP]);
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute(
        "UPDATE accounts SET screenshot_connector_id=?1 WHERE store_connector_id=?2",
        rusqlite::params![mac.connector_id, store.connector_id],
    )
    .unwrap();
    support::catalog(&registry, &store.connector_id, Platform::Android, &[APP]);
    assert_eq!(
        binding_values(&registry, &store.connector_id),
        vec![Some(mac.connector_id.clone()); 2]
    );
    assert!(registry
        .store_account_screenshot_bindings()
        .unwrap()
        .is_empty());
    let conflict = node(&registry, "Conflicted legacy account", ConnectorKind::Store);
    support::catalog(
        &registry,
        &conflict.connector_id,
        Platform::Ios,
        &["com.example.conflict"],
    );
    db.execute("UPDATE accounts SET migration_error='legacy_screenshot_conflict' WHERE store_connector_id=?1",[&conflict.connector_id]).unwrap();
    support::catalog(
        &registry,
        &conflict.connector_id,
        Platform::Android,
        &["com.example.conflict"],
    );
    assert!(registry
        .accounts()
        .unwrap()
        .iter()
        .filter(|account| account.store_connector_id == conflict.connector_id)
        .all(|account| account.migration_error.as_deref() == Some("legacy_screenshot_conflict")));
    registry
        .set_store_account_screenshot(&conflict.connector_id, Some(&mac.connector_id))
        .unwrap();
    assert!(registry
        .accounts()
        .unwrap()
        .iter()
        .filter(|account| account.store_connector_id == conflict.connector_id)
        .all(|account| account.migration_error.is_none()));
}

#[test]
fn platform_patch_changes_whole_group_and_move_or_create_respects_destination_configuration() {
    let registry = registry();
    let source = node(&registry, "Source", ConnectorKind::Store);
    let destination = node(&registry, "Destination", ConnectorKind::Store);
    let mac = node(&registry, "First Mac", ConnectorKind::Screenshot);
    let other = node(&registry, "Second Mac", ConnectorKind::Screenshot);
    group(&registry, &source);
    let ios = registry
        .accounts()
        .unwrap()
        .into_iter()
        .find(|account| account.platform == Platform::Ios)
        .unwrap();
    registry
        .patch_account(
            &ios.id,
            patch(json!({"screenshotConnectorId":mac.connector_id})),
        )
        .unwrap();
    assert_eq!(
        binding_values(&registry, &source.connector_id),
        vec![Some(mac.connector_id.clone()); 2]
    );
    registry
        .patch_account(&ios.id, patch(json!({"screenshotConnectorId":null})))
        .unwrap();
    assert_eq!(
        binding_values(&registry, &source.connector_id),
        vec![None, None]
    );
    registry
        .set_store_account_screenshot(&destination.connector_id, Some(&other.connector_id))
        .unwrap();
    registry
        .patch_account(
            &ios.id,
            patch(json!({"storeConnectorId":destination.connector_id})),
        )
        .unwrap();
    assert_eq!(
        registry.account(&ios.id).unwrap().screenshot_connector_id,
        Some(other.connector_id.clone())
    );
    assert!(registry.create_account(serde_json::from_value(json!({"name":"Conflicting creation","platform":"android","storeConnectorId":destination.connector_id,"screenshotConnectorId":mac.connector_id})).unwrap()).is_err());
    let created=registry.create_account(serde_json::from_value(json!({"name":"Inherited creation","platform":"android","storeConnectorId":destination.connector_id})).unwrap()).unwrap();
    assert_eq!(created.screenshot_connector_id, Some(other.connector_id));
    assert_eq!(binding_values(&registry, &source.connector_id), vec![None]);
}

#[test]
fn conflicting_move_preserves_both_groups_and_explicit_null() {
    let registry = registry();
    let source = node(&registry, "Source account", ConnectorKind::Store);
    let destination = node(&registry, "Destination account", ConnectorKind::Store);
    let first = node(&registry, "Source Mac", ConnectorKind::Screenshot);
    let second = node(&registry, "Destination Mac", ConnectorKind::Screenshot);
    group(&registry, &source);
    support::catalog(
        &registry,
        &destination.connector_id,
        Platform::Android,
        &["com.example.destination"],
    );
    registry
        .set_store_account_screenshot(&source.connector_id, Some(&first.connector_id))
        .unwrap();
    let ios = registry
        .accounts()
        .unwrap()
        .into_iter()
        .find(|account| {
            account.store_connector_id == source.connector_id && account.platform == Platform::Ios
        })
        .unwrap();
    for destination_binding in [Some(second.connector_id.as_str()), None] {
        registry
            .set_store_account_screenshot(&destination.connector_id, destination_binding)
            .unwrap();
        let before_accounts = serde_json::to_value(registry.accounts().unwrap()).unwrap();
        let before_bindings = registry.store_account_screenshot_bindings().unwrap();
        let before_source_apps = registry.account_apps(&ios.id).unwrap();
        for requested in [
            json!(first.connector_id),
            json!(second.connector_id),
            Value::Null,
        ] {
            if requested == json!(destination_binding) {
                continue;
            }
            let error = registry.patch_account(&ios.id, patch(json!({"storeConnectorId":destination.connector_id,"screenshotConnectorId":requested}))).unwrap_err();
            assert_eq!(error.code, "account_screenshot_conflict");
            assert_eq!(
                serde_json::to_value(registry.accounts().unwrap()).unwrap(),
                before_accounts
            );
            assert_eq!(
                registry.store_account_screenshot_bindings().unwrap(),
                before_bindings
            );
            assert_eq!(registry.account_apps(&ios.id).unwrap(), before_source_apps);
        }
    }
}

fn state() -> Arc<ApiState> {
    ApiState::new(
        registry(),
        ADMIN.into(),
        BUSINESS.into(),
        "https://center.invalid".into(),
        PathBuf::from("unused"),
        Duration::from_secs(1),
    )
    .unwrap()
}
async fn call(
    app: &Router,
    method: &str,
    path: &str,
    token: &str,
    body: Value,
    cookie: Option<&str>,
    csrf: bool,
) -> axum::response::Response {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("x-forwarded-proto", "https")
        .header(header::CONTENT_TYPE, "application/json");
    if !token.is_empty() {
        request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    if let Some(cookie) = cookie {
        request = request.header(header::COOKIE, cookie);
    }
    if csrf {
        request = request.header("x-testflying-setup", "1");
    }
    app.clone()
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap()
}
async fn value(response: axum::response::Response) -> Value {
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}

#[tokio::test]
async fn admin_session_strict_body_and_public_metadata_boundaries_are_preserved() {
    let state = state();
    let store = node(&state.registry, "Managed account", ConnectorKind::Store);
    let mac = node(&state.registry, "Offline Mac", ConnectorKind::Screenshot);
    group(&state.registry, &store);
    let app = store_api::router(state.clone());
    let integration = value(
        call(
            &app,
            "POST",
            "/v1/integrations",
            ADMIN,
            json!({"name":"Limited caller"}),
            None,
            false,
        )
        .await,
    )
    .await;
    let integration = integration["token"].as_str().unwrap();
    for prefix in ["/v1/accounts", "/connector-setup/api/accounts"] {
        let path = format!(
            "{prefix}/store-connectors/{}/screenshot-connector",
            store.connector_id
        );
        let before = state.registry.store_account_screenshot_bindings().unwrap();
        for token in ["", BUSINESS, integration] {
            assert_eq!(
                call(
                    &app,
                    "PUT",
                    &path,
                    token,
                    json!({"connectorId":mac.connector_id}),
                    None,
                    false
                )
                .await
                .status(),
                401
            );
        }
        for body in [
            json!({}),
            json!({"connectorId":3}),
            json!({"connectorId":false}),
            json!({"connectorId":[]}),
            json!({"connectorId":null,"unknown":1}),
        ] {
            assert_eq!(
                call(&app, "PUT", &path, ADMIN, body, None, false)
                    .await
                    .status(),
                422
            );
        }
        assert_eq!(
            state.registry.store_account_screenshot_bindings().unwrap(),
            before
        );
        assert_eq!(
            call(
                &app,
                "PUT",
                &path,
                ADMIN,
                json!({"connectorId":mac.connector_id}),
                None,
                false
            )
            .await
            .status(),
            200
        );
        state
            .registry
            .set_store_account_screenshot(&store.connector_id, None)
            .unwrap();
    }
    let login = call(
        &app,
        "POST",
        "/connector-setup/api/session",
        ADMIN,
        Value::Null,
        None,
        true,
    )
    .await;
    let cookie = login.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let path = format!(
        "/connector-setup/api/accounts/store-connectors/{}/screenshot-connector",
        store.connector_id
    );
    assert_eq!(
        call(
            &app,
            "PUT",
            &path,
            "",
            json!({"connectorId":null}),
            Some(&cookie),
            false
        )
        .await
        .status(),
        401
    );
    let updated = value(
        call(
            &app,
            "PUT",
            &path,
            "",
            json!({"connectorId":mac.connector_id}),
            Some(&cookie),
            true,
        )
        .await,
    )
    .await;
    assert_eq!(updated["updatedAccounts"], 2);
    let managed = value(
        call(
            &app,
            "GET",
            "/connector-setup/api/accounts",
            "",
            Value::Null,
            Some(&cookie),
            true,
        )
        .await,
    )
    .await;
    assert_eq!(
        managed["screenshotBindings"],
        json!([{"storeConnectorId":store.connector_id,"connectorId":mac.connector_id}])
    );
    let public = value(
        call(
            &app,
            "GET",
            "/v1/store-management/accounts",
            BUSINESS,
            Value::Null,
            None,
            false,
        )
        .await,
    )
    .await;
    assert!(public.get("screenshotBindings").is_none());
    assert!(!public.to_string().contains(&store.connector_id));
    assert!(!public.to_string().contains(&mac.connector_id));
}

#[tokio::test]
async fn compatibility_initialization_sets_group_but_respects_explicit_clear_and_legacy_conflicts()
{
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("registry.sqlite");
    let state = ApiState::new(
        Registry::open(&path, KEY).unwrap(),
        ADMIN.into(),
        BUSINESS.into(),
        "https://center.invalid".into(),
        PathBuf::from("unused"),
        Duration::from_secs(1),
    )
    .unwrap();
    let store = node(&state.registry, "Legacy group", ConnectorKind::Store);
    let first = node(&state.registry, "First Mac", ConnectorKind::Screenshot);
    let second = node(&state.registry, "Second Mac", ConnectorKind::Screenshot);
    group(&state.registry, &store);
    let app = store_api::router(state.clone());
    let route = format!("/v1/apps/ios/{APP}/screenshot-connector");
    assert_eq!(
        call(
            &app,
            "PUT",
            &route,
            ADMIN,
            json!({"connectorId":first.connector_id}),
            None,
            false
        )
        .await
        .status(),
        200
    );
    assert_eq!(
        binding_values(&state.registry, &store.connector_id),
        vec![Some(first.connector_id.clone()); 2]
    );
    state
        .registry
        .set_store_account_screenshot(&store.connector_id, None)
        .unwrap();
    assert_eq!(
        call(
            &app,
            "PUT",
            &route,
            ADMIN,
            json!({"connectorId":first.connector_id}),
            None,
            false
        )
        .await
        .status(),
        409
    );
    assert_eq!(
        binding_values(&state.registry, &store.connector_id),
        vec![None, None]
    );
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute("DELETE FROM store_account_screenshot_bindings", [])
        .unwrap();
    db.execute("UPDATE accounts SET screenshot_connector_id=CASE WHEN platform='ios' THEN ?1 ELSE ?2 END WHERE store_connector_id=?3",rusqlite::params![first.connector_id,second.connector_id,store.connector_id]).unwrap();
    let before = binding_values(&state.registry, &store.connector_id);
    assert_eq!(
        call(
            &app,
            "PUT",
            &route,
            ADMIN,
            json!({"connectorId":first.connector_id}),
            None,
            false
        )
        .await
        .status(),
        409
    );
    assert_eq!(binding_values(&state.registry, &store.connector_id), before);
    assert!(state
        .registry
        .store_account_screenshot_bindings()
        .unwrap()
        .is_empty());
    let ios = state
        .registry
        .accounts()
        .unwrap()
        .into_iter()
        .find(|account| account.platform == Platform::Ios)
        .unwrap();
    state
        .registry
        .patch_account(
            &ios.id,
            patch(json!({"screenshotConnectorId":second.connector_id})),
        )
        .unwrap();
    assert_eq!(
        binding_values(&state.registry, &store.connector_id),
        vec![Some(second.connector_id); 2]
    );
}
