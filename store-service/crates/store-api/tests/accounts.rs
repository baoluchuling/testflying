mod support;
use axum::{body::Body, http::Request, Router};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::{path::PathBuf, sync::Arc, time::Duration};
use store_api::{
    registry::{AccountPatch, Connector, ConnectorInput, ConnectorKind, Registry},
    ApiState,
};
use store_protocol::{
    catalog::{CatalogReport, CatalogStatus},
    Platform,
};
use tower::ServiceExt;

const KEY: &str = "account-fixture-signing-key-0123456789";
const ADMIN: &str = "account-fixture-admin-token";
const CLIENT: &str = "account-fixture-business-token";
const APP: &str = "com.example.account";
fn registry() -> Registry {
    Registry::open(":memory:", KEY).unwrap()
}

#[test]
fn android_manual_membership_survives_reports_removal_and_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("manual.sqlite");
    let r = Registry::open(&path, KEY).unwrap();
    let store = node(&r, "Android");
    let mac = r
        .create_with_kind(
            ConnectorInput { name: "Mac".into() },
            ConnectorKind::Screenshot,
        )
        .unwrap();
    support::catalog(
        &r,
        &store.connector_id,
        Platform::Android,
        &["com.example.discovered"],
    );
    let account = r.accounts().unwrap().remove(0);
    r.set_store_account_screenshot(&store.connector_id, Some(&mac.connector_id))
        .unwrap();
    r.add_android_app(&account.id, APP, Some("  Manual app  "))
        .unwrap();
    assert_eq!(
        r.resolve(Platform::Android, APP).unwrap().connector_id,
        store.connector_id
    );
    assert_eq!(
        r.account_apps(&account.id)
            .unwrap()
            .iter()
            .find(|app| app["bundleIdentifier"] == APP)
            .unwrap()["screenshotEnabled"],
        true
    );
    let row = r
        .account_apps(&account.id)
        .unwrap()
        .into_iter()
        .find(|row| row["bundleIdentifier"] == APP)
        .unwrap();
    assert_eq!(row["name"], "Manual app");
    assert_eq!(row["source"], "manual");
    assert_eq!(row["enabled"], true);
    r.set_app_enabled(&account.id, Platform::Android, APP, false)
        .unwrap();
    support::catalog(&r, &store.connector_id, Platform::Android, &[APP]);
    assert_eq!(r.account_apps(&account.id).unwrap().len(), 1);
    assert_eq!(
        r.account_apps(&account.id).unwrap()[0]["name"],
        "Manual app"
    );
    assert_eq!(
        r.resolve_account(Platform::Android, APP).unwrap_err().code,
        "app_forwarding_disabled"
    );
    r.remove_android_app(&account.id, APP).unwrap();
    r.remove_android_app(&account.id, APP).unwrap();
    support::catalog(&r, &store.connector_id, Platform::Android, &[APP]);
    assert!(r.account_apps(&account.id).unwrap().is_empty());
    drop(r);
    let r = Registry::open(&path, KEY).unwrap();
    support::catalog(&r, &store.connector_id, Platform::Android, &[APP]);
    assert_eq!(
        r.resolve_account(Platform::Android, APP).unwrap_err().code,
        "app_not_found"
    );
    r.add_android_app(&account.id, APP, None).unwrap();
    assert_eq!(r.account_apps(&account.id).unwrap()[0]["enabled"], false);
    r.set_app_enabled(&account.id, Platform::Android, APP, true)
        .unwrap();
    assert_eq!(
        r.account(&account.id)
            .unwrap()
            .screenshot_connector_id
            .as_deref(),
        Some(mac.connector_id.as_str())
    );
    assert_eq!(
        r.account_apps(&account.id).unwrap()[0]["screenshotEnabled"],
        true
    );
    assert_eq!(
        r.remove_android_app(&account.id, "com.example.missing")
            .unwrap_err()
            .status,
        404
    );
}

#[test]
fn android_upgrade_preserves_old_data_and_ios_keeps_catalog_semantics() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("upgrade.sqlite");
    let r = Registry::open(&path, KEY).unwrap();
    let store = node(&r, "Legacy");
    support::catalog(&r, &store.connector_id, Platform::Android, &[APP]);
    support::catalog(&r, &store.connector_id, Platform::Ios, &[APP]);
    let android = r
        .accounts()
        .unwrap()
        .into_iter()
        .find(|a| a.platform == Platform::Android)
        .unwrap();
    let ios = r
        .accounts()
        .unwrap()
        .into_iter()
        .find(|a| a.platform == Platform::Ios)
        .unwrap();
    r.set_app_enabled(&android.id, Platform::Android, APP, false)
        .unwrap();
    let before = r.account(&android.id).unwrap();
    drop(r);
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("DELETE FROM account_migrations WHERE name='android_manual_apps_v1'; DROP TABLE account_app_removals; ALTER TABLE account_apps DROP COLUMN source;").unwrap();
    drop(db);
    let r = Registry::open(&path, KEY).unwrap();
    let after = r.account(&android.id).unwrap();
    assert_eq!(before.id, after.id);
    assert_eq!(before.last_catalog_at, after.last_catalog_at);
    assert_eq!(
        r.account_apps(&android.id).unwrap()[0]["source"],
        "retained"
    );
    support::catalog(&r, &store.connector_id, Platform::Android, &[]);
    support::catalog(&r, &store.connector_id, Platform::Ios, &[]);
    assert!(r.account_apps(&ios.id).unwrap().is_empty());
    assert_eq!(r.account_apps(&android.id).unwrap()[0]["enabled"], false);
    assert_eq!(r.account_apps(&android.id).unwrap()[0]["name"], APP);
    drop(r);
    let r = Registry::open(&path, KEY).unwrap();
    assert_eq!(r.account_apps(&android.id).unwrap().len(), 1);
    r.remove_android_app(&android.id, APP).unwrap();
    support::catalog(&r, &store.connector_id, Platform::Android, &[APP]);
    assert!(r.account_apps(&android.id).unwrap().is_empty());
}

