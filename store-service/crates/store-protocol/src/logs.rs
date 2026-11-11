//! Read-only node diagnostics, deliberately separate from app/store operations.
use crate::{StoreError, StoreRequest};
use serde::{Deserialize, Serialize};

pub const MAX_LOG_CHUNK: usize = 32 * 1024;
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LogRequest {
    pub request_id: String,
    pub action: LogAction,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum LogAction {
    Latest,
    Read {
        file: String,
        offset: u64,
        length: u32,
        #[serde(rename = "snapshotSize")]
        snapshot_size: u64,
    },
}
impl LogRequest {
    pub fn validate(&self) -> Result<(), StoreError> {
        if self.request_id.is_empty() || self.request_id.len() > 128 {
            return Err(StoreError::new(
                422,
                "invalid_log_request",
                "Invalid log request ID",
            ));
        }
        if let LogAction::Read {
            file,
            offset,
            length,
            snapshot_size,
        } = &self.action
        {
            if log_hour(file).is_none()
                || *length == 0
                || *length as usize > MAX_LOG_CHUNK
                || *offset > *snapshot_size
            {
                return Err(StoreError::new(
                    422,
                    "invalid_log_request",
                    "Invalid log file or chunk range",
                ));
            }
        }
        Ok(())
    }
}
pub fn log_hour(name: &str) -> Option<u64> {
    let value = name
        .strip_prefix("store-connector-")?
        .strip_suffix(".jsonl")?;
    if value.len() != 10 || !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum NodeRequest {
    Store(StoreRequest),
    Logs(LogRequest),
}
impl NodeRequest {
    pub fn request_id(&self) -> &str {
        match self {
            Self::Store(r) => &r.request_id,
            Self::Logs(r) => &r.request_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn controls_do_not_require_or_invent_an_app_identity() {
        let value = serde_json::json!({"requestId":"id","action":{"type":"latest"}});
        assert!(matches!(
            serde_json::from_value::<NodeRequest>(value).unwrap(),
            NodeRequest::Logs(_)
        ));
        for file in [
            "../connector.json",
            "store-connector-0000000001.jsonl/../secret",
            "/etc/passwd",
            "store-connector-1.jsonl",
        ] {
            assert!(log_hour(file).is_none());
        }
    }
}
