//! A single authenticated ciphertext record; no plaintext settings or revision column.
use super::{database_error, Registry, Result};
use crate::screenshot_settings::{Settings, SettingsUpdate};
use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit, OsRng, Payload},
    Aes256Gcm, Nonce,
};
use hmac::{Hmac, Mac};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use sha2::Sha256;
use store_protocol::StoreError;

const PURPOSE: &[u8] = b"testflying:screenshot-settings:encryption:v1";
const AAD: &[u8] = b"testflying:screenshot-settings:record:v1";

pub(crate) fn create_schema(db: &Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS screenshot_settings (id INTEGER PRIMARY KEY CHECK(id=1), encrypted BLOB NOT NULL);")
        .map_err(database_error)?;
    db.pragma_update(None, "secure_delete", "ON")
        .map_err(database_error)?;
    Ok(())
}

fn encryption_error() -> StoreError {
    StoreError::new(
        500,
        "screenshot_settings_unavailable",
        "Screenshot destination settings are unavailable",
    )
}

fn cipher(signing_key: &[u8]) -> Aes256Gcm {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(signing_key).expect("HMAC key");
    mac.update(PURPOSE);
    Aes256Gcm::new(&mac.finalize().into_bytes())
}

pub(super) fn load(db: &Connection, signing_key: &[u8]) -> Result<Settings> {
    let record: Option<Vec<u8>> = db
        .query_row(
            "SELECT encrypted FROM screenshot_settings WHERE id=1",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(database_error)?;
    let Some(record) = record else {
        return Ok(Settings::default());
    };
    if record.len() < 12 + 16 || record.len() > 128 * 1024 {
        return Err(encryption_error());
    }
    let nonce_bytes: [u8; 12] = record[..12].try_into().map_err(|_| encryption_error())?;
    let nonce = Nonce::from(nonce_bytes);
    let plain = cipher(signing_key)
        .decrypt(
            &nonce,
            Payload {
                msg: &record[12..],
                aad: AAD,
            },
        )
        .map_err(|_| encryption_error())?;
    serde_json::from_slice(&plain).map_err(|_| encryption_error())
}

impl Registry {
    pub(crate) fn screenshot_settings(&self) -> Result<Settings> {
        let db = self.db.lock().map_err(|_| encryption_error())?;
        load(&db, &self.signing_key)
    }

    pub(crate) fn save_screenshot_settings(&self, update: &SettingsUpdate) -> Result<Settings> {
        let mut db = self.db.lock().map_err(|_| encryption_error())?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let next = load(&tx, &self.signing_key)?.apply(update.clone())?;
        let plain = serde_json::to_vec(&next).map_err(|_| encryption_error())?;
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let encrypted = cipher(&self.signing_key)
            .encrypt(
                &nonce,
                Payload {
                    msg: &plain,
                    aad: AAD,
                },
            )
            .map_err(|_| encryption_error())?;
        let mut record = nonce.to_vec();
        record.extend_from_slice(&encrypted);
        tx.execute("INSERT INTO screenshot_settings(id,encrypted) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET encrypted=excluded.encrypted", params![record]).map_err(database_error)?;
        tx.commit().map_err(database_error)?;
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const KEY: &str = "screenshot-settings-test-key-0123456789";

    #[test]
    fn encrypted_at_rest_persists_and_wrong_key_fails() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.sqlite");
        let registry = Registry::open(&path, KEY).unwrap();
        let update = serde_json::from_value(json!({"oss":{"accessKeyId":"fixture-access-id","accessKeySecret":"fixture-access-secret","securityToken":"fixture-security-token"},"notification":{"url":"https://fixture.example/notify","token":"fixture-notification-token"}})).unwrap();
        let saved = registry.save_screenshot_settings(&update).unwrap();
        assert_eq!(saved.revision, 1);
        let first: Vec<u8> = registry
            .db
            .lock()
            .unwrap()
            .query_row("SELECT encrypted FROM screenshot_settings", [], |row| {
                row.get(0)
            })
            .unwrap();
        registry
            .save_screenshot_settings(&SettingsUpdate::default())
            .unwrap();
        let second: Vec<u8> = registry
            .db
            .lock()
            .unwrap()
            .query_row("SELECT encrypted FROM screenshot_settings", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_ne!(&first[..12], &second[..12]);
        drop(registry);
        let file = std::fs::read(&path).unwrap();
        for secret in [
            "fixture-access-id",
            "fixture-access-secret",
            "fixture-security-token",
            "fixture-notification-token",
        ] {
            assert!(!file
                .windows(secret.len())
                .any(|part| part == secret.as_bytes()));
        }
        let reopened = Registry::open(&path, KEY).unwrap();
        let settings = reopened.screenshot_settings().unwrap();
        assert_eq!(settings.oss.access_key_secret, "fixture-access-secret");
        assert_eq!(settings.notification.token, "fixture-notification-token");
        assert_eq!(settings.revision, 2);
        let wrong = Registry::open(&path, "another-signing-key-with-32-characters").unwrap();
        let err = wrong.screenshot_settings().err().unwrap();
        assert_eq!(err.code, "screenshot_settings_unavailable");
        assert!(!err.message.contains("fixture"));
    }

    #[test]
    fn corrupt_record_is_rejected_without_plaintext_fallback() {
        let registry = Registry::open(":memory:", KEY).unwrap();
        registry
            .save_screenshot_settings(&SettingsUpdate::default())
            .unwrap();
        registry
            .db
            .lock()
            .unwrap()
            .execute("UPDATE screenshot_settings SET encrypted=zeroblob(32)", [])
            .unwrap();
        assert!(registry.screenshot_settings().is_err());
    }
}
