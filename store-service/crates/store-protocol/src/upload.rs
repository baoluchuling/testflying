use crate::{validate_identifier, Platform, StoreError};
use base64::Engine;
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

pub const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
pub const TOKEN_SECONDS: u64 = 30 * 60;
pub const TRANSFER_SECONDS: u64 = 30 * 60;
pub const ASSET_SECONDS: u64 = 900;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UploadInput {
    pub file_name: String,
    pub content_type: String,
    pub size_bytes: u64,
    pub sha256: String,
}

impl UploadInput {
    pub fn validate(&self) -> Result<(), StoreError> {
        if self.file_name.is_empty()
            || self.file_name.len() > 255
            || self
                .file_name
                .chars()
                .any(|c| c.is_control() || matches!(c, '/' | '\\'))
            || !matches!(self.content_type.as_str(), "image/png" | "image/jpeg")
            || self.size_bytes == 0
            || self.size_bytes > MAX_FILE_BYTES
            || self.sha256.len() != 64
            || !self.sha256.bytes().all(|c| c.is_ascii_hexdigit())
        {
            return Err(StoreError::new(
                422,
                "invalid_upload",
                "Provide a PNG/JPEG filename, size up to 64 MiB and SHA-256",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UploadClaims {
    pub iss: String,
    pub aud: String,
    pub exp: u64,
    pub iat: u64,
    pub asset_id: String,
    pub connector_id: String,
    pub platform: Platform,
    pub bundle_identifier: String,
    pub file: UploadInput,
}

pub fn now() -> Result<u64, StoreError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|v| v.as_secs())
        .map_err(|_| StoreError::new(500, "invalid_clock", "System clock is invalid"))
}

pub fn sign(claims: &UploadClaims, key: &str) -> Result<String, StoreError> {
    let key = hex::decode(key).map_err(|_| invalid_token())?;
    if key.len() != 32 {
        return Err(invalid_token());
    }
    encode(
        &Header::new(Algorithm::HS256),
        claims,
        &EncodingKey::from_secret(&key),
    )
    .map_err(|_| invalid_token())
}

pub fn verify(token: &str, key: &str, connector_id: &str) -> Result<UploadClaims, StoreError> {
    if token.len() > 4096 {
        return Err(invalid_token());
    }
    let key = hex::decode(key).map_err(|_| invalid_token())?;
    if key.len() != 32 {
        return Err(invalid_token());
    }
    let mut validation = Validation::new(Algorithm::HS256);
    validation.set_issuer(&["testflying-store-api"]);
    validation.set_audience(&[format!("connector:{connector_id}")]);
    validation.leeway = 0;
    let claims = decode::<UploadClaims>(token, &DecodingKey::from_secret(&key), &validation)
        .map_err(|_| invalid_token())?
        .claims;
    let time = now()?;
    if claims.connector_id != connector_id
        || claims.exp <= time
        || claims.iat > time + 30
        || claims.exp > claims.iat.saturating_add(TOKEN_SECONDS)
        || !valid_asset_id(&claims.asset_id)
        || validate_identifier(&claims.bundle_identifier).is_err()
        || claims.file.validate().is_err()
    {
        return Err(invalid_token());
    }
    Ok(claims)
}

pub fn valid_asset_id(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, c)| {
            if [8, 13, 18, 23].contains(&i) {
                c == b'-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
}

pub fn normalize_base_url(raw: &str) -> Result<String, StoreError> {
    let invalid = || {
        StoreError::new(
            422,
            "invalid_endpoint",
            "baseUrl must be an HTTP(S) URL without credentials, query or fragment",
        )
    };
    if raw.len() > 2048 || raw.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(invalid());
    }
    let url = url::Url::parse(raw).map_err(|_| invalid())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid());
    }
    Ok(url.as_str().trim_end_matches('/').to_owned())
}

pub fn invalid_token() -> StoreError {
    StoreError::new(
        401,
        "invalid_upload_token",
        "Upload credentials are invalid or expired",
    )
}

pub fn encode_chunk(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}
pub fn decode_chunk(raw: &str) -> Result<Vec<u8>, StoreError> {
    if raw.len() > crate::crypto::CHUNK_BYTES.div_ceil(3) * 4 {
        return Err(StoreError::new(
            413,
            "invalid_upload_chunk",
            "Chunk is too large",
        ));
    }
    base64::engine::general_purpose::STANDARD
        .decode(raw)
        .map_err(|_| StoreError::new(422, "invalid_upload_chunk", "Chunk must be Base64 encoded"))
}
