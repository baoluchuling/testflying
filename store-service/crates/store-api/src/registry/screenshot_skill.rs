use super::{database_error, Registry, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde_json::{json, Value};
use store_protocol::{
    skill::{self, SkillManifest},
    StoreError,
};

pub(super) fn create_schema(db: &Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS screenshot_skill_versions (
        version TEXT PRIMARY KEY, manifest TEXT NOT NULL, bundle BLOB NOT NULL);
        CREATE TABLE IF NOT EXISTS screenshot_skill_current (
        id INTEGER PRIMARY KEY CHECK(id=1), version TEXT NOT NULL REFERENCES screenshot_skill_versions(version));")
        .map_err(database_error)?;
    Ok(())
}

fn corrupt() -> StoreError {
    StoreError::new(
        500,
        "screenshot_skill_unavailable",
        "Screenshot skill registry is unavailable",
    )
}
fn decode(value: String) -> Result<SkillManifest> {
    serde_json::from_str(&value).map_err(|_| corrupt())
}
pub(super) fn active(db: &Connection) -> Result<Option<SkillManifest>> {
    db.query_row("SELECT manifest FROM screenshot_skill_versions v JOIN screenshot_skill_current c ON v.version=c.version WHERE c.id=1",[],|r|r.get::<_,String>(0))
        .optional().map_err(database_error)?.map(decode).transpose()
}
fn view(db: &Connection) -> Result<Value> {
    let mut stmt = db
        .prepare("SELECT manifest FROM screenshot_skill_versions ORDER BY rowid DESC")
        .map_err(database_error)?;
    let versions = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(database_error)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(database_error)?
        .into_iter()
        .map(decode)
        .collect::<Result<Vec<_>>>()?;
    Ok(json!({"active":active(db)?,"versions":versions}))
}
fn choose(db: &Connection, version: &str) -> Result<()> {
    if !skill::valid_version(version) {
        return Err(StoreError::new(
            422,
            "invalid_screenshot_skill_version",
            "Select a valid skill version",
        ));
    }
    let exists: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM screenshot_skill_versions WHERE version=?1)",
            [version],
            |r| r.get(0),
        )
        .map_err(database_error)?;
    if !exists {
        return Err(StoreError::new(
            404,
            "screenshot_skill_not_found",
            "Screenshot skill version not found",
        ));
    }
    db.execute("INSERT INTO screenshot_skill_current(id,version) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET version=excluded.version",[version]).map_err(database_error)?;
    Ok(())
}
impl Registry {
    pub(crate) fn screenshot_skills(&self) -> Result<Value> {
        let db = self.db.lock().map_err(|_| corrupt())?;
        view(&db)
    }
    pub(crate) fn publish_screenshot_skill(&self, bundle: &[u8]) -> Result<Value> {
        let published_at = i64::try_from(store_protocol::upload::now()?).map_err(|_| corrupt())?;
        let manifest = SkillManifest::from_bundle(bundle, published_at)?;
        manifest.verify(bundle)?;
        let serialized = serde_json::to_string(&manifest).map_err(|_| corrupt())?;
        let mut db = self.db.lock().map_err(|_| corrupt())?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let previous = active(&tx)?.map(|manifest| manifest.version);
        tx.execute("INSERT OR IGNORE INTO screenshot_skill_versions(version,manifest,bundle) VALUES(?1,?2,?3)",params![manifest.version,serialized,bundle]).map_err(database_error)?;
        // Idempotence preserves the first publication date and immutable bytes.
        let existing: Vec<u8> = tx
            .query_row(
                "SELECT bundle FROM screenshot_skill_versions WHERE version=?1",
                [&manifest.version],
                |r| r.get(0),
            )
            .map_err(database_error)?;
        if existing != bundle {
            return Err(corrupt());
        }
        choose(&tx, &manifest.version)?;
        // Preserve the immediately previous effective version for rollback, even
        // when the administrator had selected the oldest retained publication.
        // Running tasks also pin their bundle. A full pinned history rolls back.
        tx.execute("DELETE FROM screenshot_skill_versions WHERE version IN (
            SELECT version FROM screenshot_skill_versions
            WHERE version NOT IN (SELECT version FROM screenshot_skill_current)
            AND (?2 IS NULL OR version<>?2)
            AND NOT EXISTS (SELECT 1 FROM screenshot_jobs j WHERE j.status IN ('running','uploading')
                AND j.deadline>?1 AND json_extract(j.skill_manifest,'$.version')=screenshot_skill_versions.version)
            ORDER BY rowid ASC LIMIT MAX(0,(SELECT count(*) FROM screenshot_skill_versions)-10))",params![published_at,previous]).map_err(database_error)?;
        let count: i64 = tx
            .query_row("SELECT count(*) FROM screenshot_skill_versions", [], |r| {
                r.get(0)
            })
            .map_err(database_error)?;
        if count > 10 {
            return Err(StoreError::new(
                409,
                "screenshot_skill_versions_busy",
                "Wait for running screenshot jobs before publishing another version",
            ));
        }
        let output = view(&tx)?;
        tx.commit().map_err(database_error)?;
        Ok(output)
    }
    pub(crate) fn select_screenshot_skill(&self, version: &str) -> Result<Value> {
        let mut db = self.db.lock().map_err(|_| corrupt())?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        choose(&tx, version)?;
        let output = view(&tx)?;
        tx.commit().map_err(database_error)?;
        Ok(output)
    }
    pub(crate) fn screenshot_skill_bundle(&self, version: &str) -> Result<Vec<u8>> {
        if !skill::valid_version(version) {
            return Err(StoreError::new(
                422,
                "invalid_screenshot_skill_version",
                "Select a valid skill version",
            ));
        }
        let db = self.db.lock().map_err(|_| corrupt())?;
        let (manifest, bundle): (String, Vec<u8>) = db
            .query_row(
                "SELECT manifest,bundle FROM screenshot_skill_versions WHERE version=?1",
                [version],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(database_error)?
            .ok_or_else(|| {
                StoreError::new(
                    404,
                    "screenshot_skill_not_found",
                    "Screenshot skill version not found",
                )
            })?;
        decode(manifest)?.verify(&bundle).map_err(|_| corrupt())?;
        Ok(bundle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{ConnectorInput, ConnectorKind};
    use store_protocol::skill::{SkillBundle, SkillFile};
    fn registry() -> Registry {
        Registry::open(":memory:", "skill-registry-fixture-signing-key-0123456789").unwrap()
    }
    fn bundle(revision: usize) -> Vec<u8> {
        SkillBundle {
            schema_version: 1,
            name: "capture".into(),
            files: vec![SkillFile::new(
                "SKILL.md".into(),
                format!("---\nname: capture\ndescription: Capture\n---\nRevision {revision}")
                    .as_bytes(),
                false,
            )],
        }
        .canonical_bytes()
        .unwrap()
    }
    #[test]
    fn immutable_versions_idempotence_history_and_rollback_are_atomic() {
        let registry = registry();
        assert!(registry.screenshot_skills().unwrap()["active"].is_null());
        let first = registry.publish_screenshot_skill(&bundle(0)).unwrap();
        let version = first["active"]["version"].as_str().unwrap();
        assert_eq!(
            registry.publish_screenshot_skill(&bundle(0)).unwrap(),
            first
        );
        let second = registry.publish_screenshot_skill(&bundle(1)).unwrap();
        assert_ne!(first["active"], second["active"]);
        assert_eq!(
            registry.select_screenshot_skill(version).unwrap()["active"],
            first["active"]
        );
        assert!(registry.select_screenshot_skill(&"0".repeat(64)).is_err());
        assert!(registry.publish_screenshot_skill(b"invalid").is_err());
        assert_eq!(
            registry.screenshot_skills().unwrap()["active"],
            first["active"]
        );
        assert_eq!(
            registry.screenshot_skill_bundle(version).unwrap(),
            bundle(0)
        );
        for index in 2..15 {
            registry.publish_screenshot_skill(&bundle(index)).unwrap();
        }
        assert_eq!(
            registry.screenshot_skills().unwrap()["versions"]
                .as_array()
                .unwrap()
                .len(),
            10
        );
    }
    #[test]
    fn publishing_after_rollback_to_oldest_retains_previous_effective_version() {
        let registry = registry();
        let first = registry.publish_screenshot_skill(&bundle(0)).unwrap();
        let oldest = first["active"]["version"].as_str().unwrap();
        let second = registry.publish_screenshot_skill(&bundle(1)).unwrap();
        let evictable = second["active"]["version"].as_str().unwrap();
        for revision in 2..10 {
            registry
                .publish_screenshot_skill(&bundle(revision))
                .unwrap();
        }
        registry.select_screenshot_skill(oldest).unwrap();
        let published = registry.publish_screenshot_skill(&bundle(10)).unwrap();
        assert_eq!(published["versions"].as_array().unwrap().len(), 10);
        assert_ne!(published["active"]["version"], oldest);
        assert_eq!(registry.screenshot_skill_bundle(oldest).unwrap(), bundle(0));
        assert_eq!(
            registry
                .screenshot_skill_bundle(evictable)
                .unwrap_err()
                .code,
            "screenshot_skill_not_found"
        );
        assert_eq!(
            registry.select_screenshot_skill(oldest).unwrap()["active"],
            first["active"]
        );
    }
    #[test]
    fn capability_barrier_does_not_consume_and_claim_pins_manifest_across_publish_and_rollback() {
        let registry = registry();
        let node = registry
            .create_with_kind(
                ConnectorInput {
                    name: "Screenshot node".into(),
                },
                ConnectorKind::Screenshot,
            )
            .unwrap();
        let integration = registry
            .issue_integration(
                serde_json::from_value(json!({"name":"Skill claim fixture"})).unwrap(),
            )
            .unwrap();
        let time = store_protocol::upload::now().unwrap() as i64;
        registry.db.lock().unwrap().execute("INSERT INTO screenshot_jobs(id,connector_id,integration_id,profile,prompt,status,created_at,deadline) VALUES('fixture-job',?1,?2,'','  original prompt\n','queued',?3,?4)",params![node.connector_id,integration.id,time,time+86400]).unwrap();
        let first = registry.publish_screenshot_skill(&bundle(0)).unwrap();
        let version = first["active"]["version"].as_str().unwrap();
        assert_eq!(
            registry
                .claim_screenshot_protocol(&node.connector_id, true)
                .unwrap_err()
                .code,
            "screenshot_skill_upgrade_required"
        );
        assert_eq!(
            registry
                .screenshot_status(&integration, "fixture-job")
                .unwrap()["status"],
            "queued"
        );
        assert!(registry
            .screenshot_status(&integration, "fixture-job")
            .unwrap()["skill"]
            .is_null());
        let claim = registry
            .claim_screenshot_with_skill_protocol(&node.connector_id, true, true)
            .unwrap()
            .unwrap();
        assert_eq!(claim["skill"], first["active"]);
        assert_eq!(claim["prompt"], "  original prompt\n");
        assert!(registry
            .claim_screenshot_protocol(&node.connector_id, true)
            .unwrap()
            .is_none());
        for index in 1..14 {
            registry.publish_screenshot_skill(&bundle(index)).unwrap();
        }
        registry.select_screenshot_skill(version).unwrap();
        assert_eq!(
            registry
                .screenshot_status(&integration, "fixture-job")
                .unwrap()["skill"],
            first["active"]
        );
        assert_eq!(
            registry.screenshot_skill_bundle(version).unwrap(),
            bundle(0)
        );
        assert_eq!(
            registry.screenshot_skills().unwrap()["versions"]
                .as_array()
                .unwrap()
                .len(),
            10
        );
    }
    #[test]
    fn full_pinned_history_rolls_back_new_version_and_current_pointer() {
        let registry = registry();
        let node = registry
            .create_with_kind(
                ConnectorInput {
                    name: "Pinned worker".into(),
                },
                ConnectorKind::Screenshot,
            )
            .unwrap();
        let integration = registry
            .issue_integration(
                serde_json::from_value(json!({"name":"Pinned integration"})).unwrap(),
            )
            .unwrap();
        let time = store_protocol::upload::now().unwrap() as i64;
        for revision in 0..10 {
            let output = registry
                .publish_screenshot_skill(&bundle(revision))
                .unwrap();
            registry.db.lock().unwrap().execute("INSERT INTO screenshot_jobs(id,connector_id,integration_id,profile,prompt,status,created_at,deadline,skill_manifest) VALUES(?1,?2,?3,'','','running',?4,?5,?6)",params![format!("pinned-{revision}"),node.connector_id,integration.id,time,time+1800,output["active"].to_string()]).unwrap();
        }
        let before = registry.screenshot_skills().unwrap();
        assert_eq!(
            registry
                .publish_screenshot_skill(&bundle(10))
                .unwrap_err()
                .code,
            "screenshot_skill_versions_busy"
        );
        assert_eq!(registry.screenshot_skills().unwrap(), before);
    }
}
