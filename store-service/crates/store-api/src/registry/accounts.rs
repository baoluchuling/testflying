use super::*;
use serde_json::{json, Value};
use store_protocol::catalog::{CatalogReport, CatalogStatus};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    pub name: String,
    pub platform: Platform,
    pub store_connector_id: String,
    pub screenshot_connector_id: Option<String>,
    pub enabled: bool,
    pub catalog_status: String,
    pub catalog_error: Option<String>,
    pub last_catalog_at: Option<i64>,
    pub migration_error: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccountInput {
    pub name: String,
    pub platform: Platform,
    pub store_connector_id: String,
    pub screenshot_connector_id: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccountPatch {
    pub name: Option<String>,
    pub enabled: Option<bool>,
    pub store_connector_id: Option<String>,
    // Missing retains the setting; explicit null clears it.
    #[serde(default, deserialize_with = "nullable")]
    pub screenshot_connector_id: Option<Option<String>>,
    #[serde(default)]
    pub accept_identity_change: bool,
}

fn nullable<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(d).map(Some)
}

pub(super) fn create_schema(db: &mut Connection) -> Result<()> {
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(database_error)?;
    tx.execute_batch("CREATE TABLE IF NOT EXISTS accounts (
      id TEXT PRIMARY KEY, name TEXT NOT NULL, platform TEXT NOT NULL CHECK(platform IN ('ios','android')),
      store_connector_id TEXT NOT NULL REFERENCES connectors(id),
      screenshot_connector_id TEXT REFERENCES connectors(id), enabled INTEGER NOT NULL DEFAULT 1,
      fingerprint TEXT, pending_fingerprint TEXT, catalog_status TEXT NOT NULL DEFAULT 'awaiting_catalog',
      catalog_error TEXT, last_catalog_at INTEGER, migration_error TEXT,
      UNIQUE(platform,store_connector_id));
      CREATE TABLE IF NOT EXISTS account_apps (
      account_id TEXT NOT NULL REFERENCES accounts(id), bundle_identifier TEXT NOT NULL,
      name TEXT NOT NULL, store_app_id TEXT, PRIMARY KEY(account_id,bundle_identifier));
      CREATE TABLE IF NOT EXISTS account_bindings (
      platform TEXT NOT NULL, bundle_identifier TEXT NOT NULL, account_id TEXT REFERENCES accounts(id),
      provisional INTEGER NOT NULL DEFAULT 0, suppressed INTEGER NOT NULL DEFAULT 0,
      screenshot_disabled INTEGER NOT NULL DEFAULT 0, PRIMARY KEY(platform,bundle_identifier));
      CREATE TABLE IF NOT EXISTS account_reports (
      connector_id TEXT NOT NULL, report_id TEXT NOT NULL, PRIMARY KEY(connector_id,report_id));
      CREATE TABLE IF NOT EXISTS account_migration_issues (
      platform TEXT NOT NULL, bundle_identifier TEXT NOT NULL, code TEXT NOT NULL,
      PRIMARY KEY(platform,bundle_identifier));
      CREATE TABLE IF NOT EXISTS store_account_screenshot_bindings (
      store_connector_id TEXT PRIMARY KEY REFERENCES connectors(id),
      screenshot_connector_id TEXT REFERENCES connectors(id));
      CREATE TABLE IF NOT EXISTS account_migrations (name TEXT PRIMARY KEY);").map_err(database_error)?;
    let migrated: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM account_migrations WHERE name='accounts_v1')",
            [],
            |r| r.get(0),
        )
        .map_err(database_error)?;
    if !migrated {
        let groups: Vec<(String, String, String)> = {
            let mut stmt = tx.prepare("SELECT DISTINCT b.platform,c.id,c.name FROM app_bindings b JOIN connectors c ON c.id=b.connector_id WHERE c.mode!='screenshot'").map_err(database_error)?;
            let rows = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .map_err(database_error)?;
            rows.collect::<std::result::Result<_, _>>()
                .map_err(database_error)?
        };
        for (platform, node, name) in groups {
            let id = Uuid::new_v4().to_string();
            let screenshots: Vec<String> = {
                let mut stmt = tx.prepare("SELECT DISTINCT s.connector_id FROM screenshot_app_bindings s JOIN app_bindings b USING(platform,bundle_identifier) JOIN connectors c ON c.id=s.connector_id WHERE b.platform=?1 AND b.connector_id=?2 AND c.mode='screenshot'").map_err(database_error)?;
                let rows = stmt
                    .query_map(params![platform, node], |r| r.get(0))
                    .map_err(database_error)?;
                rows.collect::<std::result::Result<_, _>>()
                    .map_err(database_error)?
            };
            let screenshot = (screenshots.len() == 1).then(|| screenshots[0].clone());
            let issue = (screenshots.len() > 1).then_some("legacy_screenshot_conflict");
            tx.execute("INSERT INTO accounts(id,name,platform,store_connector_id,screenshot_connector_id,migration_error) VALUES(?1,?2,?3,?4,?5,?6)",params![id,name,platform,node,screenshot,issue]).map_err(database_error)?;
            tx.execute("INSERT INTO account_bindings(platform,bundle_identifier,account_id,provisional,screenshot_disabled) SELECT b.platform,b.bundle_identifier,?1,1,NOT EXISTS(SELECT 1 FROM screenshot_app_bindings s WHERE s.platform=b.platform AND s.bundle_identifier=b.bundle_identifier) FROM app_bindings b WHERE b.platform=?2 AND b.connector_id=?3",params![id,platform,node]).map_err(database_error)?;
        }
        tx.execute_batch("INSERT INTO account_migration_issues SELECT s.platform,s.bundle_identifier,'legacy_screenshot_orphan' FROM screenshot_app_bindings s WHERE NOT EXISTS(SELECT 1 FROM account_bindings b WHERE b.platform=s.platform AND b.bundle_identifier=s.bundle_identifier);
          INSERT OR REPLACE INTO account_migration_issues SELECT s.platform,s.bundle_identifier,'legacy_screenshot_invalid' FROM screenshot_app_bindings s LEFT JOIN connectors c ON c.id=s.connector_id WHERE c.id IS NULL OR c.mode!='screenshot';
          UPDATE accounts SET screenshot_connector_id=NULL,migration_error='legacy_screenshot_invalid' WHERE id IN (SELECT b.account_id FROM account_bindings b JOIN account_migration_issues i USING(platform,bundle_identifier) WHERE i.code='legacy_screenshot_invalid');
          INSERT OR IGNORE INTO account_migration_issues SELECT b.platform,b.bundle_identifier,'legacy_store_invalid' FROM app_bindings b WHERE NOT EXISTS(SELECT 1 FROM account_bindings a WHERE a.platform=b.platform AND a.bundle_identifier=b.bundle_identifier);
          INSERT INTO account_migrations VALUES('accounts_v1');").map_err(database_error)?;
    }
    let manual_migrated: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM account_migrations WHERE name='android_manual_apps_v1')",
            [],
            |r| r.get(0),
        )
        .map_err(database_error)?;
    if !manual_migrated {
        let source_exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('account_apps') WHERE name='source')", [], |r| r.get(0)).map_err(database_error)?;
        if !source_exists {
            tx.execute_batch("ALTER TABLE account_apps ADD COLUMN source TEXT NOT NULL DEFAULT 'catalog' CHECK(source IN ('catalog','manual','retained'));").map_err(database_error)?;
        }
        tx.execute_batch("UPDATE account_apps SET source='retained' WHERE source='catalog' AND account_id IN (SELECT id FROM accounts WHERE platform='android');
          CREATE TABLE IF NOT EXISTS account_app_removals (
            account_id TEXT NOT NULL REFERENCES accounts(id), bundle_identifier TEXT NOT NULL,
            PRIMARY KEY(account_id,bundle_identifier));
          INSERT INTO account_migrations VALUES('android_manual_apps_v1');").map_err(database_error)?;
    }
    let legacy_android_migrated: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM account_migrations WHERE name='android_legacy_bindings_v1')",
        [], |r| r.get(0),
    ).map_err(database_error)?;
    if !legacy_android_migrated {
        // Only carry forward bindings still awaiting merchant discovery for the
        // same account identity. A successful catalog or accepted identity change
        // clears provisional, so stale bindings cannot reappear in a new account.
        let candidates: Vec<(String, String)> = {
            let mut stmt = tx.prepare("SELECT a.id,b.bundle_identifier FROM app_bindings b
              JOIN accounts a ON a.platform=b.platform AND a.store_connector_id=b.connector_id
              JOIN account_bindings ab ON ab.platform=b.platform AND ab.bundle_identifier=b.bundle_identifier AND ab.account_id=a.id
              JOIN connectors c ON c.id=b.connector_id
              WHERE b.platform='android' AND ab.provisional=1 AND c.mode!='screenshot'
                AND a.catalog_status!='identity_changed'
                AND NOT EXISTS(SELECT 1 FROM account_app_removals r WHERE r.account_id=a.id AND r.bundle_identifier=b.bundle_identifier)").map_err(database_error)?;
            let rows = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .map_err(database_error)?;
            rows.collect::<std::result::Result<_, _>>()
                .map_err(database_error)?
        };
        for (id, identifier) in candidates {
            let conflict: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM account_apps p JOIN accounts a ON a.id=p.account_id WHERE a.platform='android' AND p.bundle_identifier=?1 AND p.account_id!=?2)",params![identifier,id],|r|r.get(0)).map_err(database_error)?;
            if conflict || validate_identifier(&identifier).is_err() {
                tx.execute("INSERT OR IGNORE INTO account_migration_issues(platform,bundle_identifier,code) VALUES('android',?1,?2)",params![identifier,if conflict {"legacy_android_account_conflict"} else {"legacy_android_identifier_invalid"}]).map_err(database_error)?;
                continue;
            }
            tx.execute("INSERT OR IGNORE INTO account_apps(account_id,bundle_identifier,name,source) VALUES(?1,?2,?2,'retained')",params![id,identifier]).map_err(database_error)?;
        }
        tx.execute(
            "INSERT INTO account_migrations VALUES('android_legacy_bindings_v1')",
            [],
        )
        .map_err(database_error)?;
    }
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS platform_screenshot_bindings (
      store_connector_id TEXT NOT NULL REFERENCES connectors(id),
      platform TEXT NOT NULL CHECK(platform IN ('ios','android')),
      screenshot_connector_id TEXT REFERENCES connectors(id),
      explicit INTEGER NOT NULL DEFAULT 0 CHECK(explicit IN (0,1)),
      PRIMARY KEY(store_connector_id,platform));",
    )
    .map_err(database_error)?;
    let split: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM account_migrations WHERE name='platform_screenshots_v1')",
            [],
            |r| r.get(0),
        )
        .map_err(database_error)?;
    if !split {
        let stores: Vec<String> = {
            let mut statement = tx
                .prepare("SELECT id FROM connectors WHERE mode!='screenshot'")
                .map_err(database_error)?;
            let rows = statement
                .query_map([], |r| r.get(0))
                .map_err(database_error)?;
            rows.collect::<std::result::Result<_, _>>()
                .map_err(database_error)?
        };
        for store in stores {
            seed_platform_screenshots(&tx, &store)?;
        }
        tx.execute(
            "INSERT INTO account_migrations VALUES('platform_screenshots_v1')",
            [],
        )
        .map_err(database_error)?;
    }
    tx.commit().map_err(database_error)
}

