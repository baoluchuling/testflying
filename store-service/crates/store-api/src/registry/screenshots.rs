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
    if !columns.iter().any(|name| name == "targets") {
        db.execute("ALTER TABLE screenshot_jobs ADD COLUMN targets TEXT", [])
            .map_err(database_error)?;
    }
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS screenshot_target_tickets (
          job_id TEXT NOT NULL REFERENCES screenshot_jobs(id) ON DELETE CASCADE,
          target_index INTEGER NOT NULL, connector_id TEXT NOT NULL, lease TEXT NOT NULL,
          object_key TEXT NOT NULL UNIQUE, size_bytes INTEGER NOT NULL, sha256 TEXT NOT NULL,
          md5 TEXT NOT NULL, config_fingerprint TEXT NOT NULL, result TEXT,
          PRIMARY KEY(job_id,target_index));",
    )
    .map_err(database_error)?;
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

#[derive(Clone, Debug)]
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
fn check_target_lease(db: &Connection, node: &str, id: &str, lease: &str) -> Result<()> {
    let valid: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM screenshot_jobs j JOIN connectors c ON c.id=j.connector_id WHERE j.id=?1 AND j.connector_id=?2 AND j.deadline>?4 AND c.enabled=1 AND c.mode='screenshot' AND EXISTS(SELECT 1 FROM integrations WHERE id=j.integration_id) AND ((j.status='running' AND j.lease=?3) OR (j.status='succeeded' AND EXISTS(SELECT 1 FROM screenshot_target_tickets t WHERE t.job_id=j.id AND t.connector_id=?2 AND t.lease=?3))))",params![id,node,lease,timestamp()?],|r|r.get(0)).map_err(database_error)?;
    if valid {
        Ok(())
    } else {
        Err(invalid_lease())
    }
}

