//! Read-only environment probes use the existing descendant-reaping supervisor.
use crate::{
    controlled_contract::{Call, CheckResult, Evaluation, Operation, ResourceCall, Verdict},
    environment::{Facts, Plan},
    environment_host::{Profile, Registry, Result},
    execution::{Launch, Receipt, RunKey},
    process,
};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub resource: ResourceCall,
    pub call: Option<Call>,
    pub invocation_id: String,
    pub stage: String,
    pub role: String,
    pub plan_digest: String,
    pub host_profile: String,
    pub resource_root: PathBuf,
    pub workspace: Option<PathBuf>,
    pub required_checks: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub request: Request,
    pub actual: Facts,
    pub checks: Vec<CheckResult>,
    pub evaluation: Option<Evaluation>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Report {
    pub request: Request,
    pub elapsed_ms: u64,
    pub response: Option<Response>,
    pub actual_digest: Option<String>,
    pub differences: Vec<crate::environment::Difference>,
    pub error: Option<String>,
    pub evidence: PathBuf,
}
impl Report {
    pub fn passed(&self) -> bool {
        self.error.is_none() && self.differences.is_empty() && self.response.is_some()
    }
}

pub async fn check(
    registry: &Registry,
    plan: &Plan,
    stage: &str,
    role: &str,
    workspace: Option<&Path>,
) -> Result<Report> {
    check_bound(registry, plan, stage, role, workspace, None).await
}

pub struct TaskContext {
    pub requirement: i64,
    pub revision: i64,
    pub frozen: crate::extension_contract::FrozenConfig,
    pub policy_digest: String,
    pub deadline_unix_ms: Option<i64>,
}

pub async fn check_bound(
    registry: &Registry,
    plan: &Plan,
    stage: &str,
    role: &str,
    workspace: Option<&Path>,
    context: Option<&TaskContext>,
) -> Result<Report> {
    reconcile(&registry.evidence_root)?;
    let profile = registry.resolve(plan)?;
    verify_workspace(profile, workspace)?;
    crate::environment_cache::check(registry, profile, plan.cache.as_ref())?;
    let request = request(plan, profile, stage, role, workspace, context)?;
    let directory = prepare_input(registry, &request)?;
    observe(plan, profile, request, directory).await
}

fn verify_workspace(profile: &Profile, workspace: Option<&Path>) -> Result<()> {
    if let Some(workspace) = workspace {
        let root = std::fs::canonicalize(workspace)?;
        if std::fs::canonicalize(&profile.executable)?.starts_with(root) {
            return Err("environment extension is inside candidate workspace".into());
        }
    }
    Ok(())
}

fn request(
    plan: &Plan,
    profile: &Profile,
    stage: &str,
    role: &str,
    workspace: Option<&Path>,
    context: Option<&TaskContext>,
) -> Result<Request> {
    let required = plan.roles.get(role).ok_or("undeclared environment role")?;
    let invocation_id = process::new_identity()?;
    let resource = resource_call(plan, profile, &invocation_id, context)?;
    let mut request = Request {
        resource,
        call: None,
        invocation_id,
        stage: stage.into(),
        role: role.into(),
        plan_digest: plan.digest(),
        host_profile: plan.controlled.environment.host_profile_ref.clone(),
        resource_root: profile.resource_root.clone(),
        workspace: workspace.map(Path::to_path_buf),
        required_checks: required.checks.clone(),
    };
    if let Some(context) = context {
        request.call = Some(task_call(&request, plan, profile, context)?);
    }
    Ok(request)
}

fn resource_call(
    plan: &Plan,
    profile: &Profile,
    invocation_id: &str,
    context: Option<&TaskContext>,
) -> Result<ResourceCall> {
    let approved = std::slice::from_ref(&profile.registration);
    let mut resource = ResourceCall {
        protocol_version: 1,
        invocation_id: invocation_id.into(),
        attempt: 1,
        resource_id: plan.controlled.environment.host_profile_ref.clone(),
        controlled_config_digest: plan
            .controlled
            .freeze(approved)
            .map_err(crate::environment::protocol)?,
        environment: plan.controlled.environment.clone(),
        extension_id: plan.extension_id.clone(),
        implementation_digest: profile.registration.implementation_digest.clone(),
        deadline_unix_ms: now_ms()? + (profile.timeout_seconds * 1000) as i64,
    };
    if let Some(context) = context {
        let deadline = context
            .deadline_unix_ms
            .unwrap_or(resource.deadline_unix_ms);
        resource.deadline_unix_ms = resource.deadline_unix_ms.min(deadline);
    }
    resource
        .validate(&plan.controlled, approved)
        .map_err(crate::environment::protocol)?;
    Ok(resource)
}