fn account(db: &Connection, id: &str) -> Result<Account> {
    db.query_row("SELECT id,name,platform,store_connector_id,screenshot_connector_id,enabled,catalog_status,catalog_error,last_catalog_at,migration_error FROM accounts WHERE id=?1",[id], |r| {
        let platform: String = r.get(2)?;
        Ok(Account { id:r.get(0)?,name:r.get(1)?,platform:if platform=="ios" {Platform::Ios} else {Platform::Android},store_connector_id:r.get(3)?,screenshot_connector_id:r.get(4)?,enabled:r.get(5)?,catalog_status:r.get(6)?,catalog_error:r.get(7)?,last_catalog_at:r.get(8)?,migration_error:r.get(9)? })
    }).optional().map_err(database_error)?.ok_or_else(|| StoreError::new(404,"account_not_found","Account not found"))
}

fn valid_name(name: &str) -> Result<()> {
    if name.trim().is_empty() || name.len() > 160 {
        return Err(StoreError::new(
            422,
            "invalid_name",
            "name must contain 1–160 bytes",
        ));
    }
    Ok(())
}

fn android_directory(value: &Account) -> Result<()> {
    if value.platform != Platform::Android {
        return Err(StoreError::new(
            422,
            "android_directory_required",
            "Only Android directories support adding or removing applications",
        ));
    }
    Ok(())
}

