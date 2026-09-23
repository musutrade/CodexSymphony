//! Original-item repair authorization. Diagnostics cannot enlarge reviewed scope.
use crate::{
    bounded_recovery,
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
        let mut identity = evidence.clone();
        for step in &mut identity.steps {
            step.exit_code = Some(0);
        }
        crate::validation::verify(&identity, &evidence.candidate, &evidence.trusted, required)
            .map_err(evidence_error)?;
        let mut paths = Vec::new();
        for step in evidence.steps.iter().filter(|s| s.exit_code != Some(0)) {
            if !classified_step(step) {
                return Err("failure is not classified code");
            }
            let allowed = self
                .checks
                .get(&step.id)
                .ok_or("failed check outside repair scope")?;
            paths.extend(allowed.iter().cloned());
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
    crate::validation::verify(evidence, &evidence.candidate, &evidence.trusted, required)
        == Err(ValidationError::ExitFailed)
        && evidence
            .steps
            .iter()
            .filter(|s| s.exit_code != Some(0))
            .all(classified_step)
}

fn classified_step(step: &crate::validation::StepEvidence) -> bool {
    step.code_failure
        && step.exit_code.is_some()
        && bounded_recovery::native_failure(&step.output) == "check_exit"
}

fn scope_error(_: serde_json::Error) -> &'static str {
    "machine-readable repair scope absent"
}
fn evidence_error(_: ValidationError) -> &'static str {
    "invalid failure evidence identity"
}