fn prepare_input(registry: &Registry, request: &Request) -> Result<PathBuf> {
    if serde_json::to_vec(&request)?.len() > crate::extension_contract::MAX_RESULT_BYTES {
        return Err("environment input exceeds 64 KiB".into());
    }
    std::fs::create_dir_all(&registry.evidence_root)?;
    let directory = registry.evidence_root.join(&request.invocation_id);
    std::fs::create_dir(&directory)?;
    process::durable_write(&directory.join("input.json"), &request)?;
    Ok(directory)
}

async fn observe(
    plan: &Plan,
    profile: &Profile,
    request: Request,
    directory: PathBuf,
) -> Result<Report> {
    let started = Instant::now();
    let result = invoke(profile, &directory, &request).await;
    let mut report = Report {
        request,
        elapsed_ms: started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
        response: None,
        actual_digest: None,
        differences: Vec::new(),
        error: None,
        evidence: directory.clone(),
    };
    match result {
        Ok(response) => {
            report.actual_digest = Some(crate::validation::sha256(serde_json::to_vec(
                &response.actual,
            )?));
            report.differences = plan.differences(&report.request.role, &response.actual)?;
            report.response = Some(response);
        }
        Err(error) => report.error = Some(error.to_string()),
    }
    process::durable_write(&directory.join("report.json"), &report)?;
    Ok(report)
}

fn task_call(
    request: &Request,
    plan: &Plan,
    profile: &Profile,
    context: &TaskContext,
) -> Result<Call> {
    let call = Call {
        identity: crate::extension_contract::InvocationIdentity {
            protocol_version: 1,
            requirement_id: context.requirement,
            revision: context.revision,
            // This is the actual infrastructure supervisor Run, not an AgentRun.
            run_id: Some(request.invocation_id.clone()),
            resource_id: request.resource.resource_id.clone(),
            invocation_id: request.invocation_id.clone(),
            attempt: 1,
            config_id: context.frozen.config_id.clone(),
        },
        controlled_config_digest: request.resource.controlled_config_digest.clone(),
        operation: Operation::EnvironmentCheck,
        extension_id: plan.extension_id.clone(),
        implementation_digest: profile.registration.implementation_digest.clone(),
        candidate: None,
        environment_digest: plan.contract_digest(),
        policy_digest: crate::validation::sha256(serde_json::to_vec(&(
            &context.policy_digest,
            &request.stage,
            &request.role,
            &request.workspace,
            &request.resource_root,
        ))?),
        deadline_unix_ms: request.resource.deadline_unix_ms,
        required_checks: request.required_checks.clone(),
    };
    call.validate(
        &context.frozen,
        &plan.controlled,
        std::slice::from_ref(&profile.registration),
    )
    .map_err(crate::environment::protocol)?;
    Ok(call)
}

fn now_ms() -> Result<i64> {
    unix_ms(std::time::SystemTime::now())
}

fn unix_ms(time: std::time::SystemTime) -> Result<i64> {
    Ok(time
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis()
        .try_into()?)
}

fn reconcile(root: &Path) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(root)? {
        let directory = entry?.path();
        if !directory.join("input.json").exists() {
            continue;
        }
        if never_started(&directory)? {
            continue;
        }
        reconcile_directory(&directory)?;
    }
    Ok(())
}

fn reconcile_directory(directory: &Path) -> Result<()> {
    let started = process::read::<Receipt>(&directory.join("identity.json"));
    let stopped = process::read::<Receipt>(&directory.join("quiescent.json"));
    if !matches!((started, stopped), (Ok(a), Ok(b)) if a == b) {
        return Err(format!(
            "environment probe outcome unknown; reconcile {} before another invocation",
            directory.display()
        )
        .into());
    }
    Ok(())
}

fn never_started(directory: &Path) -> Result<bool> {
    let Ok(proof) = process::read::<Request>(&directory.join("not-started.json")) else {
        return Ok(false);
    };
    let input: Request = process::read(&directory.join("input.json"))?;
    Ok(proof == input && !directory.join("launched").try_exists()?)
}

async fn invoke(profile: &Profile, directory: &Path, request: &Request) -> Result<Response> {
    let key = RunKey {
        run_id: request.invocation_id.clone(),
        request_id: request.invocation_id.clone(),
        incarnation: request.invocation_id.clone(),
    };
    let (mut child, deadline) = match spawn_probe(profile, directory, request, &key) {
        Ok(started) => started,
        Err(error) => {
            process::durable_write(&directory.join("not-started.json"), request)?;
            return Err(error);
        }
    };
    let timed_out = wait(directory, &key, deadline).await?;
    let _ = child.wait();
    if timed_out {
        return Err("environment timeout; descendants stopped".into());
    }
    read_response(profile, directory, request)
}

