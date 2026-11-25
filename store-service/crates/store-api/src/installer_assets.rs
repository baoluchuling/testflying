//! Installer data follows its node release, so a script update does not rebuild the API.
use crate::{registry::ConnectorKind, StoreError};
use std::{io::Read, path::Path};

pub(crate) fn read(root: &Path, kind: ConnectorKind, name: &str) -> Result<Vec<u8>, StoreError> {
    let (directory, legacy) = match kind {
        ConnectorKind::Store => ("store", "connector"),
        ConnectorKind::Screenshot => ("screenshot", "screenshot-connector"),
    };
    let allowed = match kind {
        ConnectorKind::Store => matches!(
            name,
            "start.cmd" | "start.sh" | "install.cmd" | "install.ps1" | "install.sh" | "README.md"
        ),
        ConnectorKind::Screenshot => matches!(name, "install.sh" | "run.sh" | "README.md"),
    };
    let unavailable = || {
        StoreError::new(
            503,
            "release_unavailable",
            "Installer data for this node has not been published",
        )
    };
    if !allowed {
        return Err(unavailable());
    }
    let path = root.join("installer-assets").join(directory).join(name);
    let file = std::fs::File::open(path);
    // Development/test builds read the source fixtures; release programs require
    // the per-node assets verified and staged by the publisher.
    #[cfg(debug_assertions)]
    let file = file.or_else(|error| {
        if error.kind() != std::io::ErrorKind::NotFound {
            return Err(error);
        }
        std::fs::File::open(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../packaging")
                .join(legacy)
                .join(name),
        )
    });
    #[cfg(not(debug_assertions))]
    let _ = legacy;
    let mut bytes = Vec::new();
    file.map_err(|_| unavailable())?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| unavailable())?;
    if bytes.len() > 1024 * 1024 {
        return Err(unavailable());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn node_assets_reload_without_recompiling_and_reject_other_paths() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("installer-assets/screenshot");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("run.sh"), "first").unwrap();
        assert_eq!(
            read(root.path(), ConnectorKind::Screenshot, "run.sh").unwrap(),
            b"first"
        );
        std::fs::write(directory.join("run.sh"), "second").unwrap();
        assert_eq!(
            read(root.path(), ConnectorKind::Screenshot, "run.sh").unwrap(),
            b"second"
        );
        assert!(read(root.path(), ConnectorKind::Screenshot, "../connector.json").is_err());
    }
}
