//! V1 adapters reuse the same App client, selectors, observation and store.
use crate::{
    github::{self, Capability, CheckState, MergeFact, Policy, Source},
    github_contract::{PhaseEvidence, PostMerge, PreMergeSource, RequiredCheck, Trigger},
    github_http::{AppClient, Result, invalid},
    github_observe::{self, segment},
};
use serde_json::{Value, json};

pub async fn phases(
    client: &mut AppClient,
    policy: &Policy,
    pr: &Value,
    now: i64,
) -> Result<Vec<PhaseEvidence>> {
    let Some(contract) = &policy.delivery else {
        return Ok(Vec::new());
    };
    let source = selected_sha(pr, &contract.pre_merge.source);
    let mut result = Vec::from([phase(
        client,
        policy,
        pr,
        now,
        PhaseRequest {
            name: "pre_merge",
            sha: source,
            checks: &contract.pre_merge.checks,
            wait: contract.pre_merge.wait_seconds,
        },
    )
    .await?]);
    result[0].expected_checkout_sha =
        selected_sha(pr, &contract.pre_merge.checkout).map(str::to_owned);
    if github::merge_fact(pr) != MergeFact::Merged {
        return Ok(result);
    }
    result.push(post_phase(client, policy, pr, now, &contract.post_merge).await?);
    Ok(result)
}
fn selected_sha<'a>(pr: &'a Value, source: &PreMergeSource) -> Option<&'a str> {
    match source {
        PreMergeSource::Head => pr["head"]["sha"].as_str(),
        PreMergeSource::TestMerge if github::merge_fact(pr) != MergeFact::Merged => {
            pr["merge_commit_sha"].as_str()
        }
        _ => None,
    }
}
async fn post_phase(
    client: &mut AppClient,
    policy: &Policy,
    pr: &Value,
    now: i64,
    post: &PostMerge,
) -> Result<PhaseEvidence> {
    let sha = pr["merge_commit_sha"].as_str().filter(nonempty);
    match post {
        PostMerge::Checks {
            checks,
            wait_seconds,
            ..
        } => {
            phase(
                client,
                policy,
                pr,
                now,
                PhaseRequest {
                    name: "post_merge",
                    sha,
                    checks,
                    wait: *wait_seconds,
                },
            )
            .await
        }
        PostMerge::FixedValidation { wait_seconds, .. } => Ok(PhaseEvidence {
            phase: "post_merge".into(),
            head_sha: text(pr, "head")?,
            base_sha: text(pr, "base")?,
            check_sha: sha.map(str::to_owned),
            actual_checkout_sha: None,
            expected_checkout_sha: sha.map(str::to_owned),
            validation: None,
            checks: Vec::new(),
            blockers: Vec::new(),
            wait_seconds: *wait_seconds,
        }),
    }
}

fn text(pr: &Value, side: &str) -> Result<String> {
    pr[side]["sha"]
        .as_str()
        .filter(nonempty)
        .map(str::to_owned)
        .ok_or_else(invalid)
}
struct PhaseRequest<'a> {
    name: &'a str,
    sha: Option<&'a str>,
    checks: &'a [RequiredCheck],
    wait: u64,
}
async fn phase(
    client: &mut AppClient,
    policy: &Policy,
    pr: &Value,
    now: i64,
    request: PhaseRequest<'_>,
) -> Result<PhaseEvidence> {
    let mut selected = policy.clone();
    selected.required = request.checks.iter().map(|c| c.selector.clone()).collect();
    let mut checks = match request.sha {
        Some(sha) if !sha.is_empty() => {
            github_observe::collect_checks(client, &selected, sha, pr, now).await?
        }
        _ => Vec::new(),
    };
    job_constraints(&mut checks, request.checks);
    if let Some(sha) = request.sha {
        bind_configuration(client, policy, sha, request.checks, &mut checks, now).await?;
    }
    Ok(PhaseEvidence {
        phase: request.name.into(),
        head_sha: text(pr, "head")?,
        base_sha: text(pr, "base")?,
        check_sha: request.sha.map(str::to_owned),
        actual_checkout_sha: None,
        expected_checkout_sha: request.sha.map(str::to_owned),
        validation: None,
        checks,
        blockers: Vec::new(),
        wait_seconds: request.wait,
    })
}

fn job_constraints(checks: &mut [github::Check], required: &[RequiredCheck]) {
    for (check, required) in checks.iter_mut().zip(required) {
        if let Some(job) = &required.job {
            for proof in &check.evidence {
                if proof["workflow_job"]["name"] != *job {
                    check.state = CheckState::Ambiguous;
                }
            }
        }
    }
}

