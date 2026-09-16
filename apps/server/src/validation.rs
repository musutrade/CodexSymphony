//! Fixed-candidate validation facts and the deliberately small 0a repair policy.
//!
//! This module contains no process or GitHub calls.  An adapter must collect the
//! facts and then call `verify`; a caller cannot turn a local self-check into a
//! trusted validation result by omitting the evidence.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    pub sha: String,
    pub tree: String,
    pub immutable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustedIdentity {
    pub command_sha256: String,
    pub config_sha256: String,
    pub protected_entry: String,
    pub protected_entry_sha256: String,
    pub tool: String,
    pub tool_version: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepEvidence {
    pub id: String,
    pub command: Vec<String>,
    pub exit_code: Option<i32>,
    pub output: String,
    pub output_sha256: String,
    pub log_ref: String,
    pub consumer: String,
    pub code_failure: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationEvidence {
    pub candidate: Candidate,
    pub trusted: TrustedIdentity,
    pub source_before: String,
    pub source_after: String,
    pub entry_before: String,
    pub entry_after: String,
    pub steps: Vec<StepEvidence>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValidationError {
    CandidateMutable,
    CandidateMismatch,
    TrustedIdentityMismatch,
    SourceChanged,
    ProtectedEntryChanged,
    MissingStep,
    MissingOutput,
    OutputDigestMismatch,
    ExitFailed,
    UnreadableEvidence,
}

pub fn sha256(bytes: impl AsRef<[u8]>) -> String {
    format!("{:x}", Sha256::digest(bytes.as_ref()))
}

/// Verify all independently collected facts. `required_steps` is supplied by
/// the immutable Contract, never by the executor output.
pub fn verify(
    evidence: &ValidationEvidence,
    candidate: &Candidate,
    trusted: &TrustedIdentity,
    required_steps: &[String],
) -> Result<(), ValidationError> {
    if !evidence.candidate.immutable {
        return Err(ValidationError::CandidateMutable);
    }
    if evidence.candidate != *candidate {
        return Err(ValidationError::CandidateMismatch);
    }
    if evidence.trusted != *trusted {
        return Err(ValidationError::TrustedIdentityMismatch);
    }
    if evidence.source_before != evidence.source_after {
        return Err(ValidationError::SourceChanged);
    }
    if evidence.entry_before != evidence.entry_after {
        return Err(ValidationError::ProtectedEntryChanged);
    }
    let mut seen = HashSet::new();
    for step in &evidence.steps {
        if !seen.insert(&step.id) {
            return Err(ValidationError::MissingStep);
        }
        if step.output.is_empty() || step.log_ref.is_empty() || step.consumer.is_empty() {
            return Err(ValidationError::MissingOutput);
        }
        if sha256(&step.output) != step.output_sha256 {
            return Err(ValidationError::OutputDigestMismatch);
        }
        if step.exit_code != Some(0) {
            return Err(ValidationError::ExitFailed);
        }
    }
    if required_steps.iter().any(|id| !seen.contains(id)) {
        return Err(ValidationError::MissingStep);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Declaration,
    Validation,
    RepairReservation,
    Handoff,
    Done,
}

/// Recovery resumes the durable stage. Unknown work remains in that stage for
/// reconciliation; it never starts a second validation or repair in parallel.
pub fn recover(stage: Stage, validation_active: bool, repair_reserved: bool) -> Stage {
    if validation_active {
        return Stage::Validation;
    }
    if repair_reserved {
        return Stage::RepairReservation;
    }
    stage
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureKind {
    Code,
    Infrastructure,
    Security,
    Configuration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationResult {
    Pending,
    Succeeded,
    GateFailed,
    Blocked,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepairContext {
    pub candidate: Candidate,
    pub failed_step: String,
    pub exit_code: Option<i32>,
    pub raw_output: String,
    pub remaining_acceptance: Vec<String>,
}

pub fn repair_context(
    evidence: &ValidationEvidence,
    step_id: &str,
    remaining: &[String],
) -> Option<RepairContext> {
    let step = evidence.steps.iter().find(|step| step.id == step_id)?;
    Some(RepairContext {
        candidate: evidence.candidate.clone(),
        failed_step: step.id.clone(),
        exit_code: step.exit_code,
        raw_output: step.output.clone(),
        remaining_acceptance: remaining.to_vec(),
    })
}

pub fn classify_step_failure(step: &StepEvidence) -> FailureKind {
    if step.code_failure {
        FailureKind::Code
    } else {
        FailureKind::Infrastructure
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepairLedger {
    pub count: u32,
    pub next_ordinal: u64,
    pub reserved: Option<u64>,
}

impl RepairLedger {
    pub fn new() -> Self {
        Self {
            count: 0,
            next_ordinal: 1,
            reserved: None,
        }
    }

    /// Reservation is idempotent and only code failures consume the one 0a slot.
    pub fn reserve(&mut self, kind: FailureKind) -> Option<u64> {
        if self.reserved.is_some() {
            return self.reserved;
        }
        if kind != FailureKind::Code || self.count >= 1 {
            return None;
        }
        let ordinal = self.next_ordinal;
        self.next_ordinal += 1;
        self.count += 1;
        self.reserved = Some(ordinal);
        Some(ordinal)
    }
}

impl Default for RepairLedger {
    fn default() -> Self {
        Self::new()
    }
}
