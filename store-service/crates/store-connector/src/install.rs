//! Prepare selected account changes without modifying the installed node.
use serde_json::{json, Value};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const CONFIG_LIMIT: u64 = 2 * 1024 * 1024;
const KEY_LIMIT: u64 = 256 * 1024;
const APPLE: &str = "credentials/apple-private-key.p8";
const GOOGLE: &str = "credentials/google-service-account.json";

fn read(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let metadata =
        fs::symlink_metadata(path).map_err(|_| "Required installation file is missing")?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > limit {
        return Err("Installation input is not a bounded regular file".into());
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    if bytes.is_empty() || bytes.len() as u64 > limit {
        return Err("Installation input is empty or too large".into());
    }
    Ok(bytes)
}

fn config(path: &Path) -> Result<Value> {
    serde_json::from_slice(&read(path, CONFIG_LIMIT)?)
        .map_err(|_| "Invalid installation configuration JSON".into())
}

fn string<'a>(object: &'a Value, field: &str) -> Result<&'a str> {
    object[field]
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Required installation configuration field is missing or invalid".into())
}

fn mode(config: &Value) -> Result<&str> {
    match config.get("platformMode") {
        None => Ok("all"),
        Some(Value::String(mode)) if matches!(mode.as_str(), "all" | "ios" | "android") => Ok(mode),
        _ => Err("Invalid installed platformMode".into()),
    }
}

fn merge(existing: Option<Value>, incoming: &Value) -> Result<(Value, bool, bool)> {
    let node = string(incoming, "connectorId")?;
    if node.len() > 128
        || !node
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err("Invalid installation node identity".into());
    }
    let provider = incoming
        .get("provider")
        .and_then(Value::as_object)
        .ok_or("The full package has no provider configuration")?;
    let apple = ["appleIssuerId", "appleKeyId", "applePrivateKeyPath"]
        .iter()
        .any(|key| provider.contains_key(*key));
    let google = provider.contains_key("googleServiceAccountPath");
    if !apple && !google {
        return Err("The full package has no selected platform credentials".into());
    }
    if apple {
        string(&incoming["provider"], "appleIssuerId")?;
        string(&incoming["provider"], "appleKeyId")?;
        if string(&incoming["provider"], "applePrivateKeyPath")? != APPLE {
            return Err("Apple package credentials must use the fixed relative path".into());
        }
    }
    if google && string(&incoming["provider"], "googleServiceAccountPath")? != GOOGLE {
        return Err("Google package credentials must use the fixed relative path".into());
    }
    let incoming_mode = mode(incoming)?;
    if incoming.get("platformMode").is_some()
        && ((apple && incoming_mode == "android") || (google && incoming_mode == "ios"))
    {
        return Err("Package platformMode conflicts with its provided credentials".into());
    }
    let previous_mode = if let Some(old) = &existing {
        if string(old, "connectorId")? != node {
            return Err("The package is for a different node".into());
        }
        Some(mode(old)?.to_owned())
    } else {
        None
    };
    let mut merged = existing.unwrap_or_else(|| incoming.clone());
    let object = merged
        .as_object_mut()
        .ok_or("Invalid installed configuration object")?;
    let provider = object
        .entry("provider")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("Invalid installed provider configuration")?;
    provider.insert("mode".into(), json!("live"));
    for (selected, keys) in [
        (
            apple,
            &["appleIssuerId", "appleKeyId", "applePrivateKeyPath"][..],
        ),
        (google, &["googleServiceAccountPath"][..]),
    ] {
        if selected {
            for key in keys {
                provider.insert((*key).into(), json!(string(&incoming["provider"], key)?));
            }
        }
    }
    let ios = apple
        || previous_mode
            .as_deref()
            .is_some_and(|mode| mode != "android");
    let android = google || previous_mode.as_deref().is_some_and(|mode| mode != "ios");
    object.insert(
        "platformMode".into(),
        json!(match (ios, android) {
            (true, true) => "all",
            (true, false) => "ios",
            (false, true) => "android",
            _ => unreachable!(),
        }),
    );
    Ok((merged, apple, google))
}

fn write(path: &Path, contents: &[u8]) -> Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(contents)?;
    file.sync_all()?;
    Ok(())
}

