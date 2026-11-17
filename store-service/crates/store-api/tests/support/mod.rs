use store_api::registry::Registry;
use store_protocol::{
    catalog::{CatalogApp, CatalogReport, CatalogStatus},
    Platform,
};

/// Submit a complete synthetic directory through the same authenticated path as WSS.
pub fn catalog(registry: &Registry, node_id: &str, platform: Platform, identifiers: &[&str]) {
    let node = registry.get(node_id).unwrap();
    registry
        .ingest_catalog(
            node_id,
            &registry.token(&node),
            true,
            &CatalogReport {
                schema_version: 1,
                report_id: uuid::Uuid::new_v4().to_string(),
                platform,
                status: CatalogStatus::Ready,
                account_fingerprint: Some("a".repeat(64)),
                apps: identifiers
                    .iter()
                    .map(|identifier| CatalogApp {
                        bundle_identifier: (*identifier).into(),
                        name: (*identifier).into(),
                        store_app_id: None,
                    })
                    .collect(),
                error_code: None,
            },
        )
        .unwrap();
}
