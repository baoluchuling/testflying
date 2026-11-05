//! Anonymous files are removed by the OS when the last handle closes, including crashes.
use crate::{images, StoreError};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use store_protocol::{
    upload::{self, UploadClaims},
    Platform, StoreRequest,
};

const MAX_ASSETS: usize = 20;
const MAX_RESERVED: u64 = 512 * 1024 * 1024;

struct Entry {
    session: String,
    claims: UploadClaims,
    file: File,
    received: u64,
    digest: Sha256,
    complete: bool,
    claimed: bool,
    expires: Instant,
}

pub struct AssetStore {
    connector_id: String,
    key: String,
    entries: Mutex<HashMap<String, Entry>>,
}

pub struct AssetLease {
    store: Arc<AssetStore>,
    ids: HashSet<String>,
}
impl Drop for AssetLease {
    fn drop(&mut self) {
        let mut entries = self.store.entries.lock().unwrap();
        for id in &self.ids {
            entries.remove(id);
        }
    }
}

impl AssetStore {
    pub fn new(connector_id: String, key: String) -> Arc<Self> {
        Arc::new(Self {
            connector_id,
            key,
            entries: Mutex::new(HashMap::new()),
        })
    }

    pub fn claims(&self, ticket: &str) -> Result<UploadClaims, StoreError> {
        upload::verify(ticket, &self.key, &self.connector_id)
    }

    pub fn purge(&self) {
        self.entries
            .lock()
            .unwrap()
            .retain(|_, e| e.claimed || e.expires > Instant::now());
    }

    pub fn start(&self, ticket: &str, session: &str) -> Result<UploadClaims, StoreError> {
        if !upload::valid_asset_id(session) {
            return Err(StoreError::new(
                422,
                "invalid_upload_session",
                "Upload session is invalid",
            ));
        }
        let claims = self.claims(ticket)?;
        self.purge();
        let mut entries = self.entries.lock().unwrap();
        if entries.contains_key(&claims.asset_id) {
            return Err(StoreError::new(
                409,
                "asset_exists",
                "This asset already has an upload session",
            ));
        }
        if entries.len() >= MAX_ASSETS
            || entries
                .values()
                .map(|e| e.claims.file.size_bytes)
                .sum::<u64>()
                + claims.file.size_bytes
                > MAX_RESERVED
        {
            return Err(StoreError::new(
                429,
                "upload_capacity",
                "Connector temporary upload capacity is full",
            ));
        }
        let file = tempfile::tempfile().map_err(|_| storage_error())?;
        entries.insert(
            claims.asset_id.clone(),
            Entry {
                session: session.to_owned(),
                claims: claims.clone(),
                file,
                received: 0,
                digest: Sha256::new(),
                complete: false,
                claimed: false,
                expires: Instant::now() + Duration::from_secs(upload::TRANSFER_SECONDS),
            },
        );
        Ok(claims)
    }

    pub fn chunk(
        &self,
        platform: Platform,
        app: &str,
        id: &str,
        session: &str,
        offset: u64,
        data: &[u8],
    ) -> Result<(), StoreError> {
        if data.is_empty() || data.len() > store_protocol::crypto::CHUNK_BYTES {
            return Err(StoreError::new(
                422,
                "invalid_upload_chunk",
                "Upload chunk is empty or too large",
            ));
        }
        let mut entries = self.entries.lock().unwrap();
        let entry = entries.get_mut(id).ok_or_else(missing)?;
        check_scope(entry, platform, app)?;
        check_session(entry, session)?;
        if entry.complete
            || entry.claimed
            || entry.received != offset
            || entry.received + data.len() as u64 > entry.claims.file.size_bytes
        {
            return Err(StoreError::new(
                409,
                "invalid_upload_offset",
                "Upload offset or size is invalid",
            ));
        }
        entry.file.write_all(data).map_err(|_| storage_error())?;
        entry.digest.update(data);
        entry.received += data.len() as u64;
        Ok(())
    }

    pub fn finish(
        &self,
        platform: Platform,
        app: &str,
        id: &str,
        session: &str,
    ) -> Result<Value, StoreError> {
        let mut entries = self.entries.lock().unwrap();
        let result = (|| {
            let entry = entries.get_mut(id).ok_or_else(missing)?;
            check_scope(entry, platform, app)?;
            check_session(entry, session)?;
            if entry.complete || entry.claimed {
                return Err(StoreError::new(
                    409,
                    "asset_exists",
                    "Upload has already completed",
                ));
            }
            if entry.received != entry.claims.file.size_bytes
                || hex::encode(entry.digest.clone().finalize())
                    != entry.claims.file.sha256.to_ascii_lowercase()
            {
                return Err(StoreError::new(
                    422,
                    "upload_integrity",
                    "Uploaded size or SHA-256 does not match",
                ));
            }
            entry
                .file
                .seek(SeekFrom::Start(0))
                .map_err(|_| storage_error())?;
            let mut bytes = Vec::with_capacity(entry.received as usize);
            entry
                .file
                .read_to_end(&mut bytes)
                .map_err(|_| storage_error())?;
            let (mime, _) = images::validate_bytes(&bytes)?;
            if mime != entry.claims.file.content_type {
                return Err(StoreError::new(
                    422,
                    "invalid_upload",
                    "Image content type does not match",
                ));
            }
            entry.complete = true;
            entry.expires = Instant::now() + Duration::from_secs(upload::ASSET_SECONDS);
            Ok(
                serde_json::json!({"assetId":id,"sizeBytes":entry.received,"sha256":entry.claims.file.sha256,"expiresInSeconds":upload::ASSET_SECONDS}),
            )
        })();
        // Never remove another application's asset on an invalid reference.
        if result.is_err()
            && entries.get(id).is_some_and(|e| {
                e.claims.platform == platform
                    && e.claims.bundle_identifier == app
                    && e.session == session
                    && !e.claimed
                    && !e.complete
            })
        {
            entries.remove(id);
        }
        result
    }

