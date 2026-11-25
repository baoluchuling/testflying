//! Candidate image changes are serialized with final submission in the job transaction.
use super::screenshots::{check_target_lease, timestamp};
use super::*;
use serde_json::{json, Value};

fn review_job(
    db: &Connection,
    node: &str,
    id: &str,
    lease: &str,
    allow_finished: bool,
) -> Result<Option<Value>> {
    check_target_lease(db, node, id, lease)?;
    let (review, status, current, result): (bool, String, Option<String>, Option<String>) = db
        .query_row(
            "SELECT review_before_completion,status,lease,result FROM screenshot_jobs WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .map_err(database_error)?;
    if !review {
        return Err(StoreError::new(
            409,
            "screenshot_review_unsupported",
            "This task does not support candidate review",
        ));
    }
    if allow_finished && status == "succeeded" {
        return result
            .map(|v| serde_json::from_str(&v))
            .transpose()
            .map_err(|_| database_error(rusqlite::Error::InvalidQuery));
    }
    if status != "running" || current.as_deref() != Some(lease) {
        return Err(StoreError::new(
            409,
            "invalid_job_lease",
            "This task cannot be changed",
        ));
    }
    Ok(None)
}

fn images(db: &Connection, id: &str) -> Result<Vec<Value>> {
    let mut stmt=db.prepare("SELECT result FROM screenshot_target_tickets WHERE job_id=?1 AND result IS NOT NULL ORDER BY target_index").map_err(database_error)?;
    let rows = stmt
        .query_map([id], |r| r.get::<_, String>(0))
        .map_err(database_error)?;
    rows.map(|row| {
        serde_json::from_str(&row.map_err(database_error)?)
            .map_err(|_| database_error(rusqlite::Error::InvalidQuery))
    })
    .collect()
}

fn rebuild(db: &Connection, id: &str) -> Result<Value> {
    let previous: Option<String> = db
        .query_row(
            "SELECT result FROM screenshot_jobs WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .map_err(database_error)?;
    let mut aggregate = previous
        .map(|v| serde_json::from_str::<Value>(&v))
        .transpose()
        .map_err(|_| database_error(rusqlite::Error::InvalidQuery))?
        .unwrap_or_else(|| json!({}));
    aggregate["images"] = json!(images(db, id)?);
    db.execute(
        "UPDATE screenshot_jobs SET result=?2 WHERE id=?1",
        params![id, aggregate.to_string()],
    )
    .map_err(database_error)?;
    Ok(aggregate)
}

impl Registry {
    pub(crate) fn screenshot_requires_review(&self, id: &str) -> Result<bool> {
        self.db
            .lock()
            .unwrap()
            .query_row(
                "SELECT review_before_completion FROM screenshot_jobs WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .map_err(database_error)
    }

    pub(crate) fn discard_screenshot_target(
        &self,
        node: &str,
        id: &str,
        lease: &str,
        index: usize,
        key: &str,
    ) -> Result<Value> {
        let mut db = self.db.lock().unwrap();
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        review_job(&tx, node, id, lease, false)?;
        let already:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM screenshot_discarded_targets WHERE job_id=?1 AND target_index=?2 AND object_key=?3 AND lease=?4)",params![id,index as i64,key,lease],|r|r.get(0)).map_err(database_error)?;
        if !already {
            let changed=tx.execute("DELETE FROM screenshot_target_tickets WHERE job_id=?1 AND target_index=?2 AND object_key=?3 AND lease=?4 AND result IS NOT NULL",params![id,index as i64,key,lease]).map_err(database_error)?;
            if changed != 1 {
                return Err(StoreError::new(
                    409,
                    "screenshot_discard_unavailable",
                    "Only the current confirmed image can be discarded",
                ));
            }
            tx.execute("INSERT INTO screenshot_discarded_targets(job_id,target_index,object_key,lease) VALUES(?1,?2,?3,?4)",params![id,index as i64,key,lease]).map_err(database_error)?;
        }
        let result = rebuild(&tx, id)?;
        tx.commit().map_err(database_error)?;
        drop(db);
        self.logs.record(crate::logs::Event {
            name: "screenshot.target_discarded",
            request_id: Some(id.into()),
            connector_id: Some(node.into()),
            details: json!({"targetIndex":index,"repeated":already}),
            ..Default::default()
        });
        Ok(json!({"discarded":true,"targetIndex":index,"result":result}))
    }

    pub(crate) fn finish_reviewed_screenshot(
        &self,
        node: &str,
        id: &str,
        lease: &str,
    ) -> Result<Value> {
        let mut db = self.db.lock().unwrap();
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        if let Some(result) = review_job(&tx, node, id, lease, true)? {
            return Ok(json!({"delivered":true,"result":result}));
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
        let current = images(&tx, id)?;
        if current.len() != targets.len()
            || current.is_empty()
            || current
                .iter()
                .enumerate()
                .any(|(i, v)| v["targetIndex"].as_u64() != Some(i as u64))
        {
            return Err(StoreError::new(
                409,
                "screenshot_targets_incomplete",
                "Upload every current target before final submission",
            ));
        }
        let result = rebuild(&tx, id)?;
        tx.execute(
            "UPDATE screenshot_jobs SET status='succeeded',lease=NULL WHERE id=?1",
            [id],
        )
        .map_err(database_error)?;
        super::screenshot_notifications::enqueue(&tx, id, timestamp()?)?;
        tx.commit().map_err(database_error)?;
        drop(db);
        self.logs.record(crate::logs::Event {
            name: "screenshot.finalized",
            request_id: Some(id.into()),
            connector_id: Some(node.into()),
            details: json!({"targetCount":targets.len()}),
            ..Default::default()
        });
        Ok(json!({"delivered":true,"result":result}))
    }
}
