//! Administrator supplied, inert skill packages. ZIP content is never extracted or executed.
use axum::{
    body::Bytes,
    extract::State,
    http::{header, HeaderMap, StatusCode},
    Json,
};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    io::{Cursor, Read},
    sync::Arc,
};
use store_protocol::{
    skill::{self, SkillBundle, SkillFile},
    StoreError,
};

fn invalid() -> StoreError {
    StoreError::new(
        422,
        "invalid_screenshot_skill",
        "Provide one valid SKILL.md and a bounded ZIP with safe relative paths",
    )
}

fn u16_at(bytes: &[u8], at: usize) -> std::result::Result<u16, StoreError> {
    Ok(u16::from_le_bytes(
        bytes
            .get(at..at + 2)
            .ok_or_else(invalid)?
            .try_into()
            .map_err(|_| invalid())?,
    ))
}
fn u32_at(bytes: &[u8], at: usize) -> std::result::Result<u32, StoreError> {
    Ok(u32::from_le_bytes(
        bytes
            .get(at..at + 4)
            .ok_or_else(invalid)?
            .try_into()
            .map_err(|_| invalid())?,
    ))
}

// Reject split/ZIP64 archives, duplicate central entries, ambiguous local headers and
// overlapping streams before handing bounded decompression to the ZIP library.
fn zip_structure(bytes: &[u8]) -> std::result::Result<usize, StoreError> {
    let end = (0..bytes.len().saturating_sub(21))
        .rev()
        .take(65536)
        .find(|&at| {
            bytes.get(at..at + 4) == Some(b"PK\x05\x06")
                && u16_at(bytes, at + 20).is_ok_and(|n| at + 22 + usize::from(n) == bytes.len())
        })
        .ok_or_else(invalid)?;
    let count = usize::from(u16_at(bytes, end + 10)?);
    let central = u32_at(bytes, end + 16)? as usize;
    if u16_at(bytes, end + 4)? != 0
        || u16_at(bytes, end + 6)? != 0
        || usize::from(u16_at(bytes, end + 8)?) != count
        || count == 0
        || count > 1024
        || central.checked_add(u32_at(bytes, end + 12)? as usize) != Some(end)
    {
        return Err(invalid());
    }
    let mut at = central;
    let mut streams = Vec::new();
    let mut paths = BTreeMap::new();
    let mut spelling = BTreeMap::new();
    for _ in 0..count {
        if bytes.get(at..at + 4) != Some(b"PK\x01\x02") {
            return Err(invalid());
        }
        let flags = u16_at(bytes, at + 8)?;
        let method = u16_at(bytes, at + 10)?;
        let compressed = u32_at(bytes, at + 20)? as usize;
        let uncompressed = u32_at(bytes, at + 24)? as usize;
        let name_len = usize::from(u16_at(bytes, at + 28)?);
        let next = at
            .checked_add(
                46 + name_len
                    + usize::from(u16_at(bytes, at + 30)?)
                    + usize::from(u16_at(bytes, at + 32)?),
            )
            .ok_or_else(invalid)?;
        let name = bytes.get(at + 46..at + 46 + name_len).ok_or_else(invalid)?;
        let path = std::str::from_utf8(name).map_err(|_| invalid())?;
        let directory = path.ends_with('/');
        let path = path.strip_suffix('/').unwrap_or(path);
        let mode = u32_at(bytes, at + 38)? >> 16;
        let kind = mode & 0o170000;
        if next > end
            || flags & (1 | 64 | 8192) != 0
            || !matches!(method, 0 | 8)
            || u16_at(bytes, at + 34)? != 0
            || compressed > skill::MAX_ZIP_BYTES
            || uncompressed > skill::MAX_FILE_BYTES
            || !skill::valid_path(path)
            || (kind != 0 && kind != if directory { 0o040000 } else { 0o100000 })
            || (directory && uncompressed != 0)
            || paths.insert(path.to_ascii_lowercase(), directory).is_some()
        {
            return Err(invalid());
        }
        let mut prefix = String::new();
        for component in path.split('/') {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(component);
            if spelling
                .insert(prefix.to_ascii_lowercase(), prefix.clone())
                .is_some_and(|prior| prior != prefix)
            {
                return Err(invalid());
            }
        }
        let local = u32_at(bytes, at + 42)? as usize;
        if bytes.get(local..local + 4) != Some(b"PK\x03\x04")
            || u16_at(bytes, local + 6)? != flags
            || u16_at(bytes, local + 8)? != method
            || usize::from(u16_at(bytes, local + 26)?) != name_len
            || bytes.get(local + 30..local + 30 + name_len) != Some(name)
        {
            return Err(invalid());
        }
        let data = local
            .checked_add(30 + name_len + usize::from(u16_at(bytes, local + 28)?))
            .ok_or_else(invalid)?;
        let mut stop = data.checked_add(compressed).ok_or_else(invalid)?;
        if flags & 8 == 0 {
            if u32_at(bytes, local + 14)? != u32_at(bytes, at + 16)?
                || u32_at(bytes, local + 18)? != compressed as u32
                || u32_at(bytes, local + 22)? != uncompressed as u32
            {
                return Err(invalid());
            }
        } else {
            let descriptor = if bytes.get(stop..stop + 4) == Some(b"PK\x07\x08") {
                stop + 4
            } else {
                stop
            };
            if u32_at(bytes, descriptor)? != u32_at(bytes, at + 16)?
                || u32_at(bytes, descriptor + 4)? != compressed as u32
                || u32_at(bytes, descriptor + 8)? != uncompressed as u32
            {
                return Err(invalid());
            }
            stop = descriptor + 12;
        }
        if stop > central {
            return Err(invalid());
        }
        streams.push((local, stop));
        at = next;
    }
    if at != end {
        return Err(invalid());
    }
    streams.sort_unstable();
    if streams.first().is_none_or(|v| v.0 != 0)
        || streams.windows(2).any(|v| v[0].1 != v[1].0)
        || streams.last().is_none_or(|v| v.1 != central)
    {
        return Err(invalid());
    }
    for path in paths.keys() {
        let mut rest = path.as_str();
        while let Some((parent, _)) = rest.rsplit_once('/') {
            if paths.get(parent) == Some(&false) {
                return Err(invalid());
            }
            rest = parent;
        }
    }
    Ok(count)
}

