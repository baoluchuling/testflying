//! Store-only persisted authorization snapshots. Reconnect never enlarges a batch.
use super::*;
use crate::store_upgrades::{Batch, UpgradeNode, UpgradeStatus};
use store_update::{SignedCommand, UpgradeRequest};

const RETENTION: u64 = 7 * 86400;
const MAX_BATCHES: i64 = 8;
pub(super) fn create_schema(db: &Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS store_upgrade_batches (
        id TEXT PRIMARY KEY, created INTEGER NOT NULL, expires INTEGER NOT NULL,
        version TEXT NOT NULL, snapshot TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS store_upgrade_commands (
        request TEXT PRIMARY KEY, batch TEXT NOT NULL REFERENCES store_upgrade_batches(id) ON DELETE CASCADE,
        node TEXT NOT NULL, command TEXT NOT NULL, state TEXT NOT NULL,
        confirmed INTEGER NOT NULL DEFAULT 0, ready INTEGER NOT NULL DEFAULT 0, UNIQUE(batch,node));
        CREATE TABLE IF NOT EXISTS store_upgrade_packages (
        batch TEXT NOT NULL REFERENCES store_upgrade_batches(id) ON DELETE CASCADE,
        target TEXT NOT NULL, data BLOB NOT NULL, PRIMARY KEY(batch,target));")
        .map_err(database_error)
}
fn missing() -> StoreError {
    StoreError::new(404, "store_upgrade_not_found", "Store upgrade not found")
}
fn prune(db: &Connection) -> Result<()> {
    let now = store_update::now();
    db.execute(
        "DELETE FROM store_upgrade_batches WHERE created<?1",
        [now.saturating_sub(RETENTION)],
    )
    .map_err(database_error)?;
    db.execute("UPDATE store_upgrade_commands SET state='expired' WHERE state NOT IN ('succeeded','rolled_back','failed','skipped','expired') AND batch IN (SELECT id FROM store_upgrade_batches WHERE expires<=?1)",[now]).map_err(database_error)?;
    Ok(())
}
fn read(db: &Connection, id: &str) -> Result<Batch> {
    let text: String = db
        .query_row(
            "SELECT snapshot FROM store_upgrade_batches WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(missing)?;
    let mut batch: Batch = serde_json::from_str(&text).map_err(|_| missing())?;
    let mut q = db
        .prepare("SELECT node,state FROM store_upgrade_commands WHERE batch=?1")
        .map_err(database_error)?;
    for row in q
        .query_map([id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(database_error)?
    {
        let (id, state) = row.map_err(database_error)?;
        if let Some(node) = batch.nodes.iter_mut().find(|n| n.connector_id == id) {
            node.state = state;
        }
    }
    batch.state = if batch.nodes.iter().any(|n| !terminal(&n.state)) {
        "active"
    } else {
        "completed"
    }
    .into();
    Ok(batch)
}
pub(crate) fn terminal(state: &str) -> bool {
    matches!(
        state,
        "succeeded" | "rolled_back" | "failed" | "skipped" | "expired"
    )
}
fn validate_status(node: &str, status: &UpgradeStatus) -> Result<()> {
    if status.node_id != node
        || !store_update::valid_id(node)
        || !store_update::valid_id(&status.request_id)
        || !status.valid()
    {
        return Err(StoreError::new(
            422,
            "invalid_upgrade_status",
            "Invalid Store upgrade status",
        ));
    }
    Ok(())
}
fn terminal_receipt(db: &Connection, node: &str, status: &UpgradeStatus) -> Result<bool> {
    if !matches!(
        status.state.as_str(),
        "succeeded" | "rolled_back" | "failed"
    ) {
        return Ok(false);
    }
    let previous: Option<(String, String)> = db
        .query_row(
            "SELECT node,state FROM store_upgrade_commands WHERE request=?1",
            [&status.request_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(database_error)?;
    match previous {
        Some((owner, _)) if owner != node => Err(missing()),
        Some((_, state)) => Ok(state == "expired" || state == status.state),
        None => Ok(true),
    }
}
impl Registry {
    pub(crate) fn update_public_key(&self) -> Result<String> {
        store_update::public_key(&self.signing_key).map_err(|_| {
            StoreError::new(
                500,
                "upgrade_signing_unavailable",
                "Upgrade signing unavailable",
            )
        })
    }
    pub(crate) fn latest_upgrade_batch(&self) -> Result<Option<Batch>> {
        let db = self.db.lock().unwrap();
        prune(&db)?;
        let id: Option<String> = db
            .query_row(
                "SELECT id FROM store_upgrade_batches ORDER BY created DESC,rowid DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(database_error)?;
        id.map(|id| read(&db, &id)).transpose()
    }
    pub(crate) fn upgrade_batch(&self, id: &str) -> Result<Batch> {
        let db = self.db.lock().unwrap();
        prune(&db)?;
        read(&db, id)
    }
    pub(crate) fn active_upgrade_batch(&self) -> Result<Option<Batch>> {
        let db = self.db.lock().unwrap();
        prune(&db)?;
        let id:Option<String>=db.query_row("SELECT b.id FROM store_upgrade_batches b WHERE EXISTS (SELECT 1 FROM store_upgrade_commands c WHERE c.batch=b.id AND c.state NOT IN ('succeeded','rolled_back','failed','skipped','expired')) ORDER BY b.created LIMIT 1",[],|r|r.get(0)).optional().map_err(database_error)?;
        id.map(|id| read(&db, &id)).transpose()
    }
    /// Immediate transaction serializes duplicate clicks, even across API processes.
    pub(crate) fn create_upgrade_batch(
        &self,
        version: &str,
        mut nodes: Vec<UpgradeNode>,
        packages: Vec<(String, Vec<u8>)>,
    ) -> Result<Batch> {
        use sha2::Digest;
        if nodes.len() > 256 {
            return Err(StoreError::new(
                409,
                "upgrade_node_limit",
                "At most 256 online Store Connectors per batch",
            ));
        }
        if packages.len() > 2
            || packages.iter().any(|(target, bytes)| {
                !matches!(target.as_str(), "windows-amd64" | "darwin-arm64")
                    || bytes.is_empty()
                    || bytes.len() as u64 > store_update::MAX_PACKAGE
            })
        {
            return Err(StoreError::new(
                503,
                "invalid_upgrade_package",
                "Invalid Store upgrade package",
            ));
        }
        let mut db = self.db.lock().unwrap();
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        prune(&tx)?;
        let active:Option<String>=tx.query_row("SELECT b.id FROM store_upgrade_batches b WHERE EXISTS (SELECT 1 FROM store_upgrade_commands c WHERE c.batch=b.id AND c.state NOT IN ('succeeded','rolled_back','failed','skipped','expired')) LIMIT 1",[],|r|r.get(0)).optional().map_err(database_error)?;
        if let Some(id) = active {
            return read(&tx, &id);
        }
        let retained: i64 = tx
            .query_row("SELECT COUNT(*) FROM store_upgrade_batches", [], |r| {
                r.get(0)
            })
            .map_err(database_error)?;
        if retained >= MAX_BATCHES {
            return Err(StoreError::new(
                409,
                "upgrade_history_full",
                "Store upgrade history is full; retained results expire after seven days",
            ));
        }
        let now = store_update::now();
        let id = Uuid::new_v4().to_string();
        for node in &mut nodes {
            if node.state == "skipped" {
                continue;
            }
            let Some((target, bytes)) = packages
                .iter()
                .find(|(target, _)| Some(target.as_str()) == node.target.as_deref())
            else {
                node.state = "skipped".into();
                node.reason = Some("release_unavailable".into());
                continue;
            };
            let request_id = Uuid::new_v4().to_string();
            node.request_id = Some(request_id.clone());
            let request = UpgradeRequest {
                protocol: 1,
                request_id,
                node_id: node.connector_id.clone(),
                version: version.into(),
                target: target.clone(),
                sha256: hex::encode(sha2::Sha256::digest(bytes)),
                size_bytes: bytes.len() as u64,
                expires_at: now + 86400,
                bootstrap_version: 1,
            };
            node.command = Some(store_update::sign(request, &self.signing_key).map_err(|_| {
                StoreError::new(
                    500,
                    "upgrade_signing_unavailable",
                    "Upgrade signing unavailable",
                )
            })?);
        }
        let batch = Batch {
            batch_id: id.clone(),
            version: version.into(),
            created_at: now,
            expires_at: now + 86400,
            state: if nodes.iter().any(|n| !terminal(&n.state)) {
                "active"
            } else {
                "completed"
            }
            .into(),
            nodes,
        };
        tx.execute(
            "INSERT INTO store_upgrade_batches VALUES(?1,?2,?3,?4,?5)",
            params![
                id,
                now,
                now + 86400,
                version,
                serde_json::to_string(&batch).expect("upgrade batch")
            ],
        )
        .map_err(database_error)?;
        for node in &batch.nodes {
            if let Some(command) = &node.command {
                tx.execute("INSERT INTO store_upgrade_commands(request,batch,node,command,state) VALUES(?1,?2,?3,?4,?5)",params![command.request.request_id,id,node.connector_id,serde_json::to_string(command).expect("upgrade command"),node.state]).map_err(database_error)?;
            }
        }
        for (target, bytes) in packages {
            if batch
                .nodes
                .iter()
                .any(|n| n.command.is_some() && n.target.as_deref() == Some(target.as_str()))
            {
                tx.execute(
                    "INSERT INTO store_upgrade_packages VALUES(?1,?2,?3)",
                    params![id, target, bytes],
                )
                .map_err(database_error)?;
            }
        }
        tx.commit().map_err(database_error)?;
        drop(db);
        self.logs.record(crate::logs::Event {
            name: "store_upgrade.created",
            request_id: Some(id),
            details: serde_json::json!({"version":version,"nodeCount":batch.nodes.len()}),
            ..Default::default()
        });
        Ok(batch)
    }
    pub(crate) fn pending_upgrade(&self, node: &str) -> Result<Option<SignedCommand>> {
        let db = self.db.lock().unwrap();
        prune(&db)?;
        let text:Option<String>=db.query_row("SELECT command FROM store_upgrade_commands WHERE node=?1 AND state NOT IN ('succeeded','rolled_back','failed','skipped','expired') LIMIT 1",[node],|r|r.get(0)).optional().map_err(database_error)?;
        text.map(|s| serde_json::from_str(&s).map_err(|_| missing()))
            .transpose()
    }
    pub(crate) fn upgrade_command(&self, node: &str, request: &str) -> Result<SignedCommand> {
        let db = self.db.lock().unwrap();
        prune(&db)?;
        command(&db, node, request)
    }
    /// Retained requests remain authoritative; missing history permits only a
    /// read-only connection observation, never package or installation access.
    pub(crate) fn upgrade_confirmation_command(
        &self,
        node: &str,
        request: &str,
    ) -> Result<Option<SignedCommand>> {
        if !store_update::valid_id(node) || !store_update::valid_id(request) {
            return Err(StoreError::new(
                422,
                "invalid_upgrade_request",
                "Invalid upgrade request identity",
            ));
        }
        let db = self.db.lock().unwrap();
        prune(&db)?;
        let row: Option<(String, String)> = db
            .query_row(
                "SELECT node,command FROM store_upgrade_commands WHERE request=?1",
                [request],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(database_error)?;
        match row {
            Some((owner, _)) if owner != node => Err(missing()),
            Some((_, text)) => serde_json::from_str(&text).map(Some).map_err(|_| missing()),
            None => Ok(None),
        }
    }
    pub(crate) fn upgrade_package(&self, node: &str, request: &str) -> Result<Vec<u8>> {
        let db = self.db.lock().unwrap();
        prune(&db)?;
        let command = command(&db, node, request)?;
        if command.request.expires_at <= store_update::now() {
            return Err(missing());
        }
        db.query_row("SELECT p.data FROM store_upgrade_packages p JOIN store_upgrade_commands c ON c.batch=p.batch WHERE c.request=?1 AND c.node=?2 AND p.target=?3",params![request,node,command.request.target],|r|r.get(0)).optional().map_err(database_error)?.ok_or_else(missing)
    }
    pub(crate) fn confirm_upgrade(&self, node: &str, request: &str, confirmed: bool) -> Result<()> {
        if confirmed {
            self.db
                .lock()
                .unwrap()
                .execute(
                    "UPDATE store_upgrade_commands SET confirmed=1 WHERE node=?1 AND request=?2",
                    params![node, request],
                )
                .map_err(database_error)?;
        }
        Ok(())
    }
    pub(crate) fn authorize_upgrade_ready(&self, node: &str, request: &str) -> Result<bool> {
        let db = self.db.lock().unwrap();
        prune(&db)?;
        let count=db.execute("UPDATE store_upgrade_commands SET ready=1 WHERE node=?1 AND request=?2 AND state NOT IN ('succeeded','rolled_back','failed','expired','skipped')",params![node,request]).map_err(database_error)?;
        Ok(count == 1)
    }
    /// Release a terminal outbox entry already received or past expiration/retention.
    /// Receipt acknowledgement never restores obsolete history or confirms active work.
    pub(crate) fn upgrade_status_acknowledged(
        &self,
        node: &str,
        status: &UpgradeStatus,
    ) -> Result<bool> {
        validate_status(node, status)?;
        let db = self.db.lock().unwrap();
        prune(&db)?;
        terminal_receipt(&db, node, status)
    }
    pub(crate) fn upgrade_status(&self, node: &str, status: &UpgradeStatus) -> Result<()> {
        validate_status(node, status)?;
        let mut db = self.db.lock().unwrap();
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        prune(&tx)?;
        if terminal_receipt(&tx, node, status)? {
            // Commit any expiration/retention maintenance, but neither resurrect the
            // request nor replace the retained `expired` result with a late report.
            tx.commit().map_err(database_error)?;
            return Ok(());
        }
        let _: SignedCommand = command(&tx, node, &status.request_id)?;
        let (batch_id,previous,confirmed,ready):(String,String,bool,bool)=tx.query_row("SELECT batch,state,confirmed,ready FROM store_upgrade_commands WHERE request=?1 AND node=?2",params![status.request_id,node],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(database_error)?;
        if terminal(&previous) {
            if previous == status.state {
                return Ok(());
            }
            return Err(StoreError::new(
                409,
                "upgrade_terminal",
                "Upgrade already finished",
            ));
        }
        if status.state == "succeeded" && !confirmed {
            return Err(StoreError::new(
                409,
                "upgrade_unconfirmed",
                "Target connection must be confirmed first",
            ));
        }
        if matches!(
            status.state.as_str(),
            "installing" | "awaiting_confirmation" | "cleanup_pending" | "succeeded"
        ) && !ready
        {
            return Err(StoreError::new(
                409,
                "upgrade_not_ready",
                "Center drain acknowledgement required",
            ));
        }
        if !transition(&previous, &status.state) {
            return Ok(());
        }
        // Download and local drain can overlap. Preserve the more advanced visible phase.
        let state = if previous == "draining" && status.state == "downloading" {
            previous.as_str()
        } else {
            status.state.as_str()
        };
        tx.execute(
            "UPDATE store_upgrade_commands SET state=?1 WHERE request=?2 AND node=?3",
            params![state, status.request_id, node],
        )
        .map_err(database_error)?;
        let mut batch = read(&tx, &batch_id)?;
        if let Some(n) = batch.nodes.iter_mut().find(|n| n.connector_id == node) {
            n.error_code = status.error_code.clone();
            n.updated_at = store_update::now();
        }
        tx.execute(
            "UPDATE store_upgrade_batches SET snapshot=?1 WHERE id=?2",
            params![
                serde_json::to_string(&batch).expect("upgrade batch"),
                batch_id
            ],
        )
        .map_err(database_error)?;
        tx.commit().map_err(database_error)?;
        drop(db);
        self.logs.record(crate::logs::Event {name:"store_upgrade.status",request_id:Some(status.request_id.clone()),connector_id:Some(node.into()),details:serde_json::json!({"batchId":batch_id,"version":batch.version,"state":status.state,"errorCode":status.error_code}),..Default::default()});
        Ok(())
    }
    pub(crate) fn upgrade_jobs_ready(&self, node: &str, ids: &[String]) -> Result<bool> {
        if ids.len() > 1024 || ids.iter().any(|id| !store_update::valid_id(id)) {
            return Err(StoreError::new(
                422,
                "invalid_job_ids",
                "Invalid completed job identifiers",
            ));
        }
        let db = self.db.lock().unwrap();
        // Local ready also proves the worker queue, capacity and assets are drained.
        // A persisted unknown outcome is evidence already received, never permission
        // to replay a write, and must not keep an idle node blocked indefinitely.
        let pending:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM store_image_jobs WHERE connector=?1 AND status IN ('dispatching','queued','running'))",[node],|r|r.get(0)).map_err(database_error)?;
        if pending {
            return Ok(false);
        }
        for id in ids {
            let done:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM store_image_jobs WHERE id=?1 AND connector=?2 AND status IN ('succeeded','failed','unknown'))",params![id,node],|r|r.get(0)).map_err(database_error)?;
            if !done {
                return Ok(false);
            }
        }
        Ok(true)
    }
}
fn command(db: &Connection, node: &str, request: &str) -> Result<SignedCommand> {
    let text: String = db
        .query_row(
            "SELECT command FROM store_upgrade_commands WHERE node=?1 AND request=?2",
            params![node, request],
            |r| r.get(0),
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(missing)?;
    serde_json::from_str(&text).map_err(|_| missing())
}
fn transition(from: &str, to: &str) -> bool {
    if matches!(to, "failed" | "rolled_back" | "cleanup_pending")
        || (from == "draining" && to == "downloading")
    {
        return true;
    }
    fn rank(s: &str) -> u8 {
        match s {
            "notified" => 1,
            "downloading" => 2,
            "draining" => 3,
            "installing" => 4,
            "awaiting_confirmation" | "cleanup_pending" => 5,
            "succeeded" => 6,
            _ => 0,
        }
    }
    rank(to) > 0 && rank(to) >= rank(from)
}

#[cfg(test)]
mod tests {
    use super::*;
    const KEY: &str = "upgrade-registry-fixture-signing-key-0123456789";
    fn node(id: &str) -> UpgradeNode {
        UpgradeNode {
            connector_id: id.into(),
            name: "fixture".into(),
            request_id: None,
            version: Some("1.0.0".into()),
            target: Some("darwin-arm64".into()),
            state: "notified".into(),
            reason: None,
            error_code: None,
            updated_at: store_update::now(),
            command: None,
        }
    }
    #[test]
    fn persisted_snapshot_idempotence_identity_expiration_and_retention() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("registry.sqlite");
        let registry = Registry::open(&path, KEY).unwrap();
        let id = Uuid::new_v4().to_string();
        let other = Uuid::new_v4().to_string();
        let batch = registry
            .create_upgrade_batch(
                "1.0.1",
                vec![node(&id)],
                vec![("darwin-arm64".into(), b"fixed-package".to_vec())],
            )
            .unwrap();
        let command = registry.pending_upgrade(&id).unwrap().unwrap();
        assert!(registry
            .upgrade_command(&other, &command.request.request_id)
            .is_err());
        let duplicate = registry
            .create_upgrade_batch(
                "1.0.2",
                vec![node(&other)],
                vec![("darwin-arm64".into(), b"other-package".to_vec())],
            )
            .unwrap();
        assert_eq!(duplicate.batch_id, batch.batch_id);
        assert!(registry.pending_upgrade(&other).unwrap().is_none());
        drop(registry);
        let registry = Registry::open(&path, KEY).unwrap();
        assert_eq!(registry.pending_upgrade(&id).unwrap().unwrap(), command);
        assert_eq!(
            registry
                .upgrade_package(&id, &command.request.request_id)
                .unwrap(),
            b"fixed-package"
        );
        let mut report = UpgradeStatus {
            node_id: other.clone(),
            request_id: command.request.request_id.clone(),
            state: "draining".into(),
            error_code: None,
        };
        assert!(registry.upgrade_status(&id, &report).is_err());
        report.node_id = id.clone();
        registry.upgrade_status(&id, &report).unwrap();
        report.state = "notified".into();
        registry.upgrade_status(&id, &report).unwrap();
        assert_eq!(
            registry.upgrade_batch(&batch.batch_id).unwrap().nodes[0].state,
            "draining"
        );
        report.state = "succeeded".into();
        assert!(registry.upgrade_status(&id, &report).is_err());
        registry
            .confirm_upgrade(&id, &report.request_id, true)
            .unwrap();
        assert!(registry.upgrade_status(&id, &report).is_err());
        registry
            .authorize_upgrade_ready(&id, &report.request_id)
            .unwrap();
        registry.upgrade_status(&id, &report).unwrap();
        report.state = "failed".into();
        assert!(registry.upgrade_status(&id, &report).is_err());
        assert!(registry.pending_upgrade(&id).unwrap().is_none());
        let next = registry
            .create_upgrade_batch(
                "1.0.2",
                vec![node(&other)],
                vec![("darwin-arm64".into(), b"next-package".to_vec())],
            )
            .unwrap();
        registry
            .db
            .lock()
            .unwrap()
            .execute(
                "UPDATE store_upgrade_batches SET expires=0 WHERE id=?1",
                [&next.batch_id],
            )
            .unwrap();
        assert!(registry.pending_upgrade(&other).unwrap().is_none());
        assert_eq!(
            registry.upgrade_batch(&next.batch_id).unwrap().nodes[0].state,
            "expired"
        );
        // Capacity must reject a new click rather than erase seven-day evidence.
        for _ in 2..MAX_BATCHES {
            registry
                .create_upgrade_batch("1.0.3", vec![], vec![])
                .unwrap();
        }
        let count: i64 = registry
            .db
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM store_upgrade_batches", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(count, MAX_BATCHES);
        let error = registry
            .create_upgrade_batch("1.0.3", vec![], vec![])
            .err()
            .unwrap();
        assert_eq!(error.status, 409);
        assert_eq!(error.code, "upgrade_history_full");
        assert_eq!(
            registry.upgrade_batch(&batch.batch_id).unwrap().nodes[0].state,
            "succeeded"
        );
        assert_eq!(
            registry
                .upgrade_package(&id, &command.request.request_id)
                .unwrap(),
            b"fixed-package"
        );
        // Once one result ages past seven days a new batch may occupy its slot.
        registry
            .db
            .lock()
            .unwrap()
            .execute(
                "UPDATE store_upgrade_batches SET created=0 WHERE id=?1",
                [&next.batch_id],
            )
            .unwrap();
        registry
            .create_upgrade_batch("1.0.3", vec![], vec![])
            .unwrap();
        assert!(registry.upgrade_batch(&next.batch_id).is_err());
        assert!(registry.upgrade_batch(&batch.batch_id).is_ok());
        registry
            .db
            .lock()
            .unwrap()
            .execute("UPDATE store_upgrade_batches SET created=0", [])
            .unwrap();
        assert!(registry.latest_upgrade_batch().unwrap().is_none());
        let packages: i64 = registry
            .db
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM store_upgrade_packages", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(packages, 0);
    }
    #[test]
    fn drain_requires_pending_jobs_finished_and_accepts_persisted_unknown_without_replaying_it() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("jobs.sqlite");
        let registry = Registry::open(&path, KEY).unwrap();
        let id = Uuid::new_v4().to_string();
        let job = Uuid::new_v4().to_string();
        let foreign = Uuid::new_v4().to_string();
        assert!(registry.upgrade_jobs_ready(&id, &[]).unwrap());
        assert!(!registry
            .upgrade_jobs_ready(&id, std::slice::from_ref(&job))
            .unwrap());
        registry.db.lock().unwrap().execute("INSERT INTO store_image_jobs(id,platform,app,connector,created,updated,status,snapshot) VALUES(?1,'ios','fixture',?2,1,1,'running','{}')",params![job,id]).unwrap();
        for status in ["dispatching", "queued", "running"] {
            registry
                .db
                .lock()
                .unwrap()
                .execute("UPDATE store_image_jobs SET status=?1", [status])
                .unwrap();
            assert!(!registry.upgrade_jobs_ready(&id, &[]).unwrap());
            assert!(!registry
                .upgrade_jobs_ready(&id, std::slice::from_ref(&job))
                .unwrap());
        }
        registry.db.lock().unwrap().execute("UPDATE store_image_jobs SET status='unknown',snapshot='{\"status\":\"unknown\",\"outcomeUnknown\":true}'",[]).unwrap();
        drop(registry);
        let registry = Registry::open(&path, KEY).unwrap();
        assert!(registry.upgrade_jobs_ready(&id, &[]).unwrap());
        assert!(registry
            .upgrade_jobs_ready(&id, std::slice::from_ref(&job))
            .unwrap());
        assert!(!registry
            .upgrade_jobs_ready(&foreign, std::slice::from_ref(&job))
            .unwrap());
        let (status, snapshot): (String, String) = registry
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT status,snapshot FROM store_image_jobs WHERE id=?1",
                [&job],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "unknown");
        assert_eq!(snapshot, "{\"status\":\"unknown\",\"outcomeUnknown\":true}");
        // An old unknown result must not hide a different currently queued job.
        let queued = Uuid::new_v4().to_string();
        registry.db.lock().unwrap().execute("INSERT INTO store_image_jobs(id,platform,app,connector,created,updated,status,snapshot) VALUES(?1,'ios','fixture',?2,1,1,'queued','{}')",params![queued,id]).unwrap();
        assert!(!registry
            .upgrade_jobs_ready(&id, std::slice::from_ref(&job))
            .unwrap());
        assert!(registry
            .upgrade_jobs_ready(&id, &["not-an-id".into()])
            .is_err());
    }
    #[test]
    fn cleanup_pending_requires_ready_and_remains_nonterminal_until_confirmed_success() {
        let registry = Registry::open(":memory:", KEY).unwrap();
        let id = Uuid::new_v4().to_string();
        let batch = registry
            .create_upgrade_batch(
                "1.0.1",
                vec![node(&id)],
                vec![("darwin-arm64".into(), b"package".to_vec())],
            )
            .unwrap();
        let command = registry.pending_upgrade(&id).unwrap().unwrap();
        let mut report = UpgradeStatus {
            node_id: id.clone(),
            request_id: command.request.request_id.clone(),
            state: "cleanup_pending".into(),
            error_code: None,
        };
        let error = registry.upgrade_status(&id, &report).err().unwrap();
        assert_eq!(error.code, "upgrade_not_ready");
        registry
            .authorize_upgrade_ready(&id, &report.request_id)
            .unwrap();
        registry.upgrade_status(&id, &report).unwrap();
        assert_eq!(
            registry.upgrade_batch(&batch.batch_id).unwrap().state,
            "active"
        );
        assert!(registry.pending_upgrade(&id).unwrap().is_some());
        registry
            .confirm_upgrade(&id, &report.request_id, true)
            .unwrap();
        report.state = "succeeded".into();
        registry.upgrade_status(&id, &report).unwrap();
        assert_eq!(
            registry.upgrade_batch(&batch.batch_id).unwrap().state,
            "completed"
        );
    }
    #[test]
    fn expired_and_pruned_terminal_receipts_release_outboxes_without_restoring_history() {
        let registry = Registry::open(":memory:", KEY).unwrap();
        let id = Uuid::new_v4().to_string();
        let batch = registry
            .create_upgrade_batch(
                "1.0.1",
                vec![node(&id)],
                vec![("darwin-arm64".into(), b"package".to_vec())],
            )
            .unwrap();
        let command = registry.pending_upgrade(&id).unwrap().unwrap();
        let mut report = UpgradeStatus {
            node_id: id.clone(),
            request_id: command.request.request_id.clone(),
            state: "failed".into(),
            error_code: None,
        };
        registry
            .db
            .lock()
            .unwrap()
            .execute(
                "UPDATE store_upgrade_batches SET expires=0 WHERE id=?1",
                [&batch.batch_id],
            )
            .unwrap();
        for phase in ["failed", "rolled_back", "succeeded"] {
            report.state = phase.into();
            registry.upgrade_status(&id, &report).unwrap();
            assert!(registry.upgrade_status_acknowledged(&id, &report).unwrap());
            assert_eq!(
                registry.upgrade_batch(&batch.batch_id).unwrap().nodes[0].state,
                "expired"
            );
            assert!(registry.pending_upgrade(&id).unwrap().is_none());
        }
        let foreign = Uuid::new_v4().to_string();
        report.node_id = foreign.clone();
        assert_eq!(
            registry
                .upgrade_status(&foreign, &report)
                .unwrap_err()
                .status,
            404
        );
        report.node_id = id.clone();
        // Retention removes the old command, and its late receipt remains harmless.
        registry
            .db
            .lock()
            .unwrap()
            .execute(
                "UPDATE store_upgrade_batches SET created=0 WHERE id=?1",
                [&batch.batch_id],
            )
            .unwrap();
        registry.upgrade_status(&id, &report).unwrap();
        assert!(registry.upgrade_batch(&batch.batch_id).is_err());
        report.request_id = Uuid::new_v4().to_string();
        for phase in ["failed", "rolled_back", "succeeded"] {
            report.state = phase.into();
            registry.upgrade_status(&id, &report).unwrap();
        }
        report.state = "draining".into();
        assert_eq!(
            registry.upgrade_status(&id, &report).unwrap_err().status,
            404
        );
        report.state = "failed".into();
        report.request_id = "invalid".into();
        assert_eq!(
            registry.upgrade_status(&id, &report).unwrap_err().status,
            422
        );
        let counts: (i64,i64,i64) = registry.db.lock().unwrap().query_row(
            "SELECT (SELECT COUNT(*) FROM store_upgrade_batches),(SELECT COUNT(*) FROM store_upgrade_commands),(SELECT COUNT(*) FROM store_upgrade_packages)",
            [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
        ).unwrap();
        assert_eq!(counts, (0, 0, 0));
    }
}
