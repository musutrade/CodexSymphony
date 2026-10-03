//! Original-item repair authorization. Diagnostics cannot enlarge reviewed scope.
use crate::{
    bounded_recovery,
    controlled_contract::Verdict,
    extension_feedback,
    validation::{ValidationError, ValidationEvidence},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Explicit opt-in encoded in the existing reviewed repair_scope. Historical
/// prose remains readable but cannot authorize automatic post-delivery writes.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    pub schema: String,
    pub checks: BTreeMap<String, Vec<String>>,
}
impl Scope {
    pub fn parse(value: &str) -> Result<Self, &'static str> {
        let scope: Self = serde_json::from_str(value).map_err(scope_error)?;
        if scope.schema != "linked-repair/v1" || scope.checks.is_empty() {
            return Err("linked repair authorization absent");
        }
        for paths in scope.checks.values() {
            if paths.is_empty() || paths.iter().any(|p| !valid_path(p)) {
                return Err("repair scope requires explicit repository-relative paths");
            }
        }
        Ok(scope)
    }
    pub fn paths(
        &self,
        evidence: &ValidationEvidence,
        required: &[String],
    ) -> Result<Vec<String>, &'static str> {
        // Verify provenance, output hashes and complete required coverage even
        // though the trusted invocation returned a failing exit status.
        crate::validation::verify_provenance(evidence, required).map_err(evidence_error)?;
        let mut paths = Vec::new();
        for step in &evidence.steps {
            if extension_feedback::verdict(step) == Verdict::Pass {
                continue;
            }
            if !classified_step(step) {
                return Err("failure is not classified code");
            }
            let allowed = self
                .checks
                .get(&step.id)
                .ok_or("failed check outside repair scope")?;
            for path in allowed {
                paths.push(path.clone());
            }
        }
        paths.sort();
        paths.dedup();
        if paths.is_empty() {
            return Err("no failed code check");
        }
        Ok(paths)
    }
}
fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && path
            .split('/')
            .all(|p| !p.is_empty() && p != "." && p != ".." && p != ".git")
        && !path.chars().any(char::is_control)
}

pub fn failed_code(evidence: &ValidationEvidence, required: &[String]) -> bool {
    if crate::validation::verify_provenance(evidence, required).is_err() {
        return false;
    }
    let mut failed = false;
    for step in &evidence.steps {
        if extension_feedback::verdict(step) == Verdict::Pass {
            continue;
        }
        if !classified_step(step) {
            return false;
        }
        failed = true;
    }
    failed
}

fn classified_step(step: &crate::validation::StepEvidence) -> bool {
    if !step.code_failure {
        return false;
    }
    match extension_feedback::decode(step) {
        Ok(Some(feedback)) => feedback.verdict == Verdict::Fail,
        Ok(None) => {
            step.exit_code.is_some()
                && step.exit_code != Some(0)
                && bounded_recovery::native_failure(&step.output) == "check_exit"
        }
        Err(_) => false,
    }
}

fn scope_error(_: serde_json::Error) -> &'static str {
    "machine-readable repair scope absent"
}
fn evidence_error(_: ValidationError) -> &'static str {
    "invalid failure evidence identity"
}
