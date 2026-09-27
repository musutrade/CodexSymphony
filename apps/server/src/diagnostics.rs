//! Untrusted diagnostics are content, never authority. Offsets and digests
//! describe the fully redacted export, independently from the retained original.
use crate::{extension_contract::InvocationIdentity, validation::Candidate};
use serde::{Deserialize, Serialize};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
pub const MAX_CHUNK: usize = 8192;
pub const MAX_FILE: u64 = 1024 * 1024;
pub const MAX_FILES: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub identity: InvocationIdentity,
    pub phase: String,
    pub candidate: Option<Candidate>,
    pub validation_id: Option<String>,
    pub generation: i64,
    pub implementation_digest: String,
    pub environment_digest: String,
    pub policy_digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    Available,
    Partial,
    Missing,
    Expired,
    Corrupt,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Artifact {
    pub artifact_id: String,
    pub binding: Binding,
    pub purpose: String,
    pub media_type: String,
    pub availability: Availability,
    pub reason: Option<String>,
    pub original_bytes: Option<u64>,
    pub retained_bytes: u64,
    pub raw_sha256: Option<String>,
    pub export_bytes: u64,
    pub export_sha256: Option<String>,
    pub expires_at: i64,
}

pub struct Captured {
    pub artifact: Artifact,
    pub raw: Option<Vec<u8>>,
    pub export: Option<Vec<u8>>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Page {
    pub artifacts: Vec<Artifact>,
    pub next: Option<i64>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Chunk {
    pub artifact: Artifact,
    pub offset: u64,
    pub next: u64,
    pub end: bool,
    pub unit: String,
    pub text: String,
}

pub fn chunk(artifact: Artifact, bytes: &[u8], offset: u64, limit: usize) -> Result<Chunk> {
    if limit == 0 || limit > MAX_CHUNK || offset > bytes.len() as u64 {
        return Err("invalid diagnostic byte range".into());
    }
    verify_export(&artifact, bytes)?;
    let text = std::str::from_utf8(bytes)?;
    let start = offset as usize;
    if !text.is_char_boundary(start) {
        return Err("offset must be a UTF-8 boundary".into());
    }
    let stop = boundary(text, start, limit)?;
    Ok(Chunk {
        artifact,
        offset,
        next: stop as u64,
        end: stop == bytes.len(),
        unit: "bytes".into(),
        text: text[start..stop].into(),
    })
}

fn verify_export(artifact: &Artifact, bytes: &[u8]) -> Result<()> {
    if !matches!(
        artifact.availability,
        Availability::Available | Availability::Partial
    ) {
        return Err("diagnostic content unavailable".into());
    }
    if artifact.export_bytes != bytes.len() as u64
        || artifact.export_sha256.as_deref() != Some(&crate::validation::sha256(bytes))
    {
        return Err("diagnostic export size or digest differs".into());
    }
    Ok(())
}

fn boundary(text: &str, start: usize, limit: usize) -> Result<usize> {
    let mut stop = start.saturating_add(limit).min(text.len());
    while !text.is_char_boundary(stop) {
        stop -= 1;
    }
    if stop == start && start < text.len() {
        return Err("limit cannot contain the next UTF-8 character".into());
    }
    Ok(stop)
}

pub fn identifier(binding: &Binding, purpose: &str) -> Result<String> {
    Ok(crate::validation::sha256(serde_json::to_vec(&(
        binding, purpose,
    ))?))
}

#[cfg(test)]
#[path = "../tests/unit/diagnostics.rs"]
pub(crate) mod tests;