fn node(db: &Connection, id: &str, screenshot: bool) -> Result<()> {
    let row: Option<(String, bool)> = db
        .query_row(
            "SELECT mode,enabled FROM connectors WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(database_error)?;
    match row {
        None => Err(StoreError::new(
            404,
            "connector_not_found",
            "Connector not found",
        )),
        Some((_, false)) => Err(StoreError::new(
            409,
            "connector_disabled",
            "Connector is disabled",
        )),
        Some((kind, _)) if (kind == "screenshot") != screenshot => Err(StoreError::new(
            422,
            if screenshot {
                "invalid_screenshot_connector"
            } else {
                "invalid_connector_kind"
            },
            "Select the correct Connector kind",
        )),
        _ => Ok(()),
    }
}

fn usable(value: &Account) -> Result<()> {
    if !value.enabled {
        return Err(StoreError::new(
            503,
            "account_disabled",
            "Account is disabled",
        ));
    }
    if value.catalog_status == "identity_changed" {
        return Err(StoreError::new(
            409,
            "identity_changed",
            "Administrator must accept the changed account identity",
        ));
    }
    Ok(())
}

fn store_identity(db: &Connection, id: &str) -> Result<()> {
    let kind: Option<String> = db
        .query_row("SELECT mode FROM connectors WHERE id=?1", [id], |row| {
            row.get(0)
        })
        .optional()
        .map_err(database_error)?;
    match kind.as_deref() {
        None => Err(StoreError::new(
            404,
            "connector_not_found",
            "Store Connector not found",
        )),
        // Store nodes persist their transport mode (normally "auto"), matching
        // Registry::get and node(..., false), rather than the public kind name.
        Some(kind) if kind != "screenshot" => Ok(()),
        Some(_) => Err(StoreError::new(
            422,
            "invalid_connector_kind",
            "Select a Store Connector identity",
        )),
    }
}

fn screenshot_conflict() -> StoreError {
    StoreError::new(
        409,
        "account_screenshot_conflict",
        "Configure this account platform's Screenshot Connector explicitly",
    )
}

fn legacy_explicit_screenshot(db: &Connection, store: &str) -> Result<Option<Option<String>>> {
    db.query_row("SELECT screenshot_connector_id FROM store_account_screenshot_bindings WHERE store_connector_id=?1",[store],|row|row.get(0)).optional().map_err(database_error)
}

fn explicit_screenshot(
    db: &Connection,
    store: &str,
    platform: Platform,
) -> Result<Option<Option<String>>> {
    let value = db.query_row("SELECT screenshot_connector_id,explicit FROM platform_screenshot_bindings WHERE store_connector_id=?1 AND platform=?2", params![store,platform.to_string()], |row| Ok((row.get::<_,Option<String>>(0)?,row.get::<_,bool>(1)?))).optional().map_err(database_error)?;
    match value {
        Some((selected, explicit)) => Ok((explicit || selected.is_some()).then_some(selected)),
        None => legacy_explicit_screenshot(db, store),
    }
}

/// Copy the current shared choice before either platform can be edited. Do not
/// validate availability: disabled/offline selections and explicit clears survive.
fn seed_platform_screenshots(db: &Connection, store: &str) -> Result<()> {
    let explicit = legacy_explicit_screenshot(db, store)?;
    let mut statement = db.prepare("SELECT platform,screenshot_connector_id,migration_error FROM accounts WHERE store_connector_id=?1").map_err(database_error)?;
    let rows = statement
        .query_map([store], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })
        .map_err(database_error)?;
    let accounts = rows
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(database_error)?;
    let common = accounts.first().and_then(|(_, value, _)| value.clone());
    let consistent = accounts
        .iter()
        .all(|(_, value, issue)| value == &common && issue.is_none());
    for platform in [Platform::Ios, Platform::Android] {
        let selected = if let Some(value) = &explicit {
            value.clone()
        } else if consistent {
            common.clone()
        } else {
            accounts
                .iter()
                .find(|(p, _, _)| p == &platform.to_string())
                .and_then(|(_, v, _)| v.clone())
        };
        db.execute(
            "INSERT OR IGNORE INTO platform_screenshot_bindings(store_connector_id,platform,screenshot_connector_id,explicit) VALUES(?1,?2,?3,?4)",
            params![store, platform.to_string(), selected, explicit.is_some()],
        )
        .map_err(database_error)?;
    }
    Ok(())
}

