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
            ON screenshot_jobs(connector_id,status,created_at,id);",
    )
    .map_err(database_error)
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

impl Registry {
    pub(crate) fn enqueue_screenshot(
        &self,
        access: &Integration,
        node: &str,
        profile: &str,
        prompt: &str,
    ) -> Result<Value> {
        let time = timestamp()?;
        let mut db = self.db.lock().unwrap();
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        expire(&tx, time)?;
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
                [node],
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
            "INSERT INTO screenshot_jobs(id,connector_id,integration_id,profile,prompt,status,created_at,deadline)
             VALUES(?1,?2,?3,?4,?5,'queued',?6,?7)",
            params![id, node, access.id, profile, prompt, time, time + 86400],
        ).map_err(database_error)?;
        tx.commit().map_err(database_error)?;
        Ok(json!({"jobId":id,"status":"queued"}))
    }

    pub(crate) fn screenshot_status(&self, access: &Integration, id: &str) -> Result<Value> {
        let db = self.db.lock().unwrap();
        expire(&db, timestamp()?)?;
        let job = db.query_row(
            "SELECT connector_id,profile,status,result,error FROM screenshot_jobs WHERE id=?1 AND integration_id=?2",
            params![id, access.id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?,
                      row.get::<_, Option<String>>(3)?, row.get::<_, Option<String>>(4)?)),
        ).optional().map_err(database_error)?;
        let (node, profile, status, result, error) = job.ok_or_else(|| {
            StoreError::new(404, "screenshot_job_not_found", "Screenshot job not found")
        })?;
        let result = result
            .map(|body| serde_json::from_str::<Value>(&body))
            .transpose()
            .map_err(|_| {
                StoreError::new(500, "registry_error", "Invalid screenshot delivery record")
            })?;
        Ok(
            json!({"jobId":id,"connectorId":node,"profile":profile,"status":status,"result":result,"error":error}),
        )
    }

    pub(crate) fn claim_screenshot(&self, node: &str) -> Result<Option<Value>> {
        let time = timestamp()?;
        let mut db = self.db.lock().unwrap();
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        expire(&tx, time)?;
        let job = tx.query_row(
            "SELECT id,profile,prompt FROM screenshot_jobs j WHERE connector_id=?1 AND status='queued'
             AND EXISTS(SELECT 1 FROM integrations WHERE id=j.integration_id)
             AND NOT EXISTS(SELECT 1 FROM screenshot_jobs WHERE connector_id=?1 AND status IN ('running','uploading'))
             ORDER BY created_at,rowid LIMIT 1",
            [node],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)),
        ).optional().map_err(database_error)?;
        let claimed = if let Some((id, profile, prompt)) = job {
            let lease = Uuid::new_v4().to_string();
            tx.execute(
                "UPDATE screenshot_jobs SET status='running', deadline=?2, lease=?3, prompt='' WHERE id=?1",
                params![id, time + 1800, lease],
            ).map_err(database_error)?;
            Some(json!({"jobId":id,"profile":profile,"prompt":prompt,"lease":lease}))
        } else {
            None
        };
        tx.commit().map_err(database_error)?;
        Ok(claimed)
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
