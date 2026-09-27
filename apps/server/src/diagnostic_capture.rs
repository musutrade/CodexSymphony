//! Read only explicitly registered files with descriptor-bound traversal. Copies
//! enter the database before cleanup can remove their producer's directory.
use crate::{
    diagnostics::{Artifact, Availability, Binding, Captured, Result},
    storage_files::Directory,
};
use std::{io::Read, os::unix::fs::MetadataExt, path::Path};

pub fn capture(
    directory: &Path,
    path: &str,
    binding: &Binding,
    limit: u64,
    partial: bool,
    expires_at: i64,
) -> Result<Captured> {
    let mut result = missing(binding, path, "not generated or capture failed", expires_at)?;
    match read(directory, path, limit) {
        Ok((bytes, size)) => fill(&mut result, bytes, size, partial),
        Err(_) => {
            result.artifact.reason = Some("file unavailable, unsafe, changed or unsupported".into())
        }
    }
    Ok(result)
}

fn read(directory: &Path, path: &str, limit: u64) -> Result<(Vec<u8>, u64)> {
    if !crate::extension_contract::valid_relative_path(path) || limit > crate::diagnostics::MAX_FILE
    {
        return Err("invalid diagnostic source".into());
    }
    let directory = Directory::open(directory)?;
    let mut file = directory.read(Path::new(path))?;
    let before = file.metadata()?;
    let mut bytes = Vec::new();
    file.by_ref().take(limit).read_to_end(&mut bytes)?;
    unchanged(&directory, path, &before, &file)?;
    uncompressed(&bytes)?;
    Ok((bytes, before.len()))
}

fn unchanged(
    directory: &Directory,
    path: &str,
    before: &std::fs::Metadata,
    file: &std::fs::File,
) -> Result<()> {
    let after = file.metadata()?;
    let visible = directory.read(Path::new(path))?.metadata()?;
    if stamp(before) != stamp(&after) || stamp(before) != stamp(&visible) {
        return Err("diagnostic changed during capture".into());
    }
    Ok(())
}

fn uncompressed(bytes: &[u8]) -> Result<()> {
    if bytes.starts_with(&[0x1f, 0x8b]) || bytes.starts_with(b"PK\x03\x04") || bytes.contains(&0) {
        return Err("compressed or binary diagnostic unsupported".into());
    }
    Ok(())
}

fn stamp(meta: &std::fs::Metadata) -> (u64, u64, u64, i64, i64, i64, i64) {
    (
        meta.dev(),
        meta.ino(),
        meta.len(),
        meta.mtime(),
        meta.mtime_nsec(),
        meta.ctime(),
        meta.ctime_nsec(),
    )
}

fn fill(result: &mut Captured, bytes: Vec<u8>, size: u64, partial: bool) {
    let incomplete = partial || bytes.len() as u64 != size;
    let export = redacted(&bytes, incomplete);
    let artifact = &mut result.artifact;
    artifact.availability = if incomplete {
        Availability::Partial
    } else {
        Availability::Available
    };
    artifact.reason = if incomplete {
        Some("only captured range retained; output or storage limit/interruption".into())
    } else {
        None
    };
    artifact.original_bytes = if partial { None } else { Some(size) };
    artifact.retained_bytes = bytes.len() as u64;
    artifact.raw_sha256 = Some(crate::validation::sha256(&bytes));
    artifact.export_bytes = export.len() as u64;
    artifact.export_sha256 = Some(crate::validation::sha256(&export));
    result.raw = Some(bytes);
    result.export = Some(export);
}

fn redacted(bytes: &[u8], partial: bool) -> Vec<u8> {
    let mut text = String::from_utf8_lossy(bytes).into_owned();
    // A truncated final line may hide a credential marker or PEM boundary.
    // Never disclose that line, even when the marker has not been captured yet.
    if partial {
        let end = match text.rfind('\n') {
            Some(end) => end + 1,
            None => 0,
        };
        text.truncate(end);
    }
    if let Some(export) = structured_redaction(&text) {
        return export;
    }
    let export = crate::operator_view::redact_text(&text);
    // In an incomplete structured report a credential value may be on another
    // line than its label. Do not disclose an unparsed sensitive block.
    if (text.trim_start().starts_with('{') || text.trim_start().starts_with('['))
        && export.contains("[redacted]")
    {
        return b"[redacted]".to_vec();
    }
    export.into_bytes()
}

fn structured_redaction(text: &str) -> Option<Vec<u8>> {
    let mut value: serde_json::Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(_) => return None,
    };
    if !redact_json(&mut value) {
        return None;
    }
    match serde_json::to_vec(&value) {
        Ok(export) => Some(export),
        Err(_) => Some(b"[redacted]".to_vec()),
    }
}

fn redact_json(value: &mut serde_json::Value) -> bool {
    let mut changed = false;
    match value {
        serde_json::Value::Object(values) => {
            for (key, value) in values {
                if crate::operator_view::redact_text(&format!("\"{key}\"")) == "[redacted]" {
                    *value = serde_json::Value::String("[redacted]".into());
                    changed = true;
                } else {
                    changed |= redact_json(value);
                }
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                changed |= redact_json(value);
            }
        }
        serde_json::Value::String(text) => {
            let export = crate::operator_view::redact_text(text);
            changed = export != *text;
            *text = export;
        }
        _ => {}
    }
    changed
}

pub fn missing(
    binding: &Binding,
    purpose: &str,
    reason: &str,
    expires_at: i64,
) -> Result<Captured> {
    Ok(Captured {
        artifact: Artifact {
            artifact_id: crate::diagnostics::identifier(binding, purpose)?,
            binding: binding.clone(),
            purpose: crate::operator_view::redact_text(purpose),
            media_type: media_type(purpose).into(),
            availability: Availability::Missing,
            reason: Some(reason.into()),
            original_bytes: None,
            retained_bytes: 0,
            raw_sha256: None,
            export_bytes: 0,
            export_sha256: None,
            expires_at,
        },
        raw: None,
        export: None,
    })
}

fn media_type(purpose: &str) -> &'static str {
    match Path::new(purpose)
        .extension()
        .and_then(std::ffi::OsStr::to_str)
    {
        Some("json") => "application/json",
        Some("md") => "text/markdown; charset=utf-8",
        _ => "text/plain; charset=utf-8",
    }
}

#[cfg(test)]
#[path = "../tests/unit/diagnostic_capture.rs"]
mod tests;
