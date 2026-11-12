use super::*;
use serde_json::Value;

pub(crate) fn now() -> i64 {
    chrono::Utc::now().timestamp()
}
pub(super) fn create_schema(db: &Connection) -> Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS store_image_jobs (
        id TEXT PRIMARY KEY, owner TEXT, platform TEXT NOT NULL, app TEXT NOT NULL,
        connector TEXT NOT NULL, created INTEGER NOT NULL, updated INTEGER NOT NULL,
        status TEXT NOT NULL, snapshot TEXT NOT NULL, reconcile INTEGER NOT NULL DEFAULT 1);
        CREATE INDEX IF NOT EXISTS store_image_jobs_pending ON store_image_jobs(status,created);",
    )
    .map_err(database_error)
}

#[derive(Clone)]
pub(crate) struct StoreJob {
    pub id: String,
    pub owner: Option<String>,
    pub platform: Platform,
    pub app: String,
    pub connector: String,
    pub created: i64,
    pub snapshot: Value,
}

impl Registry {
    pub(crate) fn insert_store_job(&self, job: &StoreJob) -> Result<()> {
        self.db.lock().unwrap().execute("INSERT INTO store_image_jobs(id,owner,platform,app,connector,created,updated,status,snapshot) VALUES(?1,?2,?3,?4,?5,?6,?6,'dispatching',?7)", params![job.id,job.owner,job.platform.to_string(),job.app,job.connector,job.created,job.snapshot.to_string()]).map_err(database_error)?;
        Ok(())
    }
    pub(crate) fn update_store_job(&self, id: &str, snapshot: &Value) -> Result<bool> {
        let changed = self.db.lock().unwrap().execute("UPDATE store_image_jobs SET snapshot=?1,status=?2,updated=?3 WHERE id=?4 AND reconcile=1 AND created>?5 AND status IN ('dispatching','queued','running','unknown')", params![snapshot.to_string(),snapshot["status"].as_str().unwrap_or("unknown"),now(),id,now()-86400]).map_err(database_error)?;
        Ok(changed > 0)
    }
    pub(crate) fn delete_store_job(&self, id: &str) -> Result<()> {
        self.db
            .lock()
            .unwrap()
            .execute("DELETE FROM store_image_jobs WHERE id=?1", [id])
            .map_err(database_error)?;
        Ok(())
    }
    pub(crate) fn stop_store_job(&self, id: &str) -> Result<()> {
        self.db
            .lock()
            .unwrap()
            .execute("UPDATE store_image_jobs SET reconcile=0 WHERE id=?1", [id])
            .map_err(database_error)?;
        Ok(())
    }
    pub(crate) fn store_job(
        &self,
        id: &str,
        owner: Option<&str>,
        platform: Platform,
        app: &str,
    ) -> Result<Value> {
        self.db.lock().unwrap().query_row("SELECT snapshot FROM store_image_jobs WHERE id=?1 AND owner IS ?2 AND platform=?3 AND app=?4 AND updated>=?5", params![id,owner,platform.to_string(),app,now()-7*86400], |r| r.get::<_,String>(0)).optional().map_err(database_error)?.and_then(|s|serde_json::from_str(&s).ok()).ok_or_else(||StoreError::new(404,"store_image_job_not_found","Image sync job not found"))
    }
    pub(crate) fn pending_store_jobs(&self) -> Result<Vec<StoreJob>> {
        let db = self.db.lock().unwrap();
        // Retention starts at the uncertainty deadline, even after a long offline period.
        db.execute("UPDATE store_image_jobs SET status='unknown',updated=created+86400,reconcile=0,snapshot=json_object('jobId',id,'status','unknown','result',NULL,'httpStatus',NULL,'errorCode',NULL,'outcomeUnknown',json('true')) WHERE status IN ('dispatching','queued','running','unknown') AND reconcile=1 AND created<=?1",[now()-86400]).map_err(database_error)?;
        db.execute("DELETE FROM store_image_jobs WHERE status IN ('succeeded','failed','unknown') AND updated<?1",[now()-7*86400]).map_err(database_error)?;
        let mut stmt = db.prepare("SELECT id,owner,platform,app,connector,created,snapshot FROM store_image_jobs WHERE reconcile=1 AND status IN ('dispatching','queued','running','unknown') AND created>?1").map_err(database_error)?;
        let rows = stmt
            .query_map([now() - 86400], |r| {
                Ok(StoreJob {
                    id: r.get(0)?,
                    owner: r.get(1)?,
                    platform: if r.get::<_, String>(2)? == "ios" {
                        Platform::Ios
                    } else {
                        Platform::Android
                    },
                    app: r.get(3)?,
                    connector: r.get(4)?,
                    created: r.get(5)?,
                    snapshot: serde_json::from_str(&r.get::<_, String>(6)?).unwrap_or_default(),
                })
            })
            .map_err(database_error)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(database_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    const KEY: &str = "jobs-registry-signing-key-01234567890123456789";
    fn job(id: &str, owner: Option<&str>) -> StoreJob {
        StoreJob {
            id: id.into(),
            owner: owner.map(str::to_owned),
            platform: Platform::Ios,
            app: "com.example.job".into(),
            connector: "fixed-node".into(),
            created: now(),
            snapshot: json!({"jobId":id,"status":"dispatching","result":null,"httpStatus":null,"errorCode":null,"outcomeUnknown":false}),
        }
    }
    #[test]
    fn ownership_survives_reopen_and_expiration_becomes_unknown_without_replay() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jobs.sqlite");
        {
            let registry = Registry::open(&path, KEY).unwrap();
            registry.insert_store_job(&job("business", None)).unwrap();
            registry
                .insert_store_job(&job("integration", Some("issuer")))
                .unwrap();
        }
        let registry = Registry::open(&path, KEY).unwrap();
        assert!(registry
            .store_job("business", None, Platform::Ios, "com.example.job")
            .is_ok());
        assert!(registry
            .store_job("business", Some("issuer"), Platform::Ios, "com.example.job")
            .is_err());
        assert!(registry
            .store_job("integration", None, Platform::Ios, "com.example.job")
            .is_err());
        assert!(registry
            .store_job(
                "integration",
                Some("other"),
                Platform::Ios,
                "com.example.job"
            )
            .is_err());
        assert!(registry
            .store_job(
                "integration",
                Some("issuer"),
                Platform::Ios,
                "com.example.job"
            )
            .is_ok());
        assert_eq!(registry.pending_store_jobs().unwrap().len(), 2);
        registry
            .db
            .lock()
            .unwrap()
            .execute("UPDATE store_image_jobs SET created=?1", [now() - 86401])
            .unwrap();
        assert!(registry.pending_store_jobs().unwrap().is_empty());
        let result = registry
            .store_job("business", None, Platform::Ios, "com.example.job")
            .unwrap();
        assert_eq!(result["status"], "unknown");
        assert_eq!(result["outcomeUnknown"], true);
        registry
            .db
            .lock()
            .unwrap()
            .execute(
                "UPDATE store_image_jobs SET updated=?1",
                [now() - 8 * 86400],
            )
            .unwrap();
        assert!(registry.pending_store_jobs().unwrap().is_empty());
        assert!(registry
            .store_job("business", None, Platform::Ios, "com.example.job")
            .is_err());
    }
    #[test]
    fn long_offline_restart_and_late_updates_cannot_resurrect_expired_jobs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jobs.sqlite");
        let old = now() - 30 * 86400;
        {
            let registry = Registry::open(&path, KEY).unwrap();
            let mut expired = job("old-dispatch", None);
            expired.created = old;
            registry.insert_store_job(&expired).unwrap();
        }
        let registry = Registry::open(&path, KEY).unwrap();
        assert!(registry
            .store_job("old-dispatch", None, Platform::Ios, "com.example.job")
            .is_err());
        // A delayed Start acknowledgement or status response must not refresh retention.
        assert!(!registry
            .update_store_job(
                "old-dispatch",
                &json!({"jobId":"old-dispatch","status":"queued"})
            )
            .unwrap());
        assert!(registry.pending_store_jobs().unwrap().is_empty());
        assert!(registry
            .store_job("old-dispatch", None, Platform::Ios, "com.example.job")
            .is_err());
        let count: i64 = registry
            .db
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM store_image_jobs", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);

        let mut recent_deadline = job("deadline", None);
        recent_deadline.created = now() - 86401;
        registry.insert_store_job(&recent_deadline).unwrap();
        assert!(!registry
            .update_store_job(
                "deadline",
                &json!({"jobId":"deadline","status":"succeeded"})
            )
            .unwrap());
        assert!(registry.pending_store_jobs().unwrap().is_empty());
        let updated: i64 = registry
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT updated FROM store_image_jobs WHERE id='deadline'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(updated, recent_deadline.created + 86400);
        assert_eq!(
            registry
                .store_job("deadline", None, Platform::Ios, "com.example.job")
                .unwrap()["status"],
            "unknown"
        );
    }
}
