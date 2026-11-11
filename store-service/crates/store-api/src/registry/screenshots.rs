use super::*;
use serde_json::{json, Value};
use store_protocol::upload::now;

pub(super) fn create_schema(db: &Connection) -> Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS screenshot_jobs (
            id TEXT PRIMARY KEY, connector_id TEXT NOT NULL REFERENCES connectors(id),
            integration_id TEXT NOT NULL, profile TEXT NOT NULL, prompt TEXT NOT NULL,
            status TEXT NOT NULL, created_at INTEGER NOT NULL, deadline INTEGER NOT NULL,
            lease TEXT, result TEXT, error TEXT
         );
         CREATE INDEX IF NOT EXISTS screenshot_jobs_queue
            ON screenshot_jobs(connector_id,status,created_at,id);
         CREATE TABLE IF NOT EXISTS screenshot_upload_tickets (
            job_id TEXT PRIMARY KEY REFERENCES screenshot_jobs(id) ON DELETE CASCADE,
            connector_id TEXT NOT NULL, lease TEXT NOT NULL, object_key TEXT NOT NULL UNIQUE,
            size_bytes INTEGER NOT NULL, sha256 TEXT NOT NULL, md5 TEXT NOT NULL,
            config_fingerprint TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS screenshot_app_bindings (
            platform TEXT NOT NULL CHECK(platform IN ('ios','android')),
            bundle_identifier TEXT NOT NULL,
            connector_id TEXT NOT NULL REFERENCES connectors(id),
            PRIMARY KEY(platform,bundle_identifier));",
    )
    .map_err(database_error)?;
    let columns: Vec<String> = {
        let mut statement = db
            .prepare("PRAGMA table_info(screenshot_jobs)")
            .map_err(database_error)?;
        let values = statement
            .query_map([], |row| row.get(1))
            .map_err(database_error)?;
        values
            .collect::<std::result::Result<_, _>>()
            .map_err(database_error)?
    };
    for column in ["platform", "bundle_identifier"] {
        if !columns.iter().any(|name| name == column) {
            db.execute(
                &format!("ALTER TABLE screenshot_jobs ADD COLUMN {column} TEXT"),
                [],
            )
            .map_err(database_error)?;
        }
    }
    Ok(())
}

fn timestamp() -> Result<i64> {
    i64::try_from(now()?)
        .map_err(|_| StoreError::new(500, "invalid_clock", "System clock is invalid"))
}

fn expire(db: &Connection, time: i64) -> Result<()> {
    db.execute(
        "UPDATE screenshot_jobs SET status='failed', prompt='', lease=NULL,
         error=CASE WHEN status='uploading' THEN 'delivery_unknown' ELSE 'job_expired' END
         WHERE deadline<=?1 AND status IN ('queued','running','uploading')",
        [time],
    )
    .map_err(database_error)?;
    db.execute(
        "DELETE FROM screenshot_jobs WHERE status IN ('failed','succeeded') AND created_at<=?1",
        [time.saturating_sub(7 * 86400)],
    )
    .map_err(database_error)?;
    Ok(())
}

fn invalid_lease() -> StoreError {
    StoreError::new(
        409,
        "invalid_job_lease",
        "Screenshot job is no longer assigned to this worker",
    )
}

type QueuedScreenshot = (String, String, String, Option<String>, Option<String>);

#[derive(Clone)]
pub(crate) struct ScreenshotTicket {
    pub object_key: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub md5: String,
    pub config_fingerprint: String,
    pub deadline: i64,
    pub result: Option<Value>,
}

pub(crate) fn oss_fingerprint(config: &crate::screenshot_settings::OssSettings) -> Result<String> {
    use sha2::{Digest, Sha256};
    Ok(hex::encode(Sha256::digest(
        serde_json::to_vec(config).map_err(|_| database_error(rusqlite::Error::InvalidQuery))?,
    )))
}

fn ticket_config_changed() -> StoreError {
    StoreError::new(
        409,
        "screenshot_oss_config_changed",
        "Screenshot OSS settings changed after this upload ticket was issued",
    )
}

