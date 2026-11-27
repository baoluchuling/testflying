//! Authenticated, bounded store-only update contracts. No commands or caller-selected paths.
use p256::ecdsa::{
    signature::{Signer, Verifier},
    Signature, SigningKey, VerifyingKey,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;

pub const PROTOCOL: u32 = 1;
pub const MAX_PACKAGE: u64 = 64 * 1024 * 1024;
pub const MAX_UNPACKED: u64 = 128 * 1024 * 1024;
pub const TARGETS: &[&str] = &["linux-amd64", "darwin-arm64", "windows-amd64"];

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpgradeRequest {
    pub protocol: u32,
    pub request_id: String,
    pub node_id: String,
    pub version: String,
    pub target: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub expires_at: u64,
    pub bootstrap_version: u32,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SignedCommand {
    pub request: UpgradeRequest,
    pub signature: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Trust {
    pub protocol: u32,
    pub node_id: String,
    pub server_url: String,
    pub public_key: String,
    pub target: String,
}
pub fn valid_id(s: &str) -> bool {
    s.len() == 36
        && s.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit() && !b.is_ascii_uppercase()
            }
        })
}
fn parts(s: &str) -> Result<Vec<u64>, String> {
    let p: Vec<_> = s.split('.').collect();
    if p.len() != 3
        || p.iter().any(|s| {
            s.is_empty()
                || s.len() > 1 && s.starts_with('0')
                || !s.bytes().all(|b| b.is_ascii_digit())
        })
    {
        return Err("invalid_version".into());
    }
    p.iter()
        .map(|s| s.parse().map_err(|_| "invalid_version".into()))
        .collect()
}
pub fn compare_versions(a: &str, b: &str) -> Result<Ordering, String> {
    Ok(parts(a)?.cmp(&parts(b)?))
}
pub fn target() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "darwin-arm64",
        ("windows", "x86_64") => "windows-amd64",
        ("linux", "x86_64") => "linux-amd64",
        _ => "unsupported",
    }
}
fn key(secret: &[u8]) -> Result<SigningKey, String> {
    let mut hash = Sha256::new();
    hash.update(b"testflying/store-update/signing/v1\0");
    hash.update(secret);
    SigningKey::from_slice(&hash.finalize()).map_err(|_| "update_signing_unavailable".into())
}
pub fn public_key(secret: &[u8]) -> Result<String, String> {
    Ok(hex::encode(
        key(secret)?
            .verifying_key()
            .to_encoded_point(false)
            .as_bytes(),
    ))
}
pub fn valid_public_key(value: &str) -> bool {
    hex::decode(value)
        .ok()
        .is_some_and(|bytes| VerifyingKey::from_sec1_bytes(&bytes).is_ok())
}
fn bytes(r: &UpgradeRequest) -> Result<Vec<u8>, String> {
    let mut data = b"testflying/store-update/command/v1\0".to_vec();
    data.extend(serde_json::to_vec(r).map_err(|_| "invalid_update_command")?);
    Ok(data)
}
pub fn sign(request: UpgradeRequest, secret: &[u8]) -> Result<SignedCommand, String> {
    let sig: Signature = key(secret)?.sign(&bytes(&request)?);
    Ok(SignedCommand {
        request,
        signature: hex::encode(sig.to_bytes()),
    })
}
pub fn verify(command: &SignedCommand, trust: &Trust, now: u64) -> Result<(), String> {
    let r = &command.request;
    if trust.protocol != PROTOCOL
        || r.protocol != PROTOCOL
        || r.bootstrap_version != PROTOCOL
        || !valid_id(&r.request_id)
        || !valid_id(&r.node_id)
        || r.node_id != trust.node_id
        || !TARGETS.contains(&r.target.as_str())
        || r.target != trust.target
        || r.size_bytes == 0
        || r.size_bytes > MAX_PACKAGE
        || r.sha256.len() != 64
        || !r
            .sha256
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        || r.expires_at <= now
        || r.expires_at > now.saturating_add(86400)
    {
        return Err("invalid_update_command".into());
    }
    parts(&r.version)?;
    let public = hex::decode(&trust.public_key).map_err(|_| "invalid_update_trust")?;
    let key = VerifyingKey::from_sec1_bytes(&public).map_err(|_| "invalid_update_trust")?;
    let signature = hex::decode(&command.signature).map_err(|_| "invalid_update_signature")?;
    let signature = Signature::from_slice(&signature).map_err(|_| "invalid_update_signature")?;
    key.verify(&bytes(r)?, &signature)
        .map_err(|_| "invalid_update_signature".into())
}
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signature_binds_every_field_and_node_and_target() {
        let trust = Trust {
            protocol: 1,
            node_id: "00000000-0000-4000-8000-000000000001".into(),
            server_url: "https://fixture.invalid".into(),
            public_key: public_key(b"fixture-key").unwrap(),
            target: "windows-amd64".into(),
        };
        let r = UpgradeRequest {
            protocol: 1,
            request_id: "00000000-0000-4000-8000-000000000002".into(),
            node_id: trust.node_id.clone(),
            version: "0.2.12".into(),
            target: trust.target.clone(),
            sha256: "11".repeat(32),
            size_bytes: 99,
            expires_at: 1000,
            bootstrap_version: 1,
        };
        let c = sign(r, b"fixture-key").unwrap();
        verify(&c, &trust, 100).unwrap();
        for field in [
            "version",
            "target",
            "sha256",
            "nodeId",
            "requestId",
            "sizeBytes",
            "expiresAt",
        ] {
            let mut value = serde_json::to_value(&c).unwrap();
            value["request"][field] = if field == "sizeBytes" || field == "expiresAt" {
                serde_json::json!(2000)
            } else {
                serde_json::json!("changed")
            };
            let changed: SignedCommand = serde_json::from_value(value).unwrap();
            assert!(verify(&changed, &trust, 100).is_err());
        }
        assert!(verify(&c, &trust, 1000).is_err());
        for (field, value) in [
            ("version", serde_json::json!("0.2.13")),
            ("sha256", serde_json::json!("22".repeat(32))),
            (
                "requestId",
                serde_json::json!("00000000-0000-4000-8000-000000000003"),
            ),
            ("sizeBytes", serde_json::json!(100)),
            ("expiresAt", serde_json::json!(1100)),
        ] {
            let mut value_json = serde_json::to_value(&c).unwrap();
            value_json["request"][field] = value;
            let changed: SignedCommand = serde_json::from_value(value_json).unwrap();
            assert!(
                verify(&changed, &trust, 100).is_err(),
                "Valid-shaped tamper must fail signature: {field}"
            );
        }
    }
    #[test]
    fn versions_compare_numerically_and_reject_ambiguous_or_overflowing_forms() {
        assert_eq!(
            compare_versions("0.2.11", "0.2.9").unwrap(),
            Ordering::Greater
        );
        for s in [
            "01.2.3",
            "1.2",
            "1.2.3.4",
            "1.2.-1",
            "1.2.3-pre",
            "1. 2.3",
            "999999999999999999999999.0.0",
        ] {
            assert!(compare_versions(s, "1.0.0").is_err());
        }
    }
}
