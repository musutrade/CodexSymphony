//! Execution decisions only; persistence and OS supervision are separate adapters.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunKey {
    pub run_id: String,
    pub request_id: String,
    pub incarnation: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessIdentity {
    pub pid: u32,
    pub group: u32,
    pub start_ticks: u64,
    pub boot_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Launch {
    pub key: RunKey,
    pub workspace: String,
    pub workspace_identity: String,
    pub program: String,
    pub args: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    pub key: RunKey,
    pub process: ProcessIdentity,
}

pub fn accepts_event(current: &RunKey, incoming: &RunKey, actions_allowed: bool) -> bool {
    actions_allowed && current == incoming
}

pub fn receipt_matches(key: &RunKey, process: &ProcessIdentity, receipt: &Receipt) -> bool {
    &receipt.key == key && &receipt.process == process
}

/// Local worktree/Broker and preservation exist; real coding remains disabled
/// until Runtime, preflight, disk and cumulative budget checks are implemented.
pub const CODING_BLOCKER: &str = "coding prerequisites not implemented";