fn check_upload_lease(
    db: &Connection,
    node: &str,
    id: &str,
    lease: &str,
    succeeded: bool,
) -> Result<Option<Value>> {
    let row: Option<(String, Option<String>, Option<String>)> = db.query_row(
        "SELECT j.status,j.lease,j.result FROM screenshot_jobs j JOIN connectors c ON c.id=j.connector_id
         WHERE j.id=?1 AND j.connector_id=?2 AND j.deadline>?3 AND c.enabled=1 AND c.mode='screenshot'
         AND EXISTS(SELECT 1 FROM integrations WHERE id=j.integration_id)",
        params![id,node,timestamp()?], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))
    ).optional().map_err(database_error)?;
    match row {
        Some((status, Some(current), _)) if status == "running" && current == lease => Ok(None),
        Some((status, _, Some(result))) if succeeded && status == "succeeded" => {
            let matches: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM screenshot_upload_tickets WHERE job_id=?1 AND connector_id=?2 AND lease=?3)",params![id,node,lease],|row|row.get(0)).map_err(database_error)?;
            if !matches {
                return Err(invalid_lease());
            }
            serde_json::from_str(&result)
                .map(Some)
                .map_err(|_| database_error(rusqlite::Error::InvalidQuery))
        }
        _ => Err(invalid_lease()),
    }
}

fn load_ticket(
    db: &Connection,
    node: &str,
    id: &str,
    lease: &str,
) -> Result<Option<ScreenshotTicket>> {
    db.query_row("SELECT t.object_key,t.size_bytes,t.sha256,t.md5,t.config_fingerprint,j.deadline FROM screenshot_upload_tickets t JOIN screenshot_jobs j ON j.id=t.job_id WHERE t.job_id=?1 AND t.connector_id=?2 AND t.lease=?3",params![id,node,lease],|row| {
        let size: i64 = row.get(1)?;
        let size_bytes = u64::try_from(size).map_err(|error| rusqlite::Error::FromSqlConversionFailure(1,rusqlite::types::Type::Integer,Box::new(error)))?;
        Ok(ScreenshotTicket {
            object_key:row.get(0)?,size_bytes,sha256:row.get(2)?,md5:row.get(3)?,config_fingerprint:row.get(4)?,deadline:row.get(5)?,result:None,
        })
    }).optional().map_err(database_error)
}
fn claimable_screenshot(
    db: &Connection,
    node: &str,
    time: i64,
) -> Result<Option<QueuedScreenshot>> {
    db.query_row(
        "SELECT id,profile,prompt,platform,bundle_identifier FROM screenshot_jobs j WHERE connector_id=?1 AND status='queued'
         AND deadline>?2 AND EXISTS(SELECT 1 FROM integrations WHERE id=j.integration_id)
         AND NOT EXISTS(SELECT 1 FROM screenshot_jobs WHERE connector_id=?1
                        AND status IN ('running','uploading') AND deadline>?2)
         ORDER BY created_at,rowid LIMIT 1",
        params![node, time],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?,row.get(3)?,row.get(4)?)),
    )
    .optional()
    .map_err(database_error)
}

impl Registry {
    pub(crate) fn screenshot_upload_lease(&self, node: &str, id: &str, lease: &str) -> Result<()> {
        check_upload_lease(&self.db.lock().unwrap(), node, id, lease, false).map(|_| ())
    }

