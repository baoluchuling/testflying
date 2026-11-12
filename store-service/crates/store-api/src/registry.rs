use hmac::{Hmac, Mac};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::{
    path::Path,
    sync::{Arc, Mutex},
};
use store_protocol::{validate_identifier, Platform, StoreError};
use subtle::ConstantTimeEq;
use uuid::Uuid;

mod integrations;
pub(crate) mod screenshot_notifications;
mod screenshot_settings;
pub(crate) mod screenshots;
pub(crate) mod store_jobs;
pub(crate) use integrations::{Integration, IntegrationInput, TOKEN_PREFIX};

pub type Result<T> = std::result::Result<T, StoreError>;

pub fn secret_eq(expected: &str, actual: &str) -> bool {
    bool::from(expected.as_bytes().ct_eq(actual.as_bytes()))
}

fn database_error(_: rusqlite::Error) -> StoreError {
    StoreError::new(500, "registry_error", "Connector registry unavailable")
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConnectorInput {
    pub name: String,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnectorKind {
    #[default]
    Store,
    Screenshot,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Connector {
    pub connector_id: String,
    pub name: String,
    pub kind: ConnectorKind,
    pub base_url: Option<String>,
    pub enabled: bool,
    pub app_count: u64,
    #[serde(skip)]
    nonce: String,
}

pub struct Registry {
    pub(crate) db: Arc<Mutex<Connection>>,
    pub logs: Arc<crate::logs::LogStore>,
    signing_key: Vec<u8>,
}

impl Registry {
    pub fn open(path: impl AsRef<Path>, signing_key: &str) -> Result<Self> {
        if signing_key.len() < 32 {
            return Err(StoreError::new(
                500,
                "invalid_signing_key",
                "Node signing key must contain at least 32 bytes",
            ));
        }
        let db = Connection::open(path).map_err(database_error)?;
        db.pragma_update(None, "foreign_keys", "ON")
            .map_err(database_error)?;
        db.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(database_error)?;
        // Keep the old grant table so an older binary can still read existing credentials.
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS connectors (
            id TEXT PRIMARY KEY, name TEXT NOT NULL, mode TEXT NOT NULL,
            base_url TEXT, enabled INTEGER NOT NULL DEFAULT 1, nonce TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS app_bindings (
            platform TEXT NOT NULL CHECK(platform IN ('ios','android')),
            bundle_identifier TEXT NOT NULL,
            connector_id TEXT NOT NULL REFERENCES connectors(id),
            PRIMARY KEY(platform,bundle_identifier));
            CREATE TABLE IF NOT EXISTS integrations (
            id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE);
            CREATE TABLE IF NOT EXISTS integration_connectors (
            integration_id TEXT NOT NULL REFERENCES integrations(id) ON DELETE CASCADE,
            connector_id TEXT NOT NULL REFERENCES connectors(id) ON DELETE CASCADE,
            PRIMARY KEY(integration_id,connector_id));",
        )
        .map_err(database_error)?;
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS platform_forwarding (
            platform TEXT PRIMARY KEY CHECK(platform IN ('ios','android')),
            enabled INTEGER NOT NULL CHECK(enabled IN (0,1)));
            INSERT OR IGNORE INTO platform_forwarding VALUES ('ios',1),('android',1);",
        )
        .map_err(database_error)?;
        screenshots::create_schema(&db)?;
        store_jobs::create_schema(&db)?;
        screenshot_settings::create_schema(&db)?;
        screenshot_notifications::create_schema(&db)?;
        let db = Arc::new(Mutex::new(db));
        let logs = crate::logs::LogStore::new(db.clone())?;
        Ok(Self {
            db,
            logs,
            signing_key: signing_key.as_bytes().to_vec(),
        })
    }

    pub fn create(&self, input: ConnectorInput) -> Result<Connector> {
        self.create_with_kind(input, ConnectorKind::Store)
    }

    pub fn forwarding(&self) -> Result<crate::forwarding::Settings> {
        let db = self.db.lock().unwrap();
        let enabled = |platform: &str| {
            db.query_row(
                "SELECT enabled FROM platform_forwarding WHERE platform=?1",
                [platform],
                |row| row.get(0),
            )
            .map_err(database_error)
        };
        Ok(crate::forwarding::Settings {
            ios: enabled("ios")?,
            android: enabled("android")?,
        })
    }

    pub fn set_forwarding(&self, platform: Platform, enabled: bool) -> Result<()> {
        self.db
            .lock()
            .unwrap()
            .execute(
                "UPDATE platform_forwarding SET enabled=?1 WHERE platform=?2",
                params![enabled, platform.to_string()],
            )
            .map_err(database_error)?;
        Ok(())
    }

    pub fn require_forwarding(&self, platform: Platform) -> Result<()> {
        let flags = self.forwarding()?;
        if match platform {
            Platform::Ios => flags.ios,
            Platform::Android => flags.android,
        } {
            Ok(())
        } else {
            Err(StoreError::new(
                503,
                "platform_forwarding_disabled",
                format!("{platform} store forwarding is paused"),
            ))
        }
    }

    pub fn create_with_kind(
        &self,
        input: ConnectorInput,
        kind: ConnectorKind,
    ) -> Result<Connector> {
        if input.name.trim().is_empty() || input.name.len() > 160 {
            return Err(StoreError::new(
                422,
                "invalid_name",
                "name must contain 1–160 characters",
            ));
        }
        let id = Uuid::new_v4().to_string();
        let nonce = Uuid::new_v4().to_string();
        let mode = match kind {
            ConnectorKind::Store => "auto",
            ConnectorKind::Screenshot => "screenshot",
        };
        self.db
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO connectors(id,name,mode,base_url,nonce) VALUES(?1,?2,?3,?4,?5)",
                params![id, input.name.trim(), mode, Option::<String>::None, nonce],
            )
            .map_err(database_error)?;
        self.get(&id)
    }

    pub fn get(&self, id: &str) -> Result<Connector> {
        self.db.lock().unwrap().query_row("SELECT c.id,c.name,c.mode,c.base_url,c.enabled,c.nonce,(SELECT count(*) FROM app_bindings a WHERE a.connector_id=c.id)+(SELECT count(*) FROM screenshot_app_bindings a WHERE a.connector_id=c.id) FROM connectors c WHERE c.id=?1", [id], |row| Ok(Connector {
            connector_id: row.get(0)?, name: row.get(1)?,
            kind: if row.get::<_, String>(2)? == "screenshot" { ConnectorKind::Screenshot } else { ConnectorKind::Store },
            base_url: row.get(3)?, enabled: row.get(4)?, nonce: row.get(5)?, app_count: row.get::<_, i64>(6)? as u64,
        })).optional().map_err(database_error)?.ok_or_else(|| StoreError::new(404, "connector_not_found", "Connector not found"))
    }

    pub fn list(&self) -> Result<Vec<Connector>> {
        let ids: Vec<String> = {
            let db = self.db.lock().unwrap();
            let mut stmt = db
                .prepare("SELECT id FROM connectors ORDER BY name,id")
                .map_err(database_error)?;
            let rows = stmt
                .query_map([], |row| row.get(0))
                .map_err(database_error)?;
            rows.collect::<std::result::Result<_, _>>()
                .map_err(database_error)?
        };
        ids.iter().map(|id| self.get(id)).collect()
    }

    pub fn token(&self, connector: &Connector) -> String {
        let mut mac =
            Hmac::<Sha256>::new_from_slice(&self.signing_key).expect("HMAC accepts arbitrary keys");
        mac.update(format!("{}:{}", connector.connector_id, connector.nonce).as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }

    pub fn purpose_key(&self, connector: &Connector, purpose: &str) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.signing_key).expect("HMAC key");
        mac.update(
            format!(
                "{purpose}:v1:{}:{}",
                connector.connector_id, connector.nonce
            )
            .as_bytes(),
        );
        hex::encode(mac.finalize().into_bytes())
    }

    /// Called only after the encrypted session has proved possession of current keys.
    pub fn register_endpoint(&self, id: &str, token: &str, endpoint: Option<&str>) -> Result<()> {
        let endpoint = endpoint
            .map(store_protocol::upload::normalize_base_url)
            .transpose()?;
        let db = self.db.lock().unwrap();
        let node = db
            .query_row(
                "SELECT nonce,enabled FROM connectors WHERE id=?1",
                [id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?)),
            )
            .optional()
            .map_err(database_error)?;
        let Some((nonce, enabled)) = node else {
            return Err(StoreError::new(
                401,
                "invalid_node_token",
                "Invalid Connector credentials",
            ));
        };
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.signing_key).expect("HMAC key");
        mac.update(format!("{id}:{nonce}").as_bytes());
        if !enabled || !secret_eq(&hex::encode(mac.finalize().into_bytes()), token) {
            return Err(StoreError::new(
                401,
                "invalid_node_token",
                "Invalid Connector credentials",
            ));
        }
        db.execute(
            "UPDATE connectors SET base_url=?1 WHERE id=?2",
            params![endpoint, id],
        )
        .map_err(database_error)?;
        Ok(())
    }

    pub fn authenticate(&self, id: &str, token: &str) -> Result<Connector> {
        let node = self.get(id).map_err(|_| {
            StoreError::new(401, "invalid_node_token", "Invalid Connector credentials")
        })?;
        if !node.enabled || !secret_eq(&self.token(&node), token) {
            return Err(StoreError::new(
                401,
                "invalid_node_token",
                "Invalid Connector credentials",
            ));
        }
        Ok(node)
    }

    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<Connector> {
        self.get(id)?;
        self.db
            .lock()
            .unwrap()
            .execute(
                "UPDATE connectors SET enabled=?1 WHERE id=?2",
                params![enabled, id],
            )
            .map_err(database_error)?;
        self.get(id)
    }

    pub fn rotate(&self, id: &str) -> Result<Connector> {
        self.get(id)?;
        self.db
            .lock()
            .unwrap()
            .execute(
                "UPDATE connectors SET nonce=?1,base_url=NULL WHERE id=?2",
                params![Uuid::new_v4().to_string(), id],
            )
            .map_err(database_error)?;
        self.get(id)
    }

    pub fn bind(&self, platform: Platform, identifier: &str, id: &str) -> Result<Connector> {
        validate_identifier(identifier)?;
        {
            let mut db = self.db.lock().unwrap();
            let transaction = db
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(database_error)?;
            let node: Option<(bool, String)> = transaction
                .query_row(
                    "SELECT enabled,mode FROM connectors WHERE id=?1",
                    [id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(database_error)?;
            match node {
                None => {
                    return Err(StoreError::new(
                        404,
                        "connector_not_found",
                        "Connector not found",
                    ))
                }
                Some((false, _)) => {
                    return Err(StoreError::new(
                        409,
                        "connector_disabled",
                        "Connector is disabled",
                    ))
                }
                Some((true, mode)) if mode == "screenshot" => {
                    return Err(StoreError::new(
                        422,
                        "invalid_connector_kind",
                        "App bindings require a Store Connector",
                    ))
                }
                Some((true, _)) => {}
            }
            transaction.execute("INSERT INTO app_bindings(platform,bundle_identifier,connector_id) VALUES(?1,?2,?3) ON CONFLICT(platform,bundle_identifier) DO UPDATE SET connector_id=excluded.connector_id", params![platform.to_string(),identifier,id]).map_err(database_error)?;
            transaction.commit().map_err(database_error)?;
        }
        self.get(id)
    }

    pub fn unbind(&self, platform: Platform, identifier: &str) -> Result<()> {
        validate_identifier(identifier)?;
        self.db
            .lock()
            .unwrap()
            .execute(
                "DELETE FROM app_bindings WHERE platform=?1 AND bundle_identifier=?2",
                params![platform.to_string(), identifier],
            )
            .map_err(database_error)?;
        Ok(())
    }

    pub fn resolve(&self, platform: Platform, identifier: &str) -> Result<Connector> {
        validate_identifier(identifier)?;
        let id = binding(&self.db.lock().unwrap(), platform, identifier)?.ok_or_else(|| {
            StoreError::new(
                404,
                "app_not_found",
                "No Connector is assigned to this application",
            )
        })?;
        let node = self.get(&id)?;
        if !node.enabled {
            return Err(StoreError::new(
                503,
                "connector_disabled",
                "Connector is disabled",
            ));
        }
        Ok(node)
    }
}

fn binding(db: &Connection, platform: Platform, identifier: &str) -> Result<Option<String>> {
    db.query_row(
        "SELECT connector_id FROM app_bindings WHERE platform=?1 AND bundle_identifier=?2",
        params![platform.to_string(), identifier],
        |row| row.get(0),
    )
    .optional()
    .map_err(database_error)
}