    pub fn abort(&self, platform: Platform, app: &str, id: &str, session: &str) {
        let mut entries = self.entries.lock().unwrap();
        if entries.get(id).is_some_and(|e| {
            e.claims.platform == platform
                && e.claims.bundle_identifier == app
                && e.session == session
                && !e.claimed
                && !e.complete
        }) {
            entries.remove(id);
        }
    }

    pub fn claim(self: &Arc<Self>, request: &StoreRequest) -> Result<AssetLease, StoreError> {
        self.purge();
        let ids = asset_ids(&request.payload);
        let mut entries = self.entries.lock().unwrap();
        for id in &ids {
            let entry = entries.get(id).ok_or_else(missing)?;
            check_scope(entry, request.platform, &request.bundle_identifier)?;
            if !entry.complete || entry.claimed {
                return Err(StoreError::new(
                    409,
                    "asset_unavailable",
                    "Asset is incomplete or already in use",
                ));
            }
        }
        for id in &ids {
            entries.get_mut(id).unwrap().claimed = true;
        }
        Ok(AssetLease {
            store: self.clone(),
            ids,
        })
    }

    pub fn read(&self, id: &str) -> Result<(Vec<u8>, String, String), StoreError> {
        let mut entries = self.entries.lock().unwrap();
        let entry = entries.get_mut(id).ok_or_else(missing)?;
        if !entry.complete || !entry.claimed {
            return Err(missing());
        }
        entry
            .file
            .seek(SeekFrom::Start(0))
            .map_err(|_| storage_error())?;
        let mut bytes = Vec::with_capacity(entry.received as usize);
        entry
            .file
            .read_to_end(&mut bytes)
            .map_err(|_| storage_error())?;
        Ok((
            bytes,
            entry.claims.file.file_name.clone(),
            entry.claims.file.content_type.clone(),
        ))
    }

    pub fn usage(&self) -> (usize, u64) {
        let entries = self.entries.lock().unwrap();
        (
            entries.len(),
            entries.values().map(|e| e.claims.file.size_bytes).sum(),
        )
    }
}

