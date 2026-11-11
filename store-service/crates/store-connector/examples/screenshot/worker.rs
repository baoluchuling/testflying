use super::{
    diagnostic::Log,
    goal::{self, GoalChange, ToolReply},
    Failure,
};
use reqwest::{Client, Method, RequestBuilder, StatusCode, Url};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    env,
    fs::{self, OpenOptions},
    io::{self, Read},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    process::{ExitStatus, Stdio},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    process::{Child, ChildStderr, Command},
    sync::watch,
    time::{sleep, timeout},
};
use uuid::Uuid;

const MAX_IMAGE: usize = 16 * 1024 * 1024;
const CODEX_STDERR_LIMIT: usize = 8 * 1024;
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";
const MAX_CLAIM: usize = 256 * 1024;
const MAX_CENTER_JSON: usize = 16 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Config {
    kind: String,
    pub connector_id: String,
    pub token: String,
    #[serde(deserialize_with = "https_url")]
    pub server_url: Url,
    pub message_key: String,
    codex_path: Option<PathBuf>,
    codex_home: Option<PathBuf>,
    adb_path: Option<PathBuf>,
    ca_file: Option<PathBuf>,
}

fn https_url<'de, D: serde::Deserializer<'de>>(input: D) -> Result<Url, D::Error> {
    let raw = String::deserialize(input)?;
    let url = Url::parse(&raw).map_err(serde::de::Error::custom)?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(serde::de::Error::custom("HTTPS center required"));
    }
    Ok(url)
}

impl Config {
    pub(super) fn load(path: &Path) -> Result<Self, Failure> {
        let config: Self = serde_json::from_slice(&read_owned(path, 64 * 1024, true)?)?;
        if config.kind != "screenshot"
            || !Uuid::parse_str(&config.connector_id)
                .is_ok_and(|id| id.to_string() == config.connector_id)
            || config.token.len() != 64
            || !config
                .token
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            return Err("Invalid screenshot configuration".into());
        }
        Ok(config)
    }

    fn adb(&self) -> Option<PathBuf> {
        if let Some(path) = &self.adb_path {
            return local_path(path, false).ok();
        }
        ["ANDROID_HOME", "ANDROID_SDK_ROOT", "HOME"]
            .into_iter()
            .filter_map(|key| {
                let mut root = PathBuf::from(env::var_os(key)?);
                if key == "HOME" {
                    root.push("Library/Android/sdk");
                }
                local_path(&root.join("platform-tools/adb"), false).ok()
            })
            .next()
            .or_else(|| path_executable("adb"))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Job {
    task_id: Uuid,
    job_id: Option<String>,
    platform: Option<String>,
    bundle_identifier: Option<String>,
    target: Option<Vec<store_protocol::screenshot::ScreenshotTarget>>,
    lease: Uuid,
    profile: Option<String>,
    prompt: String,
}

pub(super) struct Worker<'a> {
    config: &'a Config,
    client: Client,
    oss_client: Client,
    jobs: String,
    log: Log,
}

impl<'a> Worker<'a> {
    pub(super) fn new(config: &'a Config, path: &Path) -> Result<Self, Failure> {
        let mut client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(100));
        let ca = config
            .ca_file
            .clone()
            .or_else(|| env::var_os("SSL_CERT_FILE").map(PathBuf::from))
            .or_else(|| {
                path.parent()
                    .map(|root| root.join("testflying-prod-ca.crt"))
                    .filter(|ca| ca.is_file())
            });
        if let Some(ca) = ca {
            for cert in reqwest::Certificate::from_pem_bundle(&fs::read(local_path(&ca, false)?)?)?
            {
                client = client.add_root_certificate(cert);
            }
        }
        let jobs = format!(
            "{}/v1/connectors/{}/screenshot-jobs",
            config.server_url.as_str().trim_end_matches('/'),
            config.connector_id
        );
        Ok(Self {
            config,
            client: client.build()?,
            oss_client: Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(100))
                .build()?,
            jobs,
            log: Log::for_config(path),
        })
    }

    fn request(&self, method: Method, path: &str) -> RequestBuilder {
        self.client
            .request(method, format!("{}/{path}", self.jobs))
            .bearer_auth(&self.config.token)
    }

    pub(super) async fn once(&self, stop: &mut watch::Receiver<bool>) -> Result<bool, Failure> {
        let claim = async {
            let mut response = self
                .request(Method::POST, "claim")
                .header("X-Screenshot-Protocol", "2")
                .send()
                .await?;
            match response.status() {
                StatusCode::NO_CONTENT => return Ok(None),
                StatusCode::OK => {}
                status => {
                    self.log.event(
                        None,
                        format_args!("claim_rejected http={}", status.as_u16()),
                    );
                    return Err::<_, Failure>("Center refused the screenshot claim".into());
                }
            }
            let mut body = Vec::new();
            while let Some(chunk) = response.chunk().await? {
                if chunk.len() > MAX_CLAIM - body.len() {
                    return Err("Screenshot claim is too large".into());
                }
                body.extend_from_slice(&chunk);
            }
            Ok(Some(serde_json::from_slice::<Job>(&body)?))
        };
        let job = tokio::select! {
            result = claim => result.inspect_err(|_| self.log.event(None, "claim_failed"))?,
            _ = stop.wait_for(|stopped| *stopped) => return Ok(false),
        };
        let Some(job) = job else {
            return Ok(false);
        };
        let id = Some(job.task_id);
        self.log.prompt(job.task_id, &job.prompt);
        let outcome = self.run_goal(&job, stop).await;
        let stopping = *stop.borrow();
        let Err(code) = outcome else {
            return Ok(!stopping);
        };

        // No unknown PUT is repeated or turned into a guessed terminal failure.
        if code == "delivery_unknown" {
            return Ok(!stopping);
        }

        self.log.event(id, format_args!("job_failed code={code}"));
        let request = self
            .request(Method::POST, &format!("{}/failure", job.task_id))
            .header("X-Screenshot-Lease", job.lease.to_string())
            .json(&json!({"code":code}))
            .send();
        let response = if stopping {
            timeout(Duration::from_secs(3), request).await??
        } else {
            tokio::select! {
                response = request => response?,
                _ = stop.wait_for(|stopped| *stopped) => return Ok(false),
            }
        };
        if response.status() != StatusCode::NO_CONTENT {
            self.log.event(
                id,
                format_args!(
                    "failure_report_rejected http={}",
                    response.status().as_u16()
                ),
            );
            return Err("Center refused the screenshot failure report".into());
        }
        self.log.event(id, "failure_reported");
        Ok(!stopping)
    }

    async fn run_goal(
        &self,
        job: &Job,
        stop: &mut watch::Receiver<bool>,
    ) -> Result<(), &'static str> {
        let workspace = tempfile::Builder::new()
            .prefix("testflying-screenshot-")
            .tempdir()
            .map_err(|_| "capture_failed")?;
        let result = self.run_goal_in(workspace.path(), job, stop).await;
        if workspace.close().is_err() {
            self.log.event(Some(job.task_id), "task_cleanup_failed");
        }
        result
    }

    async fn run_goal_in(
        &self,
        root: &Path,
        job: &Job,
        stop: &mut watch::Receiver<bool>,
    ) -> Result<(), &'static str> {
        let id = Some(job.task_id);
        let unavailable = |_| "profile_unavailable";
        let codex = match self
            .config
            .codex_path
            .as_deref()
            .filter(|path| !path.as_os_str().is_empty())
        {
            Some(path) => local_path(path, false)
                .inspect_err(|_| self.log.event(id, "codex_path_invalid"))
                .map_err(unavailable)?,
            None => path_executable("codex").ok_or_else(|| {
                self.log.event(id, "codex_not_found_on_path");
                "profile_unavailable"
            })?,
        };
        let codex_home = self
            .config
            .codex_home
            .as_deref()
            .filter(|path| !path.as_os_str().is_empty())
            .map(|path| local_path(path, true))
            .transpose()
            .inspect_err(|_| self.log.event(id, "codex_home_invalid"))
            .map_err(unavailable)?;
        let adb = self.config.adb();
        let mut command = Command::new(codex);
        job_environment(&mut command, root, codex_home.as_deref());
        let instructions = match adb.as_deref() {
            Some(adb) => format!(
                "{}\nThe local ADB executable is {adb:?}.",
                goal::INSTRUCTIONS
            ),
            None => goal::INSTRUCTIONS.to_owned(),
        };
        let session = goal::Session {
            root,
            instructions,
            prompt: &job.prompt,
            job: job.task_id,
        };
        let mut tools = JobTools {
            worker: self,
            job,
            root,
            adb: adb.as_deref(),
            screenshot: None,
            current_target: None,
            completed_targets: std::collections::BTreeSet::new(),
            upload_attempted: None,
            retry_upload: false,
            center_finished: false,
            failure: "codex_failed",
        };
        self.log.event(id, "codex_starting");
        let _ = session
            .run(
                &mut command,
                &self.log,
                stop,
                async |name, arguments, mut stop| tools.call(&name, arguments, &mut stop).await,
            )
            .await;

        // The center result is authoritative even if Codex exits after delivery.
        if tools.center_finished {
            Ok(())
        } else {
            Err(tools.failure)
        }
    }
}

