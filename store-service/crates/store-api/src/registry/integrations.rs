use super::*;

pub(crate) const TOKEN_PREFIX: &str = "tf_int_";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Integration {
    pub id: String,
    pub name: String,
    pub connector_ids: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct IntegrationInput {
    name: String,
    connector_ids: Vec<String>,
}

impl Integration {
    pub fn allows(&self, id: &str) -> bool {
        self.connector_ids.iter().any(|connector| connector == id)
    }

    pub fn require_connector(&self, id: &str) -> Result<()> {
        if !self.allows(id) {
            return Err(StoreError::new(
                403,
                "connector_not_allowed",
                "Connector is outside this integration's grants",
            ));
        }
        Ok(())
    }
}

fn not_found() -> StoreError {
    StoreError::new(404, "integration_not_found", "Integration not found")
}

fn unauthorized() -> StoreError {
    StoreError::new(401, "unauthorized", "Invalid integration token")
}

fn grants(db: &Connection, id: &str) -> Result<Vec<String>> {
    let mut statement = db.prepare(
        "SELECT connector_id FROM integration_connectors WHERE integration_id=?1 ORDER BY connector_id",
    ).map_err(database_error)?;
    let rows = statement
        .query_map([id], |row| row.get(0))
        .map_err(database_error)?;
    rows.collect::<rusqlite::Result<_>>()
        .map_err(database_error)
}

impl Registry {
    pub(crate) fn integrations(&self) -> Result<Vec<Integration>> {
        let db = self.db.lock().unwrap();
        let mut statement = db
            .prepare("SELECT id,name FROM integrations ORDER BY name,id")
            .map_err(database_error)?;
        let rows = statement
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get(1)?)))
            .map_err(database_error)?;
        rows.map(|row| {
            let (id, name) = row.map_err(database_error)?;
            let connector_ids = grants(&db, &id)?;
            Ok(Integration {
                id,
                name,
                connector_ids,
            })
        })
        .collect()
    }

    pub(crate) fn issue_integration(&self, input: IntegrationInput) -> Result<Integration> {
        let name = input.name.trim();
        let mut connector_ids = input.connector_ids;
        connector_ids.sort();
        connector_ids.dedup();
        if name.is_empty()
            || name.len() > 160
            || connector_ids.is_empty()
            || connector_ids.len() > 100
        {
            return Err(StoreError::new(
                422,
                "invalid_integration",
                "Provide a system name and 1–100 Connectors",
            ));
        }
        let id = Uuid::new_v4().to_string();
        let mut db = self.db.lock().unwrap();
        let transaction = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let created = transaction
            .execute(
                "INSERT INTO integrations(id,name) VALUES(?1,?2) ON CONFLICT(name) DO NOTHING",
                params![id, name],
            )
            .map_err(database_error)?;
        if created == 0 {
            return Err(StoreError::new(
                409,
                "integration_name_in_use",
                "A token already exists for this system name",
            ));
        }
        for connector in &connector_ids {
            let inserted = transaction.execute(
                "INSERT INTO integration_connectors SELECT ?1,id FROM connectors WHERE id=?2 AND enabled=1",
                params![id, connector],
            ).map_err(database_error)?;
            if inserted == 0 {
                return Err(StoreError::new(
                    422,
                    "invalid_integration_connector",
                    "Select enabled Connectors",
                ));
            }
        }
        transaction.commit().map_err(database_error)?;
        Ok(Integration {
            id,
            name: name.to_owned(),
            connector_ids,
        })
    }

    fn integration(&self, id: &str) -> Result<Option<Integration>> {
        let db = self.db.lock().unwrap();
        let name = db
            .query_row("SELECT name FROM integrations WHERE id=?1", [id], |row| {
                row.get(0)
            })
            .optional()
            .map_err(database_error)?;
        name.map(|name| {
            Ok(Integration {
                id: id.into(),
                name,
                connector_ids: grants(&db, id)?,
            })
        })
        .transpose()
    }

    fn derive_integration_token(&self, id: &str) -> String {
        let mut mac =
            Hmac::<Sha256>::new_from_slice(&self.signing_key).expect("HMAC accepts arbitrary keys");
        mac.update(format!("testflying:integration:v1:{id}").as_bytes());
        format!(
            "{TOKEN_PREFIX}{id}.{}",
            hex::encode(mac.finalize().into_bytes())
        )
    }

    pub(crate) fn integration_token(&self, id: &str) -> Result<String> {
        self.integration(id)?.ok_or_else(not_found)?;
        Ok(self.derive_integration_token(id))
    }

    pub(crate) fn authenticate_integration(&self, token: &str) -> Result<Integration> {
        let (id, signature) = token
            .strip_prefix(TOKEN_PREFIX)
            .and_then(|value| value.split_once('.'))
            .ok_or_else(unauthorized)?;
        if id.len() != 36
            || signature.len() != 64
            || Uuid::parse_str(id).is_err()
            || !secret_eq(&self.derive_integration_token(id), token)
        {
            return Err(unauthorized());
        }
        self.integration(id)?.ok_or_else(unauthorized)
    }

    pub(crate) fn revoke_integration(&self, id: &str) -> Result<()> {
        let deleted = self
            .db
            .lock()
            .unwrap()
            .execute("DELETE FROM integrations WHERE id=?1", [id])
            .map_err(database_error)?;
        if deleted == 0 {
            return Err(not_found());
        }
        Ok(())
    }
}
