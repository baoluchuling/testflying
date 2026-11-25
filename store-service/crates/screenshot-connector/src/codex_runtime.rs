use std::{
    env,
    ffi::{OsStr, OsString},
    fs::{self, OpenOptions},
    io::Read,
    os::unix::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
};

const SYSTEM_INSTALL_DIRS: &[&str] = &[
    "/opt/homebrew/bin",
    "/usr/local/bin",
    "/Applications/Codex.app/Contents/Resources",
    "/Applications/ChatGPT.app/Contents/Resources",
];
const SYSTEM_PATH: [&str; 4] = ["/usr/bin", "/bin", "/usr/sbin", "/sbin"];
const MAX_NVM_ENTRIES: usize = 128;

pub(super) struct Runtime {
    pub(super) executable: PathBuf,
    pub(super) path: OsString,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Unavailable {
    ConfiguredPath,
    Executable,
    Node,
}

impl Unavailable {
    pub(super) fn diagnostic(&self) -> (&'static str, &'static str) {
        match self {
            Self::ConfiguredPath => (
                "codex_path_invalid",
                "connector.json 的 codexPath 必须指向可执行的绝对路径",
            ),
            Self::Executable => (
                "codex_not_found",
                "截图服务账号的 PATH 和常见安装位置中未找到可执行的 Codex。请为该账号安装 Codex 或设置 codexPath",
            ),
            Self::Node => (
                "codex_node_unavailable",
                "npm 安装的 Codex 需要 Node.js。截图服务账号的 PATH 和常见安装位置中未找到可执行的 node",
            ),
        }
    }
}

pub(super) fn for_service(configured: Option<&Path>) -> Result<Runtime, Unavailable> {
    let home = env::var_os("HOME").map(PathBuf::from);
    let path = env::var_os("PATH");
    resolve(
        configured,
        home.as_deref(),
        path.as_deref(),
        SYSTEM_INSTALL_DIRS,
    )
}

fn resolve(
    configured: Option<&Path>,
    home: Option<&Path>,
    inherited_path: Option<&OsStr>,
    system_install_dirs: &[&str],
) -> Result<Runtime, Unavailable> {
    let mut path = inherited_path
        .into_iter()
        .flat_map(env::split_paths)
        .filter(|dir| dir.is_absolute())
        .collect::<Vec<_>>();
    if path.is_empty() {
        path.extend(SYSTEM_PATH.map(PathBuf::from));
    }

    let home = home.filter(|home| home.is_absolute());
    let user_bins = home.into_iter().flat_map(|home| {
        [
            ".local/bin",
            "bin",
            ".npm-global/bin",
            ".npm/bin",
            "Library/pnpm",
        ]
        .map(|dir| home.join(dir))
    });
    // Only enumerate NVM when earlier candidates fail to provide the executable or Node.
    let nvm = std::cell::OnceCell::new();
    let install_dirs = || {
        user_bins
            .clone()
            .chain(
                std::iter::once_with(|| nvm.get_or_init(|| home.map(nvm_bins).unwrap_or_default()))
                    .flat_map(|bins| bins.iter().cloned()),
            )
            .chain(home.into_iter().flat_map(|home| {
                [
                    "Applications/Codex.app/Contents/Resources",
                    "Applications/ChatGPT.app/Contents/Resources",
                ]
                .map(|dir| home.join(dir))
            }))
            .chain(system_install_dirs.iter().copied().map(PathBuf::from))
    };
    let (entrypoint, executable) = match configured {
        Some(path) => (
            path.to_path_buf(),
            executable_path(path).ok_or(Unavailable::ConfiguredPath)?,
        ),
        None => path
            .iter()
            .cloned()
            .chain(install_dirs())
            .find_map(|dir| {
                let entrypoint = dir.join("codex");
                let executable = executable_path(&entrypoint)?;
                Some((entrypoint, executable))
            })
            .ok_or(Unavailable::Executable)?,
    };

    if needs_node(&executable) {
        // npm links bin/codex to lib/node_modules/.../bin/codex.js. Prefer the
        // Node paired with that install even when the inherited PATH has another version.
        let npm_bin = executable
            .ancestors()
            .nth(6)
            .filter(|_| executable.ends_with("lib/node_modules/@openai/codex/bin/codex.js"))
            .map(|prefix| prefix.join("bin"));
        let node_bin = entrypoint
            .parent()
            .map(Path::to_path_buf)
            .into_iter()
            .chain(npm_bin)
            .chain(path.iter().cloned())
            .chain(install_dirs())
            .find(|dir| {
                !dir.as_os_str().as_bytes().contains(&b':')
                    && executable_path(&dir.join("node")).is_some()
            })
            .ok_or(Unavailable::Node)?;
        path.retain(|dir| dir != &node_bin);
        path.insert(0, node_bin);
    }
    // The inherited directories were split on PATH separators, and node_bin passed the same check.
    let path = env::join_paths(path).expect("PATH entries cannot contain separators");
    Ok(Runtime { executable, path })
}

fn executable_path(path: &Path) -> Option<PathBuf> {
    if !path.is_absolute() {
        return None;
    }
    let path = path.canonicalize().ok()?;
    let metadata = path.metadata().ok()?;
    (metadata.is_file() && metadata.mode() & 0o111 != 0).then_some(path)
}

fn needs_node(path: &Path) -> bool {
    let mut prefix = Vec::new();
    if OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .and_then(|file| file.take(256).read_to_end(&mut prefix))
        .is_err()
    {
        return false;
    }
    let line = prefix
        .split(|byte| *byte == b'\n')
        .next()
        .unwrap_or_default();
    let Some(shebang) = line.strip_prefix(b"#!") else {
        return false;
    };
    let mut words = shebang
        .split(u8::is_ascii_whitespace)
        .filter(|word| !word.is_empty());
    if !matches!(words.next(), Some(b"/usr/bin/env")) {
        return false;
    }
    match words.next() {
        Some(b"node") => true,
        Some(b"-S") => matches!(words.next(), Some(b"node")),
        _ => false,
    }
}

fn nvm_bins(home: &Path) -> Vec<PathBuf> {
    let Ok(home) = home.canonicalize() else {
        return Vec::new();
    };
    let mut versions = Vec::new();
    for location in [".nvm/versions/node", ".config/nvm/versions/node"] {
        let root = home.join(location);
        if root.canonicalize().ok().as_ref() != Some(&root) {
            continue;
        }
        if let Ok(entries) = fs::read_dir(root) {
            for entry in entries.take(MAX_NVM_ENTRIES).flatten() {
                if let Some(version) = node_version(&entry.file_name())
                    .filter(|_| entry.file_type().is_ok_and(|kind| kind.is_dir()))
                {
                    versions.push((version, entry.path().join("bin")));
                }
            }
        }
    }
    versions.sort_unstable();
    versions.into_iter().rev().map(|(_, dir)| dir).collect()
}

fn node_version(name: &OsStr) -> Option<(u32, u32, u32)> {
    let mut parts = name.to_str()?.strip_prefix('v')?.split('.');
    let version = (
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    );
    parts.next().is_none().then_some(version)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        os::unix::fs::{symlink, PermissionsExt},
        process::Command,
    };

