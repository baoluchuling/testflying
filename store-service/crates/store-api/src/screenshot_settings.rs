//! Administrator-only screenshot destination settings. Secrets never enter the response view.
use axum::{extract::State, Json};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use store_protocol::StoreError;

#[derive(Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct Settings {
    pub oss: OssSettings,
    pub notification: NotificationSettings,
    pub revision: u64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct OssSettings {
    pub enabled: bool,
    pub region: String,
    pub bucket: String,
    /// Complete bucket hostname or CNAME; the bucket is never prepended automatically.
    pub endpoint: String,
    pub prefix: String,
    pub public_base_url: String,
    pub access_key_id: String,
    pub access_key_secret: String,
    pub security_token: String,
}

impl Default for OssSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            region: String::new(),
            bucket: String::new(),
            endpoint: String::new(),
            prefix: "screenshots".into(),
            public_base_url: String::new(),
            access_key_id: String::new(),
            access_key_secret: String::new(),
            security_token: String::new(),
        }
    }
}

#[derive(Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct NotificationSettings {
    pub url: String,
    pub token: String,
}

#[derive(Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SettingsUpdate {
    pub oss: Option<OssUpdate>,
    pub notification: Option<NotificationUpdate>,
}

#[derive(Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct OssUpdate {
    pub enabled: Option<bool>,
    pub region: Option<String>,
    pub bucket: Option<String>,
    pub endpoint: Option<String>,
    pub prefix: Option<String>,
    pub public_base_url: Option<String>,
    pub access_key_id: Option<String>,
    pub access_key_secret: Option<String>,
    pub security_token: Option<String>,
}

#[derive(Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct NotificationUpdate {
    pub url: Option<String>,
    pub token: Option<String>,
}

fn invalid() -> StoreError {
    StoreError::new(
        422,
        "invalid_screenshot_settings",
        "Screenshot destination settings are invalid",
    )
}

/// HTTP is accepted only for explicit loopback fixtures. Nonlocal destinations require TLS.
pub(crate) fn destination_url(value: &str, root_only: bool) -> Result<Url, StoreError> {
    let url = Url::parse(value).map_err(|_| invalid())?;
    let host = url.host_str().unwrap_or_default();
    let local = host == "localhost"
        || host
            .trim_matches(['[', ']'])
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback());
    if value.len() > 2048
        || !(url.scheme() == "https" || (url.scheme() == "http" && local))
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || (root_only && url.path() != "/")
        || value
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
    {
        return Err(invalid());
    }
    Ok(url)
}

