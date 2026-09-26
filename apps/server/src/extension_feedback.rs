//! Opt-in process feedback. The reviewed command selects the protocol; output
//! cannot grant authority, change the candidate, or turn a crash into PASS.
use crate::{controlled_contract::Verdict, validation::StepEvidence};
use serde::{Deserialize, Serialize};

pub const SELECTOR: &str = "--symphony-feedback-v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FaultClass {
    Input,
    Resource,
    Internal,
    Dependency,
    Unsupported,
    Protocol,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fault {
    pub class: FaultClass,
    pub code: String,
    pub message: String,
    pub owner: String,
    pub scope: Vec<String>,
    pub resume_condition: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Feedback {
    pub protocol_version: u32,
    pub check_id: String,
    pub verdict: Verdict,
    pub fault: Option<Fault>,
}

pub fn negotiated(step: &StepEvidence) -> bool {
    step.command.get(1).map(String::as_str) == Some(SELECTOR)
}

pub fn decode(step: &StepEvidence) -> Result<Option<Feedback>, &'static str> {
    if !negotiated(step) {
        return Ok(None);
    }
    if step.output.len() > 65536 || crate::validation::sha256(&step.output) != step.output_sha256 {
        return Err("feedback bytes missing, oversized or changed");
    }
    let feedback: Feedback = serde_json::from_str(&step.output).map_err(invalid_json)?;
    validate(&feedback, step)?;
    Ok(Some(feedback))
}

fn invalid_json(_: serde_json::Error) -> &'static str {
    "invalid structured feedback"
}

fn validate(feedback: &Feedback, step: &StepEvidence) -> Result<(), &'static str> {
    if feedback.protocol_version != 1 || feedback.check_id != step.id {
        return Err("feedback version or check identity differs");
    }
    match (&feedback.verdict, &feedback.fault) {
        (Verdict::Unknown, Some(fault)) => validate_fault(fault)?,
        (Verdict::Pass | Verdict::Fail, None) => {}
        _ => return Err("feedback conclusion contradicts fault"),
    }
    // Completed evaluation uses exit 0, including a business FAIL. A nonzero
    // exit can carry diagnostics but cannot attest a completed evaluation.
    if feedback.verdict != Verdict::Unknown && step.exit_code != Some(0) {
        return Err("process exit contradicts completed evaluation");
    }
    Ok(())
}

fn validate_fault(fault: &Fault) -> Result<(), &'static str> {
    for value in [
        &fault.code,
        &fault.message,
        &fault.owner,
        &fault.resume_condition,
    ] {
        if value.trim().is_empty() || value.len() > 4096 {
            return Err("invalid fault description");
        }
    }
    if fault.scope.len() > 64 {
        return Err("too many affected scopes");
    }
    for scope in &fault.scope {
        if scope.is_empty() || scope.len() > 1024 {
            return Err("invalid affected scope");
        }
    }
    Ok(())
}

pub fn verdict(step: &StepEvidence) -> Verdict {
    match decode(step) {
        Ok(Some(feedback)) => feedback.verdict,
        Err(_) => Verdict::Unknown,
        Ok(None) => legacy_verdict(step),
    }
}

fn legacy_verdict(step: &StepEvidence) -> Verdict {
    if step.output.is_empty() || crate::validation::sha256(&step.output) != step.output_sha256 {
        return Verdict::Unknown;
    }
    match step.exit_code {
        Some(0) => Verdict::Pass,
        Some(_) => Verdict::Fail,
        None => Verdict::Unknown,
    }
}

pub fn status(step: &StepEvidence) -> &'static str {
    match verdict(step) {
        Verdict::Pass => "succeeded",
        Verdict::Fail => "failed",
        Verdict::Unknown => "unknown",
    }
}

pub fn code_failure(step: &StepEvidence) -> bool {
    step.code_failure && verdict(step) == Verdict::Fail
}
