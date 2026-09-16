//! Pure business input and authorization rules. Selectors are data, never shell.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Contract {
    pub title: String,
    pub description: String,
    pub acceptance_criteria: Vec<Criterion>,
    pub validation_plan: Vec<Step>,
    pub network_access: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Criterion {
    pub description: String,
    pub verification_ref: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub id: String,
    pub check: String,
    pub selector: String,
    pub expected_result: String,
    pub timeout_seconds: i64,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub allowed_checks: Vec<String>,
    pub max_timeout_seconds: i64,
    pub token_limit: i64,
    pub turn_limit: i64,
    pub model_work_seconds: i64,
    pub gate_recovery_policy: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Repository {
    pub model: Option<String>,
    pub project: String,
    pub remote: String,
    pub github_repository_id: i64,
    pub base_branch: String,
    pub policy: Policy,
    pub revoked: bool,
    pub reason: String,
}

pub fn require(valid: bool, message: &'static str) -> Result<(), &'static str> {
    valid.then_some(()).ok_or(message)
}
pub fn validate_request_id(key: &str) -> Result<(), &'static str> {
    require(
        !key.trim().is_empty() && key.len() <= 200,
        "request_id is required (max 200 bytes)",
    )
}
fn text(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 16000
}
fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 200
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_:-./".contains(&b))
}
pub fn validate_repository(repo: &Repository) -> Result<(), &'static str> {
    require(
        text(&repo.project) && text(&repo.reason),
        "project and reason are required",
    )?;
    require(
        repo.remote.split('/').count() == 2 && identifier(&repo.remote),
        "remote must be owner/repository",
    )?;
    require(
        repo.github_repository_id > 0 && identifier(&repo.base_branch),
        "repository identity and base branch are required",
    )?;
    require(
        repo.model.as_ref().is_none_or(|model| identifier(model)),
        "invalid model identity",
    )?;
    validate_policy(&repo.policy)
}
fn validate_policy(policy: &Policy) -> Result<(), &'static str> {
    require(
        !policy.allowed_checks.is_empty(),
        "at least one trusted check is required",
    )?;
    for check in &policy.allowed_checks {
        require(
            matches!(check.as_str(), "cargo_test" | "npm_test"),
            "unsupported trusted check",
        )?;
    }
    require(
        (1..=3600).contains(&policy.max_timeout_seconds),
        "invalid timeout limit",
    )?;
    require(
        policy.token_limit > 0 && policy.turn_limit > 0 && policy.model_work_seconds > 0,
        "positive budget limits required",
    )?;
    require(
        policy.gate_recovery_policy == "one_code_repair",
        "0a permits only one code repair",
    )
}
pub fn validate_contract(contract: &Contract) -> Result<(), &'static str> {
    require(
        text(&contract.title) && text(&contract.description),
        "title and description are required",
    )?;
    require(
        !contract.acceptance_criteria.is_empty() && !contract.validation_plan.is_empty(),
        "acceptance criteria and validation plan are required",
    )?;
    let ids = validate_steps(&contract.validation_plan)?;
    validate_criteria(&contract.acceptance_criteria, &ids)?;
    for host in &contract.network_access {
        require(identifier(host), "invalid network intent")?;
    }
    Ok(())
}
fn validate_steps(steps: &[Step]) -> Result<std::collections::HashSet<&String>, &'static str> {
    let mut ids = std::collections::HashSet::new();
    for step in steps {
        validate_step(step)?;
        require(ids.insert(&step.id), "duplicate validation step ID")?;
    }
    Ok(ids)
}
fn validate_criteria(
    criteria: &[Criterion],
    ids: &std::collections::HashSet<&String>,
) -> Result<(), &'static str> {
    for ac in criteria {
        require(text(&ac.description), "AC description is required")?;
        require(
            ids.contains(&ac.verification_ref),
            "verification_ref must reference a validation step",
        )?;
    }
    Ok(())
}

fn validate_step(step: &Step) -> Result<(), &'static str> {
    require(
        identifier(&step.id) && identifier(&step.selector),
        "step ID and test selector must be safe identifiers",
    )?;
    require(
        matches!(step.check.as_str(), "cargo_test" | "npm_test"),
        "unsupported trusted check",
    )?;
    require(text(&step.expected_result), "expected result is required")?;
    require(
        (1..=3600).contains(&step.timeout_seconds),
        "timeout must be 1..3600 seconds",
    )
}
pub fn authorize(contract: &Contract, repository: &Repository) -> Result<(), &'static str> {
    require(!repository.revoked, "repository authorization revoked")?;
    validate_contract(contract)?;
    for step in &contract.validation_plan {
        require(
            repository.policy.allowed_checks.contains(&step.check),
            "check is not authorized by repository policy",
        )?;
        require(
            step.timeout_seconds <= repository.policy.max_timeout_seconds,
            "timeout exceeds repository policy",
        )?;
    }
    Ok(())
}