pub async fn preflight(
    client: &mut AppClient,
    policy: &Policy,
    probe_pr: u64,
    now: i64,
) -> Result<Capability> {
    let contract = policy.delivery.as_ref().ok_or_else(invalid)?;
    let (repo, permissions) = access(client, policy, now).await?;
    let mut blockers = contract.blockers(policy);
    let expected = crate::github_contract::permissions(policy);
    access_blockers(policy, &repo, &permissions, &expected, &mut blockers);
    let (protection, rules) = protection(client, policy, now, &mut blockers).await?;
    let observation = github_observe::observe(client, policy, probe_pr, now).await?;
    let post_probe = post_probe(client, policy, now, &mut blockers).await?;
    merge_blockers(contract, &repo, &mut blockers);
    let logs = required_logs(
        client,
        policy,
        &observation,
        post_probe.as_ref(),
        now,
        &mut blockers,
    )
    .await;
    let configurations = configurations(client, policy, &observation, now, &mut blockers).await?;
    Ok(Capability {
        policy: policy.clone(),
        checked_at: now,
        blockers,
        permissions,
        configuration: json!({"repository":repo,"protection":protection,"rules":rules,"triggers":configurations,"probe":observation,"required_permissions":expected,"post_merge_probe":post_probe,"logs":logs}),
    })
}
async fn configuration(
    client: &mut AppClient,
    policy: &Policy,
    check: &RequiredCheck,
    head: &str,
    now: i64,
    blockers: &mut Vec<String>,
) -> Result<Value> {
    let prefix = format!("/repos/{}", policy.repository);
    let (path, sha, workflow) = configured_file(client, policy, check, now, blockers).await?;
    if path.is_empty() {
        return Ok(Value::Null);
    }
    let base = client
        .get(
            policy,
            &format!(
                "{prefix}/contents/{}?ref={}",
                segment(&path),
                segment(&policy.default_branch)
            ),
            now,
        )
        .await?;
    let candidate = client
        .get(
            policy,
            &format!("{prefix}/contents/{}?ref={}", segment(&path), segment(head)),
            now,
        )
        .await?;
    if base["sha"] != sha || candidate["sha"] != sha {
        blockers.push(format!("trigger configuration changed: {path}"));
    }
    Ok(
        json!({"selector":check,"workflow":workflow,"path":path,"base_blob":base["sha"],"head_blob":candidate["sha"]}),
    )
}