struct JobTools<'a, 'config> {
    worker: &'a Worker<'config>,
    job: &'a Job,
    root: &'a Path,
    adb: Option<&'a Path>,
    screenshot: Option<(Uuid, Vec<u8>)>,
    current_target: Option<usize>,
    completed_targets: std::collections::BTreeSet<usize>,
    /// A PUT is replayed only after HEAD definitively confirms absence, on a later tool call.
    upload_attempted: Option<Uuid>,
    retry_upload: bool,
    center_finished: bool,
    failure: &'static str,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UploadInput {
    screenshot_id: Uuid,
    target_index: Option<usize>,
}

impl JobTools<'_, '_> {
    async fn call(
        &mut self,
        name: &str,
        arguments: Value,
        stop: &mut watch::Receiver<bool>,
    ) -> ToolReply {
        if name == "get_capture_task" {
            if arguments
                .as_object()
                .is_none_or(|object| !object.is_empty())
            {
                return ToolReply::error(
                    "invalid_arguments",
                    "get_capture_task accepts an empty object.",
                    true,
                    GoalChange::Continue,
                );
            }
            return ToolReply::ok(
                json!({"taskId":self.job.task_id,"jobId":self.job.job_id,"platform":self.job.platform,"bundleIdentifier":self.job.bundle_identifier,"prompt":self.job.prompt,"target":self.job.target,"profile":self.job.profile,"completedTargetIndices":self.completed_targets}),
                GoalChange::Continue,
            );
        }
        if name == "upload_screenshot" {
            return self.upload(arguments, stop).await;
        }
        if !matches!(name, "validate_simulator" | "capture_screenshot") {
            return ToolReply::error(
                "unknown_tool",
                "Use a tool from the screenshot tool catalog.",
                false,
                GoalChange::Continue,
            );
        }
        if name == "capture_screenshot" && self.screenshot.is_some() {
            return ToolReply::error(
                "upload_pending",
                "Retry upload_screenshot with the same screenshotId to confirm its pending upload.",
                true,
                GoalChange::Continue,
            );
        }
        let mut arguments = arguments;
        let target_index = arguments
            .as_object_mut()
            .and_then(|value| value.remove("targetIndex"));
        let target_index = match target_index {
            Some(value) => match value.as_u64().and_then(|value| usize::try_from(value).ok()) {
                Some(value) => Some(value),
                None => return self.invalid_target(),
            },
            None => None,
        };
        if name == "capture_screenshot" && !self.valid_target(target_index) {
            return self.invalid_target();
        }
        let Some(device) = serde_json::from_value::<DeviceInput>(arguments)
            .ok()
            .and_then(DeviceInput::device)
        else {
            self.failure = "selection_invalid";
            return ToolReply::error(
                "invalid_simulator_id",
                "Provide an iOS simulator UUID or Android emulator serial.",
                true,
                GoalChange::Continue,
            );
        };
        let Ok(scratch) = tempfile::tempdir_in(self.root) else {
            self.failure = "capture_failed";
            return ToolReply::error(
                "scratch_unavailable",
                "Connector could not create its working directory.",
                false,
                GoalChange::Continue,
            );
        };
        let id = Some(self.job.task_id);
        if device
            .verify_running(scratch.path(), self.adb, stop)
            .await
            .is_err()
        {
            self.failure = "selection_invalid";
            self.worker.log.event(id, "simulator_unavailable");
            return ToolReply::error("simulator_unavailable", "This simulator is not booted or reachable. Prepare it or select another simulator and retry.", true, GoalChange::Continue);
        }
        self.worker.log.event(id, "simulator_verified");
        if name == "validate_simulator" {
            return ToolReply::ok(json!({"available":true}), GoalChange::Continue);
        }
        match device.capture(scratch.path(), self.adb, stop).await {
            Ok(bytes) => {
                let screenshot_id = Uuid::new_v4();
                let size = bytes.len();
                self.screenshot = Some((screenshot_id, bytes));
                self.current_target = target_index;
                self.failure = "capture_failed";
                self.worker
                    .log
                    .event(id, format_args!("image_ready bytes={size}"));
                ToolReply::ok(
                    json!({"screenshotId":screenshot_id,"bytes":size,"targetIndex":target_index}),
                    GoalChange::Continue,
                )
            }
            Err(code) => {
                self.failure = "capture_failed";
                self.worker.log.event(id, code);
                ToolReply::error(
                    code,
                    "Screenshot capture failed. Check the simulator and retry.",
                    true,
                    GoalChange::Continue,
                )
            }
        }
    }