#[tokio::test]
async fn manual_android_routes_reject_ios_duplicates_bad_input_and_business_tokens() {
    let state = state();
    let a = node(&state.registry, "A");
    let b = node(&state.registry, "B");
    for (node, platform) in [
        (&a, Platform::Android),
        (&a, Platform::Ios),
        (&b, Platform::Android),
    ] {
        support::catalog(&state.registry, &node.connector_id, platform, &[]);
    }
    let accounts = state.registry.accounts().unwrap();
    let android = accounts
        .iter()
        .find(|v| v.store_connector_id == a.connector_id && v.platform == Platform::Android)
        .unwrap();
    let ios = accounts
        .iter()
        .find(|v| v.platform == Platform::Ios)
        .unwrap();
    let other = accounts
        .iter()
        .find(|v| v.store_connector_id == b.connector_id)
        .unwrap();
    let app = store_api::router(state.clone());
    for prefix in ["/v1/accounts", "/connector-setup/api/accounts"] {
        let path = format!("{prefix}/{}/apps", android.id);
        let remove = format!("{path}/{APP}");
        assert_eq!(
            call(&app, "POST", &path, CLIENT, json!({"bundleIdentifier":APP}))
                .await
                .0,
            401
        );
        assert_eq!(
            call(&app, "DELETE", &remove, CLIENT, Value::Null).await.0,
            401
        );
        for bad in [
            json!({"bundleIdentifier":"../bad"}),
            json!({"bundleIdentifier":APP,"extra":true}),
            json!({"bundleIdentifier":APP,"name":"x".repeat(161)}),
            json!({"name":"missing package"}),
        ] {
            assert_eq!(call(&app, "POST", &path, ADMIN, bad).await.0, 422);
        }
        assert_eq!(
            call(&app, "POST", &path, ADMIN, json!({"bundleIdentifier":APP}))
                .await
                .0,
            201
        );
        assert_eq!(
            call(&app, "POST", &path, ADMIN, json!({"bundleIdentifier":APP}))
                .await
                .1["error"]["code"],
            "app_exists"
        );
        assert_eq!(
            call(
                &app,
                "POST",
                &format!("{prefix}/{}/apps", other.id),
                ADMIN,
                json!({"bundleIdentifier":APP})
            )
            .await
            .1["error"]["code"],
            "account_ambiguous"
        );
        assert_eq!(
            call(
                &app,
                "POST",
                &format!("{prefix}/{}/apps", ios.id),
                ADMIN,
                json!({"bundleIdentifier":APP})
            )
            .await
            .1["error"]["code"],
            "android_directory_required"
        );
        assert_eq!(
            call(
                &app,
                "DELETE",
                &format!("{prefix}/{}/apps/{APP}", ios.id),
                ADMIN,
                Value::Null
            )
            .await
            .0,
            422
        );
        assert_eq!(
            call(&app, "DELETE", &remove, ADMIN, Value::Null).await.0,
            204
        );
    }
    let login = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/connector-setup/api/session")
                .header("x-forwarded-proto", "https")
                .header("x-testflying-setup", "1")
                .header("authorization", format!("Bearer {ADMIN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(login.status(), 204);
    let cookie = login.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap();
    for (method, scheme, marked, expected) in [
        ("POST", "http", true, 403),
        ("POST", "https", false, 401),
        ("POST", "https", true, 201),
        ("DELETE", "http", true, 403),
        ("DELETE", "https", false, 401),
        ("DELETE", "https", true, 204),
    ] {
        let mut request = Request::builder()
            .method(method)
            .uri(format!(
                "/connector-setup/api/accounts/{}/apps{}",
                android.id,
                if method == "DELETE" {
                    format!("/{APP}")
                } else {
                    String::new()
                }
            ))
            .header("x-forwarded-proto", scheme)
            .header("cookie", cookie)
            .header("content-type", "application/json");
        if marked {
            request = request.header("x-testflying-setup", "1");
        }
        let response = app
            .clone()
            .oneshot(
                request
                    .body(Body::from(json!({"bundleIdentifier":APP}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected, "{method} {scheme} {marked}");
    }
    let failed = report(
        Platform::Android,
        CatalogStatus::Failed,
        Some(&"a".repeat(64)),
    );
    state
        .registry
        .ingest_catalog(&a.connector_id, &state.registry.token(&a), true, &failed)
        .unwrap();
    state
        .registry
        .add_android_app(&android.id, APP, Some(" "))
        .unwrap();
    assert_eq!(
        state.registry.account(&android.id).unwrap().catalog_status,
        "failed"
    );
    assert_eq!(
        state
            .registry
            .resolve_account(Platform::Android, APP)
            .unwrap()
            .id,
        android.id
    );
    let changed = report(
        Platform::Android,
        CatalogStatus::Ready,
        Some(&"b".repeat(64)),
    );
    state
        .registry
        .ingest_catalog(&a.connector_id, &state.registry.token(&a), true, &changed)
        .unwrap();
    assert_eq!(
        state
            .registry
            .add_android_app(&android.id, "com.example.new", None)
            .unwrap_err()
            .code,
        "identity_changed"
    );
}
fn node(r: &Registry, name: &str) -> Connector {
    r.create(ConnectorInput { name: name.into() }).unwrap()
}
fn report(platform: Platform, status: CatalogStatus, fingerprint: Option<&str>) -> CatalogReport {
    CatalogReport {
        schema_version: 1,
        report_id: uuid::Uuid::new_v4().to_string(),
        platform,
        status,
        account_fingerprint: fingerprint.map(str::to_owned),
        apps: vec![],
        error_code: None,
    }
}
fn patch(value: Value) -> AccountPatch {
    serde_json::from_value(value).unwrap()
}

#[test]
fn precise_candidates_verified_binding_suppression_and_disabled_accounts() {
    let r = registry();
    let a = node(&r, "A");
    let b = node(&r, "B");
    assert_eq!(
        r.resolve(Platform::Ios, APP).unwrap_err().code,
        "app_not_found"
    );
    assert_eq!(
        r.bind(Platform::Ios, APP, &a.connector_id)
            .unwrap_err()
            .code,
        "account_membership_unverified"
    );
    support::catalog(&r, &a.connector_id, Platform::Ios, &[APP]);
    let account = r.resolve_account(Platform::Ios, APP).unwrap();
    assert_eq!(
        r.resolve(Platform::Ios, APP).unwrap().connector_id,
        a.connector_id
    );
    assert_eq!(r.get(&a.connector_id).unwrap().app_count, 1);
    assert!(r.resolve(Platform::Android, APP).is_err());
    assert!(r.resolve(Platform::Ios, "com.example.Account").is_err());
    support::catalog(&r, &b.connector_id, Platform::Ios, &[APP]);
    assert_eq!(r.resolve(Platform::Ios, APP).unwrap_err().status, 409);
    assert_eq!(r.get(&a.connector_id).unwrap().app_count, 0);
    assert_eq!(
        r.account_apps(&account.id).unwrap()[0]["resolutionStatus"],
        "account_ambiguous"
    );
    assert_eq!(
        r.bind_account(Platform::Ios, APP, &account.id)
            .unwrap_err()
            .code,
        "account_ambiguous"
    );
    support::catalog(&r, &b.connector_id, Platform::Ios, &[]);
    assert_eq!(
        r.resolve(Platform::Ios, APP).unwrap().connector_id,
        a.connector_id
    );
    assert_eq!(
        r.bind_account(Platform::Android, APP, &account.id)
            .unwrap_err()
            .code,
        "account_membership_unverified"
    );
    assert_eq!(
        r.bind_account(Platform::Ios, "com.example.unknown", &account.id)
            .unwrap_err()
            .status,
        422
    );
    r.set_app_enabled(&account.id, Platform::Ios, APP, false)
        .unwrap();
    support::catalog(&r, &a.connector_id, Platform::Ios, &[APP]);
    assert_eq!(
        r.resolve(Platform::Ios, APP).unwrap_err().code,
        "app_forwarding_disabled"
    );
    assert!(r.bind(Platform::Ios, APP, &a.connector_id).is_err());
    assert_eq!(r.account_apps(&account.id).unwrap()[0]["enabled"], false);
    r.set_app_enabled(&account.id, Platform::Ios, APP, true)
        .unwrap();
    r.patch_account(&account.id, patch(json!({"enabled":false})))
        .unwrap();
    assert_eq!(
        r.resolve(Platform::Ios, APP).unwrap_err().code,
        "account_disabled"
    );
    r.patch_account(&account.id, patch(json!({"enabled":true})))
        .unwrap();
    r.set_enabled(&a.connector_id, false).unwrap();
    assert_eq!(
        r.resolve(Platform::Ios, APP).unwrap_err().code,
        "connector_disabled"
    );
    let screenshot = r
        .create_with_kind(
            ConnectorInput { name: "Mac".into() },
            ConnectorKind::Screenshot,
        )
        .unwrap();
    assert_eq!(
        r.bind(Platform::Ios, APP, &screenshot.connector_id)
            .unwrap_err()
            .code,
        "invalid_connector_kind"
    );
    assert!(r.create_account(serde_json::from_value(json!({"name":"Invalid","platform":"ios","storeConnectorId":screenshot.connector_id})).unwrap()).is_err());
}

#[test]
fn full_reports_retain_last_good_on_failure_and_require_identity_acceptance() {
    let r = registry();
    let n = node(&r, "A");
    let token = r.token(&n);
    support::catalog(&r, &n.connector_id, Platform::Ios, &[APP]);
    let a = r.resolve_account(Platform::Ios, APP).unwrap();
    let mut failed = report(Platform::Ios, CatalogStatus::Failed, None);
    failed.error_code = Some("catalog_unavailable".into());
    r.ingest_catalog(&n.connector_id, &token, true, &failed)
        .unwrap();
    assert_eq!(r.account(&a.id).unwrap().catalog_status, "failed");
    assert_eq!(r.account(&a.id).unwrap().last_catalog_at, a.last_catalog_at);
    assert_eq!(
        r.resolve(Platform::Ios, APP).unwrap().connector_id,
        n.connector_id
    );
    let changed = report(Platform::Ios, CatalogStatus::Ready, Some(&"b".repeat(64)));
    r.ingest_catalog(&n.connector_id, &token, true, &changed)
        .unwrap();
    assert_eq!(r.account(&a.id).unwrap().catalog_status, "identity_changed");
    assert_eq!(r.account_apps(&a.id).unwrap().len(), 1);
    assert_eq!(
        r.resolve(Platform::Ios, APP).unwrap_err().code,
        "identity_changed"
    );
    assert_eq!(
        r.bind_account(Platform::Ios, APP, &a.id).unwrap_err().code,
        "identity_changed"
    );
    let second = node(&r, "Second");
    support::catalog(&r, &second.connector_id, Platform::Ios, &[APP]);
    assert_eq!(
        r.resolve(Platform::Ios, APP).unwrap_err().code,
        "account_ambiguous"
    );
    assert_eq!(r.get(&second.connector_id).unwrap().app_count, 0);
    r.set_enabled(&second.connector_id, false).unwrap();
    let old = report(Platform::Ios, CatalogStatus::Failed, None);
    r.ingest_catalog(&n.connector_id, &token, true, &old)
        .unwrap();
    assert_eq!(r.account(&a.id).unwrap().catalog_status, "identity_changed");
    r.patch_account(&a.id, patch(json!({"acceptIdentityChange":true})))
        .unwrap();
    assert_eq!(r.account(&a.id).unwrap().catalog_status, "awaiting_catalog");
    assert!(r.account_apps(&a.id).unwrap().is_empty());
    let mut fresh = changed.clone();
    fresh.report_id = uuid::Uuid::new_v4().to_string();
    r.ingest_catalog(&n.connector_id, &token, true, &fresh)
        .unwrap();
    assert_eq!(r.account(&a.id).unwrap().catalog_status, "ready");
    // A replay cannot revert or reapply a previously seen report.
    r.ingest_catalog(&n.connector_id, &token, true, &failed)
        .unwrap();
    assert_eq!(r.account(&a.id).unwrap().catalog_status, "ready");
    assert_eq!(
        r.patch_account(&a.id, patch(json!({"acceptIdentityChange":true})))
            .unwrap_err()
            .code,
        "identity_change_not_pending"
    );
    r.rotate(&n.connector_id).unwrap();
    assert_eq!(
        r.ingest_catalog(&n.connector_id, &token, true, &fresh)
            .unwrap_err()
            .code,
        "invalid_node_token"
    );
    let current = r.get(&n.connector_id).unwrap();
    assert_eq!(
        r.ingest_catalog(&n.connector_id, &r.token(&current), false, &fresh)
            .unwrap_err()
            .code,
        "catalog_upgrade_required"
    );
    let screenshot = r
        .create_with_kind(
            ConnectorInput { name: "Mac".into() },
            ConnectorKind::Screenshot,
        )
        .unwrap();
    assert_eq!(
        r.ingest_catalog(
            &screenshot.connector_id,
            &r.token(&screenshot),
            true,
            &fresh
        )
        .unwrap_err()
        .code,
        "invalid_node_token"
    );
}

#[test]
fn app_forwarding_persists_across_catalogs_and_restart_without_manual_ownership() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("registry.sqlite");
    let r = Registry::open(&path, KEY).unwrap();
    let n = node(&r, "Both platforms");
    for platform in [Platform::Ios, Platform::Android] {
        support::catalog(&r, &n.connector_id, platform, &[APP, "com.example.peer"]);
    }
    let a = r.resolve_account(Platform::Ios, APP).unwrap();
    assert_eq!(r.account_apps(&a.id).unwrap()[0]["enabled"], true);
    r.set_app_enabled(&a.id, Platform::Ios, APP, false).unwrap();
    support::catalog(
        &r,
        &n.connector_id,
        Platform::Ios,
        &[APP, "com.example.peer"],
    );
    let mut failed = report(Platform::Ios, CatalogStatus::Failed, None);
    failed.error_code = Some("catalog_discovery_failed".into());
    r.ingest_catalog(&n.connector_id, &r.token(&n), true, &failed)
        .unwrap();
    drop(r);
    let r = Registry::open(&path, KEY).unwrap();
    assert_eq!(
        r.resolve(Platform::Ios, APP).unwrap_err().code,
        "app_forwarding_disabled"
    );
    assert!(r.resolve(Platform::Android, APP).is_ok());
    assert!(r.resolve(Platform::Ios, "com.example.peer").is_ok());
    assert_eq!(r.get(&n.connector_id).unwrap().app_count, 3);
    assert_eq!(
        r.set_app_enabled(&a.id, Platform::Android, APP, false)
            .unwrap_err()
            .code,
        "app_catalog_changed"
    );
    assert_eq!(
        r.set_app_enabled(&a.id, Platform::Ios, "com.example.unknown", false)
            .unwrap_err()
            .code,
        "app_catalog_changed"
    );
    r.set_app_enabled(&a.id, Platform::Ios, APP, true).unwrap();
    assert!(r.resolve(Platform::Ios, APP).is_ok());
    support::catalog(&r, &n.connector_id, Platform::Ios, &[]);
    assert_eq!(
        r.set_app_enabled(&a.id, Platform::Ios, APP, false)
            .unwrap_err()
            .code,
        "app_catalog_changed"
    );
    assert_eq!(
        r.resolve(Platform::Ios, APP).unwrap_err().code,
        "app_not_found"
    );
}

#[test]
fn disabled_candidates_remain_ambiguous_until_catalog_corrected() {
    for disable_account in [true, false] {
        let r = registry();
        let a = node(&r, "First owner");
        let b = node(&r, "Second owner");
        support::catalog(&r, &a.connector_id, Platform::Ios, &[APP]);
        let first = r.resolve_account(Platform::Ios, APP).unwrap();
        support::catalog(&r, &b.connector_id, Platform::Ios, &[APP]);
        let second = r
            .accounts()
            .unwrap()
            .into_iter()
            .find(|account| account.store_connector_id == b.connector_id)
            .unwrap();
        let mac = r
            .create_with_kind(
                ConnectorInput {
                    name: "Shared screenshot node".into(),
                },
                ConnectorKind::Screenshot,
            )
            .unwrap();
        r.patch_account(
            &first.id,
            patch(json!({"screenshotConnectorId":mac.connector_id})),
        )
        .unwrap();
        r.patch_account(
            &second.id,
            patch(json!({"screenshotConnectorId":mac.connector_id})),
        )
        .unwrap();
        if disable_account {
            r.patch_account(&first.id, patch(json!({"enabled":false})))
                .unwrap();
        } else {
            r.set_enabled(&a.connector_id, false).unwrap();
        }
        let error = r.resolve(Platform::Ios, APP).unwrap_err();
        assert_eq!(error.status, 409);
        assert_eq!(error.code, "account_ambiguous");
        assert_eq!(
            r.bind_account(Platform::Ios, APP, &first.id)
                .unwrap_err()
                .code,
            "account_ambiguous"
        );
        assert_eq!(
            r.bind_account(Platform::Android, APP, &first.id)
                .unwrap_err()
                .code,
            "account_membership_unverified"
        );
        assert_eq!(r.get(&a.connector_id).unwrap().app_count, 0);
        assert_eq!(r.get(&b.connector_id).unwrap().app_count, 0);
        assert_eq!(r.get(&mac.connector_id).unwrap().app_count, 0);
        let row = &r.account_apps(&second.id).unwrap()[0];
        assert_eq!(row["screenshotEnabled"], false);
        assert_eq!(row["screenshotResolutionStatus"], "account_ambiguous");
        assert_eq!(
            r.bind_account(Platform::Ios, APP, &second.id)
                .unwrap_err()
                .code,
            "account_ambiguous"
        );
        if !disable_account {
            r.set_enabled(&a.connector_id, true).unwrap();
        }
        support::catalog(&r, &a.connector_id, Platform::Ios, &[]);
        assert_eq!(
            r.resolve(Platform::Ios, APP).unwrap().connector_id,
            b.connector_id
        );
        assert_eq!(r.get(&b.connector_id).unwrap().app_count, 1);
        assert_eq!(r.get(&mac.connector_id).unwrap().app_count, 1);
    }
}

#[tokio::test]
async fn merchant_disable_preserves_screenshot_branch_and_counts_but_account_disable_blocks_both() {
    let state = state();
    let store = node(&state.registry, "Sole owner");
    support::catalog(&state.registry, &store.connector_id, Platform::Ios, &[APP]);
    let account = state.registry.resolve_account(Platform::Ios, APP).unwrap();
    let mac = state
        .registry
        .create_with_kind(
            ConnectorInput {
                name: "Enabled screenshot node".into(),
            },
            ConnectorKind::Screenshot,
        )
        .unwrap();
    state
        .registry
        .patch_account(
            &account.id,
            patch(json!({"screenshotConnectorId":mac.connector_id})),
        )
        .unwrap();
    assert_eq!(
        state.registry.get(&store.connector_id).unwrap().app_count,
        1
    );
    assert_eq!(state.registry.get(&mac.connector_id).unwrap().app_count, 1);
    state
        .registry
        .set_enabled(&store.connector_id, false)
        .unwrap();
    let error = state.registry.resolve(Platform::Ios, APP).unwrap_err();
    assert_eq!(error.status, 503);
    assert_eq!(error.code, "connector_disabled");
    assert_eq!(
        state.registry.get(&store.connector_id).unwrap().app_count,
        0
    );
    assert_eq!(state.registry.get(&mac.connector_id).unwrap().app_count, 1);
    let app = store_api::router(state.clone());
    let row = &state.registry.account_apps(&account.id).unwrap()[0];
    assert_eq!(row["resolutionStatus"], "resolved");
    assert_eq!(row["screenshotEnabled"], true);
    assert_eq!(row["screenshotResolutionStatus"], "resolved");
    let shot = format!("/v1/apps/ios/{APP}/screenshot-connector");
    assert_eq!(
        call(&app, "GET", &shot, ADMIN, Value::Null).await.1["connector"]["connectorId"],
        mac.connector_id
    );
    let token = call(
        &app,
        "POST",
        "/v1/integrations",
        ADMIN,
        json!({"name":"Screenshot branch"}),
    )
    .await
    .1["token"]
        .as_str()
        .unwrap()
        .to_owned();
    let request = json!({"platform":"ios","bundleIdentifier":APP,"prompt":"Synthetic queue only","target":[{"locale":"en-US","page":"home","device":"phone"}]});
    let queued = call(&app, "POST", "/v1/screenshot-jobs", &token, request.clone()).await;
    assert_eq!(queued.0, 202);
    assert_eq!(queued.1["connectorId"], mac.connector_id);
    state
        .registry
        .patch_account(&account.id, patch(json!({"enabled":false})))
        .unwrap();
    let error = state.registry.resolve(Platform::Ios, APP).unwrap_err();
    assert_eq!(error.status, 503);
    assert_eq!(error.code, "account_disabled");
    let row = &state.registry.account_apps(&account.id).unwrap()[0];
    assert_eq!(row["screenshotEnabled"], false);
    assert_eq!(row["screenshotResolutionStatus"], "account_disabled");
    assert_eq!(
        call(&app, "GET", &shot, ADMIN, Value::Null).await.1["error"]["code"],
        "account_disabled"
    );
    assert_eq!(
        call(&app, "POST", "/v1/screenshot-jobs", &token, request.clone())
            .await
            .0,
        503
    );
    assert_eq!(state.registry.get(&mac.connector_id).unwrap().app_count, 0);
    state
        .registry
        .patch_account(&account.id, patch(json!({"enabled":true})))
        .unwrap();
    state
        .registry
        .set_enabled(&mac.connector_id, false)
        .unwrap();
    let shot_error = call(&app, "GET", &shot, ADMIN, Value::Null).await;
    assert_eq!(shot_error.0, 503);
    assert_eq!(
        shot_error.1["error"]["code"],
        "screenshot_connector_disabled"
    );
    assert_eq!(state.registry.get(&mac.connector_id).unwrap().app_count, 0);
    let row = &state.registry.account_apps(&account.id).unwrap()[0];
    assert_eq!(row["screenshotEnabled"], false);
    assert_eq!(
        row["screenshotResolutionStatus"],
        "screenshot_connector_disabled"
    );
}

#[test]
fn migration_is_one_time_preserves_settings_snapshots_and_flags_conflicts_and_orphans() {
    for conflict in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("registry.sqlite");
        let r = Registry::open(&path, KEY).unwrap();
        let store = node(&r, "Legacy");
        let mac = r
            .create_with_kind(
                ConnectorInput { name: "Mac".into() },
                ConnectorKind::Screenshot,
            )
            .unwrap();
        let other = r
            .create_with_kind(
                ConnectorInput {
                    name: "Other Mac".into(),
                },
                ConnectorKind::Screenshot,
            )
            .unwrap();
        r.set_forwarding(Platform::Android, false).unwrap();
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute("DELETE FROM account_migrations", []).unwrap();
        for identifier in [APP, "com.example.second"] {
            db.execute(
                "INSERT INTO app_bindings VALUES('ios',?1,?2)",
                rusqlite::params![identifier, store.connector_id],
            )
            .unwrap();
            let screenshot = if conflict && identifier != APP {
                &other.connector_id
            } else {
                &mac.connector_id
            };
            db.execute(
                "INSERT INTO screenshot_app_bindings VALUES('ios',?1,?2)",
                rusqlite::params![identifier, screenshot],
            )
            .unwrap();
        }
        db.execute(
            "INSERT INTO screenshot_app_bindings VALUES('ios','com.example.orphan',?1)",
            [&mac.connector_id],
        )
        .unwrap();
        db.execute("INSERT INTO store_image_jobs VALUES('old-store-job',NULL,'ios',?1,?2,1,1,'queued','{}',1)",rusqlite::params![APP,store.connector_id]).unwrap();
        db.execute("INSERT INTO screenshot_jobs(id,connector_id,integration_id,profile,prompt,status,created_at,deadline) VALUES('old-shot',?1,'old-owner','','original','queued',1,2)",[&mac.connector_id]).unwrap();
        drop(r);
        let r = Registry::open(&path, KEY).unwrap();
        let a = r.accounts().unwrap().remove(0);
        assert_eq!(
            r.resolve_account(Platform::Ios, APP).unwrap_err().code,
            "app_not_found"
        );
        assert_eq!(r.accounts().unwrap().len(), 1);
        assert_eq!(
            a.screenshot_connector_id,
            if conflict {
                None
            } else {
                Some(mac.connector_id.clone())
            }
        );
        assert_eq!(
            a.migration_error.as_deref(),
            if conflict {
                Some("legacy_screenshot_conflict")
            } else {
                None
            }
        );
        assert_eq!(
            r.account_migration_issues().unwrap()[0]["code"],
            "legacy_screenshot_orphan"
        );
        assert!(r.account_apps(&a.id).unwrap().is_empty());
        assert!(!r.forwarding().unwrap().android);
        assert_eq!(
            r.bind(Platform::Ios, APP, &store.connector_id)
                .unwrap_err()
                .code,
            "account_membership_unverified"
        );
        let failed = report(Platform::Ios, CatalogStatus::Failed, None);
        r.ingest_catalog(&store.connector_id, &r.token(&store), true, &failed)
            .unwrap();
        assert!(r.resolve(Platform::Ios, APP).is_err());
        drop(r);
        let r = Registry::open(&path, KEY).unwrap();
        assert_eq!(r.accounts().unwrap()[0].id, a.id);
        support::catalog(
            &r,
            &store.connector_id,
            Platform::Ios,
            &["com.example.second"],
        );
        assert_eq!(
            r.resolve(Platform::Ios, APP).unwrap_err().code,
            "app_not_found"
        );
        assert!(r.resolve(Platform::Ios, "com.example.second").is_ok());
        assert_eq!(
            db.query_row("SELECT count(*) FROM app_bindings", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            2
        );
        r.patch_account(
            &a.id,
            patch(json!({"screenshotConnectorId":other.connector_id})),
        )
        .unwrap();
        let replacement = node(&r, "Replacement store");
        r.patch_account(
            &a.id,
            patch(json!({"storeConnectorId":replacement.connector_id})),
        )
        .unwrap();
        assert_eq!(
            r.resolve(Platform::Ios, "com.example.second")
                .unwrap_err()
                .code,
            "app_not_found"
        );
        support::catalog(
            &r,
            &replacement.connector_id,
            Platform::Ios,
            &["com.example.second"],
        );
        assert_eq!(
            r.resolve(Platform::Ios, "com.example.second")
                .unwrap()
                .connector_id,
            replacement.connector_id
        );
        assert_eq!(
            db.query_row(
                "SELECT connector_id FROM screenshot_jobs WHERE id='old-shot'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            mac.connector_id
        );
        assert_eq!(
            db.query_row(
                "SELECT connector FROM store_image_jobs WHERE id='old-store-job'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            store.connector_id
        );
    }
}

#[tokio::test]
async fn business_connector_bindings_omit_version_metadata_while_admin_inventory_keeps_it() {
    let state = state();
    let store = node(&state.registry, "Versioned store");
    let token = state.registry.token(&store);
    state
        .registry
        .register_connection(
            &store.connector_id,
            &token,
            None,
            Some(store_protocol::release::CONNECTOR_VERSION),
        )
        .unwrap();
    support::catalog(&state.registry, &store.connector_id, Platform::Ios, &[APP]);
    let app = store_api::router(state);
    let (status, issued) = call(
        &app,
        "POST",
        "/v1/integrations",
        ADMIN,
        json!({"name":"Version metadata boundary"}),
    )
    .await;
    assert_eq!(status, 201);
    let integration = issued["token"].as_str().unwrap();
    let path = format!("/v1/apps/ios/{APP}/connector");
    let (status, bound) = call(
        &app,
        "PUT",
        &path,
        integration,
        json!({"connectorId":store.connector_id}),
    )
    .await;
    assert_eq!(status, 200);
    let (status, resolved) = call(&app, "GET", &path, ADMIN, Value::Null).await;
    assert_eq!(status, 200);
    for response in [bound, resolved] {
        assert_eq!(response["connectorId"], store.connector_id);
        for field in [
            "connectorVersion",
            "latestConnectorVersion",
            "versionStatus",
        ] {
            assert!(response.get(field).is_none(), "{field}");
        }
    }
    let (status, inventory) = call(&app, "GET", "/v1/connectors", ADMIN, Value::Null).await;
    assert_eq!(status, 200);
    assert_eq!(
        inventory["connectors"][0]["connectorVersion"],
        store_protocol::release::CONNECTOR_VERSION
    );
    assert_eq!(
        inventory["connectors"][0]["latestConnectorVersion"],
        store_protocol::release::CONNECTOR_VERSION
    );
    assert_eq!(inventory["connectors"][0]["versionStatus"], "current");
}

fn state() -> Arc<ApiState> {
    ApiState::new(
        registry(),
        ADMIN.into(),
        CLIENT.into(),
        "https://center.invalid".into(),
        PathBuf::from("unused"),
        Duration::from_secs(1),
    )
    .unwrap()
}
async fn call(app: &Router, method: &str, path: &str, token: &str, body: Value) -> (u16, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("authorization", format!("Bearer {token}"))
                .header("x-forwarded-proto", "https")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn all_account_routes_keep_admin_query_binding_and_session_boundaries() {
    let state = state();
    let a = node(&state.registry, "A");
    let b = node(&state.registry, "B");
    support::catalog(&state.registry, &a.connector_id, Platform::Ios, &[APP]);
    support::catalog(&state.registry, &b.connector_id, Platform::Ios, &[APP]);
    let account = state
        .registry
        .accounts()
        .unwrap()
        .into_iter()
        .find(|v| v.store_connector_id == a.connector_id)
        .unwrap();
    let app = store_api::router(state.clone());
    let create = json!({"name":"Android","platform":"android","storeConnectorId":a.connector_id});
    for prefix in ["/v1/accounts", "/connector-setup/api/accounts"] {
        for (method, path, body) in [
            ("GET", prefix.to_owned(), Value::Null),
            ("POST", prefix.to_owned(), create.clone()),
            (
                "PATCH",
                format!("{prefix}/{}", account.id),
                json!({"name":"Renamed"}),
            ),
            ("GET", format!("{prefix}/{}/apps", account.id), Value::Null),
            (
                "POST",
                format!("{prefix}/{}/refresh", account.id),
                json!({}),
            ),
            (
                "PUT",
                format!("{prefix}/{}/apps/ios/{APP}/forwarding", account.id),
                json!({"enabled":false}),
            ),
        ] {
            assert_eq!(
                call(&app, method, &path, CLIENT, body).await.0,
                401,
                "{method} {path}"
            );
        }
        assert_eq!(call(&app, "GET", prefix, ADMIN, Value::Null).await.0, 200);
        assert_eq!(
            call(
                &app,
                "GET",
                &format!("{prefix}/{}/apps", account.id),
                ADMIN,
                Value::Null
            )
            .await
            .1["apps"][0]["resolutionStatus"],
            "account_ambiguous"
        );
    }
    assert_eq!(
        call(&app, "POST", "/v1/accounts", ADMIN, create).await.0,
        201
    );
    let integration = call(
        &app,
        "POST",
        "/v1/integrations",
        ADMIN,
        json!({"name":"All accounts"}),
    )
    .await
    .1["token"]
        .as_str()
        .unwrap()
        .to_owned();
    let binding = format!("/v1/apps/ios/{APP}/account");
    for path in [
        "/v1/store-management/accounts".to_owned(),
        format!("/v1/store-management/accounts/{}/apps", account.id),
        binding.clone(),
    ] {
        assert_eq!(call(&app, "GET", &path, "wrong", Value::Null).await.0, 401);
        let (status, body) = call(&app, "GET", &path, &integration, Value::Null).await;
        assert_eq!(status, if path == binding { 409 } else { 200 });
        assert!(body.get("storeConnectorId").is_none());
        assert!(!body.to_string().contains(&a.connector_id));
        assert!(!body.to_string().contains(&b.connector_id));
    }
    let bind = json!({"accountId":account.id});
    for prefix in ["/v1/accounts", "/connector-setup/api/accounts"] {
        let forwarding = format!("{prefix}/{}/apps/ios/{APP}/forwarding", account.id);
        assert_eq!(
            call(
                &app,
                "PUT",
                &forwarding,
                &integration,
                json!({"enabled":false})
            )
            .await
            .0,
            401
        );
        assert_eq!(
            call(&app, "PUT", &forwarding, ADMIN, json!({"enabled":"false"}))
                .await
                .0,
            422
        );
    }
    assert_eq!(
        call(&app, "PUT", &binding, CLIENT, bind.clone()).await.0,
        401
    );
    assert_eq!(
        call(&app, "PUT", &binding, &integration, bind.clone())
            .await
            .0,
        409
    );
    support::catalog(&state.registry, &b.connector_id, Platform::Ios, &[]);
    assert_eq!(
        call(&app, "PUT", &binding, &integration, bind.clone())
            .await
            .0,
        200
    );
    assert_eq!(
        call(&app, "GET", &binding, CLIENT, Value::Null).await.1["account"]["id"],
        account.id
    );
    assert_eq!(
        call(
            &app,
            "PUT",
            "/v1/apps/android/com.example.account/account",
            &integration,
            bind.clone()
        )
        .await
        .0,
        422
    );
    assert_eq!(
        call(
            &app,
            "PUT",
            "/v1/apps/ios/com.example.unknown/account",
            &integration,
            bind
        )
        .await
        .0,
        422
    );
    let refresh = call(
        &app,
        "POST",
        &format!("/v1/accounts/{}/refresh", account.id),
        ADMIN,
        json!({}),
    )
    .await;
    assert_eq!(refresh.0, 503);
    assert_eq!(refresh.1["error"]["code"], "catalog_offline");
}

#[tokio::test]
async fn obsolete_screenshot_delete_is_rejected_and_app_switch_controls_both_branches() {
    let state = state();
    let store = node(&state.registry, "Store");
    support::catalog(
        &state.registry,
        &store.connector_id,
        Platform::Ios,
        &[APP, "com.example.peer"],
    );
    let account = state.registry.resolve_account(Platform::Ios, APP).unwrap();
    let mac = state
        .registry
        .create_with_kind(
            ConnectorInput { name: "Mac".into() },
            ConnectorKind::Screenshot,
        )
        .unwrap();
    let other = state
        .registry
        .create_with_kind(
            ConnectorInput {
                name: "Other".into(),
            },
            ConnectorKind::Screenshot,
        )
        .unwrap();
    let app = store_api::router(state.clone());
    let path = format!("/v1/apps/ios/{APP}/screenshot-connector");
    assert_eq!(
        call(
            &app,
            "PUT",
            &path,
            ADMIN,
            json!({"connectorId":mac.connector_id})
        )
        .await
        .0,
        200
    );
    assert_eq!(
        call(
            &app,
            "PUT",
            &path,
            ADMIN,
            json!({"connectorId":other.connector_id})
        )
        .await
        .0,
        409
    );
    assert_eq!(call(&app, "DELETE", &path, ADMIN, Value::Null).await.0, 410);
    let forwarding = format!("/v1/accounts/{}/apps/ios/{APP}/forwarding", account.id);
    assert_eq!(
        call(&app, "PUT", &forwarding, ADMIN, json!({"enabled":false}))
            .await
            .0,
        204
    );
    assert_eq!(call(&app, "GET", &path, ADMIN, Value::Null).await.0, 503);
    assert_eq!(
        state.registry.resolve(Platform::Ios, APP).unwrap_err().code,
        "app_forwarding_disabled"
    );
    let rows = state.registry.account_apps(&account.id).unwrap();
    let disabled = rows
        .iter()
        .find(|row| row["bundleIdentifier"] == APP)
        .unwrap();
    assert_eq!(disabled["resolutionStatus"], "app_forwarding_disabled");
    assert_eq!(disabled["enabled"], false);
    assert_eq!(disabled["screenshotEnabled"], false);
    assert_eq!(
        disabled["screenshotResolutionStatus"],
        "app_forwarding_disabled"
    );
    let peer = rows
        .iter()
        .find(|row| row["bundleIdentifier"] == "com.example.peer")
        .unwrap();
    assert_eq!(peer["screenshotEnabled"], true);
    assert_eq!(peer["screenshotResolutionStatus"], "resolved");
    assert_eq!(
        state
            .registry
            .account(&account.id)
            .unwrap()
            .screenshot_connector_id,
        Some(mac.connector_id.clone())
    );
    assert_eq!(
        call(
            &app,
            "GET",
            "/v1/apps/ios/com.example.peer/screenshot-connector",
            ADMIN,
            Value::Null
        )
        .await
        .1["connector"]["connectorId"],
        mac.connector_id
    );
    assert_eq!(
        call(
            &app,
            "PUT",
            &path,
            ADMIN,
            json!({"connectorId":mac.connector_id})
        )
        .await
        .0,
        503
    );
    assert_eq!(
        call(&app, "PUT", &forwarding, ADMIN, json!({"enabled":true}))
            .await
            .0,
        204
    );
    assert_eq!(call(&app, "GET", &path, ADMIN, Value::Null).await.0, 200);
    assert_eq!(
        state.registry.account_apps(&account.id).unwrap()[0]["screenshotEnabled"],
        true
    );
}

#[tokio::test]
async fn setup_session_can_toggle_only_current_catalog_rows() {
    let state = state();
    let a = node(&state.registry, "A");
    let b = node(&state.registry, "B");
    support::catalog(&state.registry, &a.connector_id, Platform::Ios, &[APP]);
    support::catalog(&state.registry, &b.connector_id, Platform::Ios, &[APP]);
    let account = state
        .registry
        .accounts()
        .unwrap()
        .into_iter()
        .find(|value| value.store_connector_id == a.connector_id)
        .unwrap();
    let app = store_api::router(state.clone());
    let login = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/connector-setup/api/session")
                .header("x-forwarded-proto", "https")
                .header("x-testflying-setup", "1")
                .header("authorization", format!("Bearer {ADMIN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(login.status(), 204);
    let cookie = login.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    for (identifier, scheme, marked, expected) in [
        (APP, "http", true, 403),
        (APP, "https", false, 401),
        ("com.example.unverified", "https", true, 409),
        (APP, "https", true, 204),
    ] {
        let mut request = Request::builder()
            .method("PUT")
            .uri(format!(
                "/connector-setup/api/accounts/{}/apps/ios/{identifier}/forwarding",
                account.id
            ))
            .header("x-forwarded-proto", scheme)
            .header("cookie", &cookie)
            .header("content-type", "application/json");
        if marked {
            request = request.header("x-testflying-setup", "1");
        }
        let response = app
            .clone()
            .oneshot(
                request
                    .body(Body::from(json!({"enabled":false}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
    assert_eq!(
        state
            .registry
            .resolve_account(Platform::Ios, APP)
            .unwrap_err()
            .code,
        "account_ambiguous"
    );
    let other = state
        .registry
        .accounts()
        .unwrap()
        .into_iter()
        .find(|value| value.store_connector_id == b.connector_id)
        .unwrap();
    let row = &state.registry.account_apps(&other.id).unwrap()[0];
    assert_eq!(row["enabled"], false);
    assert_eq!(row["resolutionStatus"], "account_ambiguous");
    assert_eq!(row["screenshotEnabled"], false);
    assert_eq!(row["screenshotResolutionStatus"], "account_ambiguous");
    state
        .registry
        .set_app_enabled(&account.id, Platform::Ios, APP, true)
        .unwrap();
    assert_eq!(
        state
            .registry
            .resolve_account(Platform::Ios, APP)
            .unwrap_err()
            .code,
        "account_ambiguous"
    );
}
