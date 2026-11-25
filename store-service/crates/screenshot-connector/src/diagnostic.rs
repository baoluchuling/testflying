use std::{
    fmt::Display,
    fs::{File, OpenOptions},
    io::{self, BufRead, BufReader, Seek, SeekFrom, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{FileExt, MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

const RETENTION_SECONDS: u64 = 24 * 60 * 60;

pub(super) struct Log {
    path: PathBuf,
}

impl Log {
    pub(super) fn for_config(path: &Path) -> Self {
        Self {
            path: path.with_file_name("connector.log"),
        }
    }

    pub(super) fn event(&self, job: Option<Uuid>, message: impl Display) {
        let job = job.map(|id| format!(" job={id}")).unwrap_or_default();
        let line = format!("time_unix={}{job} {message}", now());
        if self.append(&line).is_err() {
            eprintln!("{line}");
        }
    }

    pub(super) fn prompt(&self, job: Uuid, prompt: &str) {
        let prompt = serde_json::Value::String(prompt.into());
        if self
            .append(&format!(
                "time_unix={} job={job} job_claimed prompt={prompt}",
                now()
            ))
            .is_err()
        {
            eprintln!("Could not write the private Connector log for job {job}");
        }
    }

    fn open_locked(&self) -> io::Result<File> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&self.path)?;
        let meta = file.metadata()?;
        if !meta.is_file()
            || meta.nlink() != 1
            || meta.uid() != unsafe { libc::geteuid() }
            || meta.mode() & 0o077 != 0
        {
            return Err(io::Error::other("Expected a private Connector log"));
        }
        // Closing the descriptor releases the lock for other invocations.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(file)
    }

    fn append(&self, line: &str) -> io::Result<()> {
        let mut file = self.open_locked()?;
        file.seek(SeekFrom::End(0))?;
        writeln!(file, "{line}")
    }

    pub(super) fn expire_old_entries(&self) {
        if self.prune(now()).is_err() {
            eprintln!("Could not expire old entries from the private Connector log");
        }
    }

    fn prune(&self, time: u64) -> io::Result<()> {
        let cutoff = time.saturating_sub(RETENTION_SECONDS);
        let file = self.open_locked()?;
        let mut reader = BufReader::new(&file);
        let mut line = String::new();
        let (mut read_offset, mut keep_offset) = (0, 0);
        while reader.read_line(&mut line)? != 0 {
            let timestamp = line
                .strip_prefix("time_unix=")
                .and_then(|rest| rest.split_once(' '))
                .and_then(|(value, _)| value.parse::<u64>().ok());
            if line.ends_with('\n') && timestamp.is_some_and(|value| value >= cutoff) {
                if keep_offset < read_offset {
                    // Writes move retained entries toward the start, behind the read cursor.
                    file.write_all_at(line.as_bytes(), keep_offset)?;
                }
                keep_offset += line.len() as u64;
            }
            read_offset += line.len() as u64;
            line.clear();
        }
        if keep_offset < read_offset {
            file.set_len(keep_offset)?;
        }
        Ok(())
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub(super) fn codex_diagnostic(stderr: &[u8]) -> (&'static str, Option<&'static str>) {
    let text = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    if text.contains("unexpected argument") || text.contains("unrecognized option") {
        // Only log flags owned by the Connector, never a token from stderr.
        let flag = [
            "--ignore-user-config",
            "--sandbox",
            "--ephemeral",
            "--skip-git-repo-check",
            "--output-schema",
            "--output-last-message",
            "--ask-for-approval",
        ]
        .into_iter()
        .find(|flag| text.contains(&format!("'{flag}'")));
        return ("unexpected_argument", flag);
    }
    let code = if text.contains("not logged in")
        || text.contains("authentication")
        || text.contains("401 unauthorized")
        || text.contains("api key")
    {
        "authentication_error"
    } else if text.contains("rate limit")
        || text.contains("quota")
        || text.contains("429 too many requests")
    {
        "rate_or_quota_limit"
    } else if text.contains("permission denied")
        || text.contains("operation not permitted")
        || text.contains("sandbox denied")
    {
        "permission_denied"
    } else if text.contains("failed to load config")
        || text.contains("invalid config")
        || text.contains("unknown permission profile")
    {
        "configuration_error"
    } else if stderr.is_empty() {
        "empty_stderr"
    } else {
        "unclassified_stderr"
    };
    (code, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::symlink};

    #[test]
    fn logs_retain_the_last_day_and_refuse_symlinks() {
        let root = tempfile::tempdir().unwrap();
        let log = Log::for_config(&root.path().join("connector.json"));
        log.append("time_unix=99 expired").unwrap();
        log.append("time_unix=100 at_boundary").unwrap();
        let id = Uuid::new_v4();
        let prompt = "模拟截图任务\n\"设置\"".repeat(20_000);
        log.prompt(id, &prompt);
        log.prune(RETENTION_SECONDS + 100).unwrap();
        assert_eq!(fs::metadata(&log.path).unwrap().mode() & 0o777, 0o600);
        let text = fs::read_to_string(&log.path).unwrap();
        assert!(!text.contains("expired"));
        assert!(text.starts_with("time_unix=100 at_boundary\n"));
        let (prefix, value) = text.lines().nth(1).unwrap().split_once(" prompt=").unwrap();
        assert!(prefix.ends_with(&format!("job={id} job_claimed")));
        assert_eq!(serde_json::from_str::<String>(value).unwrap(), prompt);
        fs::remove_file(&log.path).unwrap();
        let target = root.path().join("do-not-touch");
        fs::write(&target, "fixture").unwrap();
        symlink(&target, &log.path).unwrap();
        log.event(None, "connector_started");
        assert_eq!(fs::read_to_string(target).unwrap(), "fixture");
    }

    #[test]
    fn categorizes_cli_failures_without_copying_stderr() {
        assert_eq!(
            codex_diagnostic(
                b"error: unexpected argument '--ignore-user-config' found\nBearer private-fixture"
            ),
            ("unexpected_argument", Some("--ignore-user-config"))
        );
        assert_eq!(
            codex_diagnostic(b"error: unexpected argument 'private-fixture' found"),
            ("unexpected_argument", None)
        );
        assert_eq!(
            codex_diagnostic(b"some private task data"),
            ("unclassified_stderr", None)
        );
    }
}
