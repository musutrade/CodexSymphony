//! Reviewed process adapter for P9 validation. The caller supplies the frozen
//! invocation; candidate output cannot select checks, provenance or publication.
use crate::{
    controlled_contract::{Call, CheckResult, Evaluation, EvidenceRef, Operation, Verdict},
    process,
    validation::{Candidate, StepEvidence, sha256},
    validation_runner::{self, Plan},
};
use std::path::Path;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub fn execute(
    checkout: &Path,
    directory: &Path,
    candidate: &Candidate,
    plan: &Plan,
    call: &Call,
) -> Result<(Evaluation, Vec<StepEvidence>)> {
    execute_limited(checkout, directory, candidate, plan, call, 1024 * 1024)
}

pub fn execute_limited(
    checkout: &Path,
    directory: &Path,
    candidate: &Candidate,
    plan: &Plan,
    call: &Call,
    limit: u64,
) -> Result<(Evaluation, Vec<StepEvidence>)> {
    check_inputs(candidate, plan, call)?;
    std::fs::create_dir_all(directory)?;
    if std::fs::canonicalize(&plan.entry)? != plan.entry
        || std::fs::canonicalize(directory)? != directory
    {
        return Err("approved validation entry and evidence paths must be canonical".into());
    }
    let _lock = process::InstanceLock::acquire(&directory.join("hook.lock"))?;
    let path = directory.join("invocation.json");
    if path.exists() {
        let saved: Call = process::read(&path)?;
        if saved != *call {
            return Err("validation invocation changed; reconcile original call".into());
        }
    } else {
        process::durable_write(&path, call)?;
    }
    let steps = validation_runner::execute_limited(checkout, directory, candidate, plan, limit)?;
    let evaluation = evaluation(call, &steps);
    let result_path = directory.join("evaluation.json");
    if result_path.exists() {
        let saved: Evaluation = process::read(&result_path)?;
        if saved != evaluation {
            return Err("retained validation evaluation changed".into());
        }
    } else {
        process::durable_write(&result_path, &evaluation)?;
    }
    Ok((evaluation, steps))
}

fn check_inputs(candidate: &Candidate, plan: &Plan, call: &Call) -> Result<()> {
    let source = call
        .candidate
        .as_ref()
        .ok_or("validation candidate missing")?;
    if call.operation != Operation::Validate
        || source.commit != candidate.sha
        || source.tree != candidate.tree
        || call.implementation_digest != plan.identity()?.protected_entry_sha256
    {
        return Err("validation adapter identity mismatch".into());
    }
    let checks: Vec<_> = plan.steps.iter().map(|step| step.id.clone()).collect();
    if checks != call.required_checks || checks.is_empty() {
        return Err("validation adapter required checks mismatch".into());
    }
    if crate::runtime_client::now().saturating_mul(1000) >= call.deadline_unix_ms {
        return Err("validation invocation expired".into());
    }
    Ok(())
}

pub fn evaluation(call: &Call, steps: &[StepEvidence]) -> Evaluation {
    let checks: Vec<_> = steps.iter().map(check_result).collect();
    let verdict = if checks.iter().any(|check| check.verdict == Verdict::Unknown) {
        Verdict::Unknown
    } else if checks.iter().any(|check| check.verdict == Verdict::Fail) {
        Verdict::Fail
    } else {
        Verdict::Pass
    };
    Evaluation {
        call: call.clone(),
        verdict,
        checks,
    }
}

fn check_result(step: &StepEvidence) -> CheckResult {
    let verdict = match step.exit_code {
        Some(0) if !step.output.is_empty() && sha256(&step.output) == step.output_sha256 => {
            Verdict::Pass
        }
        Some(_) => Verdict::Fail,
        None => Verdict::Unknown,
    };
    CheckResult {
        id: step.id.clone(),
        verdict,
        evidence: Vec::from([EvidenceRef {
            artifact_id: step.log_ref.clone(),
            sha256: step.output_sha256.clone(),
        }]),
    }
}