    // Keep the lease, file metadata, and signed storage snapshot explicit at this atomic boundary.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_screenshot_ticket(
        &self,
        node: &str,
        id: &str,
        lease: &str,
        size: u64,
        sha256: &str,
        md5: &str,
        fingerprint: &str,
        key: &str,
    ) -> Result<ScreenshotTicket> {
        let mut db = self.db.lock().unwrap();
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        check_upload_lease(&tx, node, id, lease, false)?;
        let stored_size = i64::try_from(size).map_err(|_| {
            StoreError::new(
                422,
                "invalid_screenshot_metadata",
                "Screenshot size is invalid",
            )
        })?;
        let deadline: i64 = tx
            .query_row(
                "SELECT deadline FROM screenshot_jobs WHERE id=?1",
                [id],
                |row| row.get(0),
            )
            .map_err(database_error)?;
        let settings = super::screenshot_settings::load(&tx, &self.signing_key)?;
        if oss_fingerprint(&settings.oss)? != fingerprint {
            return Err(ticket_config_changed());
        }
        let ticket = if let Some(ticket) = load_ticket(&tx, node, id, lease)? {
            if ticket.size_bytes != size || ticket.sha256 != sha256 || ticket.md5 != md5 {
                return Err(StoreError::new(
                    409,
                    "screenshot_file_changed",
                    "This screenshot task already has an upload ticket for another file",
                ));
            }
            if ticket.config_fingerprint != fingerprint {
                return Err(ticket_config_changed());
            }
            ticket
        } else {
            tx.execute("INSERT INTO screenshot_upload_tickets(job_id,connector_id,lease,object_key,size_bytes,sha256,md5,config_fingerprint) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![id,node,lease,key,stored_size,sha256,md5,fingerprint]).map_err(database_error)?;
            ScreenshotTicket {
                object_key: key.into(),
                size_bytes: size,
                sha256: sha256.into(),
                md5: md5.into(),
                config_fingerprint: fingerprint.into(),
                deadline,
                result: None,
            }
        };
        tx.commit().map_err(database_error)?;
        Ok(ticket)
    }

    pub(crate) fn screenshot_ticket(
        &self,
        node: &str,
        id: &str,
        lease: &str,
    ) -> Result<ScreenshotTicket> {
        let db = self.db.lock().unwrap();
        let result = check_upload_lease(&db, node, id, lease, true)?;
        let mut ticket = load_ticket(&db, node, id, lease)?.ok_or_else(|| {
            StoreError::new(
                409,
                "screenshot_ticket_missing",
                "Prepare a screenshot upload ticket before completion",
            )
        })?;
        ticket.result = result;
        Ok(ticket)
    }

    pub(crate) fn complete_screenshot_ticket(
        &self,
        node: &str,
        id: &str,
        lease: &str,
        fingerprint: &str,
        result: &Value,
    ) -> Result<Value> {
        let mut db = self.db.lock().unwrap();
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        if let Some(result) = check_upload_lease(&tx, node, id, lease, true)? {
            return Ok(result);
        }
        let ticket = load_ticket(&tx, node, id, lease)?.ok_or_else(invalid_lease)?;
        let settings = super::screenshot_settings::load(&tx, &self.signing_key)?;
        if ticket.config_fingerprint != fingerprint
            || oss_fingerprint(&settings.oss)? != fingerprint
        {
            return Err(ticket_config_changed());
        }
        let mut result = result.clone();
        result["notificationStatus"] = json!(if settings.notification.url.is_empty() {
            "not_configured"
        } else {
            "protocol_pending"
        });
        let changed = tx.execute("UPDATE screenshot_jobs SET status='succeeded',result=?4,error=NULL,lease=NULL WHERE id=?1 AND connector_id=?2 AND lease=?3 AND status='running' AND deadline>?5",params![id,node,lease,result.to_string(),timestamp()?]).map_err(database_error)?;
        if changed != 1 {
            return Err(invalid_lease());
        }
        tx.commit().map_err(database_error)?;
        Ok(result)
    }

    pub(crate) fn bind_screenshot(
        &self,
        platform: Platform,
        identifier: &str,
        id: &str,
    ) -> Result<Connector> {
        validate_identifier(identifier)?;
        {
            let mut db = self.db.lock().unwrap();
            let tx = db
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(database_error)?;
            let node: Option<(bool, String)> = tx
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
                Some((true, kind)) if kind != "screenshot" => {
                    return Err(StoreError::new(
                        422,
                        "invalid_screenshot_connector",
                        "Select a Screenshot Connector",
                    ))
                }
                _ => {}
            }
            tx.execute("INSERT INTO screenshot_app_bindings(platform,bundle_identifier,connector_id) VALUES(?1,?2,?3) ON CONFLICT(platform,bundle_identifier) DO UPDATE SET connector_id=excluded.connector_id",params![platform.to_string(),identifier,id]).map_err(database_error)?;
            tx.commit().map_err(database_error)?;
        }
        self.get(id)
    }
    pub(crate) fn screenshot_binding(
        &self,
        platform: Platform,
        identifier: &str,
    ) -> Result<Connector> {
        validate_identifier(identifier)?;
        let id:Option<String>=self.db.lock().unwrap().query_row("SELECT connector_id FROM screenshot_app_bindings WHERE platform=?1 AND bundle_identifier=?2",params![platform.to_string(),identifier],|row|row.get(0)).optional().map_err(database_error)?;
        self.get(&id.ok_or_else(missing_screenshot_binding)?)
    }
    pub(crate) fn unbind_screenshot(&self, platform: Platform, identifier: &str) -> Result<()> {
        validate_identifier(identifier)?;
        self.db
            .lock()
            .unwrap()
            .execute(
                "DELETE FROM screenshot_app_bindings WHERE platform=?1 AND bundle_identifier=?2",
                params![platform.to_string(), identifier],
            )
            .map_err(database_error)?;
        Ok(())
    }
    pub(crate) fn enqueue_screenshot(
        &self,
        access: &Integration,
        platform: Platform,
        identifier: &str,
        profile: &str,
        prompt: &str,
    ) -> Result<Value> {
        validate_identifier(identifier)?;
        let time = timestamp()?;
        let mut db = self.db.lock().unwrap();
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        expire(&tx, time)?;
        let binding:Option<(String,bool)>=tx.query_row("SELECT c.id,c.enabled FROM screenshot_app_bindings b JOIN connectors c ON c.id=b.connector_id WHERE b.platform=?1 AND b.bundle_identifier=?2 AND c.mode='screenshot'",params![platform.to_string(),identifier],|row|Ok((row.get(0)?,row.get(1)?))).optional().map_err(database_error)?;
        let (node, enabled) = binding.ok_or_else(missing_screenshot_binding)?;
        if !enabled {
            return Err(StoreError::new(
                503,
                "screenshot_connector_disabled",
                "Bound Screenshot Connector is disabled",
            ));
        }
        let permitted: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM connectors WHERE id=?1 AND enabled=1 AND mode='screenshot'
                 AND EXISTS(SELECT 1 FROM integrations WHERE id=?2))",
                params![node, access.id],
                |row| row.get(0),
            )
            .map_err(database_error)?;
        if !permitted {
            return Err(StoreError::new(
                422,
                "invalid_screenshot_connector",
                "Select an enabled Screenshot Connector",
            ));
        }
        let queued: u32 = tx
            .query_row(
                "SELECT count(*) FROM screenshot_jobs WHERE connector_id=?1 AND status='queued'",
                [&node],
                |row| row.get(0),
            )
            .map_err(database_error)?;
        if queued >= 20 {
            return Err(StoreError::new(
                429,
                "screenshot_queue_full",
                "Screenshot Connector queue is full",
            ));
        }
        let id = Uuid::new_v4().to_string();
        tx.execute(
            "INSERT INTO screenshot_jobs(id,connector_id,integration_id,profile,prompt,status,created_at,deadline,platform,bundle_identifier)
             VALUES(?1,?2,?3,?4,?5,'queued',?6,?7,?8,?9)",
            params![id, node, access.id, profile, prompt, time, time + 86400,platform.to_string(),identifier],
        ).map_err(database_error)?;
        tx.commit().map_err(database_error)?;
        Ok(
            json!({"jobId":id,"status":"queued","connectorId":node,"platform":platform,"bundleIdentifier":identifier}),
        )
    }

    pub(crate) fn screenshot_status(&self, access: &Integration, id: &str) -> Result<Value> {
        let db = self.db.lock().unwrap();
        expire(&db, timestamp()?)?;
        let job = db.query_row(
            "SELECT connector_id,profile,status,result,error,platform,bundle_identifier FROM screenshot_jobs WHERE id=?1 AND integration_id=?2",
            params![id, access.id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?,
                      row.get::<_, Option<String>>(3)?, row.get::<_, Option<String>>(4)?,row.get::<_,Option<String>>(5)?,row.get::<_,Option<String>>(6)?)),
        ).optional().map_err(database_error)?;
        let (node, profile, status, result, error, platform, identifier) =
            job.ok_or_else(|| {
                StoreError::new(404, "screenshot_job_not_found", "Screenshot job not found")
            })?;
        let result = result
            .map(|body| serde_json::from_str::<Value>(&body))
            .transpose()
            .map_err(|_| {
                StoreError::new(500, "registry_error", "Invalid screenshot delivery record")
            })?;
        let profile = (!profile.is_empty()).then_some(profile);
        Ok(
            json!({"jobId":id,"connectorId":node,"platform":platform,"bundleIdentifier":identifier,"profile":profile,"status":status,"result":result,"error":error}),
        )
    }

    pub(crate) fn claim_screenshot(&self, node: &str) -> Result<Option<Value>> {
        let time = timestamp()?;
        let mut db = self.db.lock().unwrap();
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        expire(&tx, time)?;
        let claimed = if let Some((id, profile, prompt, platform, identifier)) =
            claimable_screenshot(&tx, node, time)?
        {
            let lease = Uuid::new_v4().to_string();
            tx.execute(
                "UPDATE screenshot_jobs SET status='running', deadline=?2, lease=?3, prompt='' WHERE id=?1",
                params![id, time + 1800, lease],
            ).map_err(database_error)?;
            let profile = (!profile.is_empty()).then_some(profile);
            let prompt = match (&platform, &identifier) {
                (Some(platform), Some(identifier)) => {
                    format!("应用平台：{platform}\n应用包名 / Bundle ID：{identifier}\n\n{prompt}")
                }
                _ => prompt,
            };
            Some(
                json!({"jobId":id,"platform":platform,"bundleIdentifier":identifier,"profile":profile,"prompt":prompt,"lease":lease}),
            )
        } else {
            None
        };
        tx.commit().map_err(database_error)?;
        Ok(claimed)
    }

    pub(crate) fn has_claimable_screenshot(&self, node: &str) -> Result<bool> {
        Ok(claimable_screenshot(&self.db.lock().unwrap(), node, timestamp()?)?.is_some())
    }

    pub(crate) fn begin_screenshot_upload(&self, node: &str, id: &str, lease: &str) -> Result<()> {
        let time = timestamp()?;
        let count = self
            .db
            .lock()
            .unwrap()
            .execute(
                "UPDATE screenshot_jobs SET status='uploading', deadline=?4+120
             WHERE id=?1 AND connector_id=?2 AND lease=?3 AND status='running' AND deadline>?4
             AND EXISTS(SELECT 1 FROM integrations WHERE id=screenshot_jobs.integration_id)",
                params![id, node, lease, time],
            )
            .map_err(database_error)?;
        if count != 1 {
            return Err(invalid_lease());
        }
        Ok(())
    }

    pub(crate) fn finish_screenshot(
        &self,
        node: &str,
        id: &str,
        lease: &str,
        result: std::result::Result<&Value, &str>,
        phase: &str,
    ) -> Result<()> {
        let (status, output, error) = match result {
            Ok(value) => ("succeeded", Some(value.to_string()), None),
            Err(code) => ("failed", None, Some(code)),
        };
        let count = self
            .db
            .lock()
            .unwrap()
            .execute(
                "UPDATE screenshot_jobs SET status=?4, result=?5, error=?6, lease=NULL
             WHERE id=?1 AND connector_id=?2 AND lease=?3 AND status=?7",
                params![id, node, lease, status, output, error, phase],
            )
            .map_err(database_error)?;
        if count != 1 {
            return Err(invalid_lease());
        }
        Ok(())
    }
}

