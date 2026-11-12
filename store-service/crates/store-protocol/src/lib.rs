use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

pub mod crypto;
pub mod logs;
pub mod screenshot;
pub mod upload;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Ios,
    Android,
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Ios => "ios",
            Self::Android => "android",
        })
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Preflight,
    SupportedLocales,
    StoreListings,
    StoreImages,
    StoreReleases,
    CreateStoreRelease,
    StoreReviews,
    ProductPageOptimizations,
    CreateProductPageOptimization,
    Sync,
    StartStoreImageSync,
    StoreImageSyncStatus,
    UploadStart,
    UploadChunk,
    UploadFinish,
    UploadAbort,
}

impl Operation {
    pub fn is_upload(self) -> bool {
        matches!(
            self,
            Self::UploadStart | Self::UploadChunk | Self::UploadFinish | Self::UploadAbort
        )
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StoreRequest {
    pub request_id: String,
    pub platform: Platform,
    pub bundle_identifier: String,
    pub operation: Operation,
    #[serde(default)]
    pub payload: Value,
}

impl StoreRequest {
    pub fn is_image_only_sync(&self) -> bool {
        let owner = match self.payload["operation"].as_str() {
            Some("update_app_metadata") => "metadata",
            Some("update_marketing_page") => "marketingPage",
            _ => return false,
        };
        matches!(
            self.operation,
            Operation::Sync | Operation::StartStoreImageSync
        ) && self.payload["syncScopes"].as_array().is_some_and(|scopes| {
            !scopes.is_empty()
                && scopes
                    .iter()
                    .all(|scope| scope.as_str() == Some("store_images"))
        }) && self.payload[owner]["storeImages"].is_object()
    }

    pub fn validate(&self) -> Result<(), StoreError> {
        validate_identifier(&self.bundle_identifier)?;
        if self.request_id.is_empty() || self.request_id.len() > 128 {
            return Err(StoreError::new(
                422,
                "invalid_request_id",
                "requestId must contain 1–128 characters",
            ));
        }
        if !self.payload.is_null() && !self.payload.is_object() {
            return Err(StoreError::new(
                422,
                "invalid_payload",
                "payload must be a JSON object",
            ));
        }
        Ok(())
    }
}

pub fn validate_identifier(value: &str) -> Result<(), StoreError> {
    if value.is_empty()
        || value.len() > 180
        || value.starts_with('.')
        || value.ends_with('.')
        || value.contains("..")
        || !value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
    {
        return Err(StoreError::new(
            422,
            "invalid_app_identifier",
            "Invalid Bundle ID or package name",
        ));
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StoreReply {
    pub request_id: String,
    pub status: u16,
    pub body: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize, thiserror::Error)]
#[error("{code}: {message}")]
pub struct StoreError {
    pub status: u16,
    pub code: String,
    pub message: String,
    #[serde(
        default,
        rename = "outcomeUnknown",
        skip_serializing_if = "std::ops::Not::not"
    )]
    pub outcome_unknown: bool,
}

impl StoreError {
    pub fn new(status: u16, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            status,
            code: code.into(),
            message: message.into(),
            outcome_unknown: false,
        }
    }

    pub fn body(&self) -> Value {
        let mut body = serde_json::json!({"error": {"code": self.code, "message": self.message}});
        if self.outcome_unknown {
            body["outcomeUnknown"] = Value::Bool(true);
        }
        body
    }
}

impl StoreReply {
    pub fn from_result(request_id: String, result: Result<Value, StoreError>) -> Self {
        match result {
            Ok(body) => Self {
                request_id,
                status: 200,
                body,
            },
            Err(error) => Self {
                request_id,
                status: error.status,
                body: error.body(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_error_uncertainty_is_opt_in_and_backward_compatible() {
        let mut error: StoreError = serde_json::from_value(serde_json::json!({
            "status":502,"code":"store_upstream_error","message":"Store request failed"
        }))
        .unwrap();
        assert!(!error.outcome_unknown);
        assert!(error.body().get("outcomeUnknown").is_none());
        assert!(serde_json::to_value(&error)
            .unwrap()
            .get("outcomeUnknown")
            .is_none());
        error.outcome_unknown = true;
        let reply = StoreReply::from_result("original-id".into(), Err(error));
        assert_eq!(reply.request_id, "original-id");
        assert_eq!(reply.body["error"]["message"], "Store request failed");
        assert_eq!(reply.body["outcomeUnknown"], true);
    }

    #[test]
    fn only_explicit_image_scopes_select_async_sync() {
        let mut request = StoreRequest {
            request_id: "id".into(),
            platform: Platform::Ios,
            bundle_identifier: "com.example.app".into(),
            operation: Operation::Sync,
            payload: serde_json::json!({"operation":"update_app_metadata","syncScopes":["store_images"],"metadata":{"storeImages":{}}}),
        };
        assert!(request.is_image_only_sync());
        request.payload["syncScopes"] = serde_json::json!(["store_images", "title"]);
        assert!(!request.is_image_only_sync());
        request.payload["syncScopes"] = serde_json::json!([]);
        assert!(!request.is_image_only_sync());
        request.payload["syncScopes"] = serde_json::json!(["store_images"]);
        request.operation = Operation::Preflight;
        assert!(!request.is_image_only_sync());
    }

    #[test]
    fn app_identity_is_exact_and_platform_is_required() {
        assert_ne!(Platform::Ios, Platform::Android);
        assert!(validate_identifier("com.Example-App.demo").is_ok());
        for value in ["", "../app", "com..app", "app?target=other"] {
            assert!(validate_identifier(value).is_err());
        }
        assert!(serde_json::from_str::<Platform>("\"IOS\"").is_err());
    }
}
