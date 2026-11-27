//! Stable installed bootstrap: stage, quiesce, replace, confirm and recover independently of worker.
use super::{binary, SignedCommand, Trust};
use crate::{upgrade_os, ConnectorConfig};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

type Result<T> = std::result::Result<T, String>;
const MARKER: &str = ".store-update-owner.json";
const FILE_LIMIT: u64 = 2 * 1024 * 1024;
fn plain(path: &Path) -> Result<fs::Metadata> {
    let m = fs::symlink_metadata(path).map_err(|_| "update_file_unavailable")?;
    if m.file_type().is_symlink() {
        return Err("update_unsafe_path".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if m.file_attributes() & 0x400 != 0 {
            return Err("update_unsafe_path".into());
        }
    }
    Ok(m)
}
fn read(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let m = plain(path)?;
    if !m.is_file() || m.len() > limit {
        return Err("update_invalid_file".into());
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|_| "update_file_unavailable")?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "update_read_failed")?;
    if bytes.len() as u64 > limit {
        return Err("update_invalid_file".into());
    }
    Ok(bytes)
}
fn private_dir(path: &Path) -> Result<()> {
    if !path.exists() {
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder
            .create(path)
            .map_err(|_| "update_directory_failed")?;
    }
    let m = plain(path)?;
    if !m.is_dir() {
        return Err("update_unsafe_path".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if m.uid() != unsafe { libc::geteuid() } || m.mode() & 0o077 != 0 {
            return Err("update_unsafe_permissions".into());
        }
    }
    Ok(())
}
fn atomic(path: &Path, value: &Value, public: bool) -> Result<()> {
    let temp = path.with_extension("new");
    if temp.exists() {
        let m = plain(&temp)?;
        if !m.is_file() {
            return Err("update_unsafe_path".into());
        }
        fs::remove_file(&temp).map_err(|_| "update_write_failed")?;
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(if public { 0o644 } else { 0o600 });
    }
    let _ = public;
    let mut file = options.open(&temp).map_err(|_| "update_write_failed")?;
    file.write_all(&serde_json::to_vec(value).map_err(|_| "update_write_failed")?)
        .map_err(|_| "update_write_failed")?;
    file.sync_all().map_err(|_| "update_write_failed")?;
    fs::rename(&temp, path).map_err(|_| "update_write_failed")?;
    #[cfg(unix)]
    {
        File::open(path.parent().ok_or("update_unsafe_path")?)
            .and_then(|f| f.sync_all())
            .map_err(|_| "update_write_failed")?;
    }
    Ok(())
}
fn normalize(url: &str) -> Result<String> {
    let mut url = reqwest::Url::parse(url).map_err(|_| "invalid_update_trust")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("invalid_update_trust".into());
    }
    url.set_scheme("https")
        .map_err(|_| "invalid_update_trust")?;
    Ok(url.as_str().trim_end_matches('/').to_owned())
}
pub fn validate_trust_files(config: &Path, trust: &Path) -> Result<Trust> {
    let value: Value =
        serde_json::from_slice(&read(config, FILE_LIMIT)?).map_err(|_| "invalid_update_trust")?;
    let trust: Trust =
        serde_json::from_slice(&read(trust, 8192)?).map_err(|_| "invalid_update_trust")?;
    if trust.protocol != 1
        || !store_update::valid_id(&trust.node_id)
        || value["connectorId"] != trust.node_id
        || trust.target != store_update::target()
        || trust.target == "linux-amd64"
        || normalize(value["serverUrl"].as_str().ok_or("invalid_update_trust")?)?
            != trust.server_url.trim_end_matches('/')
        || !trust.server_url.starts_with("https://")
        || !store_update::valid_public_key(&trust.public_key)
    {
        return Err("invalid_update_trust".into());
    }
    Ok(trust)
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Owner {
    schema: u32,
    node_id: String,
    component: String,
    created_at: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CleanupIntent {
    schema: u32,
    node_id: String,
    name: String,
    owner: Owner,
}
fn cleanup_intent(path: &Path) -> Result<PathBuf> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| ["candidate", "retired", "program"].contains(n))
        .ok_or("update_unsafe_path")?;
    Ok(path.with_file_name(format!(".{name}-cleanup.json")))
}
fn mark(path: &Path, node: &str) -> Result<()> {
    atomic(
        &path.join(MARKER),
        &json!({"schema":1,"nodeId":node,"component":"store","createdAt":store_update::now()}),
        false,
    )
}
fn check_mark(path: &Path, node: &str) -> Result<()> {
    let m: Owner = serde_json::from_slice(&read(&path.join(MARKER), 8192)?)
        .map_err(|_| "update_unknown_owner")?;
    if m.schema != 1 || m.node_id != node || m.component != "store" {
        return Err("update_unknown_owner".into());
    }
    Ok(())
}
fn walk(path: &Path, device: u64, files: &mut usize, bytes: &mut u64) -> Result<()> {
    let m = plain(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if m.dev() != device {
            return Err("update_cross_device".into());
        }
    }
    #[cfg(not(unix))]
    let _ = device;
    *files += 1;
    *bytes = bytes.saturating_add(m.len());
    if *files > 32 || *bytes > store_update::MAX_UNPACKED + store_update::MAX_PACKAGE + 1024 * 1024
    {
        return Err("update_cleanup_limit".into());
    }
    if m.is_dir() {
        for e in fs::read_dir(path).map_err(|_| "update_cleanup_failed")? {
            walk(
                &e.map_err(|_| "update_cleanup_failed")?.path(),
                device,
                files,
                bytes,
            )?;
        }
    } else if !m.is_file() {
        return Err("update_unsafe_path".into());
    }
    Ok(())
}
fn remove_owned(path: &Path, node: &str) -> Result<()> {
    remove_owned_with(path, node, &mut |path, directory| {
        if directory {
            fs::remove_dir(path)
        } else {
            fs::remove_file(path)
        }
    })
}
fn clear_children(
    path: &Path,
    top: bool,
    remove: &mut dyn FnMut(&Path, bool) -> std::io::Result<()>,
) -> Result<()> {
    for entry in fs::read_dir(path).map_err(|_| "update_cleanup_pending")? {
        let entry = entry.map_err(|_| "update_cleanup_pending")?;
        // Keep the top ownership proof until every other entry has been removed.
        if top && entry.file_name() == MARKER {
            continue;
        }
        let child = entry.path();
        let metadata = plain(&child)?;
        if metadata.is_dir() {
            clear_children(&child, false, remove)?;
        }
        remove(&child, metadata.is_dir()).map_err(|_| "update_cleanup_pending")?;
    }
    Ok(())
}
fn remove_owned_with(
    path: &Path,
    node: &str,
    remove: &mut dyn FnMut(&Path, bool) -> std::io::Result<()>,
) -> Result<()> {
    let intent_path = cleanup_intent(path)?;
    let intent = match fs::symlink_metadata(&intent_path) {
        Ok(_) => {
            let intent: CleanupIntent = serde_json::from_slice(&read(&intent_path, 8192)?)
                .map_err(|_| "update_unknown_owner")?;
            if intent.schema != 1
                || intent.node_id != node
                || path.file_name().and_then(|n| n.to_str()) != Some(intent.name.as_str())
                || intent.owner.schema != 1
                || intent.owner.node_id != node
                || intent.owner.component != "store"
            {
                return Err("update_unknown_owner".into());
            }
            Some(intent)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => return Err("update_file_unavailable".into()),
    };
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if intent.is_some() {
                fs::remove_file(&intent_path).map_err(|_| "update_cleanup_pending")?;
            }
            return Ok(());
        }
        Err(_) => return Err("update_file_unavailable".into()),
        Ok(_) => {}
    }
    // The only marker-less recovery allowed by the durable intent is an empty
    // ordinary directory at this exact fixed name. Never adopt files into ownership.
    if intent.is_some()
        && fs::symlink_metadata(path.join(MARKER))
            .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
    {
        if !plain(path)?.is_dir()
            || fs::read_dir(path)
                .map_err(|_| "update_cleanup_pending")?
                .next()
                .is_some()
        {
            return Err("update_unknown_owner".into());
        }
        remove(path, true).map_err(|_| "update_cleanup_pending")?;
        fs::remove_file(intent_path).map_err(|_| "update_cleanup_pending")?;
        return Ok(());
    }
    check_mark(path, node)?;
    #[cfg(unix)]
    let device = {
        use std::os::unix::fs::MetadataExt;
        plain(path)?.dev()
    };
    #[cfg(not(unix))]
    let device = 0;
    walk(path, device, &mut 0, &mut 0)?;
    let owner: Owner = serde_json::from_slice(&read(&path.join(MARKER), 8192)?)
        .map_err(|_| "update_unknown_owner")?;
    if intent.as_ref().is_some_and(|intent| intent.owner != owner) {
        return Err("update_unknown_owner".into());
    }
    if intent.is_none() {
        atomic(
            &intent_path,
            &serde_json::to_value(CleanupIntent {
                schema: 1,
                node_id: node.into(),
                name: path.file_name().unwrap().to_str().unwrap().into(),
                owner: owner.clone(),
            })
            .map_err(|_| "update_write_failed")?,
            false,
        )?;
    }
    clear_children(path, true, remove)?;
    remove(&path.join(MARKER), false).map_err(|_| "update_cleanup_pending")?;
    if remove(path, true).is_err() {
        // The durable sibling intent also survives a crash in the marker/rmdir gap.
        return Err("update_cleanup_pending".into());
    }
    fs::remove_file(intent_path).map_err(|_| "update_cleanup_pending".into())
}
fn program_hashes(path: &Path) -> Result<BTreeMap<String, String>> {
    if !plain(path)?.is_dir() {
        return Err("update_invalid_program".into());
    }
    let mut hashes = BTreeMap::new();
    for e in fs::read_dir(path).map_err(|_| "update_invalid_program")? {
        let e = e.map_err(|_| "update_invalid_program")?;
        let name = e
            .file_name()
            .to_str()
            .ok_or("update_invalid_program")?
            .to_owned();
        if name == MARKER {
            continue;
        }
        if ![binary(), "start.cmd", "start.sh", "testflying-prod-ca.crt"].contains(&name.as_str()) {
            return Err("update_invalid_program".into());
        }
        hashes.insert(
            name,
            hex::encode(Sha256::digest(read(&e.path(), store_update::MAX_UNPACKED)?)),
        );
    }
    if !hashes.contains_key(binary()) || !hashes.contains_key("testflying-prod-ca.crt") {
        return Err("update_invalid_program".into());
    }
    Ok(hashes)
}
fn unpack(bytes: &[u8], candidate: &Path, node: &str, target: &str) -> Result<()> {
    private_dir(candidate)?;
    mark(candidate, node)?;
    let program = candidate.join("program");
    private_dir(&program)?;
    mark(&program, node)?;
    let mut zip =
        zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|_| "update_invalid_package")?;
    if !(2..=4).contains(&zip.len()) {
        return Err("update_invalid_package".into());
    }
    let bin = if target == "windows-amd64" {
        "store-connector.exe"
    } else {
        "store-connector"
    };
    let launcher = if target == "windows-amd64" {
        "start.cmd"
    } else {
        "start.sh"
    };
    let mut seen = std::collections::HashSet::new();
    let mut total = 0u64;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|_| "update_invalid_package")?;
        let name = entry.name().to_owned();
        if ![bin, launcher, "testflying-prod-ca.crt"].contains(&name.as_str())
            || !seen.insert(name.clone())
            || entry.is_dir()
            || entry
                .unix_mode()
                .is_some_and(|m| m & 0o170000 != 0 && m & 0o170000 != 0o100000)
        {
            return Err("update_invalid_package".into());
        }
        total = total
            .checked_add(entry.size())
            .ok_or("update_package_too_large")?;
        if total > store_update::MAX_UNPACKED {
            return Err("update_package_too_large".into());
        }
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(program.join(&name))
            .map_err(|_| "update_write_failed")?;
        let copied = std::io::copy(
            &mut entry.by_ref().take(store_update::MAX_UNPACKED + 1),
            &mut file,
        )
        .map_err(|_| "update_invalid_package")?;
        if copied != entry.size() {
            return Err("update_invalid_package".into());
        }
        file.sync_all().map_err(|_| "update_write_failed")?;
    }
    if !seen.contains(bin) || !seen.contains("testflying-prod-ca.crt") {
        return Err("update_invalid_package".into());
    }
    let header = read(&program.join(bin), store_update::MAX_UNPACKED)?;
    let valid = match target {
        "darwin-arm64" => header.get(..8) == Some(&[0xcf, 0xfa, 0xed, 0xfe, 0x0c, 0, 0, 1][..]),
        "windows-amd64" => header.get(0x3c..0x40).is_some_and(|bytes| {
            let offset = u32::from_le_bytes(bytes.try_into().unwrap()) as usize;
            header.starts_with(b"MZ")
                && offset.checked_add(6).is_some_and(|end| {
                    header.get(offset..end) == Some(&[b'P', b'E', 0, 0, 0x64, 0x86][..])
                })
        }),
        _ => false,
    };
    if !valid {
        return Err("update_wrong_architecture".into());
    }
    reqwest::Certificate::from_pem(&read(&program.join("testflying-prod-ca.crt"), 256 * 1024)?)
        .map_err(|_| "update_invalid_certificate")?;
    Ok(())
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Transaction {
    command: SignedCommand,
    phase: String,
    previous_hashes: BTreeMap<String, String>,
    previous_version: String,
    installed_hashes: BTreeMap<String, String>,
    health_since: Option<u64>,
    health_instance: Option<String>,
    reported: bool,
    error_code: Option<String>,
    health_restarts: u32,
    health_failures: u32,
}
#[derive(Clone, Copy)]
struct Services {
    stop: fn(&Path) -> Result<()>,
    start: fn(&Path) -> Result<()>,
    prepare: fn(&Path, &Path) -> Result<()>,
    space: fn(&Path) -> Result<u64>,
}
struct Manager {
    root: PathBuf,
    private: PathBuf,
    trust: Trust,
    config: ConnectorConfig,
    client: reqwest::Client,
    local: String,
    services: Services,
}
impl Manager {
    fn url(&self, suffix: &str) -> String {
        format!(
            "{}/v1/connectors/{}/store-upgrades{}",
            self.trust.server_url.trim_end_matches('/'),
            self.trust.node_id,
            suffix
        )
    }
    async fn response(&self, response: reqwest::Response, limit: u64) -> Result<Vec<u8>> {
        if !response.status().is_success() {
            return Err("update_request_rejected".into());
        }
        if response.content_length().is_some_and(|n| n > limit) {
            return Err("update_response_too_large".into());
        }
        let mut response = response;
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "update_network_failed")?
        {
            if bytes.len() as u64 + chunk.len() as u64 > limit {
                return Err("update_response_too_large".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }
    async fn get(&self, url: String) -> Result<Value> {
        let response = self
            .client
            .get(url)
            .bearer_auth(&self.config.token)
            .send()
            .await
            .map_err(|_| "update_network_failed")?;
        serde_json::from_slice(&self.response(response, 128 * 1024).await?)
            .map_err(|_| "update_invalid_response".into())
    }
    async fn post(&self, url: String, value: &Value) -> Result<Value> {
        let response = self
            .client
            .post(url)
            .bearer_auth(&self.config.token)
            .json(value)
            .send()
            .await
            .map_err(|_| "update_network_failed")?;
        let bytes = self.response(response, 128 * 1024).await?;
        if bytes.is_empty() {
            Ok(json!({}))
        } else {
            serde_json::from_slice(&bytes).map_err(|_| "update_invalid_response".into())
        }
    }
    async fn status(&self, t: &Transaction, state: &str, error: Option<&str>) -> Result<()> {
        self.post(self.url(&format!("/{}/status",t.command.request.request_id)),&json!({"requestId":t.command.request.request_id,"nodeId":self.trust.node_id,"state":state,"errorCode":error.map(safe_error)})).await?;
        self.log(&t.command.request.request_id, state, error);
        Ok(())
    }
    async fn report_terminal(&self, t: &mut Transaction) -> Result<()> {
        self.status(t, &t.phase, t.error_code.as_deref()).await?;
        t.reported = true;
        self.save(t)
    }
    fn log(&self, id: &str, state: &str, error: Option<&str>) {
        let hour = store_update::now() / 3600;
        let path = self.private.join(format!("upgrade-{hour:010}.jsonl"));
        let mut options = OpenOptions::new();
        options.append(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        if let Ok(mut f) = options.open(path) {
            let _ = writeln!(
                f,
                "{}",
                json!({"timestamp":store_update::now(),"requestId":id,"state":state,"errorCode":error.map(safe_error)})
            );
        }
    }
    fn save(&self, t: &Transaction) -> Result<()> {
        atomic(
            &self.private.join("transaction.json"),
            &serde_json::to_value(t).map_err(|_| "update_write_failed")?,
            false,
        )
    }
    fn maintenance(&self, t: &Transaction) -> Result<()> {
        atomic(
            &self.root.join("updater/maintenance.json"),
            &serde_json::to_value(&t.command).map_err(|_| "update_write_failed")?,
            true,
        )
    }
    fn clear_maintenance(&self, t: &Transaction) -> Result<()> {
        let path = self.root.join("updater/maintenance.json");
        if !path.exists() {
            return Ok(());
        }
        let c: SignedCommand = serde_json::from_slice(&read(&path, 128 * 1024)?)
            .map_err(|_| "update_invalid_maintenance")?;
        if c != t.command {
            return Err("update_other_maintenance".into());
        }
        fs::remove_file(path).map_err(|_| "update_write_failed".into())
    }
    fn transaction(&self) -> Result<Option<Transaction>> {
        let p = self.private.join("transaction.json");
        if !p.exists() {
            return Ok(None);
        }
        let t: Transaction = serde_json::from_slice(&read(&p, 128 * 1024)?)
            .map_err(|_| "update_invalid_transaction")?;
        if t.command.request.node_id != self.trust.node_id
            || t.command.request.target != self.trust.target
            || ![
                "downloading",
                "draining",
                "stopping",
                "switching",
                "awaiting_confirmation",
                "succeeded",
                "failed",
                "rolled_back",
                "cleanup_pending",
            ]
            .contains(&t.phase.as_str())
        {
            return Err("update_invalid_transaction".into());
        }
        store_update::verify(
            &t.command,
            &self.trust,
            t.command.request.expires_at.saturating_sub(1),
        )?;
        Ok(Some(t))
    }
    fn installing(&self) -> bool {
        fs::read_dir(&self.root).map_or(true, |entries| {
            entries.filter_map(|e| e.ok()).any(|e| {
                let n = e.file_name();
                let n = n.to_string_lossy();
                n == ".install" || n.starts_with(".install-")
            })
        })
    }
    fn cleanup(&self) -> Result<()> {
        let transaction = self.transaction()?;
        if transaction
            .as_ref()
            .is_some_and(|t| matches!(t.phase.as_str(), "succeeded" | "cleanup_pending"))
        {
            remove_owned(&self.private.join("retired"), &self.trust.node_id)?;
        }
        if !transaction.as_ref().is_some_and(|t| {
            !matches!(
                t.phase.as_str(),
                "succeeded" | "failed" | "rolled_back" | "cleanup_pending"
            )
        }) {
            let candidate = self.private.join("candidate");
            let interrupted = fs::symlink_metadata(cleanup_intent(&candidate)?).is_ok();
            let immediate = transaction.as_ref().is_some_and(|t| {
                matches!(
                    t.phase.as_str(),
                    "succeeded" | "rolled_back" | "cleanup_pending"
                )
            });
            if interrupted || immediate {
                remove_owned(&candidate, &self.trust.node_id)?;
            } else if candidate.exists() {
                let m: Owner = serde_json::from_slice(&read(&candidate.join(MARKER), 8192)?)
                    .map_err(|_| "update_unknown_owner")?;
                if store_update::now().saturating_sub(m.created_at) > 86400 {
                    remove_owned(&candidate, &self.trust.node_id)?;
                }
            }
        }
        Ok(())
    }
    fn cleanup_logs(&self) -> Result<()> {
        for e in fs::read_dir(&self.private).map_err(|_| "update_cleanup_failed")? {
            let e = e.map_err(|_| "update_cleanup_failed")?;
            let n = e.file_name();
            let n = n.to_string_lossy();
            if let Some(hour) = n
                .strip_prefix("upgrade-")
                .and_then(|s| s.strip_suffix(".jsonl"))
                .filter(|s| s.len() == 10 && s.bytes().all(|b| b.is_ascii_digit()))
                .and_then(|s| s.parse::<u64>().ok())
            {
                if hour.saturating_add(168) <= store_update::now() / 3600
                    && plain(&e.path())?.is_file()
                {
                    fs::remove_file(e.path()).map_err(|_| "update_cleanup_failed")?;
                }
            }
        }
        Ok(())
    }
    async fn download(&self, command: SignedCommand) -> Result<Transaction> {
        store_update::verify(&command, &self.trust, store_update::now())?;
        let health = self.get(format!("{}/health", self.local)).await?;
        let previous_version = health["connectorVersion"]
            .as_str()
            .ok_or("update_worker_unavailable")?
            .to_owned();
        if store_update::compare_versions(&command.request.version, &previous_version)?
            != std::cmp::Ordering::Greater
        {
            return Err("update_not_newer".into());
        }
        let mut t = Transaction {
            command,
            phase: "downloading".into(),
            previous_hashes: program_hashes(&self.root.join("program"))?,
            previous_version,
            installed_hashes: BTreeMap::new(),
            health_since: None,
            health_instance: None,
            reported: false,
            error_code: None,
            health_restarts: 0,
            health_failures: 0,
        };
        self.save(&t)?;
        let _ = self.status(&t, "downloading", None).await;
        if (self.services.space)(&self.root)?
            < store_update::MAX_UNPACKED + store_update::MAX_PACKAGE + 512 * 1024 * 1024
        {
            return Err("update_insufficient_space".into());
        }
        let candidate = self.private.join("candidate");
        remove_owned(&candidate, &self.trust.node_id)?;
        private_dir(&candidate)?;
        mark(&candidate, &self.trust.node_id)?;
        let response = self
            .client
            .get(self.url(&format!("/{}/package", t.command.request.request_id)))
            .bearer_auth(&self.config.token)
            .send()
            .await
            .map_err(|_| "update_network_failed")?;
        let bytes = self.response(response, store_update::MAX_PACKAGE).await?;
        if bytes.len() as u64 != t.command.request.size_bytes
            || hex::encode(Sha256::digest(&bytes)) != t.command.request.sha256
        {
            return Err("update_package_hash_mismatch".into());
        }
        unpack(&bytes, &candidate, &self.trust.node_id, &self.trust.target)?;
        (self.services.prepare)(&self.root, &candidate.join("program"))?;
        t.installed_hashes = program_hashes(&candidate.join("program"))?;
        t.phase = "draining".into();
        self.save(&t)?;
        Ok(t)
    }
    async fn wait_ready(&self, t: &Transaction) -> Result<bool> {
        store_update::verify(&t.command, &self.trust, store_update::now())?;
        self.post(
            format!("{}/v1/upgrades/drain", self.local),
            &serde_json::to_value(&t.command).map_err(|_| "update_invalid_command")?,
        )
        .await?;
        let local = self
            .post(
                format!("{}/v1/upgrades/ready", self.local),
                &json!({"requestId":t.command.request.request_id}),
            )
            .await?;
        if local["ready"] != true {
            return Ok(false);
        }
        let result = self
            .post(
                self.url(&format!("/{}/ready", t.command.request.request_id)),
                &json!({"completedJobIds":local["completedJobIds"]}),
            )
            .await?;
        if result["ready"] != true {
            return Ok(false);
        }
        self.maintenance(t)?;
        let local = self
            .post(
                format!("{}/v1/upgrades/freeze", self.local),
                &json!({"requestId":t.command.request.request_id}),
            )
            .await?;
        if local["ready"] != true {
            self.clear_maintenance(t)?;
            return Ok(false);
        }
        Ok(true)
    }
    async fn apply(&self, t: &mut Transaction) -> Result<()> {
        if program_hashes(&self.root.join("program"))? != t.previous_hashes {
            return Err("update_program_changed".into());
        }
        if program_hashes(&self.private.join("candidate/program"))? != t.installed_hashes {
            return Err("update_candidate_changed".into());
        }
        self.status(t, "installing", None).await?;
        t.phase = "stopping".into();
        self.save(t)?;
        (self.services.stop)(&self.root)?;
        t.phase = "switching".into();
        self.save(t)?;
        let previous = self.private.join("previous");
        let retired = self.private.join("retired");
        remove_owned(&retired, &self.trust.node_id)?;
        if previous.exists() {
            check_mark(&previous, &self.trust.node_id)?;
            fs::rename(&previous, &retired).map_err(|_| "update_switch_failed")?;
        }
        mark(&self.root.join("program"), &self.trust.node_id)?;
        fs::rename(self.root.join("program"), &previous).map_err(|_| "update_switch_failed")?;
        fs::rename(
            self.private.join("candidate/program"),
            self.root.join("program"),
        )
        .map_err(|_| "update_switch_failed")?;
        (self.services.prepare)(&self.root, &self.root.join("program"))?;
        t.phase = "awaiting_confirmation".into();
        t.health_since = None;
        t.health_instance = None;
        self.save(t)?;
        (self.services.start)(&self.root)?;
        let _ = self.status(t, "awaiting_confirmation", None).await;
        Ok(())
    }
    async fn confirm(&self, t: &mut Transaction) -> Result<bool> {
        let health = self.get(format!("{}/health", self.local)).await;
        if health.as_ref().is_ok_and(|v| {
            v["connectorVersion"] == t.command.request.version
                && v["instanceId"].as_str().is_some_and(store_update::valid_id)
        }) {
            let instance = health.as_ref().unwrap()["instanceId"]
                .as_str()
                .unwrap()
                .to_owned();
            if t.health_instance
                .as_deref()
                .is_some_and(|old| old != instance)
            {
                t.health_restarts = t.health_restarts.saturating_add(1);
            }
            if t.health_since.is_none() || t.health_instance.as_deref() != Some(instance.as_str()) {
                t.health_since = Some(store_update::now());
                t.health_instance = Some(instance);
                self.save(t)?;
            }
            if t.health_restarts >= 3 {
                return Err("update_crash_loop".into());
            }
            let remote = self
                .get(self.url(&format!(
                    "/{}/confirm?version={}",
                    t.command.request.request_id, t.command.request.version
                )))
                .await?;
            if remote["confirmed"] != true
                && store_update::now().saturating_sub(t.health_since.unwrap_or(store_update::now()))
                    >= 90
            {
                return Err("update_connection_unhealthy".into());
            }
            if remote["confirmed"] == true
                && store_update::now().saturating_sub(t.health_since.unwrap_or(store_update::now()))
                    >= 30
            {
                self.clear_maintenance(t)?;
                if remove_owned(&self.private.join("retired"), &self.trust.node_id).is_err()
                    || remove_owned(&self.private.join("candidate"), &self.trust.node_id).is_err()
                {
                    t.phase = "cleanup_pending".into();
                    self.save(t)?;
                    let _ = self
                        .status(t, "cleanup_pending", Some("update_cleanup_pending"))
                        .await;
                } else {
                    t.phase = "succeeded".into();
                    self.save(t)?;
                    let _ = self.report_terminal(t).await;
                }
                return Ok(true);
            }
            return Ok(false);
        }
        t.health_since = None;
        t.health_failures = t.health_failures.saturating_add(1);
        self.save(t)?;
        Err("update_worker_unhealthy".into())
    }
    async fn rollback(&self, t: &mut Transaction, error: &str) -> Result<()> {
        let program = self.root.join("program");
        let previous = self.private.join("previous");
        let program_present = match fs::symlink_metadata(&program) {
            Ok(_) => true,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(_) => return Err("update_file_unavailable".into()),
        };
        if program_present {
            let known = program_hashes(&program)
                .is_ok_and(|hashes| hashes == t.previous_hashes || hashes == t.installed_hashes);
            if !known {
                self.clear_maintenance(t)?;
                t.phase = "failed".into();
                t.error_code = Some("update_program_changed".into());
                self.save(t)?;
                let _ = self
                    .post(
                        format!("{}/v1/upgrades/cancel", self.local),
                        &json!({"requestId":t.command.request.request_id}),
                    )
                    .await;
                let _ = self.report_terminal(t).await;
                return Ok(());
            }
        }
        if previous.exists() && program_hashes(&previous)? == t.previous_hashes {
            (self.services.stop)(&self.root)?;
            let failed = self.private.join("candidate/program");
            if program.exists() {
                if failed.exists() {
                    remove_owned(&failed, &self.trust.node_id)?;
                }
                check_mark(&program, &self.trust.node_id)?;
                fs::rename(&program, &failed).map_err(|_| "update_rollback_failed")?;
            }
            fs::rename(&previous, &program).map_err(|_| "update_rollback_failed")?;
            self.clear_maintenance(t)?;
            (self.services.prepare)(&self.root, &program)?;
            (self.services.start)(&self.root)?;
            let retired = self.private.join("retired");
            if retired.exists() {
                fs::rename(retired, previous).map_err(|_| "update_rollback_failed")?;
            }
            t.phase = "rolled_back".into();
            t.error_code = Some(error.into());
            self.save(t)?;
            let _ = self.report_terminal(t).await;
            let _ = remove_owned(&self.private.join("candidate"), &self.trust.node_id);
        } else if program.exists() && program_hashes(&program)? == t.previous_hashes {
            self.clear_maintenance(t)?;
            let _ = self
                .post(
                    format!("{}/v1/upgrades/cancel", self.local),
                    &json!({"requestId":t.command.request.request_id}),
                )
                .await;
            (self.services.start)(&self.root)?;
            t.phase = "failed".into();
            t.error_code = Some(error.into());
            self.save(t)?;
            let _ = self.report_terminal(t).await;
        } else {
            return Err("update_rollback_unknown".into());
        }
        Ok(())
    }
    async fn step(&self) -> Result<()> {
        if self.installing() {
            return Ok(());
        }
        // Log retention is independent of transaction progress or center availability.
        // A locked log must never suppress delivery of the terminal outbox.
        if let Err(code) = self.cleanup_logs() {
            self.log("", "cleanup_pending", Some(&code));
        }
        if let Some(mut t) = self.transaction()? {
            if matches!(t.phase.as_str(), "succeeded" | "failed" | "rolled_back")
                && !t.reported
                && self.report_terminal(&mut t).await.is_err()
            {
                return Ok(());
            }
            if matches!(t.phase.as_str(), "succeeded" | "failed" | "rolled_back") {
                self.clear_maintenance(&t)?;
            }
            match t.phase.as_str() {
                "cleanup_pending" => {
                    remove_owned(&self.private.join("candidate"), &self.trust.node_id)?;
                    remove_owned(&self.private.join("retired"), &self.trust.node_id)?;
                    t.phase = "succeeded".into();
                    t.reported = false;
                    self.save(&t)?;
                    let _ = self.report_terminal(&mut t).await;
                    return Ok(());
                }
                "stopping" | "switching" => {
                    return self.rollback(&mut t, "update_interrupted").await
                }
                "awaiting_confirmation" => {
                    if !program_hashes(&self.root.join("program"))
                        .is_ok_and(|hashes| hashes == t.installed_hashes)
                    {
                        return self.rollback(&mut t, "update_program_changed").await;
                    }
                    return match self.confirm(&mut t).await {
                        Err(code)
                            if code == "update_crash_loop"
                                || code == "update_connection_unhealthy"
                                || t.health_failures >= 12 =>
                        {
                            self.rollback(&mut t, &code).await
                        }
                        _ => Ok(()),
                    };
                }
                "draining" => {
                    if t.command.request.expires_at <= store_update::now() {
                        let _ = self
                            .post(
                                format!("{}/v1/upgrades/cancel", self.local),
                                &json!({"requestId":t.command.request.request_id}),
                            )
                            .await;
                        self.clear_maintenance(&t)?;
                        t.phase = "failed".into();
                        t.error_code = Some("update_expired".into());
                        self.save(&t)?;
                        let _ = self.report_terminal(&mut t).await;
                        return Ok(());
                    }
                    if self.wait_ready(&t).await? {
                        if let Err(code) = self.apply(&mut t).await {
                            self.rollback(&mut t, &code).await?;
                        }
                    }
                    return Ok(());
                }
                "downloading" => {
                    // A failed or interrupted download never stops the old worker.
                    let _ = self
                        .post(
                            format!("{}/v1/upgrades/cancel", self.local),
                            &json!({"requestId":t.command.request.request_id}),
                        )
                        .await;
                    t.phase = "failed".into();
                    t.error_code = Some("update_download_interrupted".into());
                    self.save(&t)?;
                    let _ = self.report_terminal(&mut t).await;
                    return Ok(());
                }
                _ => {}
            }
        }
        self.cleanup()?;
        let pending = self.get(self.url("/pending")).await?;
        if pending["command"].is_null() {
            return Ok(());
        }
        let command: SignedCommand = serde_json::from_value(pending["command"].clone())
            .map_err(|_| "update_invalid_command")?;
        store_update::verify(&command, &self.trust, store_update::now())?;
        if self
            .transaction()?
            .as_ref()
            .is_some_and(|t| t.command.request.request_id == command.request.request_id)
        {
            return Ok(());
        }
        match self.download(command.clone()).await {
            Ok(t) => {
                let _ = self.status(&t, "draining", None).await;
            }
            Err(code) => {
                let _ = self
                    .post(
                        format!("{}/v1/upgrades/cancel", self.local),
                        &json!({"requestId":command.request.request_id}),
                    )
                    .await;
                let mut t = self
                    .transaction()?
                    .filter(|t| t.command.request.request_id == command.request.request_id)
                    .unwrap_or(Transaction {
                        command,
                        phase: "failed".into(),
                        previous_hashes: BTreeMap::new(),
                        previous_version: String::new(),
                        installed_hashes: BTreeMap::new(),
                        health_since: None,
                        health_instance: None,
                        reported: false,
                        error_code: None,
                        health_restarts: 0,
                        health_failures: 0,
                    });
                t.phase = "failed".into();
                t.error_code = Some(code);
                self.save(&t)?;
                let _ = self.report_terminal(&mut t).await;
            }
        }
        Ok(())
    }
}
fn safe_error(value: &str) -> &str {
    if !value.is_empty()
        && value.len() <= 64
        && value.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
    {
        value
    } else {
        "update_operation_failed"
    }
}
pub async fn run_manager() -> Result<()> {
    let root = upgrade_os::installed_root()?;
    upgrade_os::ensure_manager_identity(&root)?;
    let updater = root.join("updater");
    let private = updater.join("private");
    private_dir(&private)?;
    let lockpath = updater.join("manager.lock");
    if lockpath.exists() && !plain(&lockpath)?.is_file() {
        return Err("update_unsafe_path".into());
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = options
        .open(&lockpath)
        .map_err(|_| "update_lock_unavailable")?;
    lock.try_lock().map_err(|_| "update_already_running")?;
    let trust = validate_trust_files(
        &root.join("data/connector.json"),
        &updater.join("trust.json"),
    )?;
    let config: ConnectorConfig =
        serde_json::from_slice(&read(&root.join("data/connector.json"), FILE_LIMIT)?)
            .map_err(|_| "update_invalid_config")?;
    let ca =
        reqwest::Certificate::from_pem(&read(&updater.join("testflying-prod-ca.crt"), 256 * 1024)?)
            .map_err(|_| "invalid_update_trust")?;
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .add_root_certificate(ca)
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(100))
        .build()
        .map_err(|_| "update_client_failed")?;
    let port = config
        .listen_addr
        .rsplit(':')
        .next()
        .and_then(|s| s.parse::<u16>().ok())
        .filter(|p| *p > 0)
        .ok_or("update_invalid_config")?;
    let manager = Manager {
        root,
        private,
        trust,
        config,
        client,
        local: format!("http://127.0.0.1:{port}"),
        services: Services {
            stop: upgrade_os::stop_worker,
            start: upgrade_os::start_worker,
            prepare: upgrade_os::prepare_program,
            space: available_space,
        },
    };
    loop {
        if let Err(code) = manager.step().await {
            manager.log("", "manager_waiting", Some(&code));
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

fn available_space(path: &Path) -> Result<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let path = std::ffi::CString::new(path.as_os_str().as_bytes())
            .map_err(|_| "update_unsafe_path")?;
        let mut info = std::mem::MaybeUninit::<libc::statvfs>::uninit();
        if unsafe { libc::statvfs(path.as_ptr(), info.as_mut_ptr()) } != 0 {
            return Err("update_space_unavailable".into());
        }
        let info = unsafe { info.assume_init() };
        // Counter widths differ between macOS and Linux statvfs definitions.
        #[allow(clippy::unnecessary_cast)]
        let available = (info.f_bavail as u64).saturating_mul(info.f_frsize as u64);
        Ok(available)
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetDiskFreeSpaceExW(
                directory: *const u16,
                available: *mut u64,
                total: *mut u64,
                free: *mut u64,
            ) -> i32;
        }
        let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut available = 0;
        if unsafe {
            GetDiskFreeSpaceExW(
                path.as_ptr(),
                &mut available,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err("update_space_unavailable".into());
        }
        Ok(available)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        Err("unsupported_installation".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        extract::{Path as AxumPath, State as AxumState},
        routing::{get, post},
        Json, Router,
    };
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    };
    struct Fixture {
        root: PathBuf,
        command: Mutex<Option<SignedCommand>>,
        package: Vec<u8>,
        ready: AtomicBool,
        reports: Mutex<Vec<String>>,
        network: AtomicBool,
    }
    fn fake_stop(root: &Path) -> Result<()> {
        fs::write(root.join("running"), b"no").map_err(|_| "fixture_stop".into())
    }
    fn fake_start(root: &Path) -> Result<()> {
        let bytes = read(
            &root.join("program").join(binary()),
            store_update::MAX_UNPACKED,
        )?;
        if bytes.ends_with(b"crash") {
            return Err("update_worker_unhealthy".into());
        }
        fs::write(root.join("running"), b"yes").map_err(|_| "fixture_start")?;
        fs::write(root.join("instance"), uuid::Uuid::new_v4().to_string())
            .map_err(|_| "fixture_start".into())
    }
    fn fake_prepare(_: &Path, path: &Path) -> Result<()> {
        program_hashes(path).map(|_| ())
    }
    fn fake_space(_: &Path) -> Result<u64> {
        Ok(4 * 1024 * 1024 * 1024)
    }
    fn fake_binary(version: &str) -> Vec<u8> {
        let mut bytes = if cfg!(windows) {
            let mut b = vec![0; 128];
            b[..2].copy_from_slice(b"MZ");
            b[0x3c..0x40].copy_from_slice(&64u32.to_le_bytes());
            b[64..70].copy_from_slice(&[b'P', b'E', 0, 0, 0x64, 0x86]);
            b
        } else {
            vec![0xcf, 0xfa, 0xed, 0xfe, 0x0c, 0, 0, 1]
        };
        bytes.extend_from_slice(version.as_bytes());
        bytes
    }
    async fn fixture(
        next: &str,
    ) -> (
        tempfile::TempDir,
        Manager,
        Arc<Fixture>,
        tokio::task::JoinHandle<()>,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        private_dir(&root.join("updater")).unwrap();
        private_dir(&root.join("updater/private")).unwrap();
        private_dir(&root.join("program")).unwrap();
        private_dir(&root.join("data")).unwrap();
        private_dir(&root.join("data/credentials")).unwrap();
        fs::write(root.join("data/connector.json"), b"private-config").unwrap();
        fs::write(root.join("data/credentials/key"), b"private-key").unwrap();
        let ca = include_bytes!("../../../../deploy/testflying-prod-ca.crt");
        fs::write(root.join("program").join(binary()), fake_binary("1.0.0")).unwrap();
        fs::write(root.join("program/testflying-prod-ca.crt"), ca).unwrap();
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        zip.start_file(binary(), zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(&fake_binary(next)).unwrap();
        zip.start_file(
            "testflying-prod-ca.crt",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(ca).unwrap();
        let package = zip.finish().unwrap().into_inner();
        let state = Arc::new(Fixture {
            root: root.clone(),
            command: Mutex::new(None),
            package,
            ready: AtomicBool::new(false),
            reports: Mutex::new(Vec::new()),
            network: AtomicBool::new(true),
        });
        fake_start(&root).unwrap();
        let app=Router::new().route("/health",get(|AxumState(s):AxumState<Arc<Fixture>>|async move{
            let running=fs::read(s.root.join("running")).unwrap_or_default()==b"yes";
            let bytes=fs::read(s.root.join("program").join(binary())).unwrap_or_default();
            let current=if bytes.ends_with(b"1.0.0"){"1.0.0"}else{"9.0.0"};
            Json(json!({"connectorVersion":if running{current}else{"stopped"},"instanceId":fs::read_to_string(s.root.join("instance")).unwrap()}))
        })).route("/v1/upgrades/drain",post(||async{Json(json!({"state":"draining"}))}))
        .route("/v1/upgrades/freeze",post(||async{Json(json!({"ready":true}))}))
        .route("/v1/upgrades/cancel",post(||async{Json(json!({"canceled":true}))}))
        .route("/v1/upgrades/ready",post(|AxumState(s):AxumState<Arc<Fixture>>|async move{Json(json!({"ready":s.ready.load(Ordering::SeqCst),"completedJobIds":[]}))}))
        .route("/v1/connectors/{node}/store-upgrades/pending",get(|AxumState(s):AxumState<Arc<Fixture>>|async move{Json(json!({"command":s.command.lock().unwrap().clone()}))}))
        .route("/v1/connectors/{node}/store-upgrades/{id}/package",get(|AxumState(s):AxumState<Arc<Fixture>>|async move{s.package.clone()}))
        .route("/v1/connectors/{node}/store-upgrades/{id}/ready",post(||async{Json(json!({"ready":true}))}))
        .route("/v1/connectors/{node}/store-upgrades/{id}/confirm",get(|AxumState(s):AxumState<Arc<Fixture>>,AxumPath((_node,_id)):AxumPath<(String,String)>|async move{
            if !s.network.load(Ordering::SeqCst){return (axum::http::StatusCode::SERVICE_UNAVAILABLE,Json(json!({})));}
            (axum::http::StatusCode::OK,Json(json!({"confirmed":true})))
        }))
        .route("/v1/connectors/{node}/store-upgrades/{id}/status",post(|AxumState(s):AxumState<Arc<Fixture>>,Json(v):Json<Value>|async move{
            if !s.network.load(Ordering::SeqCst){return (axum::http::StatusCode::SERVICE_UNAVAILABLE,Json(json!({})));}
            s.reports.lock().unwrap().push(v["state"].as_str().unwrap().into());(axum::http::StatusCode::OK,Json(json!({"accepted":true})))
        })).with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let node = "00000000-0000-4000-8000-000000000001";
        let command = store_update::sign(
            store_update::UpgradeRequest {
                protocol: 1,
                node_id: node.into(),
                request_id: "00000000-0000-4000-8000-000000000002".into(),
                version: "9.0.0".into(),
                target: if cfg!(windows) {
                    "windows-amd64"
                } else {
                    "darwin-arm64"
                }
                .into(),
                sha256: hex::encode(Sha256::digest(&state.package)),
                size_bytes: state.package.len() as u64,
                expires_at: store_update::now() + 3600,
                bootstrap_version: 1,
            },
            b"fixture-signing",
        )
        .unwrap();
        *state.command.lock().unwrap() = Some(command);
        let config=serde_json::from_value(json!({"connectorId":node,"token":"fixture-token-0123456789","serverUrl":url,"messageKey":"01".repeat(32),"uploadKey":"02".repeat(32),"provider":{"mode":"mock"}})).unwrap();
        let manager = Manager {
            root: root.clone(),
            private: root.join("updater/private"),
            trust: Trust {
                protocol: 1,
                node_id: node.into(),
                server_url: url.clone(),
                public_key: store_update::public_key(b"fixture-signing").unwrap(),
                target: if cfg!(windows) {
                    "windows-amd64"
                } else {
                    "darwin-arm64"
                }
                .into(),
            },
            config,
            client: reqwest::Client::builder()
                .no_proxy()
                .retry(reqwest::retry::never())
                .build()
                .unwrap(),
            local: url,
            services: Services {
                stop: fake_stop,
                start: fake_start,
                prepare: fake_prepare,
                space: fake_space,
            },
        };
        (dir, manager, state, server)
    }
    #[tokio::test]
    async fn upgrade_waits_for_work_confirms_health_then_keeps_one_rollback_and_no_package() {
        let (_dir, manager, state, server) = fixture("9.0.0").await;
        let older = manager.private.join("previous");
        private_dir(&older).unwrap();
        mark(&older, &manager.trust.node_id).unwrap();
        fs::write(older.join(binary()), fake_binary("0.5.0")).unwrap();
        fs::write(
            older.join("testflying-prod-ca.crt"),
            include_bytes!("../../../../deploy/testflying-prod-ca.crt"),
        )
        .unwrap();
        let old = program_hashes(&manager.root.join("program")).unwrap();
        let command = state.command.lock().unwrap().clone().unwrap();
        let mut t = manager.download(command).await.unwrap();
        assert!(!manager.wait_ready(&t).await.unwrap());
        assert_eq!(program_hashes(&manager.root.join("program")).unwrap(), old);
        state.ready.store(true, Ordering::SeqCst);
        assert!(manager.wait_ready(&t).await.unwrap());
        manager.apply(&mut t).await.unwrap();
        assert!(manager.private.join("retired").exists());
        assert!(!manager.confirm(&mut t).await.unwrap());
        t.health_since = Some(store_update::now() - 31);
        manager.save(&t).unwrap();
        state.network.store(false, Ordering::SeqCst);
        assert!(manager.confirm(&mut t).await.is_err());
        assert!(read(&manager.root.join("program").join(binary()), 1024)
            .unwrap()
            .ends_with(b"9.0.0"));
        state.network.store(true, Ordering::SeqCst);
        assert!(manager.confirm(&mut t).await.unwrap());
        assert_eq!(t.phase, "succeeded");
        assert!(t.reported);
        assert_eq!(
            program_hashes(&manager.private.join("previous")).unwrap(),
            old
        );
        assert!(!manager.private.join("candidate").exists());
        assert!(!manager.private.join("retired").exists());
        manager.step().await.unwrap();
        assert_eq!(
            state
                .reports
                .lock()
                .unwrap()
                .iter()
                .filter(|s| s.as_str() == "installing")
                .count(),
            1
        );
        assert_eq!(
            read(&manager.root.join("data/credentials/key"), 128).unwrap(),
            b"private-key"
        );
        server.abort();
        let _ = server.await;
    }
    #[tokio::test]
    async fn startup_failure_and_interrupted_switch_recover_old_program_without_touching_data() {
        for crash in [false, true] {
            let (_dir, manager, state, server) =
                fixture(if crash { "crash" } else { "9.0.0" }).await;
            let old = program_hashes(&manager.root.join("program")).unwrap();
            let command = state.command.lock().unwrap().clone().unwrap();
            let mut t = manager.download(command).await.unwrap();
            if crash {
                state.ready.store(true, Ordering::SeqCst);
                assert!(manager.wait_ready(&t).await.unwrap());
                assert!(manager.apply(&mut t).await.is_err());
            } else {
                fake_stop(&manager.root).unwrap();
                mark(&manager.root.join("program"), &manager.trust.node_id).unwrap();
                fs::rename(
                    manager.root.join("program"),
                    manager.private.join("previous"),
                )
                .unwrap();
                t.phase = "switching".into();
                manager.save(&t).unwrap();
                assert!(!manager.root.join("program").exists());
            }
            manager
                .rollback(&mut t, "update_interrupted")
                .await
                .unwrap();
            assert_eq!(t.phase, "rolled_back");
            assert_eq!(program_hashes(&manager.root.join("program")).unwrap(), old);
            assert_eq!(
                read(&manager.root.join("data/connector.json"), 128).unwrap(),
                b"private-config"
            );
            assert!(!manager.private.join("candidate").exists());
            server.abort();
            let _ = server.await;
        }
    }
    #[tokio::test]
    async fn low_space_never_stops_worker_and_terminal_report_precedes_blocked_cleanup() {
        let (_dir, mut manager, state, server) = fixture("9.0.0").await;
        manager.services.space = |_| Ok(0);
        let command = state.command.lock().unwrap().clone().unwrap();
        assert_eq!(
            manager.download(command).await.unwrap_err(),
            "update_insufficient_space"
        );
        assert_eq!(read(&manager.root.join("running"), 128).unwrap(), b"yes");
        let mut t = manager.transaction().unwrap().unwrap();
        t.phase = "failed".into();
        t.error_code = Some("update_insufficient_space".into());
        manager.save(&t).unwrap();
        state.network.store(false, Ordering::SeqCst);
        assert!(manager.report_terminal(&mut t).await.is_err());
        assert!(!t.reported);
        // An unknown candidate must block deletion, but cannot block the durable terminal outbox.
        private_dir(&manager.private.join("candidate")).unwrap();
        fs::write(manager.private.join("candidate/keep"), b"unknown").unwrap();
        state.network.store(true, Ordering::SeqCst);
        assert!(manager.step().await.is_err());
        assert!(manager.transaction().unwrap().unwrap().reported);
        server.abort();
        let _ = server.await;
    }
    #[tokio::test]
    async fn wrong_package_digest_never_stops_worker() {
        let (_dir, manager, state, server) = fixture("9.0.0").await;
        let mut request = state.command.lock().unwrap().clone().unwrap().request;
        request.sha256 = "11".repeat(32);
        let command = store_update::sign(request, b"fixture-signing").unwrap();
        assert_eq!(
            manager.download(command).await.unwrap_err(),
            "update_package_hash_mismatch"
        );
        assert_eq!(read(&manager.root.join("running"), 128).unwrap(), b"yes");
        server.abort();
        let _ = server.await;
    }
    #[tokio::test]
    async fn restart_budget_survives_steps_and_rolls_back_crash_loop() {
        let (_dir, manager, state, server) = fixture("9.0.0").await;
        let command = state.command.lock().unwrap().clone().unwrap();
        let mut t = manager.download(command).await.unwrap();
        state.ready.store(true, Ordering::SeqCst);
        assert!(manager.wait_ready(&t).await.unwrap());
        manager.apply(&mut t).await.unwrap();
        manager.step().await.unwrap();
        for _ in 0..3 {
            fake_start(&manager.root).unwrap();
            manager.step().await.unwrap();
        }
        assert_eq!(manager.transaction().unwrap().unwrap().phase, "rolled_back");
        assert!(read(&manager.root.join("program").join(binary()), 1024)
            .unwrap()
            .ends_with(b"1.0.0"));
        server.abort();
        let _ = server.await;
    }
    #[tokio::test]
    async fn log_retention_runs_while_confirmation_waits_for_center() {
        let (_dir, manager, state, server) = fixture("9.0.0").await;
        let command = state.command.lock().unwrap().clone().unwrap();
        let mut t = manager.download(command).await.unwrap();
        state.ready.store(true, Ordering::SeqCst);
        assert!(manager.wait_ready(&t).await.unwrap());
        manager.apply(&mut t).await.unwrap();
        let old = manager.private.join(format!(
            "upgrade-{:010}.jsonl",
            store_update::now() / 3600 - 169
        ));
        let recent = manager
            .private
            .join(format!("upgrade-{:010}.jsonl", store_update::now() / 3600));
        fs::write(&old, b"expired").unwrap();
        fs::write(&recent, b"recent").unwrap();
        state.network.store(false, Ordering::SeqCst);
        manager.step().await.unwrap();
        assert_eq!(
            manager.transaction().unwrap().unwrap().phase,
            "awaiting_confirmation"
        );
        assert!(!old.exists());
        assert!(recent.exists());
        server.abort();
        let _ = server.await;
    }
    #[tokio::test]
    async fn manager_resumes_interrupted_candidate_cleanup_without_a_marker() {
        for phase in ["failed", "rolled_back"] {
            let (_dir, manager, state, server) = fixture("9.0.0").await;
            let command = state.command.lock().unwrap().clone().unwrap();
            let mut t = manager.download(command).await.unwrap();
            t.phase = phase.into();
            manager.save(&t).unwrap();
            let candidate = manager.private.join("candidate");
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                remove_owned_with(
                    &candidate,
                    &manager.trust.node_id,
                    &mut |entry, directory| {
                        if entry == candidate && directory {
                            panic!("simulated_power_loss");
                        }
                        if directory {
                            fs::remove_dir(entry)
                        } else {
                            fs::remove_file(entry)
                        }
                    },
                )
            }));
            assert!(result.is_err());
            assert!(!candidate.join(MARKER).exists());
            manager.step().await.unwrap();
            assert!(!candidate.exists());
            assert!(!cleanup_intent(&candidate).unwrap().exists());
            assert!(manager.transaction().unwrap().unwrap().reported);
            assert_eq!(
                read(&manager.root.join("data/credentials/key"), 128).unwrap(),
                b"private-key"
            );
            server.abort();
            let _ = server.await;
        }
    }
    #[tokio::test]
    async fn interrupted_upgrade_never_stops_or_moves_a_manually_replaced_program() {
        let (_dir, manager, state, server) = fixture("9.0.0").await;
        let command = state.command.lock().unwrap().clone().unwrap();
        let mut t = manager.download(command).await.unwrap();
        state.ready.store(true, Ordering::SeqCst);
        assert!(manager.wait_ready(&t).await.unwrap());
        manager.apply(&mut t).await.unwrap();
        fs::remove_file(manager.root.join("program").join(MARKER)).unwrap();
        fs::write(
            manager.root.join("program").join(binary()),
            fake_binary("manual-version"),
        )
        .unwrap();
        t.phase = "switching".into();
        manager.save(&t).unwrap();
        manager.step().await.unwrap();
        assert_eq!(
            manager
                .transaction()
                .unwrap()
                .unwrap()
                .error_code
                .as_deref(),
            Some("update_program_changed")
        );
        assert_eq!(read(&manager.root.join("running"), 128).unwrap(), b"yes");
        assert!(read(&manager.root.join("program").join(binary()), 1024)
            .unwrap()
            .ends_with(b"manual-version"));
        assert!(manager.private.join("previous").exists());
        server.abort();
        let _ = server.await;
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn recovery_does_not_stop_worker_when_program_is_a_dangling_link() {
        let (_dir, manager, state, server) = fixture("9.0.0").await;
        let command = state.command.lock().unwrap().clone().unwrap();
        let mut t = manager.download(command).await.unwrap();
        state.ready.store(true, Ordering::SeqCst);
        assert!(manager.wait_ready(&t).await.unwrap());
        manager.apply(&mut t).await.unwrap();
        fs::rename(
            manager.root.join("program"),
            manager.root.join("manual-backup"),
        )
        .unwrap();
        std::os::unix::fs::symlink(manager.root.join("missing"), manager.root.join("program"))
            .unwrap();
        t.phase = "switching".into();
        manager.save(&t).unwrap();
        manager.step().await.unwrap();
        assert_eq!(
            manager
                .transaction()
                .unwrap()
                .unwrap()
                .error_code
                .as_deref(),
            Some("update_program_changed")
        );
        assert_eq!(read(&manager.root.join("running"), 128).unwrap(), b"yes");
        assert!(fs::symlink_metadata(manager.root.join("program"))
            .unwrap()
            .file_type()
            .is_symlink());
        server.abort();
        let _ = server.await;
    }
    #[test]
    fn cleanup_only_owned_trees_and_never_follows_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("candidate");
        private_dir(&p).unwrap();
        fs::write(p.join("keep"), b"fixture").unwrap();
        assert!(remove_owned(&p, "node").is_err());
        assert!(p.exists());
        mark(&p, "node").unwrap();
        assert!(remove_owned(&p, "other").is_err());
        #[cfg(unix)]
        {
            let outside = dir.path().join("outside");
            fs::write(&outside, b"private").unwrap();
            std::os::unix::fs::symlink(&outside, p.join("link")).unwrap();
            assert!(remove_owned(&p, "node").is_err());
            assert!(outside.exists());
            fs::remove_file(p.join("link")).unwrap();
        }
        remove_owned(&p, "node").unwrap();
        assert!(!p.exists());
    }
    #[test]
    fn locked_file_or_directory_keeps_ownership_for_cleanup_retry() {
        for lock_directory in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("retired");
            private_dir(&path).unwrap();
            mark(&path, "node").unwrap();
            let original = read(&path.join(MARKER), 8192).unwrap();
            private_dir(&path.join("program")).unwrap();
            fs::write(path.join("program/locked-binary"), b"old").unwrap();
            let outside = dir.path().join("credentials");
            fs::write(&outside, b"keep").unwrap();
            let mut remove = |entry: &Path, directory: bool| {
                if (lock_directory && entry == path)
                    || (!lock_directory && entry.file_name().is_some_and(|n| n == "locked-binary"))
                {
                    return Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
                }
                if directory {
                    fs::remove_dir(entry)
                } else {
                    fs::remove_file(entry)
                }
            };
            assert!(remove_owned_with(&path, "node", &mut remove).is_err());
            if lock_directory {
                assert!(!path.join(MARKER).exists());
                let intent: CleanupIntent =
                    serde_json::from_slice(&read(&cleanup_intent(&path).unwrap(), 8192).unwrap())
                        .unwrap();
                assert_eq!(
                    serde_json::to_value(intent.owner).unwrap(),
                    serde_json::from_slice::<Value>(&original).unwrap()
                );
            } else {
                check_mark(&path, "node").unwrap();
                assert_eq!(read(&path.join(MARKER), 8192).unwrap(), original);
            }
            remove_owned(&path, "node").unwrap();
            assert!(!path.exists());
            assert_eq!(fs::read(outside).unwrap(), b"keep");
        }
    }
    #[test]
    fn interrupted_cleanup_recovers_only_the_exact_empty_owned_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("retired");
        private_dir(&path).unwrap();
        mark(&path, "node").unwrap();
        fs::write(path.join("binary"), b"old").unwrap();
        let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            remove_owned_with(&path, "node", &mut |entry, directory| {
                if entry == path && directory {
                    panic!("simulated_power_loss");
                }
                if directory {
                    fs::remove_dir(entry)
                } else {
                    fs::remove_file(entry)
                }
            })
        }));
        assert!(interrupted.is_err());
        assert!(!path.join(MARKER).exists());
        assert!(cleanup_intent(&path).unwrap().exists());
        assert!(remove_owned(&path, "other-node").is_err());
        fs::write(path.join("unknown"), b"keep").unwrap();
        assert!(remove_owned(&path, "node").is_err());
        assert_eq!(fs::read(path.join("unknown")).unwrap(), b"keep");
        fs::remove_file(path.join("unknown")).unwrap();
        remove_owned(&path, "node").unwrap();
        assert!(!path.exists());
        assert!(!cleanup_intent(&path).unwrap().exists());
    }
    #[test]
    fn packages_reject_unknown_paths_links_and_duplicate_entries() {
        use zip::write::SimpleFileOptions;
        for names in [
            vec!["../data/connector.json", "testflying-prod-ca.crt"],
            vec![
                "store-connector",
                "store-connector",
                "testflying-prod-ca.crt",
            ],
            vec!["store-connector", "credentials/key"],
        ] {
            let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
            let mut duplicate_rejected = false;
            for name in names {
                if zip.start_file(name, SimpleFileOptions::default()).is_err() {
                    duplicate_rejected = true;
                    break;
                }
                zip.write_all(b"fixture").unwrap();
            }
            if duplicate_rejected {
                continue;
            }
            let bytes = zip.finish().unwrap().into_inner();
            let dir = tempfile::tempdir().unwrap();
            assert!(unpack(
                &bytes,
                &dir.path().join("candidate"),
                "node",
                "darwin-arm64"
            )
            .is_err());
        }
    }
}