fn missing_screenshot_binding() -> StoreError {
    StoreError::new(
        404,
        "screenshot_app_not_found",
        "No Screenshot Connector is bound to this platform and package",
    )
}

#[cfg(test)]
mod direct_delivery_tests {
    use super::*;

    fn ticket_fixture() -> (Registry, String, String, String, String) {
        let registry = Registry::open(
            ":memory:",
            "direct-delivery-registry-fixture-key-0123456789",
        )
        .unwrap();
        let node = registry
            .create_with_kind(
                ConnectorInput {
                    name: "Fixture screenshot node".into(),
                },
                ConnectorKind::Screenshot,
            )
            .unwrap();
        let integration = registry
            .issue_integration(
                serde_json::from_value(json!({"name":"Fixture screenshots"})).unwrap(),
            )
            .unwrap();
        registry
            .bind_screenshot(Platform::Ios, "com.example.fixture", &node.connector_id)
            .unwrap();
        let queued = registry
            .enqueue_screenshot(
                &integration,
                Platform::Ios,
                "com.example.fixture",
                "",
                "Fixture prompt",
            )
            .unwrap();
        let job = queued["jobId"].as_str().unwrap().to_owned();
        let claimed = registry
            .claim_screenshot(&node.connector_id)
            .unwrap()
            .unwrap();
        let lease = claimed["lease"].as_str().unwrap().to_owned();
        let fingerprint = oss_fingerprint(&registry.screenshot_settings().unwrap().oss).unwrap();
        registry
            .prepare_screenshot_ticket(
                &node.connector_id,
                &job,
                &lease,
                64,
                &"a".repeat(64),
                &"b".repeat(32),
                &fingerprint,
                "screenshots/fixture.png",
            )
            .unwrap();
        (registry, node.connector_id, job, lease, integration.id)
    }