fn read_response(profile: &Profile, directory: &Path, request: &Request) -> Result<Response> {
    profile.verify_installation()?;
    let exit: serde_json::Value = process::read(&directory.join("exit.json"))?;
    if exit != 0 || directory.join("truncated.json").exists() {
        return Err("environment probe failed or output truncated".into());
    }
    let response: Response = process::read(&directory.join("stdout.json"))?;
    verify_response(request, &response)?;
    process::durable_write(&directory.join("actual.json"), &response.actual)?;
    Ok(response)
}

fn spawn_probe(
    profile: &Profile,
    directory: &Path,
    request: &Request,
    key: &RunKey,
) -> Result<(std::process::Child, Instant)> {
    let remaining = request.resource.deadline_unix_ms.saturating_sub(now_ms()?);
    if remaining <= 0 {
        return Err("environment deadline exhausted before launch".into());
    }
    profile.verify_installation()?;
    let launch = Launch {
        key: key.clone(),
        workspace: directory.to_string_lossy().into_owned(),
        workspace_identity: request.plan_digest.clone(),
        program: profile.executable.to_string_lossy().into_owned(),
        args: Vec::new(),
    };
    process::durable_write(&directory.join("hook.json"), request)?;
    process::durable_write(&directory.join("hook-limit.json"), &1_048_576_u64)?;
    let supervisor = std::env::var_os("SYMPHONY_SUPERVISOR")
        .map(PathBuf::from)
        .unwrap_or(std::env::current_exe()?);
    let child = process::spawn(&supervisor, directory, &launch)?;
    let deadline = Instant::now() + Duration::from_millis(remaining as u64);
    Ok((child, deadline))
}

async fn wait(directory: &Path, key: &RunKey, deadline: Instant) -> Result<bool> {
    let mut started = false;
    let mut timed_out = false;
    loop {
        if stopped(directory, key, &mut started)? {
            return Ok(timed_out);
        }
        if Instant::now() >= deadline {
            timed_out = true;
            process::durable_write(&directory.join("stop.json"), key)?;
        }
        if Instant::now() >= deadline + Duration::from_secs(20) {
            return Err("environment stop unknown; reconcile supervisor before resuming".into());
        }
        process::durable_write(&directory.join("storage-heartbeat.json"), key)?;
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn stopped(directory: &Path, key: &RunKey, started: &mut bool) -> Result<bool> {
    let Ok(identity) = process::read::<Receipt>(&directory.join("identity.json")) else {
        return Ok(false);
    };
    if identity.key != *key {
        return Err("environment process identity mismatch".into());
    }
    if !*started {
        process::durable_write(&directory.join("start.json"), key)?;
        *started = true;
    }
    let Ok(stopped) = process::read::<Receipt>(&directory.join("quiescent.json")) else {
        return Ok(false);
    };
    if stopped != identity {
        return Err("environment stop identity mismatch".into());
    }
    Ok(true)
}

pub fn verify_response(request: &Request, response: &Response) -> Result<()> {
    if response.request != *request {
        return Err("environment response identity mismatch".into());
    }
    if now_ms()? >= request.resource.deadline_unix_ms {
        return Err("environment result expired".into());
    }
    verify_evaluation(request, response)?;
    verify_checks(request, response)
}

fn verify_evaluation(request: &Request, response: &Response) -> Result<()> {
    match (&request.call, &response.evaluation) {
        (Some(call), Some(evaluation)) => {
            evaluation
                .check_pass(call, now_ms()?)
                .map_err(crate::environment::protocol)?;
            if evaluation.checks != response.checks {
                return Err("environment evaluation checks differ".into());
            }
        }
        (None, None) => (),
        _ => return Err("environment task evaluation missing or unexpected".into()),
    }
    Ok(())
}

fn verify_checks(request: &Request, response: &Response) -> Result<()> {
    let facts_digest = crate::validation::sha256(serde_json::to_vec(&response.actual)?);
    let mut seen = std::collections::BTreeSet::new();
    for check in &response.checks {
        if !seen.insert(&check.id) || check.verdict != Verdict::Pass {
            return Err("environment check failed/unknown/duplicate".into());
        }
        if check.evidence.is_empty()
            || check
                .evidence
                .iter()
                .any(|e| e.artifact_id != "actual" || e.sha256 != facts_digest)
        {
            return Err("environment check evidence differs from observed facts".into());
        }
    }
    if request.required_checks.iter().any(|id| !seen.contains(id)) {
        return Err("environment check missing".into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "../tests/unit/environment_probe.rs"]
mod time_tests;
