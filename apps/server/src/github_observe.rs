//! Read-only repository preflight and coherent PR snapshots.
use crate::{
    github::{self, Capability, CheckState, MergeFact, Observation, Policy, Source},
    github_http::{AppClient, Result, invalid},
};
use serde_json::{Value, json};

pub async fn observe(
    client: &mut AppClient,
    policy: &Policy,
    number: u64,
    now: i64,
) -> Result<Observation> {
    let path = format!("/repos/{}/pulls/{number}", policy.repository);
    let pr = client.get(policy, &path, now).await?;
    validate_pr(&pr, policy, number)?;
    let head = required_text(&pr["head"]["sha"])?;
    let phases = crate::github_delivery::phases(client, policy, &pr, now).await?;
    let checks = selected_checks(client, policy, &head, &pr, now, &phases).await?;
    let final_pr = client.get(policy, &path, now).await?;
    if pr != final_pr {
        return Err(invalid());
    }
    parse_observation(policy, number, now, &pr, head, checks, phases)
}
fn validate_pr(pr: &Value, policy: &Policy, number: u64) -> Result<()> {
    if pr["base"]["repo"]["id"] != policy.repository_id
        || pr["number"] != number
        || pr["base"]["ref"] != policy.default_branch
    {
        return Err(invalid());
    }
    Ok(())
}
async fn selected_checks(
    client: &mut AppClient,
    policy: &Policy,
    head: &str,
    pr: &Value,
    now: i64,
    phases: &[crate::github_contract::PhaseEvidence],
) -> Result<Vec<github::Check>> {
    if let Some(phase) = phases.first() {
        return Ok(phase.checks.clone());
    }
    collect_checks(client, policy, head, pr, now).await
}
fn parse_observation(
    policy: &Policy,
    number: u64,
    now: i64,
    pr: &Value,
    head: String,
    checks: Vec<github::Check>,
    phases: Vec<crate::github_contract::PhaseEvidence>,
) -> Result<Observation> {
    let merge = github::merge_fact(pr);
    let associated_sha = pr["merge_commit_sha"]
        .as_str()
        .filter(nonempty)
        .map(str::to_owned);
    let merged_sha = if merge == MergeFact::Merged {
        associated_sha.clone()
    } else {
        None
    };
    Ok(Observation {
        policy: policy.clone(),
        phases: policy.delivery.as_ref().and(Some(phases)),
        actual_checkout_sha: None,
        repository_id: policy.repository_id,
        number,
        head,
        base: required_text(&pr["base"]["sha"])?,
        head_ref: required_text(&pr["head"]["ref"])?,
        base_ref: required_text(&pr["base"]["ref"])?,
        test_merge_sha: if merge == MergeFact::Merged {
            None
        } else {
            associated_sha
        },
        merged_sha,
        merge,
        closed: pr["state"] == "closed",
        checks,
        last_synced_at: now,
    })
}
fn nonempty(value: &&str) -> bool {
    !value.is_empty()
}
fn required_text(value: &Value) -> Result<String> {
    value
        .as_str()
        .filter(nonempty)
        .map(str::to_owned)
        .ok_or_else(invalid)
}
pub(crate) async fn collect_checks(
    client: &mut AppClient,
    policy: &Policy,
    head: &str,
    pr: &Value,
    now: i64,
) -> Result<Vec<github::Check>> {
    let repo = format!("/repos/{}", policy.repository);
    let suites = client
        .pages(
            policy,
            &format!("{repo}/commits/{head}/check-suites"),
            Some("check_suites"),
            now,
        )
        .await?;
    let mut checks = Vec::new();
    for suite in suites {
        let id = suite["id"].as_u64().ok_or_else(invalid)?;
        checks.extend(
            client
                .pages(
                    policy,
                    &format!("{repo}/check-suites/{id}/check-runs?filter=all"),
                    Some("check_runs"),
                    now,
                )
                .await?,
        );
    }
    if checks.iter().any(|check| check["head_sha"] != head) {
        return Err(invalid());
    }
    // Private repositories may grant Checks/Actions without legacy Statuses.
    // Only consult this independent source when the policy actually selects it.
    let statuses = if policy.required.iter().any(is_status) {
        client
            .pages(
                policy,
                &format!("{repo}/commits/{head}/statuses"),
                None,
                now,
            )
            .await?
    } else {
        Vec::new()
    };
    let (runs, jobs) = actions(client, policy, head, now).await?;
    policy
        .required
        .iter()
        .map(|selector| {
            let bound =
                github::bind_pr_source(selector, pr, policy.repository_id).ok_or_else(invalid)?;
            let mut check = github::resolve(&bound, &checks, &statuses, &runs, &jobs);
            check.selector = selector.clone();
            Ok(check)
        })
        .collect()
}
async fn actions(
    client: &mut AppClient,
    policy: &Policy,
    head: &str,
    now: i64,
) -> Result<(Vec<Value>, Vec<Value>)> {
    if !policy.required.iter().any(is_actions) {
        return Ok((Vec::new(), Vec::new()));
    }
    let repo = format!("/repos/{}", policy.repository);
    let runs = client
        .pages(
            policy,
            &format!("{repo}/actions/runs?head_sha={head}"),
            Some("workflow_runs"),
            now,
        )
        .await?;
    let mut jobs = Vec::new();
    for run in &runs {
        if run["head_sha"] != head {
            return Err(invalid());
        }
        let id = run["id"].as_u64().ok_or_else(invalid)?;
        let attempt = run["run_attempt"].as_u64().ok_or_else(invalid)?;
        jobs.extend(
            client
                .pages(
                    policy,
                    &format!("{repo}/actions/runs/{id}/attempts/{attempt}/jobs"),
                    Some("jobs"),
                    now,
                )
                .await?,
        );
    }
    Ok((runs, jobs))
}
fn is_status(selector: &github::Selector) -> bool {
    matches!(selector.source, Source::Status { .. })
}
fn is_actions(selector: &github::Selector) -> bool {
    matches!(selector.source, Source::Actions { .. })
}

