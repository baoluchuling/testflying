use super::{
    diagnostic::{codex_diagnostic, Log},
    Failure,
};
use reqwest::{header, Client, Method, RequestBuilder, StatusCode, Url};
use serde::Deserialize;
use serde_json::json;
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
    process::Command,
    sync::watch,
    time::{sleep, timeout},
};
use uuid::Uuid;

const MAX_IMAGE: usize = 16 * 1024 * 1024;
const CODEX_STDERR_LIMIT: usize = 8 * 1024;
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";
const SELECTION_SCHEMA: &[u8] = br#"{
  "type":"object",
  "properties":{
    "ready":{"type":"boolean"},
    "projectRoot":{"type":["string","null"]},
    "platform":{"type":["string","null"],"enum":["ios","android",null]},
    "deviceId":{"type":["string","null"]},
    "notReadyReason":{"type":["string","null"],"enum":["project_not_found","simulator_unavailable","ambiguous_selection","preparation_failed",null]}
  },
  "required":["ready","projectRoot","platform","deviceId","notReadyReason"],
  "additionalProperties":false
}"#;

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
        let image = self.capture(&job, stop).await;
        let stopping = *stop.borrow();
        let sending_image = image.is_ok() && !stopping;
        let request = match image {
            Ok(image) if !stopping => {
                self.log
                    .event(id, format_args!("image_ready bytes={}", image.len()));
                self.request(Method::PUT, &format!("{}/image", job.job_id))
                    .header(header::CONTENT_TYPE, "image/png")
                    .body(image)
            }
            outcome => {
                let code = outcome.err().unwrap_or("capture_failed");
                self.log.event(id, format_args!("job_failed code={code}"));
                self.request(Method::POST, &format!("{}/failure", job.job_id))
                    .json(&json!({"code":code}))
            }
        };
        let delivery = request
            .header("X-Screenshot-Lease", job.lease.to_string())
            .send();
        let response: Result<_, Failure> = if stopping {
            match timeout(Duration::from_secs(3), delivery).await {
                Ok(response) => response.map_err(Into::into),
                Err(error) => Err(error.into()),
            }
        } else {
            tokio::select! {
                response = delivery => response.map_err(Into::into),
                _ = stop.wait_for(|stopped| *stopped) => return Ok(false),
            }
        };
        let response = response.inspect_err(|_| self.log.event(id, "result_request_failed"))?;
        if response.status() != StatusCode::NO_CONTENT {
            let status = response.status().as_u16();
            let code = timeout(Duration::from_secs(2), result_error_code(response))
                .await
                .unwrap_or("unknown");
            self.log.event(
                id,
                format_args!("result_rejected http={status} code={code}"),
            );
            return Err("Center refused the screenshot result".into());
        }
        self.log.event(
            id,
            if sending_image {
                "image_delivered"
            } else {
                "failure_reported"
            },
        );
        Ok(!stopping)
    }

    async fn capture(
        &self,
        job: &Job,
        stop: &mut watch::Receiver<bool>,
    ) -> Result<Vec<u8>, &'static str> {
        let workspace = tempfile::Builder::new()
            .prefix("testflying-screenshot-")
            .tempdir()
            .map_err(|_| "capture_failed")?;
        let result = self.capture_in(workspace.path(), job, stop).await;
        workspace.close().map_err(|_| "capture_failed")?;
        result
    }

    async fn capture_in(
        &self,
        root: &Path,
        job: &Job,
        stop: &mut watch::Receiver<bool>,
    ) -> Result<Vec<u8>, &'static str> {
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

        let schema = root.join("codex-result-schema.json");
        let result = root.join("codex-result.json");
        fs::write(&schema, SELECTION_SCHEMA)
            .inspect_err(|_| self.log.event(id, "codex_schema_write_failed"))
            .map_err(|_| "codex_failed")?;
        let prompt = self.session_prompt(job, adb.as_deref());
        let mut command = Command::new(codex);
        command
            .args(["--ask-for-approval", "never"])
            .args([
                "exec",
                "--sandbox",
                "workspace-write",
                "--ephemeral",
                "--skip-git-repo-check",
                "--output-schema",
            ])
            .arg(&schema)
            .arg("--output-last-message")
            .arg(&result)
            .arg("-")
            .stdout(Stdio::null());
        job_environment(&mut command, root, codex_home.as_deref());
        self.log.event(id, "codex_starting");
        let mut stderr = Vec::new();
        let status = run_command(
            &mut command,
            Some(prompt.as_bytes()),
            Duration::from_secs(24 * 60),
            stop,
            Some(&mut stderr),
        )
        .await
        .map_err(|error| {
            let kind = error.kind();
            let os_error = error.raw_os_error();
            self.log.event(
                id,
                format_args!("codex_process_error kind={kind:?} os_error={os_error:?}"),
            );
            "codex_failed"
        })?;
        if !status.success() {
            let (kind, flag) = codex_diagnostic(&stderr);
            self.log.event(
                id,
                format_args!(
                    "codex_exit_failed status={status} diagnostic={kind} flag={}",
                    flag.unwrap_or("none")
                ),
            );
            return Err("codex_failed");
        }
        let selection = read_owned(&result, 4096, false)
            .inspect_err(|error| {
                self.log.event(
                    id,
                    format_args!("codex_result_unreadable kind={:?}", error.kind()),
                )
            })
            .map_err(|_| "codex_failed")?;
        let selection: Selection = serde_json::from_slice(&selection)
            .inspect_err(|_| self.log.event(id, "codex_result_invalid"))
            .map_err(|_| "selection_invalid")?;
        if !selection.ready {
            let reason = match selection.not_ready_reason {
                Some(NotReadyReason::ProjectNotFound) => "project_not_found",
                Some(NotReadyReason::SimulatorUnavailable) => "simulator_unavailable",
                Some(NotReadyReason::AmbiguousSelection) => "ambiguous_selection",
                Some(NotReadyReason::PreparationFailed) => "preparation_failed",
                None => "unspecified",
            };
            self.log
                .event(id, format_args!("codex_not_ready reason={reason}"));
            return Err("codex_failed");
        }

        let device = selection.validate().ok_or_else(|| {
            self.log.event(id, "simulator_selection_invalid");
            "selection_invalid"
        })?;
        device
            .verify_running(root, adb.as_deref(), stop)
            .await
            .inspect_err(|_| self.log.event(id, "simulator_unavailable"))
            .map_err(|_| "selection_invalid")?;
        self.log.event(id, "simulator_verified");
        let png = root.join("screenshot.png");
        let (mut capture, stdout) = device
            .capture_command(&png, adb.as_deref())
            .map_err(|_| "capture_failed")?;
        if stdout {
            let output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&png)
                .map_err(|_| "capture_failed")?;
            capture.stdout(output);
        } else if png.symlink_metadata().is_ok() {
            return Err("capture_failed");
        } else {
            capture.stdout(Stdio::null());
        }
        job_environment(&mut capture, root, None);
        let status = run_command(&mut capture, None, Duration::from_secs(45), stop, None)
            .await
            .inspect_err(|error| {
                self.log.event(
                    id,
                    format_args!("capture_process_error kind={:?}", error.kind()),
                )
            })
            .map_err(|_| "capture_failed")?;
        if !status.success() {
            self.log
                .event(id, format_args!("capture_exit_failed status={status}"));
            return Err("capture_failed");
        }
        let bytes = read_owned(&png, MAX_IMAGE, false)
            .inspect_err(|error| {
                self.log
                    .event(id, format_args!("image_unreadable kind={:?}", error.kind()))
            })
            .map_err(|_| "capture_failed")?;
        if bytes.len() < 33 || !bytes.starts_with(PNG) {
            self.log.event(id, "image_invalid");
            return Err("capture_failed");
        }
        Ok(bytes)
    }

    fn session_prompt(&self, job: &Job, adb: Option<&Path>) -> String {
        let context = json!({
            "task": {
                "jobId": job.job_id,
                "connectorId": self.config.connector_id,
                "profile": job.profile,
                "prompt": job.prompt,
            },
            "adbPath": adb,
        });
        format!(
            "Find the app project for this task on this Mac using the local files and development tools available to this session. Search for the project by the app or project named in the task and verify it against local project files. Treat the optional task.profile as a hint. Choose one appropriate iOS Simulator or Android Emulator using simctl or local adb. Decide whether to build, install, launch or interact with the app and leave the requested screen visible. Treat project source as read-only. Keep generated files in the working directory and use the DerivedData and Gradle paths in the environment. Do not capture or upload screenshots yourself. Return ready=true, the absolute projectRoot, platform, deviceId and null notReadyReason only after the screen is ready on that booted simulator. Use the simulator UUID on iOS or the emulator serial on Android. If the project or device cannot be identified or the screen cannot be prepared, return ready=false, null for the selection fields, and the matching notReadyReason.\nTask context (JSON):\n{context}"
        )
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
        _ => "unknown",
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Selection {
    ready: bool,
    project_root: Option<PathBuf>,
    platform: Option<Platform>,
    device_id: Option<String>,
    not_ready_reason: Option<NotReadyReason>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum NotReadyReason {
    ProjectNotFound,
    SimulatorUnavailable,
    AmbiguousSelection,
    PreparationFailed,
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

impl Selection {
    fn validate(self) -> Option<Device> {
        let project = local_path(self.project_root.as_deref()?, true).ok()?;
        project.parent()?;
        let id = self.device_id?;
        match self.platform? {
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
    let stderr = child.stderr.take().map(|mut stderr| {
        tokio::spawn(async move {
            let mut captured = Vec::new();
            let mut buffer = [0; 2048];
            while let Ok(count @ 1..) = stderr.read(&mut buffer).await {
                let keep = count.min(CODEX_STDERR_LIMIT - captured.len());
                captured.extend_from_slice(&buffer[..keep]);
            }
            captured
        })
    });
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
    });
    let result = tokio::select! {
        result = completion => result.map_err(|e| io::Error::new(io::ErrorKind::TimedOut, e)).and_then(|r| r),
        _ = stop.wait_for(|stopped| *stopped) => Err(io::Error::new(io::ErrorKind::Interrupted, "Connector is shutting down")),
    };
    // The unreaped direct child reserves this group ID across both signals.
    if result.is_err() {
        unsafe { libc::kill(-pid, libc::SIGTERM) };
        sleep(Duration::from_secs(1)).await;
    }
    unsafe { libc::kill(-pid, libc::SIGKILL) };
    let status = timeout(Duration::from_secs(5), child.wait()).await;
    if let (Some(sink), Some(mut stderr)) = (stderr_capture, stderr) {
        match timeout(Duration::from_secs(1), &mut stderr).await {
            Ok(Ok(captured)) => *sink = captured,
            Ok(Err(_)) => {}
            Err(_) => stderr.abort(),
        }
    }
    result?;
    status.map_err(|error| io::Error::new(io::ErrorKind::TimedOut, error))?
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn accepts_existing_local_projects_and_rejects_invalid_paths_and_physical_devices() {
        let workspace = tempfile::tempdir().unwrap();
        let path = workspace.path().canonicalize().unwrap();
        let selection = |project, device: &str| Selection {
            ready: true,
            project_root: Some(project),
            platform: Some(Platform::Android),
            device_id: Some(device.into()),
            not_ready_reason: None,
        };
        assert!(selection(path.clone(), "emulator-5554")
            .validate()
            .is_some());
        assert!(selection(path.clone(), "physical-phone")
            .validate()
            .is_none());
        for invalid in [
            PathBuf::from("/"),
            PathBuf::from("relative"),
            path.join("missing"),
        ] {
            assert!(selection(invalid, "emulator-5554").validate().is_none());
        }
    }

    #[tokio::test]
    async fn starts_a_fresh_codex_session_with_task_data_without_a_profile() {
        use std::os::unix::fs::PermissionsExt;

        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let codex = root.join("fixture-codex");
        fs::write(&codex, r#"#!/bin/sh
set -eu
printf '%s\n' "$@" > args
/bin/cat > input
while [ "$1" != "--output-last-message" ]; do shift; done
printf '%s\n' '{"ready":true,"projectRoot":"/","platform":"android","deviceId":"physical-phone"}' > "$2"
"#).unwrap();
        fs::set_permissions(&codex, fs::Permissions::from_mode(0o700)).unwrap();
        let node = Uuid::new_v4();
        let token = "a".repeat(64);
        let config: Config = serde_json::from_value(json!({
            "kind":"screenshot","connectorId":node,"token":token,
            "serverUrl":"https://example.invalid","messageKey":"fixture-key",
            "codexPath":codex,"codexHome":root,
        }))
        .unwrap();
        let worker = Worker::new(&config, &root.join("connector.json")).unwrap();
        let job = Job {
            job_id: Uuid::new_v4(),
            lease: Uuid::new_v4(),
            profile: None,
            prompt: "打开设置\n截图".into(),
        };
        let (_stop, mut stopped) = watch::channel(false);
        assert_eq!(
            worker.capture_in(&root, &job, &mut stopped).await,
            Err("selection_invalid")
        );
        let args = fs::read_to_string(root.join("args")).unwrap();
        assert!(args.starts_with(concat!(
            "--ask-for-approval\nnever\nexec\n",
            "--sandbox\nworkspace-write\n--ephemeral\n",
        )));
        assert!(!args.contains("resume") && !args.contains(&job.prompt));
        let stdin = fs::read_to_string(root.join("input")).unwrap();
        let (_, context) = stdin.split_once("Task context (JSON):\n").unwrap();
        let context: serde_json::Value = serde_json::from_str(context).unwrap();
        assert_eq!(
            context["task"],
            json!({"jobId":job.job_id,"connectorId":node,"profile":null,"prompt":job.prompt})
        );
        assert!(context.get("projectRoots").is_none() && context.get("profileHint").is_none());
        assert!(stdin.contains("Find the app project for this task on this Mac"));
        assert!(
            !stdin.contains(&token)
                && !stdin.contains(&job.lease.to_string())
                && !stdin.contains("fixture-key")
        );
        assert!(!root.join("device-list").exists());
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
