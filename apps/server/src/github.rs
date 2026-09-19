//! Pure, fail-closed GitHub facts. No business state or execution side effects.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    Status {
        creator_id: u64,
    },
    CheckRun {
        app_id: u64,
    },
    Actions {
        app_id: u64,
        workflow_id: u64,
        workflow_sha: String,
        event: String,
        branch: String,
        /// Explicit opt-in for platform-generated single-repository PR branches.
        /// Missing/false retains the historical exact branch constraint.
        branch_from_pr: Option<bool>,
    },
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Selector {
    pub name: String,
    pub source: Source,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Policy {
    pub repository_id: u64,
    pub repository: String,
    pub default_branch: String,
    pub version: i64,
    /// Current 0a: head checks only. Test-merge cannot imply checkout identity.
    pub required: Vec<Selector>,
    pub wait_seconds: u64,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Capability {
    pub policy: Policy,
    pub checked_at: i64,
    pub blockers: Vec<String>,
    pub permissions: Value,
    pub configuration: Value,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum MergeFact {
    Unknown,
    Unmerged,
    Merged,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum CheckState {
    Missing,
    Ambiguous,
    Pending,
    Failure,
    Success,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Check {
    pub selector: Selector,
    pub state: CheckState,
    pub evidence: Vec<Value>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Observation {
    pub policy: Policy,
    pub actual_checkout_sha: Option<String>,
    pub repository_id: u64,
    pub number: u64,
    pub head: String,
    pub base: String,
    pub head_ref: String,
    pub base_ref: String,
    pub test_merge_sha: Option<String>,
    pub merged_sha: Option<String>,
    pub merge: MergeFact,
    pub closed: bool,
    pub checks: Vec<Check>,
    pub last_synced_at: i64,
}

pub fn merge_fact(pr: &Value) -> MergeFact {
    match (pr["merged"].as_bool(), pr["merged_at"].as_str()) {
        (Some(true), _) => MergeFact::Merged,
        (_, Some(value)) if !value.is_empty() => MergeFact::Merged,
        (Some(false), None) => MergeFact::Unmerged,
        _ => MergeFact::Unknown,
    }
}
pub fn retry_delay(failures: u32) -> i64 {
    (60_i64.saturating_mul(1_i64 << failures.min(3))).min(300)
}
pub fn validate_policy(policy: &Policy) -> bool {
    repository_name(&policy.repository)
        && policy.repository_id > 0
        && policy.version > 0
        && !policy.default_branch.is_empty()
        && !policy.required.is_empty()
        && policy.wait_seconds > 0
}
fn repository_name(name: &str) -> bool {
    name.bytes().all(repository_byte)
        && !name.contains("..")
        && name.split('/').count() == 2
        && !name.starts_with('/')
        && !name.ends_with('/')
}
fn repository_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"_-./".contains(&byte)
}

/// Bind only an explicitly authorized dynamic source to the coherent same-repo
/// PR observation. The original policy remains the stored trust identity.
pub fn bind_pr_source(selector: &Selector, pr: &Value, repository: u64) -> Option<Selector> {
    let mut bound = selector.clone();
    if let Source::Actions {
        branch,
        branch_from_pr: Some(true),
        ..
    } = &mut bound.source
    {
        if pr["head"]["repo"]["id"] != repository || pr["base"]["repo"]["id"] != repository {
            return None;
        }
        let head_ref = pr["head"]["ref"].as_str()?;
        if head_ref.is_empty() {
            return None;
        }
        *branch = head_ref.to_owned();
    }
    Some(bound)
}

/// Resolve only the selected namespace/publisher. Other publishers cannot lend
/// success. Generic suites have no cross-suite replacement proof: block ambiguity.
pub fn resolve(
    selector: &Selector,
    checks: &[Value],
    statuses: &[Value],
    runs: &[Value],
    jobs: &[Value],
) -> Check {
    let candidates = match &selector.source {
        Source::Status { creator_id } => status_candidates(selector, *creator_id, statuses),
        Source::CheckRun { app_id } => check_candidates(selector, *app_id, checks),
        Source::Actions { .. } => action_candidates(selector, checks, runs, jobs),
    };
    let state = candidate_state(&candidates, &selector.source);
    Check {
        selector: selector.clone(),
        state,
        evidence: candidates,
    }
}
fn status_candidates(selector: &Selector, creator: u64, values: &[Value]) -> Vec<Value> {
    let mut found: Vec<Value> = values
        .iter()
        .filter(|v| v["context"] == selector.name && v["creator"]["id"] == creator)
        .cloned()
        .collect();
    found.sort_by_key(status_id);
    found.into_iter().rev().take(1).collect()
}
fn status_id(value: &Value) -> u64 {
    value["id"].as_u64().unwrap_or(0)
}
fn check_candidates(selector: &Selector, app: u64, values: &[Value]) -> Vec<Value> {
    values
        .iter()
        .filter(|v| v["name"] == selector.name && v["app"]["id"] == app)
        .cloned()
        .collect()
}
fn action_candidates(
    selector: &Selector,
    checks: &[Value],
    runs: &[Value],
    jobs: &[Value],
) -> Vec<Value> {
    let Source::Actions {
        app_id,
        workflow_id,
        event,
        branch,
        ..
    } = &selector.source
    else {
        return Vec::new();
    };
    let matching: Vec<&Value> = runs
        .iter()
        .filter(|v| {
            v["workflow_id"] == *workflow_id && v["event"] == *event && v["head_branch"] == *branch
        })
        .collect();
    let current = matching.iter().copied().max_by_key(run_number);
    let Some(run) = current else {
        return Vec::new();
    };
    if matching
        .iter()
        .filter(|other| other["run_number"] == run["run_number"])
        .count()
        != 1
    {
        return Vec::from([Value::Null, Value::Null]);
    }
    let mut found = Vec::new();
    for check in check_candidates(selector, *app_id, checks) {
        if check["check_suite"]["id"] == run["check_suite_id"]
            && jobs.iter().any(|job| job_matches(job, &check, run))
        {
            let mut proof = check;
            proof["workflow_run"] = run.clone();
            found.push(proof);
        }
    }
    found
}
fn run_number(value: &&Value) -> u64 {
    value["run_number"].as_u64().unwrap_or(0)
}
fn job_matches(job: &Value, check: &Value, run: &Value) -> bool {
    let suffix = format!("/check-runs/{}", check["id"]);
    job["run_id"] == run["id"]
        && job["run_attempt"] == run["run_attempt"]
        && job["check_run_url"]
            .as_str()
            .is_some_and(|url| url.ends_with(&suffix))
}
fn candidate_state(values: &[Value], source: &Source) -> CheckState {
    if values.is_empty() {
        return CheckState::Missing;
    }
    if values.len() != 1 {
        return CheckState::Ambiguous;
    }
    let value = &values[0];
    if value["id"].as_u64().unwrap_or(0) == 0 {
        return CheckState::Ambiguous;
    }
    if matches!(source, Source::Status { .. }) {
        return conclusion(value["state"].as_str());
    }
    if value["status"] != "completed" {
        return CheckState::Pending;
    }
    conclusion(value["conclusion"].as_str())
}
fn conclusion(value: Option<&str>) -> CheckState {
    match value {
        Some("success") => CheckState::Success,
        Some("pending" | "queued" | "in_progress") | None => CheckState::Pending,
        _ => CheckState::Failure,
    }
}