    #[test]
    fn completion_transaction_rejects_oss_changes_but_accepts_notification_only_changes() {
        let (registry, node, job, lease, _) = ticket_fixture();
        let fingerprint = registry
            .screenshot_ticket(&node, &job, &lease)
            .unwrap()
            .config_fingerprint;
        registry
            .save_screenshot_settings(
                &serde_json::from_value(
                    json!({"notification":{"url":"https://notify.fixture.example"}}),
                )
                .unwrap(),
            )
            .unwrap();
        assert_eq!(
            oss_fingerprint(&registry.screenshot_settings().unwrap().oss).unwrap(),
            fingerprint
        );
        registry
            .save_screenshot_settings(
                &serde_json::from_value(json!({"oss":{"prefix":"changed"}})).unwrap(),
            )
            .unwrap();
        assert_eq!(
            registry
                .complete_screenshot_ticket(
                    &node,
                    &job,
                    &lease,
                    &fingerprint,
                    &json!({"imageUrl":"https://cdn.fixture.example/fixture.png"})
                )
                .unwrap_err()
                .code,
            "screenshot_oss_config_changed"
        );
        assert!(registry
            .screenshot_ticket(&node, &job, &lease)
            .unwrap()
            .result
            .is_none());
    }

    #[test]
    fn expired_lease_revoked_integration_or_disabled_node_cannot_complete() {
        for invalidation in ["expired", "revoked", "disabled"] {
            let (registry, node, job, lease, integration) = ticket_fixture();
            let fingerprint = registry
                .screenshot_ticket(&node, &job, &lease)
                .unwrap()
                .config_fingerprint;
            match invalidation {
                "expired" => {
                    registry
                        .db
                        .lock()
                        .unwrap()
                        .execute("UPDATE screenshot_jobs SET deadline=0 WHERE id=?1", [&job])
                        .unwrap();
                }
                "revoked" => registry.revoke_integration(&integration).unwrap(),
                _ => {
                    registry.set_enabled(&node, false).unwrap();
                }
            }
            assert!(registry.screenshot_ticket(&node, &job, &lease).is_err());
            assert_eq!(
                registry
                    .complete_screenshot_ticket(
                        &node,
                        &job,
                        &lease,
                        &fingerprint,
                        &json!({"imageUrl":"https://cdn.fixture.example/fixture.png"})
                    )
                    .unwrap_err()
                    .code,
                "invalid_job_lease"
            );
        }
    }
}
