use super::{
    diagnostic::Log,
    goal::{self, GoalChange, ToolReply},
    Failure,
};
use reqwest::{header, Client, Method, RequestBuilder, StatusCode, Url};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
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
    job_id: Uuid,
    lease: Uuid,
    profile: Option<String>,
    prompt: String,
}

pub(super) struct Worker<'a> {
    config: &'a Config,
    client: Client,
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
            let mut response = self.request(Method::POST, "claim").send().await?;
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
                if chunk.len() > 128 * 1024 - body.len() {
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
        let id = Some(job.job_id);
        self.log.prompt(job.job_id, &job.prompt);
        let outcome = self.run_goal(&job, stop).await;
        let stopping = *stop.borrow();
        let Err(code) = outcome else {
            return Ok(!stopping);
        };

        self.log.event(id, format_args!("job_failed code={code}"));
        let request = self
            .request(Method::POST, &format!("{}/failure", job.job_id))
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
            self.log.event(Some(job.job_id), "task_cleanup_failed");
        }
        result
    }

    async fn run_goal_in(
        &self,
        root: &Path,
        job: &Job,
        stop: &mut watch::Receiver<bool>,
    ) -> Result<(), &'static str> {
        let id = Some(job.job_id);
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
            profile: job.profile.as_deref(),
            job: job.job_id,
        };
        let mut tools = JobTools {
            worker: self,
            job,
            root,
            adb: adb.as_deref(),
            screenshot: None,
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
    center_finished: bool,
    failure: &'static str,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UploadInput {
    screenshot_id: Uuid,
}

impl JobTools<'_, '_> {
    async fn call(
        &mut self,
        name: &str,
        arguments: Value,
        stop: &mut watch::Receiver<bool>,
    ) -> ToolReply {
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
        let id = Some(self.job.job_id);
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
                self.failure = "capture_failed";
                self.worker
                    .log
                    .event(id, format_args!("image_ready bytes={size}"));
                ToolReply::ok(
                    json!({"screenshotId":screenshot_id,"bytes":size}),
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
        let id = Some(self.job.job_id);
        let request = self
            .worker
            .request(Method::PUT, &format!("{}/image", self.job.job_id))
            .header(header::CONTENT_TYPE, "image/png")
            .header("X-Screenshot-Lease", self.job.lease.to_string())
            .body(bytes.clone())
            .send();
        let result = tokio::select! {
            biased;
            _ = stop.wait_for(|stopped| *stopped) => {
                // The request may already have reached the center.
                self.center_finished = true;
                return ToolReply::error("delivery_unknown", "Delivery was interrupted.", false, GoalChange::Blocked);
            }
            result = request => result,
        };
        let Ok(response) = result else {
            self.center_finished = true;
            self.worker.log.event(id, "delivery_unknown");
            return ToolReply::error(
                "delivery_unknown",
                "The center did not confirm delivery. Repeating could duplicate the upload.",
                false,
                GoalChange::Blocked,
            );
        };
        if response.status() == StatusCode::NO_CONTENT {
            self.center_finished = true;
            self.screenshot = None;
            self.worker.log.event(id, "image_delivered");
            return ToolReply::ok(json!({"delivered":true}), GoalChange::Delivered);
        }
        let status = response.status();
        let code = timeout(Duration::from_secs(2), result_error_code(response))
            .await
            .unwrap_or("unknown");
        self.worker.log.event(
            id,
            format_args!("result_rejected http={} code={code}", status.as_u16()),
        );
        match (status, code) {
            (StatusCode::SERVICE_UNAVAILABLE, "screenshot_upload_busy") => ToolReply::error(
                code,
                "The center is busy. Wait before retrying the same screenshotId.",
                true,
                GoalChange::Continue,
            ),
            (StatusCode::UNPROCESSABLE_ENTITY, "invalid_image")
            | (StatusCode::PAYLOAD_TOO_LARGE, "image_too_large") => {
                self.screenshot = None;
                ToolReply::error(
                    code,
                    "The center rejected this image. Capture a fresh screenshot and retry.",
                    true,
                    GoalChange::Continue,
                )
            }
            _ => {
                self.center_finished = true;
                let message = if code == "screenshot_upload_unconfigured" {
                    "The center has no configured external upload destination."
                } else {
                    "The center rejected delivery or could not confirm it. Further upload attempts are disabled for this task."
                };
                ToolReply::error(code, message, false, GoalChange::Blocked)
            }
        }
    }
}

async fn result_error_code(mut response: reqwest::Response) -> &'static str {
    let mut body = Vec::new();
    while let Ok(Some(chunk)) = response.chunk().await {
        if chunk.len() > 4096 - body.len() {
            return "unknown";
        }
        body.extend_from_slice(&chunk);
    }
    let Ok(body) = serde_json::from_slice::<serde_json::Value>(&body) else {
        return "unknown";
    };
    match body["error"]["code"].as_str() {
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
    async fn upload_retries_busy_and_requires_the_center_to_confirm_delivery() {
        use axum::{
            body::Bytes, http::HeaderMap, response::IntoResponse, routing::put, Json, Router,
        };
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };

        let count = Arc::new(AtomicUsize::new(0));
        let job = Job {
            job_id: Uuid::new_v4(),
            lease: Uuid::new_v4(),
            profile: None,
            prompt: "Amberia 首页".into(),
        };
        let lease = job.lease.to_string();
        let calls = Arc::clone(&count);
        let app = Router::new().route(
            "/{*path}",
            put(move |headers: HeaderMap, body: Bytes| {
                let attempt = calls.fetch_add(1, Ordering::SeqCst);
                let lease = lease.clone();
                async move {
                    assert_eq!(headers["x-screenshot-lease"], lease);
                    assert!(body.starts_with(PNG));
                    if attempt == 0 {
                        (
                            StatusCode::SERVICE_UNAVAILABLE,
                            Json(json!({"error":{"code":"screenshot_upload_busy"}})),
                        )
                            .into_response()
                    } else {
                        StatusCode::NO_CONTENT.into_response()
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
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
            screenshot: Some((screenshot_id, PNG.to_vec())),
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
        assert_eq!(count.load(Ordering::SeqCst), 0);
        let args = json!({"screenshotId":screenshot_id});
        let busy = tools
            .call("upload_screenshot", args.clone(), &mut stop)
            .await;
        assert_eq!(busy.value["retryable"], true);
        assert!(!tools.center_finished);
        let delivered = tools
            .call("upload_screenshot", args.clone(), &mut stop)
            .await;
        assert!(
            delivered.success && delivered.change == GoalChange::Delivered && tools.center_finished
        );
        let repeated = tools.call("upload_screenshot", args, &mut stop).await;
        assert!(!repeated.success);
        assert_eq!(count.load(Ordering::SeqCst), 2);
        shutdown.send(()).unwrap();
        server.await.unwrap();
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
