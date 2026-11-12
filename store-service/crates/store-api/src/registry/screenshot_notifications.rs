//! Durable immutable callback snapshots. Creation is part of the terminal job transaction.
use super::*;
use serde_json::{json, Value};
use sha2::Digest;

pub(crate) const MAX_BODY: usize = 2 * 1024 * 1024;
pub(super) fn create_schema(db: &Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS screenshot_notifications (
      job_id TEXT PRIMARY KEY REFERENCES screenshot_jobs(id) ON DELETE CASCADE,
      body TEXT NOT NULL, created_at INTEGER NOT NULL, next_at INTEGER NOT NULL,
      state TEXT NOT NULL DEFAULT 'pending', attempts INTEGER NOT NULL DEFAULT 0,
      lease TEXT, lease_until INTEGER, config_hash TEXT, error TEXT, results TEXT);
      CREATE INDEX IF NOT EXISTS screenshot_notifications_due ON screenshot_notifications(state,next_at);") .map_err(database_error)
}

pub(super) fn enqueue(db: &Connection, id: &str, time: i64) -> Result<()> {
    let structured: bool = db
        .query_row(
            "SELECT callback_eligible=1 AND targets IS NOT NULL FROM screenshot_jobs WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .map_err(database_error)?;
    if !structured {
        return Ok(());
    }
    let (status, result, error): (String, Option<String>, Option<String>) = db
        .query_row(
            "SELECT status,result,error FROM screenshot_jobs WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(database_error)?;
    let result: Value = result
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null);
    let mut items = Vec::new();
    if let Some(images) = result["images"].as_array() {
        for image in images {
            if let (Some(slot), Some(language), Some(url), Some(version)) = (
                image["target"]["page"].as_str(),
                image["target"]["locale"].as_str(),
                image["imageUrl"].as_str(),
                image["sha256"].as_str(),
            ) {
                items.push(json!({"slot":slot,"language":language,"url":url,"version":version}));
            }
        }
    }
    let callback_status = if status == "succeeded" {
        "done"
    } else if items.is_empty() {
        "failed"
    } else {
        "partial"
    };
    let mut body =
        json!({"upstream_job_id":id,"revision":1,"status":callback_status,"items":items});
    if let Some(error) = error {
        body["error"] = json!(error.chars().take(2000).collect::<String>());
    }
    let body = body.to_string();
    let (state, error) = if body.len() > MAX_BODY {
        ("failed", Some("callback_body_too_large"))
    } else {
        ("pending", None)
    };
    db.execute("INSERT OR IGNORE INTO screenshot_notifications(job_id,body,created_at,next_at,state,error) VALUES(?1,?2,?3,?3,?4,?5)", params![id,body,time,state,error]).map_err(database_error)?;
    Ok(())
}

pub(crate) struct Notification {
    pub job: String,
    pub body: String,
    pub lease: String,
    pub url: String,
    pub token: String,
}
pub(crate) enum Outcome {
    Delivered(Value),
    Retry(&'static str),
    RetryResults(Value),
    Unauthorized,
    Failed(&'static str),
}

impl Registry {
    pub(crate) fn claim_notification(&self, time: i64) -> Result<Option<Notification>> {
        let mut db = self.db.lock().unwrap();
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        // Sweep newly expired jobs as well, even when no client polls their status.
        super::screenshots::expire(&tx, time)?;
        tx.execute("UPDATE screenshot_notifications SET state='failed',error=CASE WHEN error='callback_items_rejected' THEN 'callback_expired_after_rejection' ELSE 'callback_expired' END,lease=NULL,lease_until=NULL WHERE created_at<=?1 AND state IN ('pending','sending','unauthorized','not_configured')", [time-86400]).map_err(database_error)?;
        let config = super::screenshot_settings::load(&tx, &self.signing_key)?.notification;
        let hash = hex::encode(sha2::Sha256::digest(
            serde_json::to_vec(&config)
                .map_err(|_| database_error(rusqlite::Error::InvalidQuery))?,
        ));
        let configured = !config.url.is_empty() && !config.token.is_empty();
        if !configured {
            tx.execute("UPDATE screenshot_notifications SET state='not_configured',error='callback_not_configured',lease=NULL,lease_until=NULL WHERE state='pending' OR (state='sending' AND lease_until<=?1)",[time]).map_err(database_error)?;
            tx.commit().map_err(database_error)?;
            return Ok(None);
        }
        tx.execute("UPDATE screenshot_notifications SET state='pending',next_at=?1,error=NULL WHERE state='not_configured' OR (state='unauthorized' AND config_hash<>?2)",params![time,hash]).map_err(database_error)?;
        let row: Option<(String,String)> = tx.query_row("SELECT job_id,body FROM screenshot_notifications WHERE (state='pending' AND next_at<=?1) OR (state='sending' AND lease_until<=?1) ORDER BY created_at LIMIT 1", [time], |r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(database_error)?;
        let Some((job, body)) = row else {
            tx.commit().map_err(database_error)?;
            return Ok(None);
        };
        let lease = Uuid::new_v4().to_string();
        tx.execute("UPDATE screenshot_notifications SET state='sending',lease=?2,lease_until=?3,config_hash=?4,attempts=attempts+1 WHERE job_id=?1",params![job,lease,time+90,hash]).map_err(database_error)?;
        tx.commit().map_err(database_error)?;
        Ok(Some(Notification {
            job,
            body,
            lease,
            url: config.url,
            token: config.token,
        }))
    }

    pub(crate) fn finish_notification(
        &self,
        n: &Notification,
        outcome: Outcome,
        time: i64,
    ) -> Result<()> {
        let db = self.db.lock().unwrap();
        let attempts: Option<i64> = db.query_row("SELECT attempts FROM screenshot_notifications WHERE job_id=?1 AND lease=?2 AND state='sending'",params![n.job,n.lease],|r|r.get(0)).optional().map_err(database_error)?;
        let Some(attempts) = attempts else {
            return Ok(());
        };
        let delay = match attempts {
            1 => 2,
            2 => 5,
            3 => 10,
            4 => 30,
            _ => 60,
        };
        let (state, error, results) = match outcome {
            Outcome::Delivered(results) => ("delivered", None, Some(results.to_string())),
            Outcome::Retry(code) => ("pending", Some(code), None),
            Outcome::RetryResults(results) => (
                "pending",
                Some("callback_items_rejected"),
                Some(results.to_string()),
            ),
            Outcome::Unauthorized => ("unauthorized", Some("callback_unauthorized"), None),
            Outcome::Failed(code) => ("failed", Some(code), None),
        };
        db.execute("UPDATE screenshot_notifications SET state=?3,error=?4,results=COALESCE(?5,results),next_at=?6,lease=NULL,lease_until=NULL WHERE job_id=?1 AND lease=?2 AND state='sending'",params![n.job,n.lease,state,error,results,time+delay]).map_err(database_error)?;
        drop(db);
        self.logs.record(crate::logs::Event {
            name: "screenshot.callback",
            details: json!({"jobId":n.job,"status":state,"error":error}),
            ..Default::default()
        });
        Ok(())
    }
}

pub(super) fn decorate(
    db: &Connection,
    signing_key: &[u8],
    id: &str,
    result: &mut Option<Value>,
) -> Result<()> {
    let row: Option<(String, Option<String>, Option<String>)> = db
        .query_row(
            "SELECT state,error,results FROM screenshot_notifications WHERE job_id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(database_error)?;
    if let Some((state, error, results)) = row {
        let config = super::screenshot_settings::load(db, signing_key)?.notification;
        let parsed_results: Value = results
            .and_then(|r| serde_json::from_str(&r).ok())
            .unwrap_or(Value::Null);
        let partial = parsed_results.as_array().is_some_and(|rows| {
            rows.iter().any(|r| r["result"] == "rejected")
                && rows
                    .iter()
                    .any(|r| matches!(r["result"].as_str(), Some("created" | "skipped")))
        });
        let state = if state == "pending" && (config.url.is_empty() || config.token.is_empty()) {
            "not_configured".to_string()
        } else if state == "pending" && partial {
            "partial".to_string()
        } else if state == "pending" && error.is_some() {
            "retrying".to_string()
        } else {
            state
        };
        let value = result.get_or_insert_with(|| json!({}));
        value["notificationStatus"] = json!(state);
        value["notificationError"] = json!(error);
        value["notificationResults"] = parsed_results;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const KEY: &str = "callback-fixture-signing-key-0123456789";
    fn job(registry: &Registry, id: &str, status: &str, result: Value) {
        let db = registry.db.lock().unwrap();
        db.execute("INSERT OR IGNORE INTO connectors(id,name,mode,nonce) VALUES('node','node','screenshot','nonce')",[]).unwrap();
        db.execute(
            "INSERT OR IGNORE INTO integrations(id,name) VALUES('integration','integration')",
            [],
        )
        .unwrap();
        db.execute("INSERT INTO screenshot_jobs(id,connector_id,integration_id,profile,prompt,status,created_at,deadline,result,error,targets,callback_eligible) VALUES(?1,'node','integration','','',?2,100,90000,?3,'capture_failed','[]',1)",params![id,status,result.to_string()]).unwrap();
    }
    fn configured(registry: &Registry, token: &str) {
        registry.save_screenshot_settings(&serde_json::from_value(json!({"notification":{"url":"https://callback.example/api/hooks/captures","token":token}})).unwrap()).unwrap();
    }
    #[test]
    fn partial_snapshot_is_exact_immutable_and_restart_safe() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("registry.sqlite");
        let registry = Registry::open(&path, KEY).unwrap();
        job(
            &registry,
            "job",
            "failed",
            json!({"images":[{"target":{"page":"details","locale":"pt"},"imageUrl":"https://images.example/a.png","sha256":"abc"}]}),
        );
        {
            let db = registry.db.lock().unwrap();
            enqueue(&db, "job", 100).unwrap();
        }
        assert!(registry.claim_notification(101).unwrap().is_none());
        configured(&registry, "token-a");
        let first = registry.claim_notification(102).unwrap().unwrap();
        let body: Value = serde_json::from_str(&first.body).unwrap();
        assert_eq!(
            body,
            json!({"upstream_job_id":"job","revision":1,"status":"partial","items":[{"slot":"details","language":"pt","url":"https://images.example/a.png","version":"abc"}],"error":"capture_failed"})
        );
        assert!(registry.claim_notification(103).unwrap().is_none());
        drop(registry);
        let registry = Registry::open(&path, KEY).unwrap();
        let second = registry.claim_notification(193).unwrap().unwrap();
        assert_eq!(first.body, second.body);
        registry
            .finish_notification(&first, Outcome::Delivered(json!([])), 194)
            .unwrap();
        registry
            .finish_notification(&second, Outcome::Unauthorized, 194)
            .unwrap();
        assert!(registry.claim_notification(300).unwrap().is_none());
        configured(&registry, "token-b");
        let third = registry.claim_notification(301).unwrap().unwrap();
        assert_eq!(third.body, first.body);
        registry
            .finish_notification(&third, Outcome::Retry("callback_transport_failed"), 301)
            .unwrap();
        assert!(registry.claim_notification(305).unwrap().is_none());
        assert!(registry.claim_notification(312).unwrap().is_some());
        assert!(registry.claim_notification(86500).unwrap().is_none());
    }
    #[test]
    fn migration_does_not_backfill_terminal_history_and_expiry_creates_failed_intent() {
        let registry = Registry::open(":memory:", KEY).unwrap();
        job(&registry, "history", "succeeded", json!({}));
        job(&registry, "expiring", "running", json!({}));
        job(
            &registry,
            "pre_callback_structured",
            "running",
            json!({"images":[{"target":{"page":"details","locale":"en","device":"tablet"},"imageUrl":"https://images.example/old.png?Expires=123&Signature=old","sha256":"abc"}]}),
        );
        job(
            &registry,
            "legacy",
            "running",
            json!({"imageUrl":"https://images.example/legacy.png"}),
        );
        {
            let db = registry.db.lock().unwrap();
            db.execute(
                "UPDATE screenshot_jobs SET deadline=101 WHERE id='expiring'",
                [],
            )
            .unwrap();
            db.execute("UPDATE screenshot_jobs SET callback_eligible=0,deadline=101,targets='[{\"page\":\"details\",\"locale\":\"en\",\"device\":\"tablet\"},{\"page\":\"ranking\",\"locale\":\"en\",\"device\":\"phone\"}]' WHERE id='pre_callback_structured'",[]).unwrap();
            db.execute(
                "UPDATE screenshot_jobs SET targets=NULL,deadline=101 WHERE id='legacy'",
                [],
            )
            .unwrap();
        }
        assert!(registry.claim_notification(102).unwrap().is_none());
        let db = registry.db.lock().unwrap();
        let rows: i64 = db
            .query_row("SELECT COUNT(*) FROM screenshot_notifications", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(rows, 1);
        let body: String = db
            .query_row(
                "SELECT body FROM screenshot_notifications WHERE job_id='expiring'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&body).unwrap()["status"],
            "failed"
        );
    }

    #[test]
    fn missing_configuration_marks_all_pending_and_partial_results_survive_expiry() {
        let registry = Registry::open(":memory:", KEY).unwrap();
        for id in ["a", "b"] {
            job(&registry, id, "failed", json!({}));
            let db = registry.db.lock().unwrap();
            enqueue(&db, id, 100).unwrap();
        }
        assert!(registry.claim_notification(101).unwrap().is_none());
        {
            let db = registry.db.lock().unwrap();
            let count: i64 = db
                .query_row(
                    "SELECT COUNT(*) FROM screenshot_notifications WHERE state='not_configured'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, 2);
        }
        configured(&registry, "token");
        let n = registry.claim_notification(102).unwrap().unwrap();
        let results = json!([{"index":0,"result":"created"},{"index":1,"result":"rejected"}]);
        registry
            .finish_notification(&n, Outcome::RetryResults(results.clone()), 103)
            .unwrap();
        let mut result = None;
        {
            let db = registry.db.lock().unwrap();
            decorate(&db, &registry.signing_key, &n.job, &mut result).unwrap();
        }
        assert_eq!(result.unwrap()["notificationStatus"], "partial");
        registry.claim_notification(86500).unwrap();
        let mut result = None;
        {
            let db = registry.db.lock().unwrap();
            decorate(&db, &registry.signing_key, &n.job, &mut result).unwrap();
        }
        let result = result.unwrap();
        assert_eq!(result["notificationStatus"], "failed");
        assert_eq!(
            result["notificationError"],
            "callback_expired_after_rejection"
        );
        assert_eq!(result["notificationResults"], results);
    }
}
