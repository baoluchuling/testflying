//! Private, time-retained node logs. Callers pass fixed labels and safe summaries.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufRead, BufReader, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use store_protocol::{Platform, StoreRequest};

const HOURS: u64 = 7 * 24;
const OWNER: &str = "testflying-store-connector-log";
tokio::task_local! { static CONTEXT: Meta; }

#[derive(Clone)]
pub(crate) struct Meta(Value);
impl Meta {
    pub(crate) fn for_request(request: &StoreRequest) -> Self {
        let id = &request.request_id;
        let id = if id.len() == 36 && id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-') {
            id.clone()
        } else {
            format!("sha256:{}", hex::encode(Sha256::digest(id.as_bytes())))
        };
        Self(
            json!({"requestId":id,"platform":request.platform,"operation":request.operation,
            "identifier":store_protocol::validate_identifier(&request.bundle_identifier).ok().map(|_|&request.bundle_identifier)}),
        )
    }
}

pub struct Log {
    directory: PathBuf,
    node: String,
    lock: Mutex<Option<u64>>,
}
impl Log {
    pub fn for_config(config: &Path, node: &str) -> io::Result<Arc<Self>> {
        if node.is_empty()
            || node.len() > 128
            || !node
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
        {
            return Err(io::Error::other("Invalid log identity"));
        }
        let parent = config
            .canonicalize()?
            .parent()
            .ok_or_else(|| io::Error::other("Missing configuration directory"))?
            .to_path_buf();
        let directory = parent.join("logs");
        match fs::symlink_metadata(&directory) {
            Ok(meta) => check_directory(&meta)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let mut builder = fs::DirBuilder::new();
                #[cfg(unix)]
                {
                    use std::os::unix::fs::DirBuilderExt;
                    builder.mode(0o700);
                }
                builder.create(&directory)?;
                check_directory(&fs::symlink_metadata(&directory)?)?;
            }
            Err(error) => return Err(error),
        }
        let log = Arc::new(Self {
            directory,
            node: node.into(),
            lock: Mutex::new(None),
        });
        log.prune()?;
        Ok(log)
    }
    pub fn directory(&self) -> &Path {
        &self.directory
    }
    pub fn event(&self, name: &'static str, details: Value) {
        self.emit(None, name, details);
    }
    pub(crate) fn context(&self, name: &'static str, details: Value) {
        self.emit(CONTEXT.try_with(Clone::clone).ok().as_ref(), name, details);
    }
    pub fn task(&self, request: &StoreRequest, name: &'static str, details: Value) {
        self.emit(Some(&Meta::for_request(request)), name, details);
    }
    fn emit(&self, meta: Option<&Meta>, name: &'static str, details: Value) {
        let mut entry = meta.map_or_else(|| json!({}), |meta| meta.0.clone());
        entry["timestampMs"] = json!(now_ms());
        entry["timeUtc"] =
            json!(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
        entry["event"] = json!(name);
        entry["connectorId"] = json!(self.node);
        entry["details"] = details;
        if self.append(&entry).is_err() {
            eprintln!("Store Connector log write failed");
        }
    }
    fn file(&self, hour: u64) -> io::Result<File> {
        check_directory(&fs::symlink_metadata(&self.directory)?)?;
        let path = self
            .directory
            .join(format!("store-connector-{hour:010}.jsonl"));
        let mut options = secure_options();
        let (mut file, created) = match options.create_new(true).open(&path) {
            Ok(file) => (file, true),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                (secure_options().open(&path)?, false)
            }
            Err(error) => return Err(error),
        };
        check_file(&file.metadata()?)?;
        file.lock()?;
        let header = json!({"schema":1,"owner":OWNER,"connectorId":self.node,"hour":hour});
        if created {
            writeln!(file, "{header}")?;
            file.flush()?;
        } else {
            verify_header(&mut file, &header)?;
        }
        Ok(file)
    }
    fn append(&self, entry: &Value) -> io::Result<()> {
        let mut guard = self
            .lock
            .lock()
            .map_err(|_| io::Error::other("Log lock unavailable"))?;
        let hour = now_ms() / 3_600_000;
        if *guard != Some(hour) {
            self.prune_files(hour)?;
            *guard = Some(hour);
        }
        let mut file = self.file(hour)?;
        let line = entry.to_string();
        if line.len() > 16384 {
            return Err(io::Error::other("Log entry exceeded bound"));
        }
        file.seek(SeekFrom::End(0))?;
        writeln!(file, "{line}")?;
        file.flush()?;
        Ok(())
    }
    pub fn prune(&self) -> io::Result<()> {
        self.prune_at(now_ms() / 3_600_000)
    }
    fn prune_at(&self, hour: u64) -> io::Result<()> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| io::Error::other("Log lock unavailable"))?;
        self.prune_files(hour)
    }
    fn prune_files(&self, hour: u64) -> io::Result<()> {
        check_directory(&fs::symlink_metadata(&self.directory)?)?;
        for entry in fs::read_dir(&self.directory)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some(number) = name
                .strip_prefix("store-connector-")
                .and_then(|s| s.strip_suffix(".jsonl"))
            else {
                continue;
            };
            if number.len() != 10 || !number.bytes().all(|b| b.is_ascii_digit()) {
                continue;
            }
            let Ok(old_hour) = number.parse::<u64>() else {
                continue;
            };
            if old_hour.saturating_add(HOURS) > hour {
                continue;
            }
            // Only our own regular, private files with the matching node/hour marker are eligible.
            check_file(&fs::symlink_metadata(entry.path())?)?;
            let mut file = secure_options().open(entry.path())?;
            check_file(&file.metadata()?)?;
            file.lock()?;
            verify_header(
                &mut file,
                &json!({"schema":1,"owner":OWNER,"connectorId":self.node,"hour":old_hour}),
            )?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                let held = file.metadata()?;
                let current = fs::symlink_metadata(entry.path())?;
                if (held.dev(), held.ino()) != (current.dev(), current.ino()) {
                    return Err(io::Error::other("Log identity changed"));
                }
                fs::remove_file(entry.path())?;
            }
            #[cfg(not(unix))]
            {
                drop(file);
                fs::remove_file(entry.path())?;
            }
        }
        Ok(())
    }
}
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn secure_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000);
    }
    options
}
fn check_directory(meta: &fs::Metadata) -> io::Result<()> {
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err(io::Error::other("Expected private log directory"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o077 != 0 {
            return Err(io::Error::other("Expected private log directory"));
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err(io::Error::other("Unexpected log reparse point"));
        }
    }
    Ok(())
}
fn check_file(meta: &fs::Metadata) -> io::Result<()> {
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err(io::Error::other("Expected regular log file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.uid() != unsafe { libc::geteuid() } || meta.nlink() != 1 || meta.mode() & 0o077 != 0
        {
            return Err(io::Error::other("Expected private single-link log file"));
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err(io::Error::other("Unexpected log reparse point"));
        }
    }
    Ok(())
}
fn verify_header(file: &mut File, expected: &Value) -> io::Result<()> {
    file.seek(SeekFrom::Start(0))?;
    let mut reader = BufReader::new(file);
    let mut bytes = Vec::new();
    use std::io::Read;
    reader.by_ref().take(513).read_until(b'\n', &mut bytes)?;
    if bytes.len() > 512
        || !bytes.ends_with(b"\n")
        || serde_json::from_slice::<Value>(&bytes).ok().as_ref() != Some(expected)
    {
        return Err(io::Error::other("Log ownership marker mismatch"));
    }
    Ok(())
}

pub(crate) async fn scope<T>(
    request: &StoreRequest,
    future: impl std::future::Future<Output = T>,
) -> T {
    CONTEXT.scope(Meta::for_request(request), future).await
}

pub(crate) struct HttpGuard {
    log: Option<Arc<Log>>,
    meta: Option<Meta>,
    details: Value,
    start: Instant,
    finished: bool,
}
impl HttpGuard {
    pub(crate) fn start(
        log: Option<Arc<Log>>,
        platform: Option<Platform>,
        step: &'static str,
        request: &reqwest::Request,
    ) -> Self {
        let declared = request
            .headers()
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok());
        let body = request
            .body()
            .map(|b| b.as_bytes().map(|v| v.len() as u64))
            .unwrap_or(Some(0));
        let details = json!({"provider":platform,"step":step,"method":request.method().as_str(),
            "pathTemplate":if matches!(step,"image_download"|"apple_image_chunk") {"/external-image".to_owned()} else {path_template(request.url().path())},
            "contentLengthHeader":declared,"bodyBytes":body});
        let guard = Self {
            log,
            meta: CONTEXT.try_with(Clone::clone).ok(),
            details,
            start: Instant::now(),
            finished: false,
        };
        guard.record("upstream.started");
        guard
    }
    pub(crate) fn finish(
        &mut self,
        result: &std::result::Result<reqwest::Response, reqwest::Error>,
    ) {
        self.details["durationMs"] = json!(self.start.elapsed().as_millis() as u64);
        match result {
            Ok(response) => self.details["status"] = json!(response.status().as_u16()),
            Err(error) => {
                self.details["error"] = json!(if error.is_timeout() {
                    "timeout"
                } else if error.is_connect() {
                    "connect_failed"
                } else {
                    "request_failed"
                });
            }
        }
        self.record("upstream.completed");
        self.finished = true;
    }
    fn record(&self, event: &'static str) {
        if let Some(log) = &self.log {
            log.emit(self.meta.as_ref(), event, self.details.clone());
        }
    }
}
impl Drop for HttpGuard {
    fn drop(&mut self) {
        if !self.finished {
            self.details["durationMs"] = json!(self.start.elapsed().as_millis() as u64);
            self.record("upstream.cancelled");
        }
    }
}
fn path_template(path: &str) -> String {
    path.split('/')
        .map(|part| {
            if part.is_empty() {
                ""
            } else if part.ends_with(":commit") {
                "{edit}:commit"
            } else if matches!(
                part,
                "v1" | "v3"
                    | "androidpublisher"
                    | "applications"
                    | "edits"
                    | "listings"
                    | "tracks"
                    | "reviews"
                    | "token"
                    | "apps"
                    | "appInfos"
                    | "appInfoLocalizations"
                    | "appStoreVersions"
                    | "appStoreVersionLocalizations"
                    | "appAssetLibraryRefData"
                    | "assetLibrary"
                    | "appAssetLibraryImages"
                    | "appAssetLibraryPlacements"
                    | "placements"
                    | "appAssetLibraryPlacementOrderingRequests"
                    | "appCustomProductPages"
                    | "appCustomProductPageVersions"
                    | "appCustomProductPageLocalizations"
                    | "appStoreVersionExperiments"
                    | "appStoreVersionExperimentsV2"
                    | "appStoreVersionExperimentTreatments"
                    | "appStoreVersionExperimentTreatmentsV2"
                    | "appStoreVersionExperimentTreatmentLocalizations"
                    | "images"
                    | "upload"
                    | "phoneScreenshots"
                    | "sevenInchScreenshots"
                    | "tenInchScreenshots"
                    | "featureGraphic"
                    | "tvScreenshots"
                    | "wearScreenshots"
                    | "customerReviews"
            ) {
                part
            } else {
                "{id}"
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

pub fn summary(payload: &Value) -> Value {
    let mut value = json!({});
    for key in ["locale", "device", "operation"] {
        if let Some(s) = payload.get(key).and_then(Value::as_str) {
            value[key] = if s.len() <= 80
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
            {
                json!(s)
            } else {
                json!({"invalid":true})
            };
        }
    }
    if let Some(scopes) = payload.get("syncScopes").and_then(Value::as_array) {
        value["syncScopes"] = json!(scopes
            .iter()
            .take(16)
            .map(|v| match v.as_str() {
                Some("title") => "title",
                Some("subtitle") => "subtitle",
                Some("description") => "description",
                Some("short_description") => "short_description",
                Some("keywords") => "keywords",
                Some("metadata") => "metadata",
                Some("promotional_text") => "promotional_text",
                Some("store_images") => "store_images",
                Some("marketing_text") => "marketing_text",
                Some("video") => "video",
                _ => "unknown",
            })
            .collect::<Vec<_>>());
    }
    for container in [
        payload,
        payload.get("metadata").unwrap_or(&Value::Null),
        payload.get("marketingPage").unwrap_or(&Value::Null),
    ] {
        for key in [
            "title",
            "subtitle",
            "description",
            "fullDescription",
            "shortDescription",
            "keywords",
            "promotionalText",
            "releaseNotes",
        ] {
            if let Some(s) = container.get(key).and_then(Value::as_str) {
                value["fields"][key] = json!({"characters":s.chars().count(),"bytes":s.len(),"sha256":hex::encode(Sha256::digest(s.as_bytes()))});
            }
        }
    }
    value
}

pub fn elapsed(start: Instant) -> u64 {
    start.elapsed().as_millis() as u64
}
pub fn maintenance_interval() -> Duration {
    Duration::from_secs(60)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retention_uses_time_and_preserves_unowned_files() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("connector.json");
        fs::write(&config, "{}").unwrap();
        let log = Log::for_config(&config, "test-node").unwrap();
        let hour = now_ms() / 3_600_000;
        let old = hour - HOURS;
        let expired = log.file(old).unwrap();
        drop(expired);
        let recent = log.file(old + 1).unwrap();
        drop(recent);
        let unrelated = log.directory.join("user-notes.txt");
        fs::write(&unrelated, "keep").unwrap();
        log.prune_at(hour).unwrap();
        assert!(!log
            .directory
            .join(format!("store-connector-{old:010}.jsonl"))
            .exists());
        assert!(log
            .directory
            .join(format!("store-connector-{:010}.jsonl", old + 1))
            .exists());
        assert!(unrelated.exists());
        log.event("test", json!({"ok":true}));
        let text = fs::read_to_string(
            log.directory
                .join(format!("store-connector-{hour:010}.jsonl")),
        )
        .unwrap();
        assert!(text.contains("\"event\":\"test\""));
    }
    #[test]
    fn sensitive_payloads_and_paths_are_not_copied() {
        let value = summary(
            &json!({"locale":"en-US","metadata":{"title":"private-title"},"token":"private-token"}),
        );
        assert!(!value.to_string().contains("private"));
        assert_eq!(
            path_template(
                "/androidpublisher/v3/applications/private-app/edits/private-edit:commit"
            ),
            "/androidpublisher/v3/applications/{id}/edits/{edit}:commit"
        );
    }
    #[cfg(unix)]
    #[test]
    fn refuses_log_directory_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let config = dir.path().join("connector.json");
        fs::write(&config, "{}").unwrap();
        std::os::unix::fs::symlink(target.path(), dir.path().join("logs")).unwrap();
        assert!(Log::for_config(&config, "test-node").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn refuses_file_symlinks_and_does_not_delete_foreign_markers() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("connector.json");
        fs::write(&config, "{}").unwrap();
        let log = Log::for_config(&config, "test-node").unwrap();
        let target = dir.path().join("user-data");
        fs::write(&target, "keep").unwrap();
        let path = log.directory.join(format!(
            "store-connector-{:010}.jsonl",
            now_ms() / 3_600_000
        ));
        symlink(&target, &path).unwrap();
        assert!(log.append(&json!({"event":"test"})).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "keep");
        fs::remove_file(&path).unwrap();
        let foreign = log.directory.join("store-connector-0000000001.jsonl");
        fs::write(&foreign, "{\"owner\":\"someone-else\"}\n").unwrap();
        fs::set_permissions(&foreign, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(log.prune().is_err());
        assert!(foreign.exists());
    }
}
