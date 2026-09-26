//! Reviewed exact-version integration contract; no execution or persistence.
use crate::{
    draft::Document,
    group_review::{Item, RepositorySnapshot},
    validation::{Candidate, TrustedIdentity, ValidationEvidence},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Authorization {
    pub configuration_sha256: String,
    pub repositories: Vec<Repository>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Repository {
    pub repository_id: i64,
    pub repository_version: i64,
    pub selection: Selection,
    pub repair_scope: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Selection {
    Fixed { sha: String },
    CompletedDependencies,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Version {
    pub repository_id: i64,
    pub github_repository_id: i64,
    pub repository_version: i64,
    pub candidate: Candidate,
    pub artifacts: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub requirement: i64,
    pub revision: i64,
    pub authorization: i64,
    pub input_sha256: String,
    pub versions: Vec<Version>,
    pub trusted: TrustedIdentity,
    pub required: Vec<String>,
}
pub fn validate(
    a: &Authorization,
    item: &Item,
    document: &Document,
    repositories: &[RepositorySnapshot],
) -> Result<(), String> {
    let child = document
        .children
        .iter()
        .find(|c| c.id == item.child_id)
        .ok_or("missing child")?;
    if child.kind != "validation_only"
        || !hex(&a.configuration_sha256, 64)
        || a.repositories.is_empty()
    {
        return Err(
            "integration authorization requires validation_only and a pinned configuration".into(),
        );
    }
    let mut seen = BTreeSet::new();
    for r in &a.repositories {
        validate_repository(r, repositories)?;
        if !seen.insert(r.repository_id) {
            return Err("unique authorized repositories required".into());
        }
    }
    validate_coverage(document, child, &seen)
}
fn validate_coverage(
    document: &Document,
    child: &crate::draft::Child,
    seen: &BTreeSet<i64>,
) -> Result<(), String> {
    for previous in &document.children {
        if previous.kind == "code_change"
            && previous.order < child.order
            && !previous.repository_id.is_some_and(|id| seen.contains(&id))
        {
            return Err(
                "integration version set must include every preceding code repository".into(),
            );
        }
    }
    if !child.repository_id.is_some_and(|id| seen.contains(&id)) {
        return Err("integration primary repository must be authorized".into());
    }
    Ok(())
}
fn validate_repository(r: &Repository, repositories: &[RepositorySnapshot]) -> Result<(), String> {
    if r.repair_scope.trim().is_empty() {
        return Err("explicit repair scope required".into());
    }
    if !repositories.iter().any(|p| {
        p.id == r.repository_id && p.version == r.repository_version && !p.repository.revoked
    }) {
        return Err("integration repository policy absent, stale or revoked".into());
    }
    if let Selection::Fixed { sha } = &r.selection
        && !hex(sha, 40)
    {
        return Err("fixed integration version requires a full commit SHA".into());
    }
    Ok(())
}
pub fn verify(binding: &Binding, versions: &[Version], evidence: &ValidationEvidence) -> bool {
    binding.versions == versions
        && versions.first().is_some_and(|v| {
            crate::validation::verify(evidence, &v.candidate, &binding.trusted, &binding.required)
                .is_ok()
        })
}
fn hex(value: &str, size: usize) -> bool {
    value.len() == size && value.bytes().all(|b| b.is_ascii_hexdigit())
}

impl Serialize for Version {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut value = serde_json::json!({"repository_id":self.repository_id,"repository_version":self.repository_version,"candidate":self.candidate,"artifacts":self.artifacts});
        if self.github_repository_id > 0 {
            value["github_repository_id"] = serde_json::json!(self.github_repository_id);
        }
        value.serialize(serializer)
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VersionWire {
    repository_id: i64,
    repository_version: i64,
    github_repository_id: Option<i64>,
    candidate: Candidate,
    artifacts: Vec<String>,
}
impl<'de> Deserialize<'de> for Version {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = VersionWire::deserialize(deserializer)?;
        Ok(Self {
            repository_id: wire.repository_id,
            repository_version: wire.repository_version,
            github_repository_id: wire.github_repository_id.unwrap_or(0),
            candidate: wire.candidate,
            artifacts: wire.artifacts,
        })
    }
}