pub async fn preflight(
    client: &mut AppClient,
    policy: &Policy,
    probe_pr: u64,
    now: i64,
) -> Result<Capability> {
    if policy.delivery.is_some() {
        return crate::github_delivery::preflight(client, policy, probe_pr, now).await;
    }
    let repo = repository(client, policy, now).await?;
    let permissions = client.permissions(policy, now).await?;
    let mut blockers = permission_blockers(&permissions);
    if repo["default_branch"] != policy.default_branch || repo["archived"] != false {
        blockers.push("repository/default branch unavailable or changed".into());
    }
    let observation = observe(client, policy, probe_pr, now).await?;
    let workflows =
        workflow_configuration(client, policy, &observation.head, now, &mut blockers).await?;
    let (branch, rules) = repository_rules(client, policy, now, &mut blockers).await?;
    source_blockers(&observation, &mut blockers);
    Ok(Capability {
        policy: policy.clone(),
        checked_at: now,
        blockers,
        permissions,
        configuration: json!({"repository":repo,"workflows":workflows,"branch":branch,"rules":rules,"probe":observation}),
    })
}
fn permission_blockers(permissions: &Value) -> Vec<String> {
    let mut blockers = Vec::new();
    for (name, required) in [
        ("contents", "write"),
        ("pull_requests", "write"),
        ("checks", "read"),
        ("actions", "read"),
    ] {
        let actual = permissions[name].as_str().unwrap_or("");
        if actual != "write" && actual != required {
            blockers.push(format!("permission: {name}:{required}"));
        }
    }
    blockers
}
pub(crate) fn segment(value: &str) -> String {
    value.bytes().map(|byte| format!("%{byte:02X}")).collect()
}
async fn workflow_configuration(
    client: &mut AppClient,
    policy: &Policy,
    head: &str,
    now: i64,
    blockers: &mut Vec<String>,
) -> Result<Vec<Value>> {
    let mut configuration = Vec::new();
    for selector in &policy.required {
        if let Source::Actions {
            workflow_id,
            workflow_sha,
            event,
            ..
        } = &selector.source
        {
            let path = format!(
                "/repos/{}/actions/workflows/{workflow_id}",
                policy.repository
            );
            let workflow = client.get(policy, &path, now).await?;
            let file = required_text(&workflow["path"])?;
            let content = client
                .get(
                    policy,
                    &format!(
                        "/repos/{}/contents/{}?ref={}",
                        policy.repository,
                        segment(&file),
                        segment(&policy.default_branch)
                    ),
                    now,
                )
                .await?;
            let head_content = client
                .get(
                    policy,
                    &format!(
                        "/repos/{}/contents/{}?ref={}",
                        policy.repository,
                        segment(&file),
                        segment(head)
                    ),
                    now,
                )
                .await?;
            if workflow_changed(&workflow, &content, &head_content, workflow_sha, event) {
                blockers.push(format!(
                    "workflow trigger/configuration changed: {workflow_id}"
                ));
            }
            configuration.push(json!({"workflow":workflow,"blob_sha":content["sha"]}));
        }
    }
    Ok(configuration)
}
pub(crate) fn check_rules(policy: &Policy, rules: &[Value], blockers: &mut Vec<String>) {
    for rule in rules {
        if rule["type"] != "required_status_checks" {
            continue;
        }
        let Some(checks) = rule["parameters"]["required_status_checks"].as_array() else {
            blockers.push("invalid repository check rules".into());
            continue;
        };
        for check in checks {
            if !policy
                .required
                .iter()
                .any(|selector| rule_matches(selector, check))
            {
                blockers.push(format!("unmapped required check: {}", check["context"]));
            }
        }
    }
}
fn rule_matches(selector: &github::Selector, check: &Value) -> bool {
    if check["context"] != selector.name {
        return false;
    }
    let integration = check
        .get("integration_id")
        .or_else(|| check.get("app_id"))
        .unwrap_or(&Value::Null);
    match selector.source {
        Source::Status { .. } => integration.is_null(),
        Source::CheckRun { app_id } | Source::Actions { app_id, .. } => {
            integration.is_null() || *integration == app_id
        }
    }
}