impl OssSettings {
    pub(crate) fn validate(&self) -> Result<(), StoreError> {
        if (!self.region.is_empty()
            && (self.region.len() > 64
                || !self
                    .region
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')))
            || (!self.bucket.is_empty()
                && (!(3..=63).contains(&self.bucket.len())
                    || !self
                        .bucket
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                    || self.bucket.starts_with('-')
                    || self.bucket.ends_with('-')))
            || self.prefix.len() > 512
            || self.prefix.starts_with('/')
            || self.prefix.ends_with('/')
            || self.prefix.split('/').any(|part| {
                part == "." || part == ".." || (part.is_empty() && !self.prefix.is_empty())
            })
            || self
                .prefix
                .chars()
                .any(|c| c.is_control() || matches!(c, '\\' | '?' | '#' | '%'))
            || !valid_secret(&self.access_key_id)
            || !valid_secret(&self.access_key_secret)
            || !valid_secret(&self.security_token)
        {
            return Err(invalid());
        }
        if !self.endpoint.is_empty() {
            destination_url(&self.endpoint, true)?;
        }
        if !self.public_base_url.is_empty() {
            destination_url(&self.public_base_url, false)?;
        }
        if self.enabled
            && [
                self.region.as_str(),
                &self.bucket,
                &self.endpoint,
                &self.access_key_id,
                &self.access_key_secret,
            ]
            .iter()
            .any(|s| s.is_empty())
        {
            return Err(invalid());
        }
        Ok(())
    }
}

fn valid_secret(value: &str) -> bool {
    value.len() <= 16384 && value.bytes().all(|byte| byte.is_ascii_graphic())
}

fn assign(target: &mut String, input: Option<String>) {
    if let Some(value) = input {
        *target = value.trim().to_owned();
    }
}
fn password(target: &mut String, input: Option<String>) {
    if let Some(value) = input.filter(|value| !value.trim().is_empty()) {
        *target = value;
    }
}

impl Settings {
    pub(crate) fn view(&self) -> Value {
        json!({
            "revision": self.revision,
            "oss": {
                "enabled": self.oss.enabled, "region": self.oss.region,
                "bucket": self.oss.bucket, "endpoint": self.oss.endpoint,
                "prefix": self.oss.prefix, "publicBaseUrl": self.oss.public_base_url,
                "accessKeyId": self.oss.access_key_id,
                "accessKeySecretConfigured": !self.oss.access_key_secret.is_empty(),
                "securityTokenConfigured": !self.oss.security_token.is_empty()
            },
            "notification": {
                "url": self.notification.url, "tokenConfigured": !self.notification.token.is_empty(),
                "status": if self.notification.url.is_empty() || self.notification.token.is_empty() { "not_configured" } else { "configured" },
                "protocol": "capture-v1"
            }
        })
    }

    pub(crate) fn apply(&self, update: SettingsUpdate) -> Result<Self, StoreError> {
        let mut next = self.clone();
        if let Some(input) = update.oss {
            let new_secret = input
                .access_key_secret
                .as_ref()
                .is_some_and(|value| !value.trim().is_empty());
            let key_changed = input
                .access_key_id
                .as_ref()
                .is_some_and(|value| value.trim() != self.oss.access_key_id);
            if key_changed && !new_secret {
                return Err(StoreError::new(
                    422,
                    "screenshot_secret_required",
                    "Changing AccessKey ID requires a new AccessKey Secret",
                ));
            }
            // STS tokens belong to the previous credential pair. A new pair may be permanent.
            if key_changed {
                next.oss.security_token.clear();
            }
            if let Some(value) = input.enabled {
                next.oss.enabled = value;
            }
            assign(&mut next.oss.region, input.region);
            assign(&mut next.oss.bucket, input.bucket);
            assign(&mut next.oss.endpoint, input.endpoint);
            assign(&mut next.oss.prefix, input.prefix);
            assign(&mut next.oss.public_base_url, input.public_base_url);
            assign(&mut next.oss.access_key_id, input.access_key_id);
            password(&mut next.oss.access_key_secret, input.access_key_secret);
            password(&mut next.oss.security_token, input.security_token);
        }
        if let Some(input) = update.notification {
            assign(&mut next.notification.url, input.url);
            password(&mut next.notification.token, input.token);
        }
        next.oss.validate()?;
        if !next.notification.url.is_empty() {
            let url = destination_url(&next.notification.url, false)?;
            if url.scheme() != "https" {
                return Err(invalid());
            }
        }
        if !valid_secret(&next.notification.token) {
            return Err(invalid());
        }
        next.revision = self.revision.checked_add(1).ok_or_else(invalid)?;
        Ok(next)
    }
}

pub(crate) async fn read(State(state): State<Arc<crate::ApiState>>) -> crate::Result<Json<Value>> {
    Ok(Json(state.registry.screenshot_settings()?.view()))
}

pub(crate) async fn update(
    State(state): State<Arc<crate::ApiState>>,
    Json(input): Json<SettingsUpdate>,
) -> crate::Result<Json<Value>> {
    Ok(Json(
        state.registry.save_screenshot_settings(&input)?.view(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotating_sts_credentials_to_permanent_keys_clears_old_token() {
        let initial=Settings::default().apply(serde_json::from_value(json!({"oss":{"accessKeyId":"fixture-sts-id","accessKeySecret":"fixture-sts-secret","securityToken":"fixture-sts-token"}})).unwrap()).unwrap();
        let permanent=initial.apply(serde_json::from_value(json!({"oss":{"accessKeyId":"fixture-permanent-id","accessKeySecret":"fixture-permanent-secret","securityToken":""}})).unwrap()).unwrap();
        assert!(permanent.oss.security_token.is_empty());
        assert_eq!(permanent.view()["oss"]["securityTokenConfigured"], false);
        let sts=initial.apply(serde_json::from_value(json!({"oss":{"accessKeyId":"fixture-new-sts-id","accessKeySecret":"fixture-new-sts-secret","securityToken":"fixture-new-sts-token"}})).unwrap()).unwrap();
        assert_eq!(sts.oss.security_token, "fixture-new-sts-token");
        let omitted=initial.apply(serde_json::from_value(json!({"oss":{"accessKeyId":"fixture-permanent-id","accessKeySecret":"fixture-permanent-secret"}})).unwrap()).unwrap();
        assert!(omitted.oss.security_token.is_empty());
    }

    #[test]
    fn blank_passwords_preserve_and_key_rotation_requires_secret() {
        let initial = Settings::default().apply(serde_json::from_value(json!({"oss":{"accessKeyId":"fixture-id","accessKeySecret":"fixture-secret","securityToken":"fixture-sts"},"notification":{"token":"fixture-token"}})).unwrap()).unwrap();
        let next = initial.apply(serde_json::from_value(json!({"oss":{"accessKeySecret":"","securityToken":" "},"notification":{"token":""}})).unwrap()).unwrap();
        assert_eq!(next.oss.access_key_secret, "fixture-secret");
        assert_eq!(next.oss.security_token, "fixture-sts");
        assert_eq!(next.notification.token, "fixture-token");
        assert_eq!(next.revision, 2);
        assert!(next
            .apply(serde_json::from_value(json!({"oss":{"accessKeyId":"changed"}})).unwrap())
            .is_err());
        let view = next.view().to_string();
        for secret in ["fixture-secret", "fixture-sts", "fixture-token"] {
            assert!(!view.contains(secret));
        }
    }

    #[test]
    fn invalid_urls_and_traversal_are_rejected_without_echoing_input() {
        for value in [
            "http://example.com",
            "https://user:password@example.com",
            "https://example.com?token=secret",
            "https://example.com#secret",
            "https://example.com/path",
        ] {
            assert!(destination_url(value, true).is_err());
        }
        assert!(destination_url("http://127.0.0.1:1234", true).is_ok());
        for prefix in ["../secret", "a/../b", "a//b", "a\\b", "a/%2e%2e/b"] {
            assert!(Settings::default()
                .apply(serde_json::from_value(json!({"oss":{"prefix":prefix}})).unwrap())
                .is_err());
        }
        assert!(serde_json::from_value::<SettingsUpdate>(
            json!({"notification":{"status":"sent"}})
        )
        .is_err());
        assert!(Settings::default()
            .apply(serde_json::from_value(json!({"oss":{"enabled":true}})).unwrap())
            .is_err());
    }
}
