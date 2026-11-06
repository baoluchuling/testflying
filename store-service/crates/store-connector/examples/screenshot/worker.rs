use super::Failure;
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
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::AsyncWriteExt,
    net::TcpStream,
    process::Command,
    sync::watch,
    time::{sleep, timeout},
};
use uuid::Uuid;

const MAX_IMAGE: usize = 16 * 1024 * 1024;
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
    #[serde(default)]
    codex_path: PathBuf,
    #[serde(default)]
    codex_home: PathBuf,
    #[serde(default)]
    codex_permission_profile: String,
    #[serde(default)]
    profiles: HashMap<String, Profile>,
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
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Profile {
    platform: String,
    project_root: PathBuf,
    device_id: String,
    adb_path: Option<PathBuf>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Job {
    job_id: Uuid,
    lease: Uuid,
    profile: String,
    prompt: String,
}

pub(super) struct Worker<'a> {
    config: &'a Config,
    client: Client,
    jobs: String,
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
                _ => return Err::<_, Failure>("Center refused the screenshot claim".into()),
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
            result = claim => result?,
            _ = stop.wait_for(|stopped| *stopped) => return Ok(false),
        };
        let Some(job) = job else {
            return Ok(false);
        };
        let image = self.capture(&job, stop).await;
        let stopping = *stop.borrow();
        let request = match image {
            Ok(image) if !stopping => self
                .request(Method::PUT, &format!("{}/image", job.job_id))
                .header(header::CONTENT_TYPE, "image/png")
                .body(image),
            outcome => {
                let code = outcome.err().unwrap_or("capture_failed");
                eprintln!("Screenshot job failed: {code}");
                self.request(Method::POST, &format!("{}/failure", job.job_id))
                    .json(&json!({"code":code}))
            }
        };
        let delivery = request
            .header("X-Screenshot-Lease", job.lease.to_string())
            .send();
        let response = if stopping {
            timeout(Duration::from_secs(3), delivery).await??
        } else {
            tokio::select! {
                response = delivery => response?,
                _ = stop.wait_for(|stopped| *stopped) => return Ok(false),
            }
        };
        if response.status() != StatusCode::NO_CONTENT {
            return Err("Center refused the screenshot result".into());
        }
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
        let unavailable = |_| "profile_unavailable";
        let profile = self
            .config
            .profiles
            .get(&job.profile)
            .ok_or("profile_unavailable")?;
        let project = local_path(&profile.project_root, true).map_err(unavailable)?;
        let codex = local_path(&self.config.codex_path, false).map_err(unavailable)?;
        let codex_home = local_path(&self.config.codex_home, true).map_err(unavailable)?;
        let permission = &self.config.codex_permission_profile;
        if !valid_permission(permission) {
            return Err("profile_unavailable");
        }
        let png = root.join("screenshot.png");
        let (mut capture, device, stdout) = profile.capture_command(&png).map_err(unavailable)?;
        if stdout {
            // Only use the operator-started local ADB server.
            tokio::select! {
                result = timeout(Duration::from_secs(2), TcpStream::connect(("127.0.0.1", 5037))) => {
                    result.map_err(|_| "profile_unavailable")?.map_err(unavailable)?;
                }
                _ = stop.wait_for(|stopped| *stopped) => return Err("capture_failed"),
            }
        }

        let schema = root.join("codex-result-schema.json");
        let result = root.join("codex-result.json");
        fs::write(&schema, br#"{"type":"object","properties":{"ready":{"type":"boolean"}},"required":["ready"],"additionalProperties":false}"#)
            .map_err(|_| "codex_failed")?;
        let prompt = format!(
            "Prepare an app screen for a screenshot on the configured simulator. Decide whether to build, install, launch, or interact with the app. Use only the indicated simulator. Treat the project as a local source reference. Keep generated files in the current working directory, copy source there if a build writes in-tree, and use the DerivedData and Gradle cache paths in the environment. Do not start or stop other devices or upload files. Leave the requested screen visible. Report ready=true only when ready for capture.\nProject: {}\nPlatform: {}\nSimulator: {}\nTask:\n{}",
            project.display(), profile.platform, device, job.prompt);
        let mut command = Command::new(codex);
        command
            .args(["--ask-for-approval", "never", "--ignore-user-config", "-c"])
            .arg(format!("default_permissions={}", json!(permission)))
            .args([
                "exec",
                "--ephemeral",
                "--skip-git-repo-check",
                "--output-schema",
            ])
            .arg(&schema)
            .arg("--output-last-message")
            .arg(&result)
            .arg("-")
            .stdout(Stdio::null());
        job_environment(&mut command, root, &codex_home);
        run_command(
            &mut command,
            Some(prompt.as_bytes()),
            Duration::from_secs(24 * 60),
            stop,
        )
        .await
        .map_err(|_| "codex_failed")?;
        let ready = read_owned(&result, 4096, false).map_err(|_| "codex_failed")?;
        if serde_json::from_slice::<serde_json::Value>(&ready).ok() != Some(json!({"ready":true})) {
            return Err("codex_failed");
        }

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
        job_environment(&mut capture, root, &codex_home);
        run_command(&mut capture, None, Duration::from_secs(45), stop)
            .await
            .map_err(|_| "capture_failed")?;
        let bytes = read_owned(&png, MAX_IMAGE, false).map_err(|_| "capture_failed")?;
        if bytes.len() < 33 || !bytes.starts_with(PNG) {
            return Err("capture_failed");
        }
        Ok(bytes)
    }
}

