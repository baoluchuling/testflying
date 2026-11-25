//! Independent program versions; node publication metadata can change without an API rebuild.
use crate::registry::ConnectorKind;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{io::Read, path::Path};
use store_protocol::release::ConnectorVersion;

pub const CENTER_VERSION: &str = env!("CARGO_PKG_VERSION");
const LEGACY_NODE_VERSION: &str = "0.2.8";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    store: String,
    screenshot: String,
}

pub(crate) fn versions(root: &Path) -> Value {
    let path = root.join("connector-versions.json");
    let data = std::fs::File::open(&path).and_then(|file| {
        let mut data = Vec::new();
        file.take(1025).read_to_end(&mut data)?;
        Ok(data)
    });
    let catalog = match data {
        Ok(data) if data.len() <= 1024 => {
            serde_json::from_slice::<Catalog>(&data).ok().filter(|c| {
                [&c.store, &c.screenshot]
                    .iter()
                    .all(|v| ConnectorVersion::parse(v).is_some())
            })
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(Catalog {
            store: LEGACY_NODE_VERSION.into(),
            screenshot: LEGACY_NODE_VERSION.into(),
        }),
        _ => None,
    };
    match catalog {
        Some(c) => json!({"center":CENTER_VERSION,"store":c.store,"screenshot":c.screenshot}),
        None => json!({"center":CENTER_VERSION,"store":null,"screenshot":null}),
    }
}

pub(crate) fn node_version(root: &Path, kind: ConnectorKind) -> Option<String> {
    versions(root)[match kind {
        ConnectorKind::Store => "store",
        ConnectorKind::Screenshot => "screenshot",
    }]
    .as_str()
    .map(str::to_owned)
}

pub(crate) fn status(reported: Option<&str>, latest: Option<&str>) -> &'static str {
    let Some((reported, latest)) = reported
        .and_then(ConnectorVersion::parse)
        .zip(latest.and_then(ConnectorVersion::parse))
    else {
        return "unknown";
    };
    match reported.cmp(&latest) {
        std::cmp::Ordering::Less => "outdated",
        std::cmp::Ordering::Equal => "current",
        std::cmp::Ordering::Greater => "newer",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn runtime_catalog_can_upgrade_one_kind_without_changing_the_api_or_other_node() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(
            node_version(root.path(), ConnectorKind::Store).as_deref(),
            Some("0.2.8")
        );
        std::fs::write(
            root.path().join("connector-versions.json"),
            r#"{"store":"0.2.8","screenshot":"0.9.0"}"#,
        )
        .unwrap();
        assert_eq!(versions(root.path())["center"], CENTER_VERSION);
        assert_eq!(
            node_version(root.path(), ConnectorKind::Store).as_deref(),
            Some("0.2.8")
        );
        assert_eq!(
            node_version(root.path(), ConnectorKind::Screenshot).as_deref(),
            Some("0.9.0")
        );
        assert_eq!(status(Some("0.2.8"), Some("0.9.0")), "outdated");
        std::fs::write(
            root.path().join("connector-versions.json"),
            r#"{"store":"0.2.8","screenshot":"0.10.0"}"#,
        )
        .unwrap();
        assert_eq!(
            node_version(root.path(), ConnectorKind::Screenshot).as_deref(),
            Some("0.10.0")
        );
        assert_eq!(versions(root.path())["center"], CENTER_VERSION);
    }
    #[test]
    fn malformed_or_unreadable_publication_is_unknown_and_never_claimed_current() {
        let root = tempfile::tempdir().unwrap();
        for bytes in [
            r#"{"store":"01.2.8","screenshot":"0.9.0"}"#,
            r#"{"store":"0.2.8"}"#,
            "bad",
        ] {
            std::fs::write(root.path().join("connector-versions.json"), bytes).unwrap();
            assert!(node_version(root.path(), ConnectorKind::Store).is_none());
            assert_eq!(status(Some("0.2.8"), None), "unknown");
        }
    }
}