/// Configuration inheritance is independent of current node availability. A saved
/// target becoming disabled must not silently clear it when a new platform arrives.
fn inherited_screenshot(
    db: &Connection,
    store: &str,
    platform: Platform,
) -> Result<Option<String>> {
    let configured: Option<Option<String>> = db.query_row("SELECT screenshot_connector_id FROM platform_screenshot_bindings WHERE store_connector_id=?1 AND platform=?2",params![store,platform.to_string()],|row|row.get(0)).optional().map_err(database_error)?;
    if let Some(selected) = configured {
        return Ok(selected);
    }
    if let Some(configured) = explicit_screenshot(db, store, platform)? {
        return Ok(configured);
    }
    let mut statement=db.prepare("SELECT screenshot_connector_id,migration_error FROM accounts WHERE store_connector_id=?1").map_err(database_error)?;
    let values = statement
        .query_map([store], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<String>>(1)?,
            ))
        })
        .map_err(database_error)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(database_error)?;
    if values.iter().any(|(_, issue)| issue.is_some()) {
        return Err(StoreError::new(
            409,
            "legacy_screenshot_conflict",
            "Resolve the legacy screenshot configuration explicitly",
        ));
    }
    let mut settings = values.into_iter().map(|(setting, _)| setting);
    let first = settings.next().flatten();
    if settings.any(|setting| setting != first) {
        return Err(screenshot_conflict());
    }
    Ok(first)
}

fn set_group_screenshot(db: &Connection, store: &str, screenshot: Option<&str>) -> Result<usize> {
    store_identity(db, store)?;
    if let Some(id) = screenshot {
        node(db, id, true)?;
    }
    db.execute("INSERT INTO store_account_screenshot_bindings(store_connector_id,screenshot_connector_id) VALUES(?1,?2) ON CONFLICT(store_connector_id) DO UPDATE SET screenshot_connector_id=excluded.screenshot_connector_id",params![store,screenshot]).map_err(database_error)?;
    let updated = set_platform_screenshot(db, store, Platform::Ios, screenshot)?
        + set_platform_screenshot(db, store, Platform::Android, screenshot)?;
    Ok(updated)
}

fn set_platform_screenshot(
    db: &Connection,
    store: &str,
    platform: Platform,
    screenshot: Option<&str>,
) -> Result<usize> {
    store_identity(db, store)?;
    if let Some(id) = screenshot {
        node(db, id, true)?;
    }
    seed_platform_screenshots(db, store)?;
    db.execute("INSERT INTO platform_screenshot_bindings(store_connector_id,platform,screenshot_connector_id,explicit) VALUES(?1,?2,?3,1) ON CONFLICT(store_connector_id,platform) DO UPDATE SET screenshot_connector_id=excluded.screenshot_connector_id,explicit=1",params![store,platform.to_string(),screenshot]).map_err(database_error)?;
    let updated = db.execute("UPDATE accounts SET screenshot_connector_id=?3,migration_error=CASE WHEN migration_error LIKE 'legacy_screenshot_%' THEN NULL ELSE migration_error END WHERE store_connector_id=?1 AND platform=?2",params![store,platform.to_string(),screenshot]).map_err(database_error)?;
    db.execute("DELETE FROM account_migration_issues WHERE code LIKE 'legacy_screenshot_%' AND platform=?2 AND EXISTS(SELECT 1 FROM accounts a JOIN account_bindings b ON b.account_id=a.id WHERE a.store_connector_id=?1 AND a.platform=?2 AND b.platform=account_migration_issues.platform AND b.bundle_identifier=account_migration_issues.bundle_identifier)",params![store,platform.to_string()]).map_err(database_error)?;
    Ok(updated)
}

fn member(db: &Connection, id: &str, identifier: &str) -> Result<bool> {
    db.query_row(
        "SELECT EXISTS(SELECT 1 FROM account_apps WHERE account_id=?1 AND bundle_identifier=?2)",
        params![id, identifier],
        |r| r.get(0),
    )
    .map_err(database_error)
}

fn app_enabled(db: &Connection, platform: Platform, identifier: &str) -> Result<bool> {
    db.query_row("SELECT NOT COALESCE((SELECT suppressed FROM account_bindings WHERE platform=?1 AND bundle_identifier=?2),0)", params![platform.to_string(),identifier], |row| row.get(0)).map_err(database_error)
}

pub(crate) fn resolve_account(
    db: &Connection,
    platform: Platform,
    identifier: &str,
) -> Result<Account> {
    validate_identifier(identifier)?;
    let ids: Vec<String> = {
        // Availability cannot choose ownership: disabling one matching account or
        // merchant node must never make another account the implicit destination.
        let mut stmt=db.prepare("SELECT a.id FROM accounts a JOIN account_apps p ON p.account_id=a.id WHERE a.platform=?1 AND p.bundle_identifier=?2 ORDER BY a.id").map_err(database_error)?;
        let rows = stmt
            .query_map(params![platform.to_string(), identifier], |r| r.get(0))
            .map_err(database_error)?;
        rows.collect::<std::result::Result<_, _>>()
            .map_err(database_error)?
    };
    match ids.as_slice() {
        [id] => {
            if !app_enabled(db, platform, identifier)? {
                return Err(StoreError::new(
                    503,
                    "app_forwarding_disabled",
                    "Application forwarding is disabled",
                ));
            }
            let value = account(db, id)?;
            usable(&value)?;
            Ok(value)
        }
        [] => Err(StoreError::new(
            404,
            "app_not_found",
            "No verified account contains this application",
        )),
        _ => Err(StoreError::new(
            409,
            "account_ambiguous",
            "Multiple account catalogs contain this application; correct the merchant catalogs",
        )),
    }
}