    fn script(path: &Path, body: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[test]
    fn honors_configured_codex_and_rejects_invalid_overrides() {
        let fixture = tempfile::tempdir().unwrap();
        let home = fixture.path();
        let explicit = home.join("custom/codex");
        let fallback = home.join(".local/bin/codex");
        let non_executable = home.join("non-executable");
        script(&explicit, "#!/bin/sh\nexit 0\n");
        script(&fallback, "#!/bin/sh\nexit 0\n");
        fs::write(&non_executable, "").unwrap();
        let inherited = fallback.parent().map(Path::as_os_str);
        let codex = resolve(Some(&explicit), Some(home), inherited, &[]).unwrap();
        assert_eq!(codex.executable, explicit.canonicalize().unwrap());
        for invalid in [
            Path::new(""),
            Path::new("relative/codex"),
            home,
            &non_executable,
            &home.join("missing"),
        ] {
            assert!(matches!(
                resolve(Some(invalid), Some(home), inherited, &[]),
                Err(Unavailable::ConfiguredPath)
            ));
        }
    }

    #[test]
    fn inherited_absolute_path_precedes_known_installations() {
        let fixture = tempfile::tempdir().unwrap();
        let home = fixture.path();
        let on_path = home.join("custom bin");
        let codex = on_path.join("codex");
        script(&codex, "#!/bin/sh\n# #!/usr/bin/env node\nexit 0\n");
        script(&home.join(".local/bin/codex"), "#!/bin/sh\nexit 0\n");
        let inherited = env::join_paths([Path::new(""), Path::new("relative"), &on_path]).unwrap();
        let runtime = resolve(None, Some(home), Some(&inherited), &[]).unwrap();
        assert_eq!(runtime.executable, codex.canonicalize().unwrap());
        assert_eq!(runtime.path, on_path.as_os_str());
    }

    #[test]
    fn launchd_path_falls_back_to_user_standalone_and_application_installs() {
        let fixture = tempfile::tempdir().unwrap();
        let home = fixture.path();
        let path = Some(OsStr::new("/usr/bin:/bin:/usr/sbin:/sbin"));
        let standalone = home.join(".local/bin/codex");
        let app = home.join("Applications/ChatGPT.app/Contents/Resources/codex");
        script(&standalone, "#!/bin/sh\nexit 0\n");
        script(&app, "#!/bin/sh\nexit 0\n");
        assert_eq!(
            resolve(None, Some(home), path, &[]).unwrap().executable,
            standalone.canonicalize().unwrap()
        );
        fs::remove_file(standalone).unwrap();
        assert_eq!(
            resolve(None, Some(home), path, &[]).unwrap().executable,
            app.canonicalize().unwrap()
        );
    }

    #[test]
    fn npm_symlink_with_minimal_path_uses_the_paired_nvm_node() {
        let fixture = tempfile::tempdir().unwrap();
        let home = fixture.path();
        let inherited = home.join("launchd");
        script(&inherited.join("node"), "#!/bin/sh\nexit 1\n");
        let nvm = home.join(".nvm/versions/node");
        script(&nvm.join("v9.10.0/bin/codex"), "#!/bin/sh\nexit 1\n");
        let prefix = nvm.join("v24.2.0");
        let bin = prefix.join("bin");
        let shim = bin.join("codex");
        let entrypoint = prefix.join("lib/node_modules/@openai/codex/bin/codex.js");
        script(&entrypoint, "#!/usr/bin/env node\n/* fixture */\n");
        script(&bin.join("node"), "#!/bin/sh\nprintf paired-node\n");
        symlink("../lib/node_modules/@openai/codex/bin/codex.js", &shim).unwrap();
        symlink(nvm.join("v9.10.0"), nvm.join("v99.0.0")).unwrap();

        for configured in [None, Some(shim.as_path()), Some(entrypoint.as_path())] {
            let runtime =
                resolve(configured, Some(home), Some(inherited.as_os_str()), &[]).unwrap();
            assert_eq!(runtime.executable, entrypoint.canonicalize().unwrap());
            let first_path = env::split_paths(&runtime.path).next().unwrap();
            assert_eq!(
                first_path.canonicalize().unwrap(),
                bin.canonicalize().unwrap()
            );
            let result = Command::new(runtime.executable)
                .env_clear()
                .env("HOME", home)
                .env("PATH", runtime.path)
                .output()
                .unwrap();
            assert!(result.status.success());
            assert_eq!(result.stdout, b"paired-node");
        }
    }

    #[test]
    fn distinguishes_missing_codex_from_missing_node() {
        let fixture = tempfile::tempdir().unwrap();
        let home = fixture.path();
        let inherited = home.join("launchd");
        let path = Some(inherited.as_os_str());
        assert!(matches!(
            resolve(None, Some(home), path, &[]),
            Err(Unavailable::Executable)
        ));
        script(&inherited.join("codex"), "#!/usr/bin/env node\n");
        assert!(matches!(
            resolve(None, Some(home), path, &[]),
            Err(Unavailable::Node)
        ));
    }
}