    async fn upload(&mut self, arguments: Value, stop: &mut watch::Receiver<bool>) -> ToolReply {
        let Ok(input) = serde_json::from_value::<UploadInput>(arguments) else {
            return ToolReply::error(
                "invalid_screenshot_id",
                "Use the screenshotId returned by capture_screenshot.",
                true,
                GoalChange::Continue,
            );
        };
        if !self.valid_target(input.target_index) || input.target_index != self.current_target {
            return self.invalid_target();
        }
        let Some((_, bytes)) = self
            .screenshot
            .as_ref()
            .filter(|(id, _)| *id == input.screenshot_id)
        else {
            return ToolReply::error(
                "screenshot_not_found",
                "Capture the screen and use its current screenshotId.",
                true,
                GoalChange::Continue,
            );
        };
        let id = Some(self.job.task_id);
        if self.upload_attempted.is_none() || self.retry_upload {
            if !bytes.starts_with(PNG) || !(33..=MAX_IMAGE).contains(&bytes.len()) {
                self.failure = "capture_failed";
                return ToolReply::error(
                    "invalid_image",
                    "Capture a valid PNG screenshot within 16 MiB.",
                    true,
                    GoalChange::Continue,
                );
            }
            let sha256 = hex::encode(Sha256::digest(bytes));
            let md5 = format!("{:x}", md5::compute(bytes));
            self.worker.log.event(id, "upload_ticket_request");
            let prepare = self
                .worker
                .request(Method::POST, &format!("{}/upload-ticket", self.job.task_id))
                .header("X-Screenshot-Lease", self.job.lease.to_string())
                .json(&self.target_body(json!({"sizeBytes":bytes.len(),"sha256":sha256,"md5":md5})))
                .send();
            let prepared = tokio::select! {
                result = prepare => match result { Ok(value) => value, Err(_) => return self.unknown_upload(false) },
                _ = stop.wait_for(|value| *value) => return self.unknown_upload(true),
            };
            let status = prepared.status();
            let body = match bounded_json(prepared).await {
                Ok(body) => body,
                Err(_) => return self.unknown_upload(false),
            };
            if status != StatusCode::OK {
                return self.upload_rejected(status, safe_error_code(&body));
            }
            let ticket: UploadTicket = match serde_json::from_value(body) {
                Ok(ticket) => ticket,
                Err(_) => return self.invalid_ticket(),
            };
            let expected_headers = BTreeMap::from([
                ("content-type".to_owned(), "image/png".to_owned()),
                ("content-length".to_owned(), bytes.len().to_string()),
                (
                    "content-md5".to_owned(),
                    store_protocol::upload::encode_chunk(&md5::compute(bytes).0),
                ),
                ("x-oss-meta-sha256".to_owned(), sha256),
                ("x-oss-meta-md5".to_owned(), md5),
                ("x-oss-forbid-overwrite".to_owned(), "true".to_owned()),
            ]);
            let Ok(upload_url) = signed_upload_url(&ticket.upload_url) else {
                return self.invalid_ticket();
            };
            if ticket.headers != expected_headers
                || ticket.object_key.is_empty()
                || ticket.object_key.len() > 1024
                || ticket.expires_at <= store_protocol::upload::now().unwrap_or(u64::MAX)
            {
                return self.invalid_ticket();
            }
            let mut request = self.worker.oss_client.put(upload_url);
            for (name, value) in &ticket.headers {
                request = request.header(name.as_str(), value.as_str());
            }
            let request = request.body(bytes.clone()).send();
            self.upload_attempted = Some(input.screenshot_id);
            self.retry_upload = false;
            self.failure = "delivery_unknown";
            self.worker.log.event(id, "oss_upload_started");
            let uploaded = tokio::select! {
                result = request => result,
                _ = stop.wait_for(|value| *value) => return self.unknown_upload(true),
            };
            if let Ok(response) = uploaded {
                if !response.status().is_success() && response.status() != StatusCode::CONFLICT {
                    self.failure = "screenshot_upload_rejected";
                    return ToolReply::error(
                        "screenshot_upload_rejected",
                        "OSS rejected the screenshot upload.",
                        false,
                        GoalChange::Blocked,
                    );
                }
            }
            // A lost PUT acknowledgement is resolved by HEAD; it never causes another PUT.
        }
        self.worker.log.event(id, "upload_completion_request");
        let completion = self
            .worker
            .request(Method::POST, &format!("{}/complete", self.job.task_id))
            .header("X-Screenshot-Lease", self.job.lease.to_string())
            .json(&self.target_body(json!({})))
            .send();
        let response = tokio::select! {
            result = completion => match result { Ok(response)=>response, Err(_)=>return self.unknown_upload(false) },
            _ = stop.wait_for(|value| *value) => return self.unknown_upload(true),
        };
        let status = response.status();
        let body = match bounded_json(response).await {
            Ok(body) => body,
            Err(_) => return self.unknown_upload(false),
        };
        if status == StatusCode::OK && body["delivered"].is_boolean() && body["result"].is_object()
        {
            return self.confirmed_upload(body);
        }
        self.upload_rejected(status, safe_error_code(&body))
    }

    fn valid_target(&self, index: Option<usize>) -> bool {
        match (&self.job.target, index) {
            (None, None) => true,
            (Some(targets), Some(index)) => {
                index < targets.len() && !self.completed_targets.contains(&index)
            }
            _ => false,
        }
    }

    fn invalid_target(&self) -> ToolReply {
        ToolReply::error("invalid_target_index", "Use an unfinished targetIndex from get_capture_task and the same index for capture and upload.", true, GoalChange::Continue)
    }

    fn target_body(&self, mut body: Value) -> Value {
        if let Some(index) = self.current_target {
            body["targetIndex"] = json!(index);
        }
        body
    }