pub(crate) fn resolve_screenshot(
    db: &Connection,
    platform: Platform,
    identifier: &str,
) -> Result<String> {
    let value = resolve_account(db, platform, identifier)?;
    let id = value.screenshot_connector_id.ok_or_else(|| {
        StoreError::new(
            if value.migration_error.is_some() {
                409
            } else {
                404
            },
            if value.migration_error.is_some() {
                "legacy_screenshot_conflict"
            } else {
                "screenshot_app_not_found"
            },
            "Configure the account Screenshot Connector",
        )
    })?;
    node(db, &id, true).map_err(|mut e| {
        if e.code == "connector_disabled" {
            e.status = 503;
            e.code = "screenshot_connector_disabled".into();
        }
        e
    })?;
    Ok(id)
}

pub(super) fn app_count(db: &Connection, id: &str, screenshot: bool) -> Result<u64> {
    db.query_row("WITH candidates AS (
      SELECT a.id,a.platform,p.bundle_identifier FROM accounts a JOIN account_apps p ON p.account_id=a.id),
      routes AS (
      SELECT c.platform,c.bundle_identifier,MIN(c.id) AS account_id FROM candidates c
      GROUP BY c.platform,c.bundle_identifier HAVING count(*)=1)
      SELECT count(*) FROM routes r JOIN accounts a ON a.id=r.account_id
      WHERE a.enabled=1 AND a.catalog_status!='identity_changed'
      AND NOT EXISTS(SELECT 1 FROM account_bindings b WHERE b.platform=r.platform AND b.bundle_identifier=r.bundle_identifier AND b.suppressed=1)
      AND ((?2=0 AND a.store_connector_id=?1
      AND EXISTS(SELECT 1 FROM connectors c WHERE c.id=?1 AND c.enabled=1 AND c.mode!='screenshot')) OR (?2=1 AND a.screenshot_connector_id=?1
      AND EXISTS(SELECT 1 FROM connectors c WHERE c.id=?1 AND c.enabled=1 AND c.mode='screenshot')))",params![id,screenshot],|r|r.get::<_,i64>(0)).map(|count|count as u64).map_err(database_error)
}

impl Registry {
    pub fn platform_screenshot_bindings(&self) -> Result<Vec<Value>> {
        let db = self.db.lock().unwrap();
        let mut statement = db.prepare("SELECT store_connector_id,platform,screenshot_connector_id FROM platform_screenshot_bindings ORDER BY store_connector_id,platform").map_err(database_error)?;
        let rows = statement.query_map([], |r| Ok(json!({"storeConnectorId":r.get::<_,String>(0)?,"platform":r.get::<_,String>(1)?,"connectorId":r.get::<_,Option<String>>(2)?}))).map_err(database_error)?;
        rows.collect::<std::result::Result<_, _>>()
            .map_err(database_error)
    }

    pub fn set_platform_screenshot(
        &self,
        store: &str,
        platform: Platform,
        screenshot: Option<&str>,
    ) -> Result<Value> {
        let mut db = self.db.lock().unwrap();
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let updated = set_platform_screenshot(&tx, store, platform, screenshot)?;
        tx.commit().map_err(database_error)?;
        Ok(
            json!({"storeConnectorId":store,"platform":platform,"connectorId":screenshot,"updatedAccounts":updated}),
        )
    }
    pub fn store_account_screenshot_bindings(&self) -> Result<Vec<Value>> {
        let db = self.db.lock().unwrap();
        let mut statement=db.prepare("SELECT store_connector_id,screenshot_connector_id FROM store_account_screenshot_bindings ORDER BY store_connector_id").map_err(database_error)?;
        let bindings=statement.query_map([],|row|Ok(json!({"storeConnectorId":row.get::<_,String>(0)?,"connectorId":row.get::<_,Option<String>>(1)?}))).map_err(database_error)?.collect::<std::result::Result<Vec<_>,_>>().map_err(database_error)?;
        Ok(bindings)
    }

    pub fn set_store_account_screenshot(
        &self,
        store: &str,
        screenshot: Option<&str>,
    ) -> Result<Value> {
        let mut db = self.db.lock().unwrap();
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let updated = set_group_screenshot(&tx, store, screenshot)?;
        tx.commit().map_err(database_error)?;
        Ok(json!({"storeConnectorId":store,"connectorId":screenshot,"updatedAccounts":updated}))
    }
    pub fn accounts(&self) -> Result<Vec<Account>> {
        let db = self.db.lock().unwrap();
        let mut stmt = db
            .prepare("SELECT id FROM accounts ORDER BY name,id")
            .map_err(database_error)?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(database_error)?;
        let ids = rows
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(database_error)?;
        ids.iter().map(|id| account(&db, id)).collect()
    }

    pub fn account(&self, id: &str) -> Result<Account> {
        account(&self.db.lock().unwrap(), id)
    }