pub(super) fn prepare(package: &Path, installed: &Path, prepared: &Path) -> Result<()> {
    let package = package.canonicalize()?;
    let installed = if installed.try_exists()? {
        let metadata = fs::symlink_metadata(installed)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err("Invalid installed data directory".into());
        }
        installed.canonicalize()?
    } else {
        installed
            .parent()
            .ok_or("Invalid installed data directory")?
            .canonicalize()?
            .join(
                installed
                    .file_name()
                    .ok_or("Invalid installed data directory")?,
            )
    };
    let parent = prepared
        .parent()
        .ok_or("Invalid preparation directory")?
        .canonicalize()?;
    let output = parent.join(
        prepared
            .file_name()
            .ok_or("Invalid preparation directory")?,
    );
    if fs::symlink_metadata(&output).is_ok()
        || output.starts_with(&package)
        || output.starts_with(&installed)
    {
        return Err("Preparation needs a new directory outside package and installed data".into());
    }
    let incoming = config(&package.join("connector.json"))?;
    let old = if installed.try_exists()? {
        Some(config(&installed.join("connector.json"))?)
    } else {
        None
    };
    let (merged, apple, google) = merge(old, &incoming)?;
    let metadata = fs::symlink_metadata(package.join("credentials"))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("Invalid package credentials directory".into());
    }
    let mut keys = Vec::new();
    if apple {
        let bytes = read(&package.join(APPLE), KEY_LIMIT)?;
        let pem = std::str::from_utf8(&bytes)
            .map_err(|_| "Invalid Apple private key text")?
            .trim();
        if !pem.starts_with("-----BEGIN PRIVATE KEY-----")
            || !pem.ends_with("-----END PRIVATE KEY-----")
        {
            return Err("Invalid Apple private key envelope".into());
        }
        keys.push((APPLE, bytes));
    }
    if google {
        let bytes = read(&package.join(GOOGLE), KEY_LIMIT)?;
        let key: Value =
            serde_json::from_slice(&bytes).map_err(|_| "Invalid Google credential JSON")?;
        if key["type"] != "service_account" || !string(&key, "client_email")?.contains('@') {
            return Err("Invalid Google service account".into());
        }
        let pem = string(&key, "private_key")?;
        if !(pem.starts_with("-----BEGIN PRIVATE KEY-----")
            && pem.ends_with("-----END PRIVATE KEY-----")
            || pem.starts_with("-----BEGIN RSA PRIVATE KEY-----")
                && pem.ends_with("-----END RSA PRIVATE KEY-----"))
        {
            return Err("Invalid Google private key envelope".into());
        }
        keys.push((GOOGLE, bytes));
    }
    let mut contents = serde_json::to_vec_pretty(&merged)?;
    contents.push(b'\n');
    if contents.len() as u64 > CONFIG_LIMIT {
        return Err("Merged configuration exceeds the size limit".into());
    }
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(&output)?;
    builder.create(output.join("credentials"))?;
    for (relative, bytes) in keys {
        write(&output.join(relative), &bytes)?;
    }
    write(&output.join("connector.json"), &contents)?;
    println!(
        "Prepared platform configuration: {}.",
        merged["platformMode"].as_str().expect("merged mode")
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apple() -> Value {
        json!({"connectorId":"same-node","token":"incoming-token","provider":{
        "mode":"live","appleIssuerId":"new-issuer","appleKeyId":"new-key","applePrivateKeyPath":APPLE}})
    }
    fn android() -> Value {
        json!({"connectorId":"same-node","token":"original-token","messageKey":"keep-message-key",
        "listenAddr":"127.0.0.1:8100","platformMode":"android","provider":{"mode":"live","googleServiceAccountPath":GOOGLE,"storeProxyUrl":"http://proxy.test"}})
    }

    #[test]
    fn adding_ios_retains_android_identity_and_custom_settings() {
        let (merged, apple, google) = merge(Some(android()), &self::apple()).unwrap();
        assert!(apple && !google);
        assert_eq!(merged["platformMode"], "all");
        for key in ["connectorId", "token", "messageKey", "listenAddr"] {
            assert_eq!(merged[key], android()[key]);
        }
        assert_eq!(merged["provider"]["googleServiceAccountPath"], GOOGLE);
        assert_eq!(
            merged["provider"]["storeProxyUrl"],
            android()["provider"]["storeProxyUrl"]
        );
        assert_eq!(merged["provider"]["appleIssuerId"], "new-issuer");
    }

    #[test]
    fn adding_android_retains_ios_and_selected_account_replacement_is_scoped() {
        let mut existing = apple();
        existing["platformMode"] = json!("ios");
        let (merged, _, _) = merge(Some(existing.clone()), &android()).unwrap();
        assert_eq!(merged["platformMode"], "all");
        assert_eq!(
            merged["provider"]["appleKeyId"],
            existing["provider"]["appleKeyId"]
        );
        let mut rotation = apple();
        rotation["provider"]["appleKeyId"] = json!("rotated");
        let (rotated, _, _) = merge(Some(merged.clone()), &rotation).unwrap();
        assert_eq!(rotated["provider"]["appleKeyId"], "rotated");
        assert_eq!(
            rotated["provider"]["googleServiceAccountPath"],
            merged["provider"]["googleServiceAccountPath"]
        );
        assert_eq!(rotated["token"], existing["token"]);
    }

    #[test]
    fn first_install_selects_a_platform_and_existing_all_never_narrows() {
        assert_eq!(merge(None, &apple()).unwrap().0["platformMode"], "ios");
        assert_eq!(
            merge(None, &android()).unwrap().0["platformMode"],
            "android"
        );
        let mut old = android();
        old["platformMode"] = json!("all");
        assert_eq!(
            merge(Some(old), &android()).unwrap().0["platformMode"],
            "all"
        );
        let mut old = android();
        old.as_object_mut().unwrap().remove("platformMode");
        assert_eq!(merge(Some(old), &apple()).unwrap().0["platformMode"], "all");
    }

    #[test]
    fn mismatched_nodes_invalid_modes_and_partial_credentials_are_rejected() {
        let mut bad = apple();
        bad["connectorId"] = json!("other-node");
        assert!(merge(Some(android()), &bad).is_err());
        let mut bad = android();
        bad["platformMode"] = json!("invalid");
        assert!(merge(Some(bad), &apple()).is_err());
        let mut bad = apple();
        bad["provider"]
            .as_object_mut()
            .unwrap()
            .remove("appleKeyId");
        assert!(merge(Some(android()), &bad).is_err());
        let mut bad = apple();
        bad["provider"]["applePrivateKeyPath"] = json!("../outside.p8");
        assert!(merge(Some(android()), &bad).is_err());
        let mut bad = apple();
        bad["platformMode"] = json!("android");
        assert!(merge(Some(android()), &bad).is_err());
    }

    #[test]
    fn adding_android_validates_selected_file_and_keeps_existing_ios_bytes() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("package");
        fs::create_dir(&package).unwrap();
        fs::create_dir(package.join("credentials")).unwrap();
        let installed = root.path().join("installed");
        fs::create_dir(&installed).unwrap();
        fs::create_dir(installed.join("credentials")).unwrap();
        let mut old = apple();
        old["platformMode"] = json!("ios");
        let original = serde_json::to_vec(&old).unwrap();
        fs::write(installed.join("connector.json"), &original).unwrap();
        fs::write(installed.join(APPLE), b"unchanged existing private key").unwrap();
        fs::write(
            package.join("connector.json"),
            serde_json::to_vec(&android()).unwrap(),
        )
        .unwrap();
        let prepared = root.path().join("prepared");
        fs::write(package.join(GOOGLE), b"invalid json").unwrap();
        assert!(prepare(&package, &installed, &prepared).is_err());
        assert!(!prepared.exists());
        let key = json!({"type":"service_account","client_email":"fixture@example.invalid","private_key":"-----BEGIN PRIVATE KEY-----\nfixture-only\n-----END PRIVATE KEY-----"});
        let bytes = serde_json::to_vec(&key).unwrap();
        fs::write(package.join(GOOGLE), &bytes).unwrap();
        prepare(&package, &installed, &prepared).unwrap();
        assert_eq!(fs::read(prepared.join(GOOGLE)).unwrap(), bytes);
        assert!(!prepared.join(APPLE).exists());
        assert_eq!(
            config(&prepared.join("connector.json")).unwrap()["platformMode"],
            "all"
        );
        assert_eq!(
            fs::read(installed.join("connector.json")).unwrap(),
            original
        );
        assert_eq!(
            fs::read(installed.join(APPLE)).unwrap(),
            b"unchanged existing private key"
        );
    }

    #[cfg(unix)]
    #[test]
    fn selected_credentials_reject_symlinks_and_oversized_files_before_output() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("package");
        fs::create_dir(&package).unwrap();
        fs::create_dir(package.join("credentials")).unwrap();
        fs::write(
            package.join("connector.json"),
            serde_json::to_vec(&apple()).unwrap(),
        )
        .unwrap();
        let installed = root.path().join("not-installed");
        let prepared = root.path().join("prepared");
        let outside = root.path().join("outside");
        fs::write(&outside, b"outside data").unwrap();
        std::os::unix::fs::symlink(&outside, package.join(APPLE)).unwrap();
        assert!(prepare(&package, &installed, &prepared).is_err());
        assert!(!prepared.exists());
        fs::remove_file(package.join(APPLE)).unwrap();
        fs::File::create(package.join(APPLE))
            .unwrap()
            .set_len(KEY_LIMIT + 1)
            .unwrap();
        assert!(prepare(&package, &installed, &prepared).is_err());
        assert!(!prepared.exists());
        assert_eq!(fs::read(outside).unwrap(), b"outside data");
    }

    #[test]
    fn preparation_only_writes_staging_and_missing_credentials_leave_no_output() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("package");
        fs::create_dir(&package).unwrap();
        fs::create_dir(package.join("credentials")).unwrap();
        let installed = root.path().join("installed");
        fs::create_dir(&installed).unwrap();
        let original = serde_json::to_vec(&android()).unwrap();
        fs::write(installed.join("connector.json"), &original).unwrap();
        fs::write(
            package.join("connector.json"),
            serde_json::to_vec(&apple()).unwrap(),
        )
        .unwrap();
        let prepared = root.path().join("prepared");
        assert!(prepare(&package, &installed, &prepared).is_err());
        assert!(!prepared.exists());
        fs::write(
            package.join(APPLE),
            b"-----BEGIN PRIVATE KEY-----\nfixture-only\n-----END PRIVATE KEY-----\n",
        )
        .unwrap();
        prepare(&package, &installed, &prepared).unwrap();
        assert_eq!(
            config(&prepared.join("connector.json")).unwrap()["platformMode"],
            "all"
        );
        assert!(prepared.join(APPLE).is_file());
        assert!(!prepared.join(GOOGLE).exists());
        assert_eq!(
            fs::read(installed.join("connector.json")).unwrap(),
            original
        );
        assert!(prepare(&package, &installed, &prepared).is_err());
    }
}
