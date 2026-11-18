//! Per-claim, verified skill installation. The cache is data, never a Codex configuration.
use super::diagnostic::Log;
use reqwest::StatusCode;
use std::{
    ffi::CString,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
};
use store_protocol::skill::{
    valid_name, valid_version, SkillManifest, MAX_BUNDLE_BYTES, MAX_FILES,
};
use uuid::Uuid;

pub(super) struct InstalledSkill {
    pub(super) name: String,
    pub(super) path: PathBuf,
    pub(super) version: String,
    pub(super) sha256: String,
}

fn invalid() -> io::Error {
    io::Error::other("Unsafe screenshot skill cache")
}

fn owned_directory(file: &File) -> io::Result<()> {
    let meta = file.metadata()?;
    // geteuid has no arguments or caller-owned memory.
    if !meta.is_dir() || meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o022 != 0 {
        return Err(invalid());
    }
    Ok(())
}

/// All cache operations stay relative to verified directory descriptors.
struct Cache(File);

impl Cache {
    fn open(data: &Path) -> io::Result<Self> {
        let parent = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(data)?;
        owned_directory(&parent)?;
        let name = c"skill-cache";
        // Constant relative name and an owned descriptor; never follows a link.
        let made = unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) };
        if made != 0 && io::Error::last_os_error().kind() != io::ErrorKind::AlreadyExists {
            return Err(io::Error::last_os_error());
        }
        let raw = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if raw < 0 {
            return Err(io::Error::last_os_error());
        }
        // openat returned a new descriptor owned by this File.
        let file = unsafe { File::from_raw_fd(raw) };
        owned_directory(&file)?;
        Ok(Self(file))
    }

    fn read(&self) -> io::Result<Option<Vec<u8>>> {
        let raw = unsafe {
            libc::openat(
                self.0.as_raw_fd(),
                c"current.json".as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            )
        };
        if raw < 0 {
            let error = io::Error::last_os_error();
            return if error.kind() == io::ErrorKind::NotFound {
                Ok(None)
            } else {
                Err(error)
            };
        }
        // openat returned a new descriptor owned by this File.
        let file = unsafe { File::from_raw_fd(raw) };
        let meta = file.metadata()?;
        if !meta.is_file()
            || meta.nlink() != 1
            || meta.uid() != unsafe { libc::geteuid() }
            || meta.mode() & 0o022 != 0
            || meta.len() > MAX_BUNDLE_BYTES as u64
        {
            return Err(invalid());
        }
        let mut bytes = Vec::new();
        file.take(MAX_BUNDLE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_BUNDLE_BYTES {
            return Err(invalid());
        }
        Ok(Some(bytes))
    }

    fn replace(&self, bytes: &[u8]) -> io::Result<()> {
        let name = CString::new(format!(".download-{}", Uuid::new_v4())).expect("UUID has no NUL");
        let raw = unsafe {
            libc::openat(
                self.0.as_raw_fd(),
                name.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        };
        if raw < 0 {
            return Err(io::Error::last_os_error());
        }
        // openat returned a new descriptor owned by this File.
        let mut file = unsafe { File::from_raw_fd(raw) };
        let result = (|| {
            file.write_all(bytes)?;
            file.sync_all()?;
            if unsafe {
                libc::renameat(
                    self.0.as_raw_fd(),
                    name.as_ptr(),
                    self.0.as_raw_fd(),
                    c"current.json".as_ptr(),
                )
            } != 0
            {
                return Err(io::Error::last_os_error());
            }
            self.0.sync_all()
        })();
        // Remove only the exact exclusive temporary entry created above; rename may have consumed it.
        unsafe {
            libc::unlinkat(self.0.as_raw_fd(), name.as_ptr(), 0);
        }
        result
    }
}

fn valid_manifest(manifest: &SkillManifest) -> bool {
    manifest.schema_version == 1
        && valid_name(&manifest.name)
        && valid_version(&manifest.version)
        && manifest.sha256 == manifest.version
        && manifest.bytes > 0
        && manifest.bytes <= MAX_BUNDLE_BYTES
        && manifest.files > 0
        && manifest.files <= MAX_FILES
}

/// The caller supplies the fixed authenticated center route, never a bundle-provided URL.
pub(super) async fn prepare(
    manifest: &SkillManifest,
    data: &Path,
    root: &Path,
    request: reqwest::RequestBuilder,
    log: &Log,
    job: Uuid,
) -> Result<InstalledSkill, &'static str> {
    if !valid_manifest(manifest) {
        return Err("screenshot_skill_invalid");
    }
    let event = |stage: &str| {
        log.event(
            Some(job),
            format_args!(
                "{stage} name={} version={} sha256={}",
                manifest.name, manifest.version, manifest.sha256
            ),
        )
    };
    event("skill_check");
    let cache = Cache::open(data).map_err(|_| "screenshot_skill_cache_unsafe")?;
    let bytes = match cache.read().map_err(|_| "screenshot_skill_cache_unsafe")? {
        Some(bytes) if manifest.verify(&bytes).is_ok() => {
            event("skill_cache_hit");
            bytes
        }
        _ => {
            event("skill_download");
            let mut response = request
                .send()
                .await
                .map_err(|_| "screenshot_skill_download_failed")?;
            if response.status() != StatusCode::OK
                || response
                    .content_length()
                    .is_some_and(|size| size > manifest.bytes as u64)
            {
                return Err("screenshot_skill_download_failed");
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| "screenshot_skill_download_failed")?
            {
                if chunk.len() > manifest.bytes.saturating_sub(bytes.len()) {
                    return Err("screenshot_skill_invalid");
                }
                bytes.extend_from_slice(&chunk);
            }
            manifest
                .verify(&bytes)
                .map_err(|_| "screenshot_skill_invalid")?;
            cache
                .replace(&bytes)
                .map_err(|_| "screenshot_skill_cache_unsafe")?;
            event("skill_cache_replaced");
            bytes
        }
    };
    let files = manifest
        .verify(&bytes)
        .map_err(|_| "screenshot_skill_invalid")?;
    // This is a fresh task directory; reject existing entries rather than following them.
    let agents = root.join(".agents");
    let skills = agents.join("skills");
    fs::create_dir(&agents).map_err(|_| "screenshot_skill_install_failed")?;
    fs::create_dir(&skills).map_err(|_| "screenshot_skill_install_failed")?;
    let stage = tempfile::Builder::new()
        .prefix(".install-")
        .tempdir_in(&skills)
        .map_err(|_| "screenshot_skill_install_failed")?;
    for file in files {
        let path = stage.path().join(file.path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|_| "screenshot_skill_install_failed")?;
        }
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)
            .map_err(|_| "screenshot_skill_install_failed")?;
        output
            .write_all(&file.bytes)
            .map_err(|_| "screenshot_skill_install_failed")?;
        output
            .set_permissions(fs::Permissions::from_mode(if file.executable {
                0o500
            } else {
                0o400
            }))
            .map_err(|_| "screenshot_skill_install_failed")?;
    }
    let destination = skills.join(&manifest.name);
    fs::rename(stage.path(), &destination).map_err(|_| "screenshot_skill_install_failed")?;
    let path = destination
        .join("SKILL.md")
        .canonicalize()
        .map_err(|_| "screenshot_skill_install_failed")?;
    event("skill_ready");
    Ok(InstalledSkill {
        name: manifest.name.clone(),
        path,
        version: manifest.version.clone(),
        sha256: manifest.sha256.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use store_protocol::skill::{SkillBundle, SkillFile};

    fn bundle(text: &str) -> (SkillManifest, Vec<u8>) {
        let document = format!("---\nname: capture\ndescription: Capture screens\n---\n{text}\n");
        let bytes = SkillBundle {
            schema_version: 1,
            name: "capture".into(),
            files: vec![
                SkillFile::new("SKILL.md".into(), document.as_bytes(), false),
                SkillFile::new("scripts/check.sh".into(), b"#!/bin/sh\nexit 0\n", true),
                SkillFile::new("references/check.md".into(), b"Check this image", false),
            ],
        }
        .canonical_bytes()
        .unwrap();
        (SkillManifest::from_bundle(&bytes, 1).unwrap(), bytes)
    }

    #[tokio::test]
    async fn verifies_cache_each_claim_refetches_tampering_and_pins_each_task_version() {
        use axum::{routing::get, Router};
        let data = tempfile::tempdir().unwrap();
        let task1 = tempfile::tempdir().unwrap();
        let task2 = tempfile::tempdir().unwrap();
        let task3 = tempfile::tempdir().unwrap();
        let (manifest, bytes) = bundle("first immutable version");
        let count = Arc::new(AtomicUsize::new(0));
        let calls = count.clone();
        let payload = bytes.clone();
        let app = Router::new().route(
            "/bundle",
            get(move |headers: axum::http::HeaderMap| {
                assert_eq!(headers["authorization"], "Bearer fixture-node");
                calls.fetch_add(1, Ordering::SeqCst);
                let payload = payload.clone();
                async move { payload }
            }),
        );
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let url = format!("http://{}/bundle", listener.local_addr().unwrap());
        let (shutdown, stopped) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = stopped.await;
                })
                .await
                .unwrap();
        });
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let request = || client.get(&url).bearer_auth("fixture-node");
        let log = Log::for_config(&data.path().join("connector.json"));
        let first = prepare(
            &manifest,
            data.path(),
            task1.path(),
            request(),
            &log,
            Uuid::new_v4(),
        )
        .await
        .unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert!(first.path.is_absolute());
        assert_eq!(fs::metadata(&first.path).unwrap().mode() & 0o777, 0o400);
        assert_eq!(
            fs::metadata(first.path.parent().unwrap().join("scripts/check.sh"))
                .unwrap()
                .mode()
                & 0o777,
            0o500
        );
        let original = fs::read(&first.path).unwrap();
        let second = prepare(
            &manifest,
            data.path(),
            task2.path(),
            request(),
            &log,
            Uuid::new_v4(),
        )
        .await
        .unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert_ne!(first.path, second.path);
        let cache = Cache::open(data.path()).unwrap();
        cache.replace(b"tampered").unwrap();
        prepare(
            &manifest,
            data.path(),
            task3.path(),
            request(),
            &log,
            Uuid::new_v4(),
        )
        .await
        .unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 2);
        assert_eq!(cache.read().unwrap().unwrap(), bytes);

        // A known new version must not run using the older cached version when fetch fails.
        let (new_manifest, _) = bundle("new version");
        let task4 = tempfile::tempdir().unwrap();
        let error = prepare(
            &new_manifest,
            data.path(),
            task4.path(),
            client.get(format!("{url}/missing")),
            &log,
            Uuid::new_v4(),
        )
        .await
        .err()
        .unwrap();
        assert_eq!(error, "screenshot_skill_download_failed");
        assert!(!task4.path().join(".agents").exists());
        assert_eq!(fs::read(&first.path).unwrap(), original);
        assert_eq!(cache.read().unwrap().unwrap(), bytes);
        shutdown.send(()).unwrap();
        server.await.unwrap();
    }

    #[test]
    fn rejects_symlinks_writable_cache_and_hardlinks_without_touching_targets() {
        use std::os::unix::fs::symlink;
        let data = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        symlink(outside.path(), data.path().join("skill-cache")).unwrap();
        assert!(Cache::open(data.path()).is_err());
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
        fs::remove_file(data.path().join("skill-cache")).unwrap();
        let cache = Cache::open(data.path()).unwrap();
        cache.replace(b"original").unwrap();
        fs::hard_link(
            data.path().join("skill-cache/current.json"),
            outside.path().join("linked"),
        )
        .unwrap();
        assert!(cache.read().is_err());
        fs::remove_file(outside.path().join("linked")).unwrap();
        fs::set_permissions(
            data.path().join("skill-cache"),
            fs::Permissions::from_mode(0o777),
        )
        .unwrap();
        assert!(Cache::open(data.path()).is_err());
        fs::set_permissions(
            data.path().join("skill-cache"),
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        fs::remove_file(data.path().join("skill-cache/current.json")).unwrap();
        symlink(
            outside.path().join("missing"),
            data.path().join("skill-cache/current.json"),
        )
        .unwrap();
        assert!(cache.read().is_err());
    }

    #[tokio::test]
    async fn rejects_bad_sha_oversize_and_redirect_downloads_without_replacing_cache() {
        use axum::{response::Redirect, routing::get, Router};
        let data = tempfile::tempdir().unwrap();
        let (old_manifest, old_bytes) = bundle("old");
        let (manifest, bytes) = bundle("new");
        assert_ne!(manifest.version, old_manifest.version);
        let cache = Cache::open(data.path()).unwrap();
        cache.replace(&old_bytes).unwrap();
        let mut tampered = bytes.clone();
        tampered[0] ^= 1;
        let too_big = vec![b' '; manifest.bytes + 1];
        let app = Router::new()
            .route(
                "/tampered",
                get(move || {
                    let bytes = tampered.clone();
                    async move { bytes }
                }),
            )
            .route(
                "/oversize",
                get(move || {
                    let bytes = too_big.clone();
                    async move { bytes }
                }),
            )
            .route(
                "/redirect",
                get(|| async { Redirect::temporary("http://127.0.0.1:1/must-not-fetch") }),
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
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let log = Log::for_config(&data.path().join("connector.json"));
        for (path, expected) in [
            ("tampered", "screenshot_skill_invalid"),
            ("oversize", "screenshot_skill_download_failed"),
            ("redirect", "screenshot_skill_download_failed"),
        ] {
            let task = tempfile::tempdir().unwrap();
            let error = prepare(
                &manifest,
                data.path(),
                task.path(),
                client.get(format!("http://{address}/{path}")),
                &log,
                Uuid::new_v4(),
            )
            .await
            .err()
            .unwrap();
            assert_eq!(error, expected);
            assert!(!task.path().join(".agents").exists());
            assert_eq!(cache.read().unwrap().unwrap(), old_bytes);
        }
        shutdown.send(()).unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn rejects_invalid_manifest_and_unsafe_task_paths_before_installation() {
        let data = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let (mut manifest, bytes) = bundle("safe");
        let client = reqwest::Client::new();
        let log = Log::for_config(&data.path().join("connector.json"));
        manifest.name = "../outside".into();
        assert_eq!(
            prepare(
                &manifest,
                data.path(),
                root.path(),
                client.get("http://127.0.0.1:1"),
                &log,
                Uuid::new_v4()
            )
            .await
            .err(),
            Some("screenshot_skill_invalid")
        );
        let (manifest, _) = bundle("safe");
        Cache::open(data.path()).unwrap().replace(&bytes).unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join(".agents")).unwrap();
        assert_eq!(
            prepare(
                &manifest,
                data.path(),
                root.path(),
                client.get("http://127.0.0.1:1"),
                &log,
                Uuid::new_v4()
            )
            .await
            .err(),
            Some("screenshot_skill_install_failed")
        );
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
    }
}
