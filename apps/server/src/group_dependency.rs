//! Pure dependency-completion identity contract. No runtime or persistence.
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fact {
    pub requirement_id: i64,
    pub authorization_id: i64,
    pub child_revision: i64,
    pub repository_id: i64,
    pub github_repository_id: i64,
    pub pr_number: i64,
    pub head_sha: String,
    pub merged_sha: String,
    pub acceptance_sha: String,
    pub acceptance_plan: Value,
    pub source: String,
    pub evidence_sha256: String,
    pub artifact: String,
}
/// The host adapter must authenticate provenance and applicability to the exact
/// merged version and approved plan. HTTP/Agent input cannot implement this trait.
pub trait Verifier {
    fn verify(&self, fact: &Fact) -> bool;
}
pub fn validate(fact: &Fact) -> bool {
    identities(fact) && versions(fact) && provenance(fact)
}
fn identities(fact: &Fact) -> bool {
    fact.requirement_id > 0
        && fact.authorization_id > 0
        && fact.child_revision > 0
        && fact.repository_id > 0
        && fact.github_repository_id > 0
        && fact.pr_number > 0
}
fn versions(fact: &Fact) -> bool {
    hex(&fact.head_sha, 40) && hex(&fact.merged_sha, 40) && fact.acceptance_sha == fact.merged_sha
}
fn provenance(fact: &Fact) -> bool {
    hex(&fact.evidence_sha256, 64)
        && !fact.source.trim().is_empty()
        && !fact.artifact.trim().is_empty()
        && fact
            .acceptance_plan
            .as_array()
            .is_some_and(|p| !p.is_empty())
}
fn hex(value: &str, len: usize) -> bool {
    value.len() == len && value.bytes().all(|b| b.is_ascii_hexdigit())
}