fn fixed_plan(path: &str) -> Option<crate::validation::TrustedIdentity> {
    if !std::path::Path::new(path).is_absolute() {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let plan: crate::validation_runner::Plan = serde_json::from_slice(&bytes).ok()?;
    plan.identity().ok()
}

async fn post_probe(
    client: &mut AppClient,
    policy: &Policy,
    now: i64,
    blockers: &mut Vec<String>,
) -> Result<Option<github::Observation>> {
    Ok(
        match &policy.delivery.as_ref().ok_or_else(invalid)?.post_merge {
            PostMerge::Checks { probe_pr, .. } => {
                let probe = github_observe::observe(client, policy, *probe_pr, now).await?;
                if probe.merge != MergeFact::Merged || probe.merged_sha.is_none() {
                    blockers.push(
                        "delivery.post_merge: probe must have confirmed merged identity".into(),
                    );
                }
                let post = probe
                    .phases
                    .as_ref()
                    .and_then(|phases| phases.iter().find(|p| p.phase == "post_merge"));
                if post.is_none_or(|p| p.checks.is_empty() || p.checks.iter().any(unresolved)) {
                    blockers.push(
                    "delivery.post_merge: trusted source/trigger not demonstrated on merged SHA"
                        .into(),
                );
                }
                Some(probe)
            }
            PostMerge::FixedValidation {
                plan_id,
                configuration_sha256,
                ..
            } => {
                if fixed_plan(plan_id)
                    .is_none_or(|identity| identity.config_sha256 != *configuration_sha256)
                {
                    blockers.push(
                        "delivery.post_merge: fixed validation plan unavailable or changed".into(),
                    );
                }
                None
            }
        },
    )
}

fn merge_blockers(
    contract: &crate::github_contract::DeliveryContract,
    repo: &Value,
    blockers: &mut Vec<String>,
) {
    if contract.actions.merge {
        let option = match contract.actions.merge_method.as_deref() {
            Some("merge") => "allow_merge_commit",
            Some("squash") => "allow_squash_merge",
            Some("rebase") => "allow_rebase_merge",
            _ => "unknown",
        };
        if repo[option] != true {
            blockers.push("delivery.merge: selected merge method unavailable".into());
        }
    }
}

async fn log_blockers(
    client: &mut AppClient,
    policy: &Policy,
    checks: &[github::Check],
    now: i64,
    blockers: &mut Vec<String>,
) -> Vec<Value> {
    let mut proofs = Vec::new();
    for check in checks {
        if !matches!(check.selector.source, Source::Actions { .. }) {
            blockers.push("delivery.logs: external publisher log adapter unavailable".into());
        }
        for proof in &check.evidence {
            if let Some(job) = proof["workflow_job"]["id"].as_u64() {
                match client.logs_readable(policy, job, now).await {
                    Ok(proof) => proofs.push(proof),
                    Err(_) => blockers.push(format!(
                        "delivery.logs: required job logs inaccessible: {job}"
                    )),
                }
            } else {
                blockers.push("delivery.logs: job identity missing".into());
            }
        }
    }
    proofs
}

fn access_blockers(
    policy: &Policy,
    repo: &Value,
    permissions: &Value,
    expected: &Value,
    blockers: &mut Vec<String>,
) {
    for (name, required) in expected.as_object().into_iter().flatten() {
        if permissions[name] != "write" && permissions[name] != *required {
            blockers.push(format!("permission: {name}:{required}"));
        }
    }
    if repo["default_branch"] != policy.default_branch || repo["archived"] != false {
        blockers.push("repository/default branch unavailable or changed".into());
    }
}

async fn protection(
    client: &mut AppClient,
    policy: &Policy,
    now: i64,
    blockers: &mut Vec<String>,
) -> Result<(Value, Vec<Value>)> {
    let contract = policy.delivery.as_ref().ok_or_else(invalid)?;
    let prefix = format!("/repos/{}", policy.repository);
    let branch = segment(&policy.default_branch);
    let protection = client
        .get(
            policy,
            &format!("{prefix}/branches/{branch}/protection"),
            now,
        )
        .await?;
    let rules = client
        .pages(
            policy,
            &format!("{prefix}/rules/branches/{branch}"),
            None,
            now,
        )
        .await?;
    if protection != contract.protection || rules != contract.rules {
        blockers.push("delivery.protection: current rules differ from reviewed policy".into());
    }
    github_observe::check_rules(policy, &rules, blockers);
    github_observe::check_rules(
        policy,
        &[github_observe::branch_rule(
            &json!({"protection":protection}),
        )],
        blockers,
    );
    Ok((protection, rules))
}

async fn configurations(
    client: &mut AppClient,
    policy: &Policy,
    observation: &github::Observation,
    now: i64,
    blockers: &mut Vec<String>,
) -> Result<Vec<Value>> {
    let mut configurations = Vec::new();
    for check in policy.delivery.as_ref().ok_or_else(invalid)?.all_checks() {
        configurations
            .push(configuration(client, policy, check, &observation.head, now, blockers).await?);
    }
    for check in &observation.checks {
        if matches!(check.state, CheckState::Missing | CheckState::Ambiguous) {
            blockers.push(format!(
                "missing/unresolved trusted source: {}",
                check.selector.name
            ));
        }
    }
    Ok(configurations)
}

async fn configured_file(
    client: &mut AppClient,
    policy: &Policy,
    check: &RequiredCheck,
    now: i64,
    blockers: &mut Vec<String>,
) -> Result<(String, String, Value)> {
    let prefix = format!("/repos/{}", policy.repository);
    Ok(match &check.selector.source {
        Source::Actions {
            workflow_id,
            workflow_sha,
            ..
        } => {
            let workflow = client
                .get(
                    policy,
                    &format!("{prefix}/actions/workflows/{workflow_id}"),
                    now,
                )
                .await?;
            if workflow["state"] != "active" {
                blockers.push(format!("inactive workflow: {workflow_id}"));
            }
            let path = workflow["path"].as_str().ok_or_else(invalid)?.to_owned();
            (path, workflow_sha.clone(), workflow)
        }
        _ => match &check.trigger {
            Trigger::External { path, blob_sha, .. } => {
                (path.clone(), blob_sha.clone(), Value::Null)
            }
            _ => {
                blockers.push("external trigger configuration missing".into());
                return Ok((String::new(), String::new(), Value::Null));
            }
        },
    })
}

async fn access(client: &mut AppClient, policy: &Policy, now: i64) -> Result<(Value, Value)> {
    let permissions = client.permissions(policy, now).await?;
    let repo = github_observe::repository(client, policy, now).await?;
    Ok((repo, permissions))
}

async fn required_logs(
    client: &mut AppClient,
    policy: &Policy,
    pre: &github::Observation,
    post: Option<&github::Observation>,
    now: i64,
    blockers: &mut Vec<String>,
) -> Vec<Value> {
    if !policy.delivery.as_ref().is_some_and(read_logs) {
        return Vec::new();
    }
    let mut logs = log_blockers(client, policy, &pre.checks, now, blockers).await;
    if let Some(phases) = post.and_then(|p| p.phases.as_ref()) {
        for phase in phases {
            if phase.phase == "post_merge" {
                logs.extend(log_blockers(client, policy, &phase.checks, now, blockers).await);
            }
        }
    }
    logs
}

async fn bind_configuration(
    client: &mut AppClient,
    policy: &Policy,
    sha: &str,
    required: &[RequiredCheck],
    checks: &mut [github::Check],
    now: i64,
) -> Result<()> {
    for (required, check) in required.iter().zip(checks) {
        let mut blockers = Vec::new();
        let proof = configuration(client, policy, required, sha, now, &mut blockers).await?;
        if !blockers.is_empty() {
            check.state = CheckState::Ambiguous;
        }
        for selected in &mut check.evidence {
            if selected.is_object() {
                selected["configuration"] = proof.clone();
            }
        }
    }
    Ok(())
}

fn unresolved(check: &github::Check) -> bool {
    matches!(check.state, CheckState::Missing | CheckState::Ambiguous)
}

fn nonempty(value: &&str) -> bool {
    !value.is_empty()
}

fn read_logs(contract: &crate::github_contract::DeliveryContract) -> bool {
    contract.actions.read_logs
}