    fn confirmed_upload(&mut self, mut body: Value) -> ToolReply {
        let mut completed = self.completed_targets.clone();
        if let Some(index) = self.current_target {
            completed.insert(index);
        }
        let all_done = self
            .job
            .target
            .as_ref()
            .is_none_or(|targets| completed.len() == targets.len());
        if (body["delivered"] == true) != all_done {
            return self.unknown_upload(false);
        }
        // Public queries retain the aggregate. The model only needs this item's
        // confirmation; repeating every signed URL on every call grows quadratically.
        if let (Some(index), Some(images)) =
            (self.current_target, body["result"]["images"].as_array_mut())
        {
            images.retain(|image| image["targetIndex"].as_u64() == Some(index as u64));
        }
        self.completed_targets = completed;
        self.screenshot = None;
        self.current_target = None;
        self.upload_attempted = None;
        self.retry_upload = false;
        self.failure = "capture_failed";
        self.center_finished = all_done;
        self.worker
            .log
            .event(Some(self.job.task_id), "image_delivered");
        ToolReply::ok(
            body,
            if all_done {
                GoalChange::Delivered
            } else {
                GoalChange::Continue
            },
        )
    }

    fn unknown_upload(&mut self, stopping: bool) -> ToolReply {
        self.failure = "delivery_unknown";
        self.worker
            .log
            .event(Some(self.job.task_id), "delivery_unknown");
        ToolReply::error(
            "delivery_unknown",
            "Delivery is not confirmed. Retry this same screenshotId to check completion.",
            !stopping,
            if stopping {
                GoalChange::Blocked
            } else {
                GoalChange::Continue
            },
        )
    }

    fn invalid_ticket(&mut self) -> ToolReply {
        self.failure = "invalid_upload_ticket";
        ToolReply::error(
            "invalid_upload_ticket",
            "The center returned an invalid upload ticket.",
            false,
            GoalChange::Blocked,
        )
    }

    fn upload_rejected(&mut self, status: StatusCode, code: &'static str) -> ToolReply {
        self.worker.log.event(
            Some(self.job.task_id),
            format_args!("upload_rejected http={} code={code}", status.as_u16()),
        );
        // A definitive HEAD 404 permits a later explicit retry of this exact ticket/image.
        // Unavailable HEAD responses keep the PUT outcome uncertain and never enable replay.
        if code == "screenshot_object_missing" {
            self.retry_upload = true;
        }
        if matches!(
            code,
            "screenshot_object_missing"
                | "screenshot_verification_unavailable"
                | "screenshot_upload_busy"
                | "delivery_unknown"
        ) || status.is_server_error() && code == "unknown"
        {
            self.failure = "delivery_unknown";
            return ToolReply::error(
                code,
                "Storage has not confirmed the screenshot. Wait and retry this same screenshotId.",
                true,
                GoalChange::Continue,
            );
        }
        if matches!(
            code,
            "invalid_job_lease"
                | "screenshot_oss_not_configured"
                | "screenshot_upload_unconfigured"
        ) {
            self.center_finished = true;
        }
        self.failure = match code {
            "screenshot_object_mismatch" => "screenshot_object_mismatch",
            "screenshot_oss_config_changed" => "screenshot_oss_config_changed",
            _ => "invalid_upload_ticket",
        };
        ToolReply::error(code,"Screenshot delivery was rejected. Check the center storage settings or start a new task.",false,GoalChange::Blocked)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadTicket {
    upload_url: String,
    headers: BTreeMap<String, String>,
    object_key: String,
    expires_at: u64,
}

fn signed_upload_url(value: &str) -> Result<Url, ()> {
    if value.len() > 32 * 1024 {
        return Err(());
    }
    let url = Url::parse(value).map_err(|_| ())?;
    let local = cfg!(test)
        && url.host_str().is_some_and(|host| {
            host == "localhost"
                || host
                    .trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        });
    if !(url.scheme() == "https" || (local && url.scheme() == "http"))
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(());
    }
    Ok(url)
}

async fn bounded_json(mut response: reqwest::Response) -> Result<Value, ()> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| ())? {
        if chunk.len() > MAX_CENTER_JSON - body.len() {
            return Err(());
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).map_err(|_| ())
}

