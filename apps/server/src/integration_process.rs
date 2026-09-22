//! Validation child hosted by the existing subreaper, without an AgentRun.
use crate::{
    integration::{Binding, Version},
    process,
    validation::{self, ValidationEvidence},
    validation_runner::{self, Plan},
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    pub invocation: String,
    pub binding: Binding,
    pub plan: Plan,
    pub checkouts: Vec<PathBuf>,
    pub output_limit: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Outcome {
    pub binding_sha256: String,
    pub evidence: Option<ValidationEvidence>,
    pub error: Option<String>,
}
pub fn run(directory: &Path) -> Result<()> {
    let job: Job = process::read(&directory.join("job.json"))?;
    if directory.file_name().and_then(|v| v.to_str()) != Some(&job.invocation) {
        return Err("validation invocation identity differs".into());
    }
    let binding_sha256 = validation::sha256(serde_json::to_vec(&job)?);
    let (evidence, error) = match execute(directory, &job) {
        Ok(evidence) => (Some(evidence), None),
        Err(error) => (
            None,
            Some(crate::operator_view::redact_text(&error.to_string())),
        ),
    };
    process::durable_write(
        &directory.join("outcome.json"),
        &Outcome {
            binding_sha256,
            evidence,
            error,
        },
    )?;
    Ok(())
}
pub(crate) fn versions(job: &Job) -> Result<Vec<Version>> {
    if job.checkouts.len() != job.binding.versions.len() {
        return Err("version checkout set changed".into());
    }
    job.binding
        .versions
        .iter()
        .zip(&job.checkouts)
        .map(|(version, path)| {
            let mut actual = version.clone();
            actual.candidate = validation_runner::candidate(path)?;
            Ok(actual)
        })
        .collect()
}
fn execute(directory: &Path, job: &Job) -> Result<ValidationEvidence> {
    let actual = checked_versions(job)?;
    let primary = actual.first().ok_or("empty version set")?;
    let steps = validation_runner::execute_limited(
        &job.checkouts[0],
        &directory.join("checks"),
        &primary.candidate,
        &job.plan,
        job.output_limit,
    )?;
    let evidence = evidence(job, primary, steps)?;
    if versions(job)? != actual {
        return Err("integration version drift".into());
    }
    Ok(evidence)
}
fn checked_versions(job: &Job) -> Result<Vec<Version>> {
    let actual = versions(job)?;
    if actual != job.binding.versions || job.plan.identity()? != job.binding.trusted {
        return Err("integration input identity changed".into());
    }
    Ok(actual)
}
fn evidence(
    job: &Job,
    primary: &Version,
    steps: Vec<crate::validation::StepEvidence>,
) -> Result<ValidationEvidence> {
    if !commands_match(&steps, &job.plan) {
        return Err("validation step command identity differs".into());
    }
    let evidence = ValidationEvidence {
        candidate: primary.candidate.clone(),
        trusted: job.binding.trusted.clone(),
        source_before: primary.candidate.tree.clone(),
        source_after: validation_runner::candidate(&job.checkouts[0])?.tree,
        entry_before: job.binding.trusted.protected_entry_sha256.clone(),
        entry_after: job.plan.identity()?.protected_entry_sha256,
        steps,
    };
    Ok(evidence)
}
fn commands_match(steps: &[crate::validation::StepEvidence], plan: &Plan) -> bool {
    steps.len() == plan.steps.len()
        && steps.iter().zip(&plan.steps).all(|(actual, expected)| {
            actual.id == expected.id && actual.command == expected.command
        })
}
/// Recovery can consume a completed invocation, never create another invocation.
pub(crate) fn reconcile(directory: &Path, job: &Job) -> Result<Outcome> {
    let outcome: Outcome = process::read(&directory.join("outcome.json"))?;
    if directory.file_name().and_then(|v| v.to_str()) != Some(&job.invocation) {
        return Err("validation invocation identity differs".into());
    }
    if outcome.binding_sha256 != validation::sha256(serde_json::to_vec(job)?) {
        return Err("integration evidence binding changed".into());
    }
    if let Some(expected) = &outcome.evidence {
        if !directory.join("checks/result.json").is_file() {
            return Err("original validation output missing".into());
        }
        if execute(directory, job)? != *expected {
            return Err("retained integration evidence changed".into());
        }
    }
    Ok(outcome)
}