pub(crate) fn branch_rule(branch: &Value) -> Value {
    let required = &branch["protection"]["required_status_checks"];
    let checks = match required["checks"].as_array() {
        Some(checks) => checks.clone(),
        None => required["contexts"]
            .as_array()
            .unwrap_or(&Vec::new())
            .iter()
            .map(context_rule)
            .collect(),
    };
    json!({"type":"required_status_checks","parameters":{"required_status_checks":checks}})
}
fn context_rule(context: &Value) -> Value {
    json!({"context":context,"integration_id":null})
}

async fn repository_rules(
    client: &mut AppClient,
    policy: &Policy,
    now: i64,
    blockers: &mut Vec<String>,
) -> Result<(Value, Vec<Value>)> {
    let branch = client
        .get(
            policy,
            &format!(
                "/repos/{}/branches/{}",
                policy.repository,
                segment(&policy.default_branch)
            ),
            now,
        )
        .await?;
    check_rules(policy, &[branch_rule(&branch)], blockers);
    let rules = client
        .pages(
            policy,
            &format!(
                "/repos/{}/rules/branches/{}",
                policy.repository,
                segment(&policy.default_branch)
            ),
            None,
            now,
        )
        .await?;
    check_rules(policy, &rules, blockers);
    Ok((branch, rules))
}

fn source_blockers(observation: &Observation, blockers: &mut Vec<String>) {
    for check in &observation.checks {
        if !matches!(check.selector.source, Source::Actions { .. }) {
            blockers.push(format!(
                "external trigger configuration unverified: {}",
                check.selector.name
            ));
        }
        if matches!(check.state, CheckState::Missing | CheckState::Ambiguous) {
            blockers.push(format!(
                "missing/unresolved trusted source: {}",
                check.selector.name
            ));
        }
    }
}

pub(crate) async fn repository(client: &mut AppClient, policy: &Policy, now: i64) -> Result<Value> {
    if !github::validate_policy(policy) {
        return Err(invalid());
    }
    let repo = client
        .get(policy, &format!("/repos/{}", policy.repository), now)
        .await?;
    if repo["id"] != policy.repository_id || repo["full_name"] != policy.repository {
        return Err(invalid());
    }
    Ok(repo)
}

fn workflow_changed(
    workflow: &Value,
    content: &Value,
    head_content: &Value,
    expected: &str,
    event: &str,
) -> bool {
    workflow["state"] != "active"
        || content["sha"] != expected
        || head_content["sha"] != expected
        || event != "pull_request"
}