fn safe_error_code(body: &Value) -> &'static str {
    match body["error"]["code"].as_str() {
        Some("screenshot_oss_not_configured") => "screenshot_oss_not_configured",
        Some("screenshot_verification_unavailable") => "screenshot_verification_unavailable",
        Some("screenshot_object_missing") => "screenshot_object_missing",
        Some("screenshot_object_mismatch") => "screenshot_object_mismatch",
        Some("screenshot_oss_config_changed") => "screenshot_oss_config_changed",
        Some("screenshot_upload_unconfigured") => "screenshot_upload_unconfigured",
        Some("screenshot_upload_busy") => "screenshot_upload_busy",
        Some("delivery_unknown") => "delivery_unknown",
        Some("invalid_job_lease") => "invalid_job_lease",
        Some("invalid_image") => "invalid_image",
        Some("image_too_large") => "image_too_large",
        Some("upstream_rejected") => "upstream_rejected",
        Some("invalid_upstream_reply") => "invalid_upstream_reply",
        _ => "unknown",
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DeviceInput {
    platform: Platform,
    device_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Platform {
    Ios,
    Android,
}

enum Device {
    Ios(Uuid),
    Android(String),
}

impl DeviceInput {
    fn device(self) -> Option<Device> {
        let id = self.device_id;
        match self.platform {
            Platform::Ios => Uuid::parse_str(&id).ok().map(Device::Ios),
            Platform::Android => {
                let serial = id.strip_prefix("emulator-")?;
                ((4..=5).contains(&serial.len()) && serial.bytes().all(|b| b.is_ascii_digit()))
                    .then_some(Device::Android(id))
            }
        }
    }
}

impl Device {
    async fn capture(
        &self,
        root: &Path,
        adb: Option<&Path>,
        stop: &mut watch::Receiver<bool>,
    ) -> Result<Vec<u8>, &'static str> {
        let png = root.join("screenshot.png");
        let (mut command, stdout) = self
            .capture_command(&png, adb)
            .map_err(|_| "capture_command_unavailable")?;
        if stdout {
            command.stdout(
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&png)
                    .map_err(|_| "capture_output_unavailable")?,
            );
        } else if png.symlink_metadata().is_ok() {
            return Err("capture_output_unavailable");
        } else {
            command.stdout(Stdio::null());
        }
        job_environment(&mut command, root, None);
        let status = run_command(&mut command, None, Duration::from_secs(45), stop, None)
            .await
            .map_err(|_| "capture_process_error")?;
        if !status.success() {
            return Err("capture_exit_failed");
        }
        let bytes = read_owned(&png, MAX_IMAGE, false).map_err(|_| "image_unreadable")?;
        if bytes.len() < 33 || !bytes.starts_with(PNG) {
            return Err("image_invalid");
        }
        Ok(bytes)
    }

    async fn verify_running(
        &self,
        root: &Path,
        adb: Option<&Path>,
        stop: &mut watch::Receiver<bool>,
    ) -> io::Result<()> {
        let mut list = match self {
            Self::Ios(_) => {
                let mut command = Command::new("/usr/bin/xcrun");
                command.args(["simctl", "list", "devices", "booted", "-j"]);
                command
            }
            Self::Android(_) => {
                timeout(
                    Duration::from_secs(2),
                    TcpStream::connect(("127.0.0.1", 5037)),
                )
                .await
                .map_err(io::Error::other)??;
                let mut command = adb_command(adb)?;
                command.arg("devices");
                command
            }
        };
        let output = root.join("device-list");
        list.stdout(
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&output)?,
        );
        job_environment(&mut list, root, None);
        let status = run_command(&mut list, None, Duration::from_secs(10), stop, None).await?;
        if status.success() && self.is_running(&read_owned(&output, 1024 * 1024, false)?) {
            Ok(())
        } else {
            Err(io::Error::other("Selected simulator is unavailable"))
        }
    }

    fn is_running(&self, output: &[u8]) -> bool {
        match self {
            Self::Ios(selected) => {
                #[derive(Deserialize)]
                struct Devices {
                    devices: HashMap<String, Vec<Simulator>>,
                }
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Simulator {
                    udid: Uuid,
                    state: String,
                    is_available: bool,
                }
                serde_json::from_slice::<Devices>(output).is_ok_and(|inventory| {
                    inventory
                        .devices
                        .iter()
                        .filter(|(runtime, _)| {
                            runtime.starts_with("com.apple.CoreSimulator.SimRuntime.iOS-")
                        })
                        .flat_map(|(_, devices)| devices)
                        .any(|device| {
                            device.udid == *selected
                                && device.state == "Booted"
                                && device.is_available
                        })
                })
            }
            Self::Android(selected) => std::str::from_utf8(output).is_ok_and(|inventory| {
                inventory.lines().any(|line| {
                    let mut fields = line.split_ascii_whitespace();
                    fields.next() == Some(selected.as_str()) && fields.next() == Some("device")
                })
            }),
        }
    }

    fn capture_command(&self, png: &Path, adb: Option<&Path>) -> io::Result<(Command, bool)> {
        match self {
            Self::Ios(device) => {
                let mut command = Command::new("/usr/bin/xcrun");
                command
                    .args([
                        "simctl",
                        "io",
                        &device.to_string().to_uppercase(),
                        "screenshot",
                    ])
                    .arg(png);
                Ok((command, false))
            }
            Self::Android(device) => {
                let mut command = adb_command(adb)?;
                command.args(["-s", device, "exec-out", "screencap", "-p"]);
                Ok((command, true))
            }
        }
    }
}

fn adb_command(adb: Option<&Path>) -> io::Result<Command> {
    let adb = adb.ok_or_else(|| io::Error::other("ADB path required"))?;
    let mut command = Command::new(adb);
    command.args(["-H", "127.0.0.1", "-P", "5037"]);
    Ok(command)
}

fn path_executable(name: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .filter(|dir| dir.is_absolute())
        .find_map(|dir| {
            let candidate = local_path(&dir.join(name), false).ok()?;
            fs::metadata(&candidate)
                .ok()
                .filter(|metadata| metadata.mode() & 0o111 != 0)
                .map(|_| candidate)
        })
}

fn job_environment(command: &mut Command, root: &Path, codex_home: Option<&Path>) {
    command.current_dir(root).env_clear();
    for key in [
        "HOME",
        "PATH",
        "USER",
        "LOGNAME",
        "LANG",
        "CODEX_HOME",
        "DEVELOPER_DIR",
        "ANDROID_HOME",
        "ANDROID_SDK_ROOT",
    ] {
        if let Some(value) = env::var_os(key) {
            command.env(key, value);
        }
    }
    if let Some(codex_home) = codex_home {
        command.env("CODEX_HOME", codex_home);
    }
    command
        .env("TMPDIR", root)
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env("GRADLE_USER_HOME", root.join("gradle"))
        .env("XCODE_DERIVED_DATA_PATH", root.join("derived-data"));
}

fn read_owned(path: &Path, limit: usize, private: bool) -> io::Result<Vec<u8>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let meta = file.metadata()?;
    // geteuid has no arguments or caller-owned memory.
    if !meta.is_file()
        || meta.nlink() != 1
        || meta.uid() != unsafe { libc::geteuid() }
        || (private && meta.mode() & 0o077 != 0)
        || meta.len() > limit as u64
    {
        return Err(io::Error::other("Expected an owned, bounded file"));
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(io::Error::other("File is too large"));
    }
    Ok(bytes)
}

fn local_path(path: &Path, directory: bool) -> io::Result<PathBuf> {
    if !path.is_absolute() {
        return Err(io::Error::other("Use an absolute local path"));
    }
    let path = path.canonicalize()?;
    let valid = if directory {
        path.is_dir()
    } else {
        path.is_file()
    };
    if !valid {
        return Err(io::Error::other("Invalid local path"));
    }
    Ok(path)
}

async fn run_command(
    command: &mut Command,
    input: Option<&[u8]>,
    limit: Duration,
    stop: &mut watch::Receiver<bool>,
    stderr_capture: Option<&mut Vec<u8>>,
) -> io::Result<ExitStatus> {
    if *stop.borrow() {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Connector is shutting down",
        ));
    }
    let mut child = command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stderr(if stderr_capture.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .process_group(0)
        .kill_on_drop(true)
        .spawn()?;
    let pid = child.id().expect("new child has a PID") as libc::pid_t;
    let stderr = child
        .stderr
        .take()
        .map(|stderr| tokio::spawn(drain_stderr(stderr)));
    let completion = timeout(limit, async {
        if let Some(input) = input {
            let mut stdin = child.stdin.take().expect("child stdin is piped");
            if let Err(error) = stdin.write_all(input).await {
                // A CLI usage error can exit before reading the prompt. Preserve its exit code.
                if error.kind() != io::ErrorKind::BrokenPipe {
                    return Err(error);
                }
            }
        }
        wait_for_exit(pid).await
    });
    let result = tokio::select! {
        result = completion => result.map_err(|e| io::Error::new(io::ErrorKind::TimedOut, e)).and_then(|r| r),
        _ = stop.wait_for(|stopped| *stopped) => Err(io::Error::new(io::ErrorKind::Interrupted, "Connector is shutting down")),
    };
    let status = finish_process(&mut child, pid, result.is_err()).await;
    if let (Some(sink), Some(mut stderr)) = (stderr_capture, stderr) {
        match timeout(Duration::from_secs(1), &mut stderr).await {
            Ok(Ok(captured)) => *sink = captured,
            Ok(Err(_)) => {}
            Err(_) => stderr.abort(),
        }
    }
    result?;
    status
}