    pub fn create_account(&self, input: AccountInput) -> Result<Account> {
        valid_name(&input.name)?;
        let mut db = self.db.lock().unwrap();
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        node(&tx, &input.store_connector_id, false)?;
        if let Some(id) = &input.screenshot_connector_id {
            node(&tx, id, true)?;
        }
        let exists: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM accounts WHERE platform=?1 AND store_connector_id=?2)",
                params![input.platform.to_string(), input.store_connector_id],
                |r| r.get(0),
            )
            .map_err(database_error)?;
        if exists {
            return Err(StoreError::new(
                409,
                "account_exists",
                "This platform and Store Connector already have an account",
            ));
        }
        let explicit = explicit_screenshot(&tx, &input.store_connector_id, input.platform)?;
        let inherited = inherited_screenshot(&tx, &input.store_connector_id, input.platform)?;
        let screenshot = match input.screenshot_connector_id.as_deref() {
            Some(id)
                if explicit
                    .as_ref()
                    .is_some_and(|value| value.as_deref() != Some(id))
                    || inherited.as_deref().is_some_and(|value| value != id) =>
            {
                return Err(screenshot_conflict())
            }
            Some(id) => Some(id.to_owned()),
            None => inherited,
        };
        let id = Uuid::new_v4().to_string();
        tx.execute("INSERT INTO accounts(id,name,platform,store_connector_id,screenshot_connector_id) VALUES(?1,?2,?3,?4,?5)",params![id,input.name.trim(),input.platform.to_string(),input.store_connector_id,screenshot]).map_err(database_error)?;
        if input.screenshot_connector_id.is_some() && explicit.is_none() {
            set_platform_screenshot(
                &tx,
                &input.store_connector_id,
                input.platform,
                screenshot.as_deref(),
            )?;
        }
        let result = account(&tx, &id)?;
        tx.commit().map_err(database_error)?;
        Ok(result)
    }

    pub fn patch_account(&self, id: &str, input: AccountPatch) -> Result<Account> {
        if let Some(name) = &input.name {
            valid_name(name)?;
        }
        let mut db = self.db.lock().unwrap();
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let old = account(&tx, id)?;
        if let Some(store) = &input.store_connector_id {
            node(&tx, store, false)?;
            let exists:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM accounts WHERE platform=?1 AND store_connector_id=?2 AND id!=?3)",params![old.platform.to_string(),store,id],|r|r.get(0)).map_err(database_error)?;
            if exists {
                return Err(StoreError::new(
                    409,
                    "account_exists",
                    "This platform and Store Connector already have an account",
                ));
            }
            if store != &old.store_connector_id {
                let explicit = explicit_screenshot(&tx, store, old.platform)?;
                let inherited = inherited_screenshot(&tx, store, old.platform)?;
                let screenshot = match &input.screenshot_connector_id {
                    Some(requested)
                        if explicit
                            .as_ref()
                            .is_some_and(|configured| configured != requested)
                            || inherited.as_ref().is_some_and(|configured| {
                                requested.as_ref() != Some(configured)
                            }) =>
                    {
                        return Err(screenshot_conflict());
                    }
                    Some(screenshot) => screenshot.clone(),
                    None => inherited,
                };
                tx.execute("DELETE FROM account_apps WHERE account_id=?1", [id])
                    .map_err(database_error)?;
                tx.execute(
                    "UPDATE account_bindings SET provisional=0 WHERE account_id=?1",
                    [id],
                )
                .map_err(database_error)?;
                tx.execute("UPDATE accounts SET store_connector_id=?2,screenshot_connector_id=?3,migration_error=NULL,fingerprint=NULL,pending_fingerprint=NULL,catalog_status='awaiting_catalog',catalog_error=NULL,last_catalog_at=NULL WHERE id=?1",params![id,store,screenshot]).map_err(database_error)?;
            }
        }
        if let Some(screenshot) = &input.screenshot_connector_id {
            let store = input
                .store_connector_id
                .as_deref()
                .unwrap_or(&old.store_connector_id);
            set_platform_screenshot(&tx, store, old.platform, screenshot.as_deref())?;
        }
        if let Some(name) = &input.name {
            tx.execute(
                "UPDATE accounts SET name=?2 WHERE id=?1",
                params![id, name.trim()],
            )
            .map_err(database_error)?;
        }
        if let Some(enabled) = input.enabled {
            tx.execute(
                "UPDATE accounts SET enabled=?2 WHERE id=?1",
                params![id, enabled],
            )
            .map_err(database_error)?;
        }
        if input.accept_identity_change {
            let changed=tx.execute("UPDATE accounts SET fingerprint=pending_fingerprint,pending_fingerprint=NULL,catalog_status='awaiting_catalog',catalog_error=NULL WHERE id=?1 AND pending_fingerprint IS NOT NULL",[id]).map_err(database_error)?;
            if changed == 0 {
                return Err(StoreError::new(
                    409,
                    "identity_change_not_pending",
                    "No account identity change is pending",
                ));
            }
            tx.execute("DELETE FROM account_apps WHERE account_id=?1", [id])
                .map_err(database_error)?;
            tx.execute(
                "UPDATE account_bindings SET provisional=0 WHERE account_id=?1",
                [id],
            )
            .map_err(database_error)?;
        }
        let result = account(&tx, id)?;
        tx.commit().map_err(database_error)?;
        Ok(result)
    }

    /// Only encrypted, capability-checked sessions may call this entry point.
    /// Authentication is rechecked under the same write transaction as ingestion.
    pub fn ingest_catalog(
        &self,
        node_id: &str,
        token: &str,
        capable: bool,
        report: &CatalogReport,
    ) -> Result<()> {
        report.validate()?;
        if !capable {
            return Err(StoreError::new(
                409,
                "catalog_upgrade_required",
                "Upgrade this Store Connector for account catalogs",
            ));
        }
        let authenticated = self.authenticate(node_id, token)?;
        let mut db = self.db.lock().unwrap();
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let current:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM connectors WHERE id=?1 AND nonce=?2 AND enabled=1 AND mode!='screenshot')",params![node_id,authenticated.nonce],|r|r.get(0)).map_err(database_error)?;
        if !current {
            return Err(StoreError::new(
                401,
                "invalid_node_token",
                "Invalid Store Connector credentials",
            ));
        }
        let seen:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM account_reports WHERE connector_id=?1 AND report_id=?2)",params![node_id,report.report_id],|r|r.get(0)).map_err(database_error)?;
        if seen {
            return Ok(());
        }
        let exists: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM accounts WHERE platform=?1 AND store_connector_id=?2)",
                params![report.platform.to_string(), node_id],
                |row| row.get(0),
            )
            .map_err(database_error)?;
        if !exists {
            let (screenshot, issue) = match inherited_screenshot(&tx, node_id, report.platform) {
                Ok(screenshot) => (screenshot, None),
                Err(error)
                    if matches!(
                        error.code.as_str(),
                        "legacy_screenshot_conflict" | "account_screenshot_conflict"
                    ) =>
                {
                    (None, Some("legacy_screenshot_conflict"))
                }
                Err(error) => return Err(error),
            };
            tx.execute("INSERT INTO accounts(id,name,platform,store_connector_id,screenshot_connector_id,migration_error) VALUES(?1,?2,?3,?4,?5,?6)",params![Uuid::new_v4().to_string(),authenticated.name,report.platform.to_string(),node_id,screenshot,issue]).map_err(database_error)?;
        }
        let (id,fingerprint,pending):(String,Option<String>,Option<String>)=tx.query_row("SELECT id,fingerprint,pending_fingerprint FROM accounts WHERE platform=?1 AND store_connector_id=?2",params![report.platform.to_string(),node_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(database_error)?;
        if report
            .account_fingerprint
            .as_ref()
            .is_some_and(|new| fingerprint.as_ref().is_some_and(|old| old != new))
        {
            tx.execute("UPDATE accounts SET pending_fingerprint=?2,catalog_status='identity_changed',catalog_error='identity_changed' WHERE id=?1",params![id,report.account_fingerprint]).map_err(database_error)?;
        } else if pending.is_some() {
            // Even an old-identity or failed report cannot silently dismiss a pending change.
        } else if report.status == CatalogStatus::Ready {
            tx.execute(
                "DELETE FROM account_apps WHERE account_id=?1 AND source='catalog'",
                [&id],
            )
            .map_err(database_error)?;
            for app in &report.apps {
                tx.execute("INSERT INTO account_apps(account_id,bundle_identifier,name,store_app_id)
                  SELECT ?1,?2,?3,?4 WHERE NOT EXISTS(SELECT 1 FROM account_app_removals WHERE account_id=?1 AND bundle_identifier=?2)
                  ON CONFLICT(account_id,bundle_identifier) DO NOTHING",params![id,app.bundle_identifier,app.name,app.store_app_id]).map_err(database_error)?;
            }
            tx.execute(
                "UPDATE account_bindings SET provisional=0 WHERE account_id=?1",
                [&id],
            )
            .map_err(database_error)?;
            tx.execute("UPDATE accounts SET fingerprint=?2,catalog_status='ready',catalog_error=NULL,last_catalog_at=unixepoch() WHERE id=?1",params![id,report.account_fingerprint]).map_err(database_error)?;
        } else {
            let status = match report.status {
                CatalogStatus::Failed => "failed",
                CatalogStatus::NotConfigured => "not_configured",
                CatalogStatus::Unsupported => "unsupported",
                CatalogStatus::Ready => unreachable!(),
            };
            tx.execute(
                "UPDATE accounts SET catalog_status=?2,catalog_error=?3 WHERE id=?1",
                params![id, status, report.error_code],
            )
            .map_err(database_error)?;
        }
        tx.execute(
            "INSERT INTO account_reports VALUES(?1,?2)",
            params![node_id, report.report_id],
        )
        .map_err(database_error)?;
        tx.commit().map_err(database_error)
    }

    pub fn resolve_account(&self, platform: Platform, identifier: &str) -> Result<Account> {
        resolve_account(&self.db.lock().unwrap(), platform, identifier)
    }

    pub fn bind_account(&self, platform: Platform, identifier: &str, id: &str) -> Result<Account> {
        validate_identifier(identifier)?;
        let mut db = self.db.lock().unwrap();
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let value = account(&tx, id)?;
        if value.platform != platform || !member(&tx, id, identifier)? {
            return Err(StoreError::new(
                422,
                "account_membership_unverified",
                "Select a verified account containing this platform and application",
            ));
        }
        // Legacy binding requests may confirm ownership, but cannot pin a route,
        // resolve a catalog conflict, or re-enable a disabled application.
        let selected = resolve_account(&tx, platform, identifier)?;
        if selected.id != id {
            return Err(StoreError::new(
                409,
                "app_catalog_changed",
                "Application ownership has changed",
            ));
        }
        tx.commit().map_err(database_error)?;
        Ok(value)
    }

    pub fn account_apps(&self, id: &str) -> Result<Vec<Value>> {
        let db = self.db.lock().unwrap();
        let value = account(&db, id)?;
        let apps: Vec<(String, String, Option<String>, String)> = {
            let mut stmt=db.prepare("SELECT bundle_identifier,name,store_app_id,source FROM account_apps WHERE account_id=?1 ORDER BY bundle_identifier").map_err(database_error)?;
            let rows = stmt
                .query_map([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
                .map_err(database_error)?;
            rows.collect::<std::result::Result<_, _>>()
                .map_err(database_error)?
        };
        apps.into_iter().map(|(identifier,name,store_app_id,source)| {
            let enabled = app_enabled(&db, value.platform, &identifier)?;
            let resolution=match resolve_account(&db,value.platform,&identifier) {Ok(_)=>"resolved".to_owned(),Err(e)=>e.code};
            // Use execution's resolver rather than inferring per-app availability
            // from the account setting (legacy DELETE can suppress one app).
            let screenshot_resolution = if resolution == "resolved" {
                match resolve_screenshot(&db, value.platform, &identifier) {
                    Ok(_) => "resolved".to_owned(),
                    Err(error) => error.code,
                }
            } else {
                resolution.clone()
            };
            Ok(json!({"platform":value.platform,"bundleIdentifier":identifier,"name":name,"storeAppId":store_app_id,"accountId":id,"source":source,"enabled":enabled,"resolutionStatus":resolution,"screenshotEnabled":screenshot_resolution=="resolved","screenshotResolutionStatus":screenshot_resolution}))
        }).collect()
    }

    pub fn add_android_app(&self, id: &str, identifier: &str, name: Option<&str>) -> Result<()> {
        validate_identifier(identifier)?;
        let name = name.map(str::trim).filter(|s| !s.is_empty());
        if let Some(name) = name {
            valid_name(name)?;
        }
        let name = name.unwrap_or(identifier);
        let mut db = self.db.lock().unwrap();
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let value = account(&tx, id)?;
        android_directory(&value)?;
        usable(&value)?;
        if member(&tx, id, identifier)? {
            return Err(StoreError::new(
                409,
                "app_exists",
                "Application already exists in this directory",
            ));
        }
        let other: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM account_apps p JOIN accounts a ON a.id=p.account_id WHERE a.platform='android' AND p.bundle_identifier=?1)", [identifier], |r| r.get(0)).map_err(database_error)?;
        if other {
            return Err(StoreError::new(
                409,
                "account_ambiguous",
                "Application already exists in another Android account",
            ));
        }
        tx.execute("INSERT INTO account_apps(account_id,bundle_identifier,name,source) VALUES(?1,?2,?3,'manual')", params![id,identifier,name]).map_err(database_error)?;
        tx.execute(
            "DELETE FROM account_app_removals WHERE account_id=?1 AND bundle_identifier=?2",
            params![id, identifier],
        )
        .map_err(database_error)?;
        tx.commit().map_err(database_error)
    }

    pub fn remove_android_app(&self, id: &str, identifier: &str) -> Result<()> {
        validate_identifier(identifier)?;
        let mut db = self.db.lock().unwrap();
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        android_directory(&account(&tx, id)?)?;
        let removed = tx
            .execute(
                "DELETE FROM account_apps WHERE account_id=?1 AND bundle_identifier=?2",
                params![id, identifier],
            )
            .map_err(database_error)?;
        let prior: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM account_app_removals WHERE account_id=?1 AND bundle_identifier=?2)", params![id,identifier], |r| r.get(0)).map_err(database_error)?;
        if removed == 0 && !prior {
            return Err(StoreError::new(
                404,
                "app_not_found",
                "Application not found in this directory",
            ));
        }
        tx.execute(
            "INSERT OR IGNORE INTO account_app_removals VALUES(?1,?2)",
            params![id, identifier],
        )
        .map_err(database_error)?;
        tx.commit().map_err(database_error)
    }

    pub fn set_app_enabled(
        &self,
        id: &str,
        platform: Platform,
        identifier: &str,
        enabled: bool,
    ) -> Result<()> {
        validate_identifier(identifier)?;
        let mut db = self.db.lock().unwrap();
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let value = account(&tx, id)?;
        if value.platform != platform || !member(&tx, id, identifier)? {
            return Err(StoreError::new(
                409,
                "app_catalog_changed",
                "Reload the account catalog before changing application forwarding",
            ));
        }
        tx.execute("INSERT INTO account_bindings(platform,bundle_identifier,suppressed) VALUES(?1,?2,?3) ON CONFLICT(platform,bundle_identifier) DO UPDATE SET suppressed=excluded.suppressed",params![platform.to_string(),identifier,!enabled]).map_err(database_error)?;
        tx.commit().map_err(database_error)
    }

    pub fn account_migration_issues(&self) -> Result<Vec<Value>> {
        let db = self.db.lock().unwrap();
        let mut stmt=db.prepare("SELECT platform,bundle_identifier,code FROM account_migration_issues ORDER BY platform,bundle_identifier").map_err(database_error)?;
        let rows=stmt.query_map([],|r|Ok(json!({"platform":r.get::<_,String>(0)?,"bundleIdentifier":r.get::<_,String>(1)?,"code":r.get::<_,String>(2)?}))).map_err(database_error)?;
        rows.collect::<std::result::Result<_, _>>()
            .map_err(database_error)
    }

    pub(crate) fn bind_account_screenshot(
        &self,
        platform: Platform,
        identifier: &str,
        node_id: &str,
    ) -> Result<Connector> {
        validate_identifier(identifier)?;
        {
            let mut db = self.db.lock().unwrap();
            let tx = db
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(database_error)?;
            node(&tx, node_id, true)?;
            let value = resolve_account(&tx, platform, identifier)?;
            if !member(&tx, &value.id, identifier)? {
                return Err(StoreError::new(
                    422,
                    "account_membership_unverified",
                    "Screenshot bindings require verified account membership",
                ));
            }
            if explicit_screenshot(&tx, &value.store_connector_id, platform)?
                .is_some_and(|configured| configured.as_deref() != Some(node_id))
                || inherited_screenshot(&tx, &value.store_connector_id, platform)?
                    .as_deref()
                    .is_some_and(|configured| configured != node_id)
            {
                return Err(screenshot_conflict());
            }
            set_platform_screenshot(&tx, &value.store_connector_id, platform, Some(node_id))?;
            tx.execute("INSERT INTO account_bindings(platform,bundle_identifier) VALUES(?1,?2) ON CONFLICT(platform,bundle_identifier) DO UPDATE SET screenshot_disabled=0",params![platform.to_string(),identifier]).map_err(database_error)?;
            tx.execute("INSERT INTO screenshot_app_bindings(platform,bundle_identifier,connector_id) VALUES(?1,?2,?3) ON CONFLICT(platform,bundle_identifier) DO UPDATE SET connector_id=excluded.connector_id",params![platform.to_string(),identifier,node_id]).map_err(database_error)?;
            tx.commit().map_err(database_error)?;
        }
        self.get(node_id)
    }
}
