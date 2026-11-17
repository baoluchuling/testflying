//! Credential-free, complete account application catalogs.
use crate::{crypto::MAX_JSON_BYTES, validate_identifier, Platform, StoreError};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const SCHEMA_VERSION: u8 = 1;
pub const MAX_APPS: usize = 10_000;
pub const MAX_REPORT_BYTES: usize = MAX_JSON_BYTES - 65_536;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CatalogApp {
    pub bundle_identifier: String,
    pub name: String,
    pub store_app_id: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccountCatalog {
    pub fingerprint: String,
    pub apps: Vec<CatalogApp>,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CatalogStatus {
    Ready,
    Failed,
    NotConfigured,
    Unsupported,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CatalogReport {
    pub schema_version: u8,
    pub report_id: String,
    pub platform: Platform,
    pub status: CatalogStatus,
    pub account_fingerprint: Option<String>,
    pub apps: Vec<CatalogApp>,
    pub error_code: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CatalogRequest {
    pub request_id: String,
    pub catalog_action: String,
}
pub fn invalid() -> StoreError {
    StoreError::new(422, "invalid_catalog", "Invalid account catalog")
}
pub fn validate_apps(apps: &[CatalogApp]) -> Result<(), StoreError> {
    if apps.len() > MAX_APPS {
        return Err(invalid());
    }
    let mut identifiers = HashSet::new();
    for app in apps {
        validate_identifier(&app.bundle_identifier)?;
        if !identifiers.insert(&app.bundle_identifier)
            || app.name.len() > 512
            || app.store_app_id.as_ref().is_some_and(|id| {
                id.is_empty()
                    || id.len() > 160
                    || !id.bytes().all(|b| {
                        b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'/')
                    })
            })
        {
            return Err(invalid());
        }
    }
    Ok(())
}
fn fingerprint_valid(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
impl AccountCatalog {
    pub fn validate(&self) -> Result<(), StoreError> {
        if !fingerprint_valid(&self.fingerprint) {
            return Err(invalid());
        }
        validate_apps(&self.apps)
    }
}
impl CatalogRequest {
    pub fn validate(&self) -> Result<(), StoreError> {
        if self.request_id.is_empty()
            || self.request_id.len() > 128
            || self.catalog_action != "refresh"
        {
            return Err(invalid());
        }
        Ok(())
    }
}
impl CatalogReport {
    pub fn validate(&self) -> Result<(), StoreError> {
        if self.schema_version != SCHEMA_VERSION
            || uuid::Uuid::parse_str(&self.report_id).is_err()
            || self
                .account_fingerprint
                .as_deref()
                .is_some_and(|s| !fingerprint_valid(s))
            || self.error_code.as_ref().is_some_and(|s| {
                s.is_empty()
                    || s.len() > 80
                    || !s
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            })
            || (self.status == CatalogStatus::Ready
                && (self.account_fingerprint.is_none() || self.error_code.is_some()))
            || (self.status != CatalogStatus::Ready && !self.apps.is_empty())
        {
            return Err(invalid());
        }
        validate_apps(&self.apps)?;
        if serde_json::to_vec(self).map_err(|_| invalid())?.len() > MAX_REPORT_BYTES {
            return Err(invalid());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ready() -> CatalogReport {
        CatalogReport {
            schema_version: 1,
            report_id: "7da6a0fb-16e7-45e5-9782-1f5b47697b73".into(),
            platform: Platform::Ios,
            status: CatalogStatus::Ready,
            account_fingerprint: Some("a".repeat(64)),
            apps: vec![CatalogApp {
                bundle_identifier: "com.example.app".into(),
                name: "Example".into(),
                store_app_id: Some("123".into()),
            }],
            error_code: None,
        }
    }
    #[test]
    fn catalog_requires_complete_bounded_unique_apps_and_identity() {
        assert!(validate_identifier("com.example.app").is_ok());
        assert!(ready().validate().is_ok());
        let mut report = ready();
        report.account_fingerprint = None;
        assert!(report.validate().is_err());
        let mut report = ready();
        report.apps.push(report.apps[0].clone());
        assert!(report.validate().is_err());
        let mut report = ready();
        report.apps[0].name = "界".repeat(171);
        assert!(report.validate().is_err());
        let mut report = ready();
        report.status = CatalogStatus::Failed;
        assert!(report.validate().is_err());
        let mut report = ready();
        report.error_code = Some("secret token".into());
        assert!(report.validate().is_err());
        assert!(CatalogRequest {
            request_id: "id".into(),
            catalog_action: "delete".into()
        }
        .validate()
        .is_err());
    }
    #[test]
    fn catalog_wire_rejects_unknown_fields() {
        assert!(serde_json::from_value::<CatalogRequest>(
            serde_json::json!({"requestId":"id","catalogAction":"refresh","platform":"ios"})
        )
        .is_err());
        let value = serde_json::to_value(ready()).unwrap();
        assert_eq!(value["status"], "ready");
        assert_eq!(value["schemaVersion"], 1);
    }
    #[test]
    fn catalog_rejects_invalid_version_identity_and_serialized_size() {
        let mut report = ready();
        report.schema_version = 2;
        assert!(report.validate().is_err());
        let mut report = ready();
        report.report_id = "not-a-uuid".into();
        assert!(report.validate().is_err());
        let mut report = ready();
        report.account_fingerprint = Some("A".repeat(64));
        assert!(report.validate().is_err());
        let mut report = ready();
        report.apps[0].store_app_id = Some("unsafe?token=x".into());
        assert!(report.validate().is_err());
        let mut report = ready();
        report.apps = (0..4000)
            .map(|i| CatalogApp {
                bundle_identifier: format!("com.example.app{i}"),
                name: "n".repeat(512),
                store_app_id: None,
            })
            .collect();
        assert!(validate_apps(&report.apps).is_ok());
        assert!(report.validate().is_err());
    }
}