pub(super) async fn drain_stderr(mut stderr: ChildStderr) -> Vec<u8> {
    let mut captured = Vec::new();
    let mut buffer = [0; 2048];
    while let Ok(count @ 1..) = stderr.read(&mut buffer).await {
        let keep = count.min(CODEX_STDERR_LIMIT - captured.len());
        captured.extend_from_slice(&buffer[..keep]);
    }
    captured
}

pub(super) async fn wait_for_exit(pid: libc::pid_t) -> io::Result<()> {
    loop {
        // WNOWAIT keeps the PID reserved until we finish signaling this process group.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let wait = unsafe {
            libc::waitid(
                libc::P_PID,
                pid as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if wait == 0 && info.si_signo != 0 {
            return Ok(());
        }
        if wait != 0 {
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
        sleep(Duration::from_millis(100)).await;
    }
}

pub(super) async fn finish_process(
    child: &mut Child,
    pid: libc::pid_t,
    force: bool,
) -> io::Result<ExitStatus> {
    // The unreaped direct child reserves this group ID across both signals.
    if force {
        unsafe { libc::kill(-pid, libc::SIGTERM) };
        sleep(Duration::from_secs(1)).await;
    }
    unsafe { libc::kill(-pid, libc::SIGKILL) };
    timeout(Duration::from_secs(5), child.wait())
        .await
        .map_err(|error| io::Error::new(io::ErrorKind::TimedOut, error))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connector::diagnostic::codex_diagnostic;

    #[test]
    fn accepts_only_running_simulators_and_targets_the_selected_id() {
        let id = Uuid::parse_str("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa").unwrap();
        let device = Device::Ios(id);
        let mut inventory = json!({"devices":{"com.apple.CoreSimulator.SimRuntime.iOS-26-0":[
            {"udid":id,"state":"Booted","isAvailable":true}
        ]}});
        assert!(device.is_running(inventory.to_string().as_bytes()));
        inventory["devices"]["com.apple.CoreSimulator.SimRuntime.iOS-26-0"][0]["state"] =
            json!("Shutdown");
        assert!(!device.is_running(inventory.to_string().as_bytes()));
        let (command, stdout) = device
            .capture_command(Path::new("/fixture/screenshot.png"), None)
            .unwrap();
        assert!(!stdout);
        assert_eq!(command.as_std().get_program(), "/usr/bin/xcrun");
        assert_eq!(
            command.as_std().get_args().collect::<Vec<_>>(),
            [
                "simctl",
                "io",
                "AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA",
                "screenshot",
                "/fixture/screenshot.png",
            ]
        );
        let device = Device::Android("emulator-5554".into());
        assert!(device.is_running(
            b"List of devices attached\nphysical-phone\tdevice\nemulator-5554\tdevice\n"
        ));
        assert!(!device.is_running(
            b"List of devices attached\nemulator-5554\toffline\nemulator-5556\tdevice\n"
        ));
        let (command, stdout) = device
            .capture_command(
                Path::new("/fixture/screenshot.png"),
                Some(Path::new("/bin/sh")),
            )
            .unwrap();
        assert!(stdout);
        assert_eq!(
            command.as_std().get_args().collect::<Vec<_>>(),
            [
                "-H",
                "127.0.0.1",
                "-P",
                "5037",
                "-s",
                "emulator-5554",
                "exec-out",
                "screencap",
                "-p",
            ]
        );
    }

    #[test]
    fn accepts_only_explicit_simulator_identifiers() {
        let device = |platform, id: &str| {
            DeviceInput {
                platform,
                device_id: id.into(),
            }
            .device()
        };
        assert!(matches!(
            device(Platform::Ios, "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"),
            Some(Device::Ios(_))
        ));
        assert!(device(Platform::Ios, "booted").is_none());
        assert!(matches!(
            device(Platform::Android, "emulator-5554"),
            Some(Device::Android(_))
        ));
        assert!(device(Platform::Android, "physical-phone").is_none());
        assert!(serde_json::from_value::<DeviceInput>(json!({
            "platform":"android","deviceId":"emulator-5554","path":"/etc/passwd"
        }))
        .is_err());
    }

    #[tokio::test]
    async fn direct_oss_upload_omits_node_token_and_only_definitive_missing_allows_explicit_put_retry(
    ) {
        use axum::{
            body::Bytes,
            http::HeaderMap,
            response::IntoResponse,
            routing::{post, put},
            Json, Router,
        };
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };

        let prepares = Arc::new(AtomicUsize::new(0));
        let puts = Arc::new(AtomicUsize::new(0));
        let completions = Arc::new(AtomicUsize::new(0));
        let job = Job {
            task_id: Uuid::new_v4(),
            job_id: None,
            platform: Some("ios".into()),
            bundle_identifier: Some("fixture.app".into()),
            target: None,
            lease: Uuid::new_v4(),
            profile: None,
            prompt: "Amberia 首页".into(),
        };
        let lease = job.lease.to_string();
        let png = store_protocol::upload::decode_chunk("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLbtAAAAABJRU5ErkJggg==").unwrap();
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let upload_url = format!("http://{address}/oss?x-oss-signature=fixture");
        let calls = Arc::clone(&prepares);
        let app = Router::new().route(
            "/v1/connectors/{connector}/screenshot-jobs/{job}/upload-ticket",
            post(move |headers: HeaderMap, Json(metadata): Json<Value>| {
                let attempt = calls.fetch_add(1, Ordering::SeqCst);
                let lease = lease.clone();
                let upload_url=upload_url.clone();
                async move {
                    assert_eq!(headers["x-screenshot-lease"], lease);
                    assert_eq!(headers["authorization"],format!("Bearer {}","a".repeat(64)));
                    if attempt == 0 {
                        (
                            StatusCode::SERVICE_UNAVAILABLE,
                            Json(json!({"error":{"code":"screenshot_upload_busy"}})),
                        )
                            .into_response()
                    } else {
                        let md5=hex::decode(metadata["md5"].as_str().unwrap()).unwrap();
                        Json(json!({"uploadUrl":upload_url,"objectKey":"screenshots/fixture.png","expiresAt":store_protocol::upload::now().unwrap()+300,
                            "headers":{"content-type":"image/png","content-length":metadata["sizeBytes"].as_u64().unwrap().to_string(),"content-md5":store_protocol::upload::encode_chunk(&md5),"x-oss-meta-sha256":metadata["sha256"],"x-oss-meta-md5":metadata["md5"],"x-oss-forbid-overwrite":"true"}})).into_response()
                    }
                }
            }),
        );
        let calls = puts.clone();
        let expected = png.clone();
        let app = app.route(
            "/oss",
            put(move |headers: HeaderMap, body: Bytes| {
                let calls = calls.clone();
                let expected = expected.clone();
                async move {
                    assert!(headers.get("authorization").is_none());
                    assert_eq!(headers["content-type"], "image/png");
                    assert_eq!(body.as_ref(), expected.as_slice());
                    calls.fetch_add(1, Ordering::SeqCst);
                    StatusCode::OK
                }
            }),
        );
        let calls = completions.clone();
        let lease = job.lease.to_string();
        let app=app.route("/v1/connectors/{connector}/screenshot-jobs/{job}/complete",post(move |headers:HeaderMap,Json(body):Json<Value>|{
            let attempt=calls.fetch_add(1,Ordering::SeqCst);let lease=lease.clone();
            async move {
                assert_eq!(headers["x-screenshot-lease"],lease);
                assert_eq!(headers["authorization"],format!("Bearer {}","a".repeat(64)));
                assert_eq!(body,json!({}));
                if attempt==0 { (StatusCode::SERVICE_UNAVAILABLE,Json(json!({"error":{"code":"screenshot_verification_unavailable"}}))).into_response() }
                else if attempt==1 { (StatusCode::CONFLICT,Json(json!({"error":{"code":"screenshot_object_missing"}}))).into_response() }
                else { Json(json!({"delivered":true,"result":{"imageUrl":"https://cdn.fixture.example/fixture.png"}})).into_response() }
            }
        }));
        let (shutdown, stopped) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = stopped.await;
                })
                .await
                .unwrap();
        });
        let config = Config {
            kind: "screenshot".into(),
            connector_id: Uuid::new_v4().to_string(),
            token: "a".repeat(64),
            server_url: Url::parse(&format!("http://{address}")).unwrap(),
            message_key: "fixture".into(),
            codex_path: None,
            codex_home: None,
            adb_path: None,
            ca_file: None,
        };
        let workspace = tempfile::tempdir().unwrap();
        let worker = Worker::new(&config, &workspace.path().join("connector.json")).unwrap();
        let screenshot_id = Uuid::new_v4();
        let mut tools = JobTools {
            worker: &worker,
            job: &job,
            root: workspace.path(),
            adb: None,
            screenshot: Some((screenshot_id, png)),
            current_target: None,
            completed_targets: std::collections::BTreeSet::new(),
            upload_attempted: None,
            retry_upload: false,
            center_finished: false,
            failure: "codex_failed",
        };
        let (_stop, mut stop) = watch::channel(false);
        let wrong = tools
            .call(
                "upload_screenshot",
                json!({"screenshotId":Uuid::new_v4()}),
                &mut stop,
            )
            .await;
        assert_eq!(wrong.value["code"], "screenshot_not_found");
        assert_eq!(prepares.load(Ordering::SeqCst), 0);
        let args = json!({"screenshotId":screenshot_id});
        let busy = tools
            .call("upload_screenshot", args.clone(), &mut stop)
            .await;
        assert_eq!(busy.value["retryable"], true);
        assert!(!tools.center_finished);
        let pending = tools
            .call("upload_screenshot", args.clone(), &mut stop)
            .await;
        assert_eq!(pending.value["code"], "screenshot_verification_unavailable");
        assert_eq!(pending.value["retryable"], true);
        assert_eq!(puts.load(Ordering::SeqCst), 1);
        let missing = tools
            .call("upload_screenshot", args.clone(), &mut stop)
            .await;
        assert_eq!(missing.value["code"], "screenshot_object_missing");
        assert_eq!(puts.load(Ordering::SeqCst), 1);
        let delivered = tools
            .call("upload_screenshot", args.clone(), &mut stop)
            .await;
        assert!(
            delivered.success && delivered.change == GoalChange::Delivered && tools.center_finished
        );
        let repeated = tools.call("upload_screenshot", args, &mut stop).await;
        assert!(!repeated.success);
        assert_eq!(prepares.load(Ordering::SeqCst), 3);
        assert_eq!(puts.load(Ordering::SeqCst), 2);
        assert_eq!(completions.load(Ordering::SeqCst), 3);
        assert_eq!(
            delivered.value["result"]["imageUrl"],
            "https://cdn.fixture.example/fixture.png"
        );
        shutdown.send(()).unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn multiple_targets_keep_identity_and_continue_until_every_image_is_confirmed() {
        let workspace = tempfile::tempdir().unwrap();
        let job: Job = serde_json::from_value(json!({
            "taskId":Uuid::new_v4(),"jobId":"external/订单-42","lease":Uuid::new_v4(),
            "platform":"ios","bundleIdentifier":"fixture.app","profile":"optional hint",
            "prompt":"原文\n  keep spacing and \"quotes\"",
            "target":[
                {"locale":"zh-CN","page":"home","device":"phone"},
                {"locale":"en-US","page":"settings","device":"tablet","theme":"dark"}
            ]
        }))
        .unwrap();
        let config = Config {
            kind: "screenshot".into(),
            connector_id: Uuid::new_v4().to_string(),
            token: "a".repeat(64),
            server_url: Url::parse("https://fixture.example").unwrap(),
            message_key: "fixture".into(),
            codex_path: None,
            codex_home: None,
            adb_path: None,
            ca_file: None,
        };
        let worker = Worker::new(&config, &workspace.path().join("connector.json")).unwrap();
        let image = Uuid::new_v4();
        let mut tools = JobTools {
            worker: &worker,
            job: &job,
            root: workspace.path(),
            adb: None,
            screenshot: Some((image, vec![1, 2, 3])),
            current_target: Some(1),
            completed_targets: std::collections::BTreeSet::new(),
            upload_attempted: Some(image),
            retry_upload: true,
            center_finished: false,
            failure: "delivery_unknown",
        };
        let (_stop, mut stop) = watch::channel(false);
        let context = tools.call("get_capture_task", json!({}), &mut stop).await;
        assert_eq!(context.value["taskId"], job.task_id.to_string());
        assert_eq!(context.value["jobId"], "external/订单-42");
        assert_eq!(context.value["prompt"], job.prompt);
        assert_eq!(context.value["target"][1]["theme"], "dark");
        let invalid_context = tools
            .call("get_capture_task", json!({"unexpected":true}), &mut stop)
            .await;
        assert_eq!(invalid_context.value["code"], "invalid_arguments");
        assert_eq!(tools.target_body(json!({})), json!({"targetIndex":1}));
        let mismatch = tools
            .call(
                "upload_screenshot",
                json!({"screenshotId":image,"targetIndex":0}),
                &mut stop,
            )
            .await;
        assert_eq!(mismatch.value["code"], "invalid_target_index");
        assert!(!tools.valid_target(None));
        assert!(!tools.valid_target(Some(2)));
        let first = tools
            .confirmed_upload(json!({"delivered":false,"result":{"images":[{"targetIndex":1}]}}));
        assert!(first.success && first.change == GoalChange::Continue);
        assert!(!tools.center_finished);
        assert!(tools.screenshot.is_none() && tools.upload_attempted.is_none());
        assert!(!tools.retry_upload);
        assert!(!tools.valid_target(Some(1)));
        assert!(tools.valid_target(Some(0)));
        tools.current_target = Some(0);
        tools.screenshot = Some((Uuid::new_v4(), vec![4, 5, 6]));
        let last = tools.confirmed_upload(
            json!({"delivered":true,"result":{"images":[{"targetIndex":0},{"targetIndex":1}]}}),
        );
        assert!(last.success && last.change == GoalChange::Delivered);
        assert!(tools.center_finished && tools.screenshot.is_none());
        assert_eq!(last.value["result"]["images"], json!([{"targetIndex":0}]));
    }

    #[test]
    fn accepts_nullable_legacy_identity_and_largest_escaped_claim_fixture() {
        let legacy = json!({
            "taskId":Uuid::new_v4(),"jobId":null,"platform":null,
            "bundleIdentifier":null,"target":null,"profile":null,
            "lease":Uuid::new_v4(),"prompt":"historical single image"
        });
        let decoded: Job = serde_json::from_value(legacy.clone()).unwrap();
        assert!(decoded.platform.is_none() && decoded.bundle_identifier.is_none());
        assert!(decoded.target.is_none() && decoded.job_id.is_none());
        let prompt = "\u{0001}".repeat(16 * 1024);
        let targets: Vec<Value> = (0..256)
            .map(|index| {
                json!({
                    "locale":"a".repeat(80),
                    "page":format!("{index:03}{}", "p".repeat(97)),
                    "device":"phone","theme":"dark"
                })
            })
            .collect();
        assert!(serde_json::to_vec(&targets).unwrap().len() <= 64 * 1024);
        let mut claim = legacy;
        claim["prompt"] = json!(prompt);
        claim["target"] = json!(targets);
        claim["jobId"] = json!("e".repeat(180));
        claim["platform"] = json!("ios");
        claim["bundleIdentifier"] = json!("fixture.app");
        let body = serde_json::to_vec(&claim).unwrap();
        assert!(body.len() > 128 * 1024);
        assert!(body.len() <= MAX_CLAIM);
        let decoded: Job = serde_json::from_slice(&body).unwrap();
        assert_eq!(decoded.prompt, prompt);
        assert_eq!(decoded.target.unwrap().len(), 256);
    }

    #[tokio::test]
    async fn sends_prompt_on_stdin_and_ends_the_owned_process_group() {
        let workspace = tempfile::tempdir().unwrap();
        let mut command = Command::new("/bin/sh");
        command
            .current_dir(workspace.path())
            .args(["-c", "cat > input; (sleep 1; touch leaked) &"]);
        let (_stop, mut stopped) = watch::channel(false);
        let status = run_command(
            &mut command,
            Some("合成截图任务".as_bytes()),
            Duration::from_secs(5),
            &mut stopped,
            None,
        )
        .await
        .unwrap();
        assert!(status.success());
        assert_eq!(
            fs::read_to_string(workspace.path().join("input")).unwrap(),
            "合成截图任务"
        );
        sleep(Duration::from_millis(1200)).await;
        assert!(!workspace.path().join("leaked").exists());
    }

    #[tokio::test]
    async fn shutdown_cancels_a_running_command_before_it_writes() {
        let workspace = tempfile::tempdir().unwrap();
        let mut command = Command::new("/bin/sh");
        command
            .current_dir(workspace.path())
            .args(["-c", "touch started; sleep 2; touch leaked"]);
        let (stop, mut stopped) = watch::channel(false);
        let started = workspace.path().join("started");
        let cancel = async {
            timeout(Duration::from_secs(2), async {
                while !started.exists() {
                    sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            stop.send_replace(true);
        };
        let (result, ()) = tokio::join!(
            run_command(
                &mut command,
                None,
                Duration::from_secs(10),
                &mut stopped,
                None
            ),
            cancel
        );
        assert!(result.is_err());
        sleep(Duration::from_millis(1100)).await;
        assert!(!workspace.path().join("leaked").exists());
    }

    #[tokio::test]
    async fn drains_stderr_after_the_capture_limit_and_returns_the_cli_exit_status() {
        let mut command = Command::new("/bin/sh");
        command.args([
            "-c",
            r#"
            printf '%s\n' "error: unexpected argument '--sandbox' found" >&2
            i=0
            while [ "$i" -lt 3000 ]; do
                printf '%s\n' 'synthetic output that must not block the process' >&2
                i=$((i+1))
            done
            exit 2
        "#,
        ]);
        let (_stop, mut stopped) = watch::channel(false);
        let mut stderr = Vec::new();
        let status = run_command(
            &mut command,
            Some(b"fixture task"),
            Duration::from_secs(5),
            &mut stopped,
            Some(&mut stderr),
        )
        .await
        .unwrap();
        assert_eq!(status.code(), Some(2));
        assert_eq!(stderr.len(), CODEX_STDERR_LIMIT);
        assert_eq!(
            codex_diagnostic(&stderr),
            ("unexpected_argument", Some("--sandbox"))
        );
    }
}