impl Profile {
    fn capture_command(&self, png: &Path) -> io::Result<(Command, String, bool)> {
        if self.platform == "ios" {
            let device = Uuid::parse_str(&self.device_id)
                .map_err(io::Error::other)?
                .to_string()
                .to_uppercase();
            let mut command = Command::new("/usr/bin/xcrun");
            command
                .args(["simctl", "io", &device, "screenshot"])
                .arg(png);
            return Ok((command, device, false));
        }
        let device = &self.device_id;
        if self.platform == "android"
            && device.strip_prefix("emulator-").is_some_and(|serial| {
                (4..=5).contains(&serial.len()) && serial.bytes().all(|b| b.is_ascii_digit())
            })
        {
            let adb = self
                .adb_path
                .as_deref()
                .ok_or_else(|| io::Error::other("ADB path required"))?;
            let mut command = Command::new(local_path(adb, false)?);
            command.args([
                "-H",
                "127.0.0.1",
                "-P",
                "5037",
                "-s",
                device,
                "exec-out",
                "screencap",
                "-p",
            ]);
            return Ok((command, device.clone(), true));
        }
        Err(io::Error::other(
            "Configure an iOS simulator or Android emulator",
        ))
    }
}

fn valid_permission(permission: &str) -> bool {
    let mut bytes = permission.bytes();
    permission.len() <= 64
        && bytes
            .next()
            .is_some_and(|first| first.is_ascii_alphabetic())
        && bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

fn job_environment(command: &mut Command, root: &Path, codex_home: &Path) {
    command.current_dir(root).env_clear();
    for key in [
        "HOME",
        "PATH",
        "USER",
        "LOGNAME",
        "LANG",
        "DEVELOPER_DIR",
        "ANDROID_HOME",
        "ANDROID_SDK_ROOT",
    ] {
        if let Some(value) = env::var_os(key) {
            command.env(key, value);
        }
    }
    command
        .env("CODEX_HOME", codex_home)
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
) -> io::Result<()> {
    if *stop.borrow() {
        return Err(io::Error::other("Connector is shutting down"));
    }
    let mut child = command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stderr(Stdio::null())
        .process_group(0)
        .kill_on_drop(true)
        .spawn()?;
    let pid = child.id().expect("new child has a PID") as libc::pid_t;
    let completion = timeout(limit, async {
        if let Some(input) = input {
            let mut stdin = child.stdin.take().expect("child stdin is piped");
            stdin.write_all(input).await?;
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
        result = completion => result.map_err(io::Error::other).and_then(|r| r),
        _ = stop.wait_for(|stopped| *stopped) => Err(io::Error::other("Connector is shutting down")),
    };
    // The unreaped direct child reserves this group ID across both signals.
    if result.is_err() {
        unsafe { libc::kill(-pid, libc::SIGTERM) };
        sleep(Duration::from_secs(1)).await;
    }
    unsafe { libc::kill(-pid, libc::SIGKILL) };
    let status = timeout(Duration::from_secs(5), child.wait())
        .await
        .map_err(io::Error::other)??;
    result?;
    if !status.success() {
        return Err(io::Error::other("Screenshot command failed"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_only_the_configured_emulator_or_simulator() {
        let mut profile = Profile {
            platform: "ios".into(),
            project_root: PathBuf::from("/fixture"),
            device_id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".into(),
            adb_path: None,
        };
        let (command, device, stdout) = profile
            .capture_command(Path::new("/fixture/screenshot.png"))
            .unwrap();
        assert_eq!(device, "AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA");
        assert!(!stdout);
        assert_eq!(command.as_std().get_program(), "/usr/bin/xcrun");
        assert_eq!(
            command.as_std().get_args().collect::<Vec<_>>(),
            [
                "simctl",
                "io",
                &device,
                "screenshot",
                "/fixture/screenshot.png",
            ]
        );
        profile.platform = "android".into();
        profile.device_id = "emulator-5554".into();
        profile.adb_path = Some(PathBuf::from("/bin/sh"));
        let (command, _, stdout) = profile
            .capture_command(Path::new("/fixture/screenshot.png"))
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
        profile.device_id = "physical-phone".into();
        assert!(profile
            .capture_command(Path::new("/fixture/screenshot.png"))
            .is_err());
    }

    #[tokio::test]
    async fn sends_prompt_on_stdin_and_ends_the_owned_process_group() {
        let workspace = tempfile::tempdir().unwrap();
        let mut command = Command::new("/bin/sh");
        command
            .current_dir(workspace.path())
            .args(["-c", "cat > input; (sleep 1; touch leaked) &"]);
        let (_stop, mut stopped) = watch::channel(false);
        run_command(
            &mut command,
            Some("合成截图任务".as_bytes()),
            Duration::from_secs(5),
            &mut stopped,
        )
        .await
        .unwrap();
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
            run_command(&mut command, None, Duration::from_secs(10), &mut stopped),
            cancel
        );
        assert!(result.is_err());
        sleep(Duration::from_millis(1100)).await;
        assert!(!workspace.path().join("leaked").exists());
    }
}