pub fn asset_ids(payload: &Value) -> HashSet<String> {
    let mut ids = HashSet::new();
    let owners: &[&str] = match payload["operation"].as_str() {
        Some("update_app_metadata") => &["metadata"],
        Some("update_marketing_page") => &["marketingPage"],
        _ => &[],
    };
    for &owner in owners {
        if let Some(slots) = payload[owner]["storeImages"].as_object() {
            for slot in slots.values() {
                if let Some(assets) = slot["assets"].as_array() {
                    for asset in assets {
                        if let Some(id) = asset["assetId"].as_str() {
                            ids.insert(id.to_owned());
                        }
                    }
                }
            }
        }
    }
    ids
}
fn check_scope(entry: &Entry, platform: Platform, app: &str) -> Result<(), StoreError> {
    if entry.claims.platform != platform || entry.claims.bundle_identifier != app {
        return Err(StoreError::new(
            403,
            "asset_scope_mismatch",
            "Asset belongs to a different application",
        ));
    }
    if entry.expires <= Instant::now() {
        return Err(missing());
    }
    Ok(())
}
fn check_session(entry: &Entry, session: &str) -> Result<(), StoreError> {
    if entry.session != session {
        return Err(StoreError::new(
            409,
            "upload_session_mismatch",
            "Another upload owns this asset",
        ));
    }
    Ok(())
}
fn missing() -> StoreError {
    StoreError::new(404, "asset_not_found", "Upload asset is missing or expired")
}
fn storage_error() -> StoreError {
    StoreError::new(
        507,
        "upload_storage",
        "Cannot access temporary upload storage",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageEncoder;
    use store_protocol::upload::UploadInput;
    const SESSION: &str = "00000000-0000-4000-8000-000000000099";
    fn ticket(index: usize, size: u64) -> String {
        let now = upload::now().unwrap();
        let claims = UploadClaims {
            iss: "testflying-store-api".into(),
            aud: "connector:test-node".into(),
            iat: now,
            exp: now + upload::TOKEN_SECONDS,
            connector_id: "test-node".into(),
            asset_id: format!("00000000-0000-4000-8000-{index:012x}"),
            platform: Platform::Ios,
            bundle_identifier: "com.example.app".into(),
            file: UploadInput {
                file_name: "a.png".into(),
                content_type: "image/png".into(),
                size_bytes: size,
                sha256: "11".repeat(32),
            },
        };
        upload::sign(&claims, &"22".repeat(32)).unwrap()
    }
    #[test]
    fn reserved_bytes_count_and_expiration_bound_anonymous_files() {
        let store = AssetStore::new("test-node".into(), "22".repeat(32));
        for i in 0..8 {
            store
                .start(&ticket(i, upload::MAX_FILE_BYTES), SESSION)
                .unwrap();
        }
        assert_eq!(store.usage(), (8, MAX_RESERVED));
        assert_eq!(
            store.start(&ticket(8, 1), SESSION).unwrap_err().code,
            "upload_capacity"
        );
        for entry in store.entries.lock().unwrap().values_mut() {
            entry.expires = Instant::now() - Duration::from_secs(1);
        }
        store.purge();
        assert_eq!(store.usage(), (0, 0));
        for i in 0..MAX_ASSETS {
            store.start(&ticket(i, 1), SESSION).unwrap();
        }
        assert_eq!(
            store
                .start(&ticket(MAX_ASSETS, 1), SESSION)
                .unwrap_err()
                .code,
            "upload_capacity"
        );
    }
    #[test]
    fn incomplete_upload_survives_fifteen_minutes_but_expires_after_thirty() {
        let store = AssetStore::new("test-node".into(), "22".repeat(32));
        let claims = store.start(&ticket(0, 3), SESSION).unwrap();
        // Shift deadlines to simulate elapsed time without sleeping or changing the wall clock.
        store
            .entries
            .lock()
            .unwrap()
            .get_mut(&claims.asset_id)
            .unwrap()
            .expires -= Duration::from_secs(15 * 60 + 1);
        store.purge();
        store
            .chunk(
                Platform::Ios,
                "com.example.app",
                &claims.asset_id,
                SESSION,
                0,
                b"abc",
            )
            .unwrap();
        store
            .entries
            .lock()
            .unwrap()
            .get_mut(&claims.asset_id)
            .unwrap()
            .expires -= Duration::from_secs(15 * 60);
        store.purge();
        assert_eq!(store.usage(), (0, 0));
        assert_eq!(
            store
                .chunk(
                    Platform::Ios,
                    "com.example.app",
                    &claims.asset_id,
                    SESSION,
                    3,
                    b"x"
                )
                .unwrap_err()
                .code,
            "asset_not_found"
        );
    }
    #[test]
    fn authenticated_upload_can_finish_after_ticket_expiration_and_then_expires_in_fifteen_minutes()
    {
        let store = AssetStore::new("test-node".into(), "22".repeat(32));
        let mut image = Vec::new();
        image::codecs::png::PngEncoder::new(&mut image)
            .write_image(&[0; 12], 2, 2, image::ExtendedColorType::Rgb8)
            .unwrap();
        let mut claims = store.claims(&ticket(0, image.len() as u64)).unwrap();
        claims.file.sha256 = hex::encode(Sha256::digest(&image));
        let signed = upload::sign(&claims, &"22".repeat(32)).unwrap();
        store.start(&signed, SESSION).unwrap();
        // The authenticated transfer has its own deadline, even if the ticket expires afterward.
        store
            .entries
            .lock()
            .unwrap()
            .get_mut(&claims.asset_id)
            .unwrap()
            .claims
            .exp = upload::now().unwrap() - 1;
        store
            .chunk(
                Platform::Ios,
                "com.example.app",
                &claims.asset_id,
                SESSION,
                0,
                &image,
            )
            .unwrap();
        let result = store
            .finish(Platform::Ios, "com.example.app", &claims.asset_id, SESSION)
            .unwrap();
        assert_eq!(result["expiresInSeconds"], 900);
        store
            .entries
            .lock()
            .unwrap()
            .get_mut(&claims.asset_id)
            .unwrap()
            .expires -= Duration::from_secs(15 * 60 + 1);
        store.purge();
        assert_eq!(store.usage(), (0, 0));
    }
    #[test]
    fn offsets_and_failed_finalization_do_not_leave_a_file() {
        let store = AssetStore::new("test-node".into(), "22".repeat(32));
        let claims = store.start(&ticket(0, 3), SESSION).unwrap();
        assert!(store
            .chunk(
                Platform::Android,
                "com.example.app",
                &claims.asset_id,
                SESSION,
                0,
                b"abc"
            )
            .is_err());
        assert!(store
            .chunk(
                Platform::Ios,
                "com.example.app",
                &claims.asset_id,
                SESSION,
                1,
                b"abc"
            )
            .is_err());
        store
            .chunk(
                Platform::Ios,
                "com.example.app",
                &claims.asset_id,
                SESSION,
                0,
                b"abc",
            )
            .unwrap();
        assert!(store
            .finish(Platform::Ios, "com.example.app", &claims.asset_id, SESSION)
            .is_err());
        assert_eq!(store.usage(), (0, 0));
    }
}