pub(crate) fn normalize_zip(bytes: &[u8]) -> std::result::Result<Vec<u8>, StoreError> {
    if bytes.len() > skill::MAX_ZIP_BYTES {
        return Err(invalid());
    }
    let count = zip_structure(bytes)?;
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| invalid())?;
    if archive.len() != count {
        return Err(invalid());
    }
    let mut files = Vec::new();
    let mut total = 0;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|_| invalid())?;
        let path = entry.name().to_owned();
        if entry.is_dir() {
            continue;
        }
        // Finder metadata is irrelevant to the package but its headers and paths
        // still passed the structural safety checks above.
        if path
            .split('/')
            .any(|part| part == "__MACOSX" || part == ".DS_Store")
        {
            continue;
        }
        if files.len() >= skill::MAX_FILES {
            return Err(invalid());
        }
        let executable = entry.unix_mode().is_some_and(|mode| mode & 0o111 != 0);
        let mut contents = Vec::new();
        entry
            .by_ref()
            .take((skill::MAX_FILE_BYTES + 1) as u64)
            .read_to_end(&mut contents)
            .map_err(|_| invalid())?;
        total += contents.len();
        if contents.len() > skill::MAX_FILE_BYTES
            || total > skill::MAX_TOTAL_BYTES
            || contents.len() as u64 != entry.size()
        {
            return Err(invalid());
        }
        files.push((path, contents, executable));
    }
    let documents: Vec<_> = files
        .iter()
        .filter(|(path, _, _)| path == "SKILL.md" || path.ends_with("/SKILL.md"))
        .collect();
    if documents.len() != 1 {
        return Err(invalid());
    }
    let document = documents[0];
    let prefix = document
        .0
        .strip_suffix("SKILL.md")
        .ok_or_else(invalid)?
        .to_owned();
    let name = skill::document_name(&document.1)?;
    let files = files
        .into_iter()
        .map(|(path, data, executable)| {
            let relative = path
                .strip_prefix(&prefix)
                .filter(|p| !p.is_empty())
                .ok_or_else(invalid)?;
            Ok(SkillFile::new(relative.to_owned(), &data, executable))
        })
        .collect::<std::result::Result<Vec<_>, StoreError>>()?;
    SkillBundle {
        schema_version: 1,
        name,
        files,
    }
    .canonical_bytes()
}

