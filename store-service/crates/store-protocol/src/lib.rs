use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

pub mod crypto;
pub mod logs;
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
    StoreReviews,
    ProductPageOptimizations,
    CreateProductPageOptimization,
    Sync,
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
}

impl StoreError {
    pub fn new(status: u16, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            status,
            code: code.into(),
            message: message.into(),
        }
    }

    pub fn body(&self) -> Value {
        serde_json::json!({"error": {"code": self.code, "message": self.message}})
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
    fn app_identity_is_exact_and_platform_is_required() {
        assert_ne!(Platform::Ios, Platform::Android);
        assert!(validate_identifier("com.Example-App.demo").is_ok());
        for value in ["", "../app", "com..app", "app?target=other"] {
            assert!(validate_identifier(value).is_err());
        }
        assert!(serde_json::from_str::<Platform>("\"IOS\"").is_err());
    }
}
