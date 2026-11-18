//! Bounded, credential-free Firebase discovery diagnostics.
use crate::{crypto::MAX_JSON_BYTES, StoreError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;

pub const MAX_FIREBASE_RESPONSE_BYTES: usize = MAX_JSON_BYTES - 65_536;
pub const MAX_FIREBASE_ITEMS: usize = 10_000;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FirebaseRequest {
    pub request_id: String,
    pub firebase_action: FirebaseAction,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FirebaseAction {
    Projects,
    AndroidApps {
        #[serde(rename = "projectId")]
        project_id: String,
    },
}

impl<'de> Deserialize<'de> for FirebaseAction {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Serde's internally tagged unit variants ignore extra fields. An empty
        // struct variant enforces the same strict object shape as AndroidApps.
        #[derive(Deserialize)]
        #[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
        enum RawAction {
            Projects {},
            AndroidApps {
                #[serde(rename = "projectId")]
                project_id: String,
            },
        }
        Ok(match RawAction::deserialize(deserializer)? {
            RawAction::Projects {} => Self::Projects,
            RawAction::AndroidApps { project_id } => Self::AndroidApps { project_id },
        })
    }
}

pub fn valid_project_id(id: &str) -> bool {
    (6..=30).contains(&id.len())
        && id.as_bytes()[0].is_ascii_lowercase()
        && id.as_bytes()[id.len() - 1].is_ascii_alphanumeric()
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn invalid_request() -> StoreError {
    StoreError::new(
        422,
        "invalid_firebase_request",
        "Invalid Firebase discovery request",
    )
}
impl FirebaseAction {
    pub fn validate(&self) -> Result<(), StoreError> {
        if matches!(self, Self::AndroidApps { project_id } if !valid_project_id(project_id)) {
            return Err(invalid_request());
        }
        Ok(())
    }
}
impl FirebaseRequest {
    pub fn validate(&self) -> Result<(), StoreError> {
        if self.request_id.is_empty() || self.request_id.len() > 128 {
            return Err(invalid_request());
        }
        self.firebase_action.validate()
    }
}
fn invalid_response() -> StoreError {
    StoreError::new(
        502,
        "firebase_invalid_response",
        "Invalid Firebase discovery response",
    )
}
fn keys(value: &Value, expected: &[&str]) -> bool {
    value
        .as_object()
        .is_some_and(|o| o.len() == expected.len() && expected.iter().all(|k| o.contains_key(*k)))
}
fn display_name(value: &Value) -> bool {
    value.is_null()
        || value
            .as_str()
            .is_some_and(|s| s.len() <= 512 && !s.chars().any(char::is_control))
}
fn safe_text(value: &Value, max: usize) -> bool {
    value
        .as_str()
        .is_some_and(|s| !s.is_empty() && s.len() <= max && !s.chars().any(char::is_control))
}

pub fn validate_response(action: &FirebaseAction, body: &Value) -> Result<(), StoreError> {
    action.validate()?;
    if serde_json::to_vec(body)
        .map_err(|_| invalid_response())?
        .len()
        > MAX_FIREBASE_RESPONSE_BYTES
    {
        return Err(invalid_response());
    }
    let mut identities = HashSet::new();
    let valid = match action {
        FirebaseAction::Projects => {
            keys(body, &["projects"])
                && body["projects"].as_array().is_some_and(|items| {
                    items.len() <= MAX_FIREBASE_ITEMS
                        && items.iter().all(|item| {
                            keys(item, &["projectId", "projectNumber", "displayName"])
                                && item["projectId"].as_str().is_some_and(valid_project_id)
                                && item["projectNumber"].as_str().is_some_and(|s| {
                                    !s.is_empty()
                                        && s.len() <= 30
                                        && s.bytes().all(|b| b.is_ascii_digit())
                                })
                                && display_name(&item["displayName"])
                                && identities.insert(item["projectId"].as_str().unwrap().to_owned())
                        })
                })
        }
        FirebaseAction::AndroidApps { project_id } => {
            keys(body, &["projectId", "apps"])
                && body["projectId"].as_str() == Some(project_id.as_str())
                && body["apps"].as_array().is_some_and(|items| {
                    items.len() <= MAX_FIREBASE_ITEMS
                        && items.iter().all(|item| {
                            keys(item, &["projectId", "appId", "packageName", "displayName"])
                                && item["projectId"].as_str() == Some(project_id.as_str())
                                && safe_text(&item["appId"], 256)
                                && item["packageName"]
                                    .as_str()
                                    .is_some_and(|s| crate::validate_identifier(s).is_ok())
                                && display_name(&item["displayName"])
                                && identities.insert(item["appId"].as_str().unwrap().to_owned())
                        })
                })
        }
    };
    if valid {
        Ok(())
    } else {
        Err(invalid_response())
    }
}

/// Only fixed codes/statuses are allowed across the node trust boundary.
pub fn public_error(code: &str) -> StoreError {
    let (status, code, message) = match code {
        "firebase_not_configured" => (
            503,
            "firebase_not_configured",
            "Firebase credentials are not configured",
        ),
        "firebase_api_disabled" => (
            403,
            "firebase_api_disabled",
            "Firebase Management API is disabled",
        ),
        "firebase_permission_denied" => {
            (403, "firebase_permission_denied", "Firebase access denied")
        }
        "firebase_auth_failed" => (502, "firebase_auth_failed", "Firebase authorization failed"),
        "firebase_rate_limited" => (429, "firebase_rate_limited", "Firebase rate limit reached"),
        "firebase_busy" => (
            429,
            "firebase_busy",
            "Another Firebase discovery request is running",
        ),
        "firebase_not_found" => (404, "firebase_not_found", "Firebase project was not found"),
        "firebase_unsupported" => (
            501,
            "firebase_unsupported",
            "Firebase discovery is unavailable in this mode",
        ),
        "firebase_invalid_response" => (
            502,
            "firebase_invalid_response",
            "Invalid Firebase discovery response",
        ),
        _ => (502, "firebase_upstream_error", "Firebase discovery failed"),
    };
    StoreError::new(status, code, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn strict_requests_and_project_paths() {
        for id in ["project-a", "a12345", "project-name-123"] {
            assert!(valid_project_id(id));
        }
        for id in [
            "",
            "123456",
            "abc",
            "Project-a",
            "abcde-",
            "../project",
            "https://x",
            "a234567890123456789012345678901",
        ] {
            assert!(!valid_project_id(id));
        }
        assert!(serde_json::from_value::<FirebaseRequest>(
            json!({"requestId":"x","firebaseAction":{"type":"projects","config":true}})
        )
        .is_err());
        for action in [
            json!({}),
            json!({"projectId":"project-a"}),
            json!(null),
            json!("projects"),
            json!([]),
        ] {
            assert!(serde_json::from_value::<FirebaseAction>(action).is_err());
        }
        assert!(matches!(
            serde_json::from_value::<FirebaseAction>(json!({"type":"projects"})).unwrap(),
            FirebaseAction::Projects
        ));
        assert!(serde_json::from_value::<FirebaseRequest>(json!({"requestId":"x","firebaseAction":{"type":"android_apps","projectId":"project-a"},"extra":1})).is_err());
        assert!(FirebaseRequest {
            request_id: "".into(),
            firebase_action: FirebaseAction::Projects
        }
        .validate()
        .is_err());
    }
    #[test]
    fn response_whitelist_identity_duplicates_and_size() {
        let action = FirebaseAction::AndroidApps {
            project_id: "project-a".into(),
        };
        let app = json!({"projectId":"project-a","appId":"1:123:android:abc","packageName":"com.example.app","displayName":null});
        let mut body = json!({"projectId":"project-a","apps":[app.clone()]});
        assert!(validate_response(&action, &body).is_ok());
        body["apps"][0]["apiKeyId"] = json!("private");
        assert!(validate_response(&action, &body).is_err());
        assert!(validate_response(
            &action,
            &json!({"projectId":"project-a","apps":[app.clone(), app]})
        )
        .is_err());
        assert!(validate_response(&action, &json!({"projectId":"project-b","apps":[]})).is_err());
        assert!(validate_response(&FirebaseAction::Projects, &json!({"projects":[]})).is_ok());
        let project = json!({"projectId":"project-a","projectNumber":"123","displayName":null});
        assert!(
            validate_response(&FirebaseAction::Projects, &json!({"projects":[project]})).is_ok()
        );
        assert!(validate_response(
            &FirebaseAction::Projects,
            &json!({"projects":[],"token":"private"})
        )
        .is_err());
        let excessive: Vec<_> = (0..=MAX_FIREBASE_ITEMS).map(|i| json!({"projectId":format!("project-{i}"),"projectNumber":i.to_string(),"displayName":null})).collect();
        assert!(
            validate_response(&FirebaseAction::Projects, &json!({"projects":excessive})).is_err()
        );
        let oversized: Vec<_> = (0..4000).map(|i| json!({"projectId":format!("project-{i}"),"projectNumber":i.to_string(),"displayName":"x".repeat(512)})).collect();
        assert!(
            validate_response(&FirebaseAction::Projects, &json!({"projects":oversized})).is_err()
        );
        let envelope = serde_json::from_value::<crate::logs::NodeRequest>(
            json!({"requestId":"fixture","firebaseAction":{"type":"projects"}}),
        )
        .unwrap();
        assert!(matches!(envelope, crate::logs::NodeRequest::Firebase(_)));
    }
}