fn load_target_ticket(
    db: &Connection,
    node: &str,
    id: &str,
    lease: &str,
    index: usize,
) -> Result<Option<ScreenshotTicket>> {
    db.query_row("SELECT t.object_key,t.size_bytes,t.sha256,t.md5,t.config_fingerprint,j.deadline,t.result FROM screenshot_target_tickets t JOIN screenshot_jobs j ON j.id=t.job_id WHERE t.job_id=?1 AND t.connector_id=?2 AND t.lease=?3 AND t.target_index=?4",params![id,node,lease,index as i64],|row| {
        let result: Option<String> = row.get(6)?;
        let result = result.map(|v| serde_json::from_str(&v)).transpose().map_err(|e|rusqlite::Error::FromSqlConversionFailure(6,rusqlite::types::Type::Text,Box::new(e)))?;
        let size: i64 = row.get(1)?;
        let size_bytes = u64::try_from(size).map_err(|error| rusqlite::Error::FromSqlConversionFailure(1,rusqlite::types::Type::Integer,Box::new(error)))?;
        Ok(ScreenshotTicket {object_key:row.get(0)?,size_bytes,sha256:row.get(2)?,md5:row.get(3)?,config_fingerprint:row.get(4)?,deadline:row.get(5)?,result})
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
    pub(crate) fn screenshot_target_index(
        &self,
        id: &str,
        index: Option<usize>,
    ) -> Result<Option<usize>> {
        let db = self.db.lock().unwrap();
        let targets: Option<String> = db
            .query_row(
                "SELECT targets FROM screenshot_jobs WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .optional()
            .map_err(database_error)?
            .flatten();
        match (targets, index) {
            (None, None) => Ok(None),
            (Some(targets), Some(index)) => {
                let targets: Vec<Value> = serde_json::from_str(&targets)
                    .map_err(|_| database_error(rusqlite::Error::InvalidQuery))?;
                if index < targets.len() {
                    Ok(Some(index))
                } else {
                    Err(StoreError::new(
                        422,
                        "invalid_target_index",
                        "targetIndex is outside the task targets",
                    ))
                }
            }
            _ => Err(StoreError::new(
                422,
                "invalid_target_index",
                "targetIndex is required only for structured screenshot tasks",
            )),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_target_ticket(
        &self,
        node: &str,
        id: &str,
        lease: &str,
        index: usize,
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
        check_target_lease(&tx, node, id, lease)?;
        if oss_fingerprint(&super::screenshot_settings::load(&tx, &self.signing_key)?.oss)?
            != fingerprint
        {
            return Err(ticket_config_changed());
        }
        if let Some(ticket) = load_target_ticket(&tx, node, id, lease, index)? {
            if ticket.result.is_some() {
                return Err(StoreError::new(
                    409,
                    "screenshot_target_completed",
                    "This target is already confirmed",
                ));
            }
            if ticket.size_bytes != size || ticket.sha256 != sha256 || ticket.md5 != md5 {
                return Err(StoreError::new(
                    409,
                    "screenshot_file_changed",
                    "This target already has a ticket for another file",
                ));
            }
            if ticket.config_fingerprint != fingerprint {
                return Err(ticket_config_changed());
            }
            return Ok(ticket);
        }
        let pending: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM screenshot_target_tickets WHERE job_id=?1 AND result IS NULL)",[id],|r|r.get(0)).map_err(database_error)?;
        if pending {
            return Err(StoreError::new(
                409,
                "screenshot_target_pending",
                "Confirm the current target before preparing another",
            ));
        }
        let size = i64::try_from(size)
            .map_err(|_| StoreError::new(422, "invalid_screenshot_metadata", "Invalid size"))?;
        tx.execute("INSERT INTO screenshot_target_tickets(job_id,target_index,connector_id,lease,object_key,size_bytes,sha256,md5,config_fingerprint) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![id,index as i64,node,lease,key,size,sha256,md5,fingerprint]).map_err(database_error)?;
        let ticket = load_target_ticket(&tx, node, id, lease, index)?.ok_or_else(invalid_lease)?;
        tx.commit().map_err(database_error)?;
        Ok(ticket)
    }

    pub(crate) fn target_ticket(
        &self,
        node: &str,
        id: &str,
        lease: &str,
        index: usize,
    ) -> Result<ScreenshotTicket> {
        let db = self.db.lock().unwrap();
        check_target_lease(&db, node, id, lease)?;
        load_target_ticket(&db, node, id, lease, index)?.ok_or_else(|| {
            StoreError::new(
                409,
                "screenshot_ticket_missing",
                "Prepare an upload ticket first",
            )
        })
    }

    pub(crate) fn complete_target_ticket(
        &self,
        node: &str,
        id: &str,
        lease: &str,
        index: usize,
        fingerprint: &str,
        result: &Value,
    ) -> Result<Value> {
        let mut db = self.db.lock().unwrap();
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        check_target_lease(&tx, node, id, lease)?;
        let ticket = load_target_ticket(&tx, node, id, lease, index)?.ok_or_else(invalid_lease)?;
        if ticket.result.is_none() {
            let settings = super::screenshot_settings::load(&tx, &self.signing_key)?;
            if ticket.config_fingerprint != fingerprint
                || oss_fingerprint(&settings.oss)? != fingerprint
            {
                return Err(ticket_config_changed());
            }
            let targets: String = tx
                .query_row(
                    "SELECT targets FROM screenshot_jobs WHERE id=?1",
                    [id],
                    |r| r.get(0),
                )
                .map_err(database_error)?;
            let targets: Vec<Value> = serde_json::from_str(&targets)
                .map_err(|_| database_error(rusqlite::Error::InvalidQuery))?;
            let mut image = result.clone();
            image["targetIndex"] = json!(index);
            image["target"] = targets.get(index).cloned().ok_or_else(invalid_lease)?;
            tx.execute("UPDATE screenshot_target_tickets SET result=?3 WHERE job_id=?1 AND target_index=?2",params![id,index as i64,image.to_string()]).map_err(database_error)?;
            let images: Vec<Value> = {
                let mut statement = tx.prepare("SELECT result FROM screenshot_target_tickets WHERE job_id=?1 AND result IS NOT NULL ORDER BY target_index").map_err(database_error)?;
                let rows = statement
                    .query_map([id], |r| r.get::<_, String>(0))
                    .map_err(database_error)?;
                let mut images = Vec::new();
                for row in rows {
                    images.push(
                        serde_json::from_str(&row.map_err(database_error)?)
                            .map_err(|_| database_error(rusqlite::Error::InvalidQuery))?,
                    );
                }
                images
            };
            let delivered = images.len() == targets.len();
            let aggregate = json!({"images":images,"notificationStatus":if settings.notification.url.is_empty() {"not_configured"} else {"protocol_pending"}});
            tx.execute("UPDATE screenshot_jobs SET result=?2,status=?3,lease=CASE WHEN ?4 THEN NULL ELSE lease END WHERE id=?1",params![id,aggregate.to_string(),if delivered {"succeeded"} else {"running"},delivered]).map_err(database_error)?;
        }
        let (status, result): (String, String) = tx
            .query_row(
                "SELECT status,result FROM screenshot_jobs WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(database_error)?;
        let result: Value = serde_json::from_str(&result)
            .map_err(|_| database_error(rusqlite::Error::InvalidQuery))?;
        tx.commit().map_err(database_error)?;
        Ok(json!({"delivered":status == "succeeded","result":result}))
    }

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
    #[cfg(test)]
    pub(crate) fn enqueue_screenshot(
        &self,
        access: &Integration,
        platform: Platform,
        identifier: &str,
        profile: &str,
        prompt: &str,
    ) -> Result<Value> {
        self.enqueue_screenshot_inner(access, platform, identifier, profile, prompt, None)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn enqueue_screenshot_v2(
        &self,
        access: &Integration,
        platform: Platform,
        identifier: &str,
        profile: &str,
        prompt: &str,
        targets: &[store_protocol::screenshot::ScreenshotTarget],
    ) -> Result<Value> {
        store_protocol::screenshot::validate_targets(targets)?;
        self.enqueue_screenshot_inner(
            access,
            platform,
            identifier,
            profile,
            prompt,
            Some(
                serde_json::to_string(targets)
                    .map_err(|_| database_error(rusqlite::Error::InvalidQuery))?,
            ),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn enqueue_screenshot_inner(
        &self,
        access: &Integration,
        platform: Platform,
        identifier: &str,
        profile: &str,
        prompt: &str,
        targets: Option<String>,
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
            "INSERT INTO screenshot_jobs(id,connector_id,integration_id,profile,prompt,status,created_at,deadline,platform,bundle_identifier,targets)
             VALUES(?1,?2,?3,?4,?5,'queued',?6,?7,?8,?9,?10)",
            params![id, node, access.id, profile, prompt, time, time + 86400,platform.to_string(),identifier,targets],
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
            "SELECT connector_id,profile,status,result,error,platform,bundle_identifier,id,targets FROM screenshot_jobs WHERE integration_id=?2 AND id=?1",
            params![id, access.id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?,
                      row.get::<_, Option<String>>(3)?, row.get::<_, Option<String>>(4)?,row.get::<_,Option<String>>(5)?,row.get::<_,Option<String>>(6)?,row.get::<_,String>(7)?,row.get::<_,Option<String>>(8)?)),
        ).optional().map_err(database_error)?;
        let (node, profile, status, result, error, platform, identifier, job_id, targets) = job
            .ok_or_else(|| {
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
            json!({"jobId":job_id,"target":targets.and_then(|v| serde_json::from_str::<Value>(&v).ok()).unwrap_or_else(|| json!([])),"connectorId":node,"platform":platform,"bundleIdentifier":identifier,"profile":profile,"status":status,"result":result,"error":error}),
        )
    }

    #[cfg(test)]
    pub(crate) fn claim_screenshot(&self, node: &str) -> Result<Option<Value>> {
        self.claim_screenshot_protocol(node, false)
    }

    pub(crate) fn claim_screenshot_protocol(
        &self,
        node: &str,
        current_protocol: bool,
    ) -> Result<Option<Value>> {
        let time = timestamp()?;
        let mut db = self.db.lock().unwrap();
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        expire(&tx, time)?;
        let claimed = if let Some((id, profile, prompt, platform, identifier)) =
            claimable_screenshot(&tx, node, time)?
        {
            let targets: Option<String> = tx
                .query_row(
                    "SELECT targets FROM screenshot_jobs WHERE id=?1",
                    [&id],
                    |row| row.get(0),
                )
                .map_err(database_error)?;
            if targets.is_some() && !current_protocol {
                return Err(StoreError::new(
                    409,
                    "screenshot_connector_upgrade_required",
                    "Upgrade the Screenshot Connector to protocol 3",
                ));
            }
            let lease = Uuid::new_v4().to_string();
            tx.execute(
                "UPDATE screenshot_jobs SET status='running', deadline=?2, lease=?3, prompt='' WHERE id=?1",
                params![id, time + 1800, lease],
            ).map_err(database_error)?;
            let profile = (!profile.is_empty()).then_some(profile);
            let prompt = match (&platform, &identifier) {
                _ if current_protocol => prompt,
                (Some(platform), Some(identifier)) => {
                    format!("应用平台：{platform}\n应用包名 / Bundle ID：{identifier}\n\n{prompt}")
                }
                _ => prompt,
            };
            Some(
                json!({"jobId":id,"target":targets.and_then(|v| serde_json::from_str::<Value>(&v).ok()),"platform":platform,"bundleIdentifier":identifier,"profile":profile,"prompt":prompt,"lease":lease}),
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
                "UPDATE screenshot_jobs SET status=?4, result=COALESCE(?5,result), error=?6, lease=NULL
             WHERE id=?1 AND connector_id=?2 AND lease=?3 AND status=?7 AND deadline>?8
             AND EXISTS(SELECT 1 FROM integrations WHERE id=screenshot_jobs.integration_id)
             AND EXISTS(SELECT 1 FROM connectors WHERE id=?2 AND enabled=1 AND mode='screenshot')",
                params![id, node, lease, status, output, error, phase, timestamp()?],
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

    #[test]
    fn expired_failure_lease_cannot_write_before_expiration_sweep() {
        let (registry, node, job, lease, _) = ticket_fixture();
        registry
            .db
            .lock()
            .unwrap()
            .execute("UPDATE screenshot_jobs SET deadline=0 WHERE id=?1", [&job])
            .unwrap();
        assert_eq!(
            registry
                .finish_screenshot(&node, &job, &lease, Err("capture_failed"), "running")
                .unwrap_err()
                .code,
            "invalid_job_lease"
        );
        let status: String = registry
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT status FROM screenshot_jobs WHERE id=?1",
                [&job],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "running");
    }

    #[test]
    fn structured_partial_delivery_survives_reopen_and_later_failure() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("screenshots.sqlite");
        let key = "persisted-delivery-fixture-signing-key-0123456789";
        let registry = Registry::open(&path, key).unwrap();
        let node = registry
            .create_with_kind(
                ConnectorInput {
                    name: "Persistent node".into(),
                },
                ConnectorKind::Screenshot,
            )
            .unwrap()
            .connector_id;
        let access = registry
            .issue_integration(
                serde_json::from_value(json!({"name":"Persistent integration"})).unwrap(),
            )
            .unwrap();
        registry
            .bind_screenshot(Platform::Ios, "com.example.persisted", &node)
            .unwrap();
        let targets =
            serde_json::from_value::<Vec<store_protocol::screenshot::ScreenshotTarget>>(json!([
                {"locale":"en-US","page":"first","device":"phone"},
                {"locale":"en-US","page":"second","device":"phone"}
            ]))
            .unwrap();
        let job = registry
            .enqueue_screenshot_v2(
                &access,
                Platform::Ios,
                "com.example.persisted",
                "",
                "Capture",
                &targets,
            )
            .unwrap();
        let id = job["jobId"].as_str().unwrap();
        let claim = registry
            .claim_screenshot_protocol(&node, true)
            .unwrap()
            .unwrap();
        let lease = claim["lease"].as_str().unwrap();
        let fp = oss_fingerprint(&registry.screenshot_settings().unwrap().oss).unwrap();
        registry
            .prepare_target_ticket(
                &node,
                id,
                lease,
                0,
                64,
                &"a".repeat(64),
                &"b".repeat(32),
                &fp,
                "screenshots/persistent-first.png",
            )
            .unwrap();
        let first = registry
            .complete_target_ticket(
                &node,
                id,
                lease,
                0,
                &fp,
                &json!({"imageUrl":"https://cdn.example/first.png"}),
            )
            .unwrap();
        assert_eq!(first["delivered"], false);
        drop(registry);
        let registry = Registry::open(&path, key).unwrap();
        let status = registry.screenshot_status(&access, id).unwrap();
        assert_eq!(status["result"], first["result"]);
        assert_eq!(status["status"], "running");
        assert!(registry
            .target_ticket(&node, id, lease, 0)
            .unwrap()
            .result
            .is_some());
        registry
            .prepare_target_ticket(
                &node,
                id,
                lease,
                1,
                64,
                &"a".repeat(64),
                &"b".repeat(32),
                &fp,
                "screenshots/persistent-second.png",
            )
            .unwrap();
        registry
            .finish_screenshot(&node, id, lease, Err("capture_failed"), "running")
            .unwrap();
        drop(registry);
        let registry = Registry::open(&path, key).unwrap();
        let failed = registry.screenshot_status(&access, id).unwrap();
        assert!(Uuid::parse_str(id).is_ok());
        assert_eq!(failed["jobId"], id);
        assert_eq!(failed["status"], "failed");
        assert_eq!(failed["error"], "capture_failed");
        assert_eq!(failed["result"], first["result"]);
        assert_eq!(failed["result"]["images"][0]["targetIndex"], 0);
        assert!(registry.target_ticket(&node, id, lease, 1).is_err());
        assert_eq!(
            registry
                .complete_target_ticket(&node, id, lease, 1, &fp, &json!({}))
                .unwrap_err()
                .code,
            "invalid_job_lease"
        );
    }

    #[test]
    fn structured_jobs_preserve_identity_targets_and_partial_results() {
        let (registry, node, old, old_lease, integration_id) = ticket_fixture();
        registry
            .finish_screenshot(&node, &old, &old_lease, Err("capture_failed"), "running")
            .unwrap();
        let integration = registry
            .issue_integration(serde_json::from_value(json!({"name":"V2 fixture"})).unwrap())
            .unwrap();
        let targets: Vec<store_protocol::screenshot::ScreenshotTarget> =
            serde_json::from_value(json!([
                {"locale":"en-US","page":"home","device":"phone"},
                {"locale":"zh-CN","page":"settings","device":"tablet","theme":"dark"}
            ]))
            .unwrap();
        let prompt = "  exact prompt\n";
        let job = registry
            .enqueue_screenshot_v2(
                &integration,
                Platform::Ios,
                "com.example.fixture",
                "",
                prompt,
                &targets,
            )
            .unwrap();
        let id = job["jobId"].as_str().unwrap();
        assert!(Uuid::parse_str(id).is_ok());
        assert_eq!(
            registry.claim_screenshot(&node).unwrap_err().code,
            "screenshot_connector_upgrade_required"
        );
        assert_eq!(
            registry.screenshot_status(&integration, id).unwrap()["status"],
            "queued"
        );
        assert!(job.get("taskId").is_none());
        let claimed = registry
            .claim_screenshot_protocol(&node, true)
            .unwrap()
            .unwrap();
        assert_eq!(claimed["prompt"], prompt);
        assert_eq!(claimed["target"], serde_json::to_value(&targets).unwrap());
        assert_eq!(claimed["jobId"], id);
        let lease = claimed["lease"].as_str().unwrap();
        let fp = oss_fingerprint(&registry.screenshot_settings().unwrap().oss).unwrap();
        for index in 0..2 {
            registry
                .prepare_target_ticket(
                    &node,
                    id,
                    lease,
                    index,
                    64,
                    &"a".repeat(64),
                    &"b".repeat(32),
                    &fp,
                    &format!("screenshots/{index}.png"),
                )
                .unwrap();
            if index == 0 {
                assert_eq!(
                    registry
                        .prepare_target_ticket(
                            &node,
                            id,
                            lease,
                            1,
                            64,
                            &"a".repeat(64),
                            &"b".repeat(32),
                            &fp,
                            "screenshots/other.png"
                        )
                        .unwrap_err()
                        .code,
                    "screenshot_target_pending"
                );
            }
            let result = registry
                .complete_target_ticket(
                    &node,
                    id,
                    lease,
                    index,
                    &fp,
                    &json!({"imageUrl":"https://cdn.example/fixture.png"}),
                )
                .unwrap();
            assert_eq!(result["delivered"], index == 1);
            assert_eq!(
                result["result"]["images"].as_array().unwrap().len(),
                index + 1
            );
            let repeated = registry
                .complete_target_ticket(&node, id, lease, index, &fp, &json!({}))
                .unwrap();
            assert_eq!(repeated, result);
        }
        assert_eq!(
            registry.screenshot_status(&integration, id).unwrap()["status"],
            "succeeded"
        );
        registry
            .db
            .lock()
            .unwrap()
            .execute("UPDATE screenshot_jobs SET deadline=0 WHERE id=?1", [id])
            .unwrap();
        assert_eq!(
            registry
                .target_ticket(&node, id, lease, 0)
                .unwrap_err()
                .code,
            "invalid_job_lease"
        );
        assert!(!integration_id.is_empty());
    }

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