pub(crate) async fn read(State(state): State<Arc<crate::ApiState>>) -> crate::Result<Json<Value>> {
    Ok(Json(state.registry.screenshot_skills()?))
}
pub(crate) async fn publish(
    State(state): State<Arc<crate::ApiState>>,
    headers: HeaderMap,
    bytes: Bytes,
) -> crate::Result<(StatusCode, Json<Value>)> {
    if !headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(';')
                .next()
                .is_some_and(|v| v.trim().eq_ignore_ascii_case("application/zip"))
        })
    {
        return Err(
            StoreError::new(415, "invalid_skill_content_type", "Upload application/zip").into(),
        );
    }
    let bundle = normalize_zip(&bytes)?;
    let output = state.registry.publish_screenshot_skill(&bundle)?;
    skill_event(&state, "screenshot_skill.published", &output);
    Ok((StatusCode::CREATED, Json(output)))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Selection {
    version: String,
}
pub(crate) async fn select(
    State(state): State<Arc<crate::ApiState>>,
    Json(input): Json<Selection>,
) -> crate::Result<Json<Value>> {
    let output = state.registry.select_screenshot_skill(&input.version)?;
    skill_event(&state, "screenshot_skill.selected", &output);
    Ok(Json(output))
}
fn skill_event(state: &crate::ApiState, name: &'static str, output: &Value) {
    state.registry.logs.record(crate::logs::Event {
        name,
        details:serde_json::json!({"name":output["active"]["name"],"version":output["active"]["version"],"sha256":output["active"]["sha256"]}),
        ..Default::default()
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::{write::SimpleFileOptions, ZipWriter};

    const DOC:&[u8]=b"---\nname: capture\ndescription: Capture the current task\n---\nDo not execute attachments during publishing.";
    fn package(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut archive = ZipWriter::new(Cursor::new(Vec::new()));
        for (path, bytes) in files {
            archive
                .start_file(
                    *path,
                    SimpleFileOptions::default()
                        .compression_method(zip::CompressionMethod::Deflated)
                        .unix_permissions(if path.ends_with(".sh") { 0o755 } else { 0o644 }),
                )
                .unwrap();
            archive.write_all(bytes).unwrap();
        }
        archive.finish().unwrap().into_inner()
    }
    #[test]
    fn wrapped_package_preserves_inert_scripts_references_and_agents() {
        let first = normalize_zip(&package(&[
            ("capture/SKILL.md", DOC),
            ("capture/scripts/check.sh", b"#!/bin/sh\nexit 12"),
            ("capture/references/task.md", b"reference"),
            ("capture/agents/openai.yaml", b"interface: {}"),
            ("__MACOSX/._SKILL.md", b"finder"),
            ("capture/.DS_Store", b"finder"),
        ]))
        .unwrap();
        let second = normalize_zip(&package(&[
            ("capture/agents/openai.yaml", b"interface: {}"),
            ("capture/references/task.md", b"reference"),
            ("capture/scripts/check.sh", b"#!/bin/sh\nexit 12"),
            ("capture/SKILL.md", DOC),
        ]))
        .unwrap();
        assert_eq!(first, second);
        let manifest = skill::SkillManifest::from_bundle(&first, 1).unwrap();
        let files = manifest.verify(&first).unwrap();
        assert_eq!(files.len(), 4);
        assert!(files.iter().any(|f| f.path == "scripts/check.sh"
            && f.executable
            && f.bytes == b"#!/bin/sh\nexit 12"));
    }
    #[test]
    fn rejects_traversal_links_case_conflicts_missing_documents_and_outside_files() {
        for path in [
            "../escape",
            "/absolute",
            "a\\b",
            "a/../b",
            "scripts",
            "skill.md",
            "Scripts/a",
            "outside.txt",
        ] {
            let files = match path {
                "scripts" => vec![
                    ("SKILL.md", DOC),
                    ("scripts", b"file".as_slice()),
                    ("scripts/check.sh", b"x".as_slice()),
                ],
                "skill.md" => vec![("SKILL.md", DOC), ("skill.md", DOC)],
                "Scripts/a" => vec![
                    ("SKILL.md", DOC),
                    ("Scripts/a", b"a".as_slice()),
                    ("scripts/b", b"b".as_slice()),
                ],
                "outside.txt" => vec![
                    ("capture/SKILL.md", DOC),
                    ("outside.txt", b"outside".as_slice()),
                ],
                _ => vec![("SKILL.md", DOC), (path, b"unsafe".as_slice())],
            };
            assert!(normalize_zip(&package(&files)).is_err(), "{path}");
        }
        assert!(normalize_zip(&package(&[("README.md", b"missing")])).is_err());
        assert!(normalize_zip(&package(&[("one/SKILL.md", DOC), ("two/SKILL.md", DOC)])).is_err());
        let mut linked = package(&[("SKILL.md", DOC), ("link", b"destination")]);
        let central = linked
            .windows(4)
            .rposition(|part| part == b"PK\x01\x02")
            .unwrap();
        linked[central + 38..central + 42].copy_from_slice(&(0o120777u32 << 16).to_le_bytes());
        assert!(normalize_zip(&linked).is_err());
    }
    #[test]
    fn rejects_zip_anomalies_and_bounded_expansion() {
        let original = package(&[("SKILL.md", DOC)]);
        let mut trailer = original.clone();
        trailer.push(0);
        assert!(normalize_zip(&trailer).is_err());
        let mut mismatched = original.clone();
        mismatched[30] = b'X';
        assert!(normalize_zip(&mismatched).is_err());
        let mut encrypted = original.clone();
        encrypted[6] |= 1;
        assert!(normalize_zip(&encrypted).is_err());
        assert!(normalize_zip(&original[..original.len() - 1]).is_err());
        assert!(normalize_zip(&vec![0; skill::MAX_ZIP_BYTES + 1]).is_err());
        assert!(normalize_zip(&package(&[
            ("SKILL.md", DOC),
            ("huge", &vec![0; skill::MAX_FILE_BYTES + 1])
        ]))
        .is_err());
        let mut files = vec![("SKILL.md".to_owned(), DOC.to_vec())];
        for index in 0..5 {
            files.push((format!("file-{index}"), vec![0; skill::MAX_FILE_BYTES]));
        }
        let borrowed = files
            .iter()
            .map(|(path, data)| (path.as_str(), data.as_slice()))
            .collect::<Vec<_>>();
        assert!(normalize_zip(&package(&borrowed)).is_err());
    }
}
