//! Work identities and recovery choices, independent of Git, SQL and runtime.
use crate::execution::RunKey;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workspace {
    pub key: RunKey,
    pub identity: String,
    pub requirement: i64,
    pub revision: i64,
    pub phase: String,
    pub baseline: String,
    pub branch: String,
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: String,
    pub digest: String,
    pub executable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub workspace: Workspace,
    pub head: String,
    pub index_tree: String,
    pub files: Vec<FileEntry>,
    pub excluded: Vec<String>,
    pub bundle_digest: String,
    pub index_digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Recovery {
    Handoff,
    Validate,
    PreserveDeclaration,
    Work,
    Blocked,
}

/// Validation identity is supplied by the validation owner, never inferred
/// from a commit or manifest. Invalidated candidates cannot re-enter handoff.
pub fn recovery(phase: &str, candidate: bool, validation: bool, work: bool) -> Recovery {
    match (phase, candidate, validation, work) {
        ("handoff", true, true, _) => Recovery::Handoff,
        ("validation", true, _, _) => Recovery::Validate,
        ("declaration", _, _, _) => Recovery::PreserveDeclaration,
        ("execution", _, _, true) => Recovery::Work,
        _ => Recovery::Blocked,
    }
}
