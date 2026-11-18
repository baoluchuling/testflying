//! Bounded, immutable screenshot skills shared by the center and worker.
use crate::StoreError;
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_ZIP_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_BUNDLE_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_TOTAL_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_FILE_BYTES: usize = 1024 * 1024;
pub const MAX_FILES: usize = 256;

fn invalid() -> StoreError {
    StoreError::new(
        422,
        "invalid_screenshot_skill",
        "Provide one valid SKILL.md and a bounded skill package with safe relative file paths",
    )
}

pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !name.starts_with('-')
        && !name.ends_with('-')
}

pub fn valid_version(version: &str) -> bool {
    version.len() == 64
        && version
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && path.is_ascii()
        && path.len() <= 240
        && !path
            .chars()
            .any(|c| c.is_control() || matches!(c, '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*'))
        && path.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && part != ".git"
                && !part.ends_with(['.', ' '])
        })
}

pub fn document_name(bytes: &[u8]) -> Result<String, StoreError> {
    if bytes.len() > 64 * 1024 {
        return Err(invalid());
    }
    let doc = std::str::from_utf8(bytes)
        .map_err(|_| invalid())?
        .replace("\r\n", "\n");
    let header = doc
        .strip_prefix("---\n")
        .and_then(|value| value.split_once("\n---").map(|(header, _)| header))
        .ok_or_else(invalid)?;
    let mut names = header.lines().filter_map(|line| line.strip_prefix("name:"));
    let raw = names.next().ok_or_else(invalid)?.trim();
    if names.next().is_some() {
        return Err(invalid());
    }
    let name = if raw.starts_with('"') && raw.ends_with('"')
        || raw.starts_with('\'') && raw.ends_with('\'')
    {
        raw.get(1..raw.len().saturating_sub(1))
            .ok_or_else(invalid)?
    } else {
        raw
    };
    if !valid_name(name)
        || !header.lines().any(|line| {
            line.strip_prefix("description:")
                .is_some_and(|value| !value.trim().is_empty())
        })
    {
        return Err(invalid());
    }
    Ok(name.to_owned())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkillFile {
    pub path: String,
    pub data: String,
    pub executable: bool,
}

impl SkillFile {
    pub fn new(path: String, bytes: &[u8], executable: bool) -> Self {
        Self {
            path,
            data: STANDARD.encode(bytes),
            executable,
        }
    }
}

pub struct DecodedSkillFile {
    pub path: String,
    pub bytes: Vec<u8>,
    pub executable: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkillBundle {
    pub schema_version: u32,
    pub name: String,
    pub files: Vec<SkillFile>,
}

impl SkillBundle {
    pub fn decoded_files(&self) -> Result<Vec<DecodedSkillFile>, StoreError> {
        if self.schema_version != 1
            || !valid_name(&self.name)
            || self.files.is_empty()
            || self.files.len() > MAX_FILES
        {
            return Err(invalid());
        }
        let mut paths = BTreeSet::new();
        let mut components = BTreeMap::new();
        let mut total = 0;
        let mut output = Vec::new();
        for file in &self.files {
            if !valid_path(&file.path)
                || !paths.insert(file.path.to_ascii_lowercase())
                || file.data.len() > MAX_FILE_BYTES.div_ceil(3) * 4
            {
                return Err(invalid());
            }
            let mut prefix = String::new();
            for part in file.path.split('/') {
                if !prefix.is_empty() {
                    prefix.push('/');
                }
                prefix.push_str(part);
                if components
                    .insert(prefix.to_ascii_lowercase(), prefix.clone())
                    .is_some_and(|old| old != prefix)
                {
                    return Err(invalid());
                }
            }
            let bytes = STANDARD.decode(&file.data).map_err(|_| invalid())?;
            total += bytes.len();
            if bytes.len() > MAX_FILE_BYTES || total > MAX_TOTAL_BYTES {
                return Err(invalid());
            }
            if file.path == "SKILL.md" && document_name(&bytes)? != self.name {
                return Err(invalid());
            }
            output.push(DecodedSkillFile {
                path: file.path.clone(),
                bytes,
                executable: file.executable,
            });
        }
        if !self.files.iter().any(|file| file.path == "SKILL.md") {
            return Err(invalid());
        }
        for path in &paths {
            let mut rest = path.as_str();
            while let Some((parent, _)) = rest.rsplit_once('/') {
                if paths.contains(parent) {
                    return Err(invalid());
                }
                rest = parent;
            }
        }
        Ok(output)
    }

    pub fn canonical_bytes(mut self) -> Result<Vec<u8>, StoreError> {
        self.files.sort_by(|a, b| a.path.cmp(&b.path));
        self.decoded_files()?;
        let bytes = serde_json::to_vec(&self).map_err(|_| invalid())?;
        if bytes.len() > MAX_BUNDLE_BYTES {
            return Err(invalid());
        }
        Ok(bytes)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkillManifest {
    pub schema_version: u32,
    pub name: String,
    pub version: String,
    pub sha256: String,
    pub bytes: usize,
    pub files: usize,
    pub published_at: i64,
}

impl SkillManifest {
    pub fn from_bundle(bytes: &[u8], published_at: i64) -> Result<Self, StoreError> {
        if bytes.len() > MAX_BUNDLE_BYTES {
            return Err(invalid());
        }
        let bundle: SkillBundle = serde_json::from_slice(bytes).map_err(|_| invalid())?;
        bundle.decoded_files()?;
        let sha256 = hex::encode(Sha256::digest(bytes));
        Ok(Self {
            schema_version: 1,
            name: bundle.name,
            version: sha256.clone(),
            sha256,
            bytes: bytes.len(),
            files: bundle.files.len(),
            published_at,
        })
    }

    pub fn verify(&self, bytes: &[u8]) -> Result<Vec<DecodedSkillFile>, StoreError> {
        if self.schema_version != 1
            || !valid_name(&self.name)
            || !valid_version(&self.version)
            || self.sha256 != self.version
            || self.bytes == 0
            || self.bytes > MAX_BUNDLE_BYTES
            || self.bytes != bytes.len()
            || self.files == 0
            || self.files > MAX_FILES
            || hex::encode(Sha256::digest(bytes)) != self.sha256
        {
            return Err(invalid());
        }
        let bundle: SkillBundle = serde_json::from_slice(bytes).map_err(|_| invalid())?;
        if bundle.name != self.name || bundle.files.len() != self.files {
            return Err(invalid());
        }
        bundle.decoded_files()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bundle() -> SkillBundle {
        SkillBundle {
            schema_version: 1,
            name: "capture".into(),
            files: vec![SkillFile::new(
                "SKILL.md".into(),
                b"---\nname: capture\ndescription: Capture an app\n---\nUse local tools.",
                false,
            )],
        }
    }
    #[test]
    fn portable_bundle_verifies_all_files_and_rejects_tampering() {
        let bytes = bundle().canonical_bytes().unwrap();
        let manifest = SkillManifest::from_bundle(&bytes, 1).unwrap();
        assert_eq!(manifest.verify(&bytes).unwrap().len(), 1);
        let mut changed = bytes.clone();
        changed.push(b' ');
        assert!(manifest.verify(&changed).is_err());
        for path in [
            "../escape",
            "/absolute",
            "a/../b",
            "a\\b",
            "a:b",
            "a//b",
            ".git/config",
            "a. ",
        ] {
            let mut value = bundle();
            value.files.push(SkillFile::new(path.into(), b"x", false));
            assert!(value.canonical_bytes().is_err(), "{path}");
        }
        let mut value = bundle();
        value
            .files
            .push(SkillFile::new("SKILL.md".into(), b"duplicate", false));
        assert!(value.canonical_bytes().is_err());
        let mut value = bundle();
        value
            .files
            .push(SkillFile::new("Scripts/a.py".into(), b"x", true));
        value
            .files
            .push(SkillFile::new("scripts/b.py".into(), b"x", true));
        assert!(value.canonical_bytes().is_err());
        let mut value = bundle();
        value
            .files
            .push(SkillFile::new("scripts".into(), b"file", false));
        value
            .files
            .push(SkillFile::new("scripts/a.py".into(), b"x", true));
        assert!(value.canonical_bytes().is_err());
    }
}
