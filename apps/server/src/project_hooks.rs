//! Reviewed project hooks. The existing preparation ledger owns admission;
//! this module only records and supervises optional project side effects.
use crate::{
    contract::Repository,
    execution::{Launch, RunKey},
    extension_contract::{
        Capabilities, DeliveryMode, ExtensionConfig, FrozenConfig, HookConfig, HookEvent,
        HookInvocation, HookOutcome, HookRole, InvocationIdentity, ModelConfig, PROTOCOL_VERSION,
        ReplayPolicy, parse_hook_result,
    },
    process, run_store,
    workspace::Workspace,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::{
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

struct Registration {
    requirement: i64,
    revision: i64,
    resource: String,
    workspace: String,
    role: HookRole,
    frozen: FrozenConfig,
}

/// The review snapshot, rather than the mutable repository default, owns the
/// selected hook list. The deployment allowlist separately owns executable paths.
pub async fn register(
    pool: &PgPool,
    launch: &Launch,
    workspace: &Workspace,
    role: HookRole,
    allowlist: &Value,
) -> Result<bool> {
    let frozen = frozen_review(pool, workspace, allowlist).await?;
    save_run(pool, launch, workspace, &role, &frozen).await?;
    Ok(frozen
        .value
        .hooks
        .iter()
        .any(|hook| hook.event == HookEvent::BeforeRun && hook.roles.contains(&role)))
}

async fn frozen_review(
    pool: &PgPool,
    workspace: &Workspace,
    allowlist: &Value,
) -> Result<FrozenConfig> {
    let reviewed: Value = sqlx::query_scalar(
        "SELECT document->'repository' FROM execution_revision WHERE requirement_id=$1 AND revision=$2",
    )
    .bind(workspace.requirement)
    .bind(workspace.revision)
    .fetch_one(pool)
    .await?;
    let reviewed_hooks = reviewed
        .get("hooks")
        .is_some_and(|hooks| !hooks.as_array().is_some_and(Vec::is_empty));
    let extension = if reviewed_hooks {
        ExtensionConfig::from_legacy_repository(&serde_json::from_value::<Repository>(reviewed)?)
    } else {
        // Historical review snapshots may contain only the fields used by the
        // old Runtime. Do not make their no-hook path depend on new parsing.
        ExtensionConfig {
            protocol_version: PROTOCOL_VERSION,
            agent: "codex".into(),
            model: ModelConfig {
                provider: "codex".into(),
                model: reviewed
                    .get("model")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                effort: None,
            },
            delivery: DeliveryMode::GithubPr,
            hooks: Vec::new(),
            decision: None,
        }
    };
    let mut capabilities = Capabilities::legacy_codex(extension.model.model.clone());
    if reviewed_hooks {
        capabilities.hooks = serde_json::from_value(
            allowlist
                .get("hook_allowlist")
                .cloned()
                .unwrap_or_else(|| json!([])),
        )?;
    }
    extension
        .freeze(&capabilities)
        .map_err(|error| format!("reviewed hook is not deployed: {error:?}").into())
}

async fn save_run(
    pool: &PgPool,
    launch: &Launch,
    workspace: &Workspace,
    role: &HookRole,
    frozen: &FrozenConfig,
) -> Result<()> {
    let role = serde_json::to_value(role)?;
    let mut tx = run_store::lock(pool).await?;
    sqlx::query("INSERT INTO project_hook_run(run_id,requirement_id,revision,resource_id,workspace,role,frozen) VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT DO NOTHING")
        .bind(&launch.key.run_id)
        .bind(workspace.requirement)
        .bind(workspace.revision)
        .bind(&workspace.identity)
        .bind(&workspace.path)
        .bind(role.as_str().ok_or("invalid hook role")?)
        .bind(json!(frozen))
        .execute(&mut *tx).await?;
    let saved: (i64, i64, String, String, String, Value) = sqlx::query_as("SELECT requirement_id,revision,resource_id,workspace,role,frozen FROM project_hook_run WHERE run_id=$1 FOR UPDATE")
        .bind(&launch.key.run_id).fetch_one(&mut *tx).await?;
    if saved
        != (
            workspace.requirement,
            workspace.revision,
            workspace.identity.clone(),
            workspace.path.clone(),
            role.as_str().unwrap().to_owned(),
            json!(frozen),
        )
    {
        return Err("frozen hook or workspace identity changed".into());
    }
    tx.commit().await?;
    Ok(())
}

pub async fn event(
    pool: &PgPool,
    root: &Path,
    run_id: &str,
    event: HookEvent,
    resource_override: Option<(&str, &Path)>,
) -> Result<bool> {
    let Some(saved) = load_registration(pool, run_id).await? else {
        return Ok(true);
    };
    let (resource, workspace) = resource_override
        .map(|(id, path)| (id.to_owned(), path.to_string_lossy().into_owned()))
        .unwrap_or_else(|| (saved.resource.clone(), saved.workspace.clone()));
    for hook in &saved.frozen.value.hooks {
        if hook.event == event
            && hook.roles.contains(&saved.role)
            && !one(pool, root, run_id, &saved, &resource, &workspace, hook).await?
        {
            return Ok(false);
        }
    }
    Ok(true)
}

async fn load_registration(pool: &PgPool, run_id: &str) -> Result<Option<Registration>> {
    let row: Option<(i64, i64, String, String, String, Value)> = sqlx::query_as(
        "SELECT requirement_id,revision,resource_id,workspace,role,frozen FROM project_hook_run WHERE run_id=$1",
    )
    .bind(run_id)
    .fetch_optional(pool)
    .await?;
    let Some((requirement, revision, resource, workspace, role, frozen)) = row else {
        return Ok(None);
    };
    let frozen: FrozenConfig = serde_json::from_value(frozen)?;
    frozen
        .validate_identity()
        .map_err(|e| format!("invalid frozen hook: {e:?}"))?;
    let role: HookRole = serde_json::from_value(json!(role))?;
    Ok(Some(Registration {
        requirement,
        revision,
        resource,
        workspace,
        role,
        frozen,
    }))
}

/// Auxiliary collection is eligible only after the core has a quiescent Run
/// and a verified preservation snapshot. A hook failure never rewrites either.
pub async fn after_run(pool: &PgPool, root: &Path) -> Result<()> {
    let runs = pending_after_runs(pool).await?;
    if runs.is_empty() {
        return Ok(());
    }
    let broker = crate::git_broker::GitBroker::open(&root.join("workspaces"))?;
    for (run_id, manifest) in runs {
        finish_after_run(pool, root, &broker, &run_id, manifest).await?;
    }
    Ok(())
}

async fn pending_after_runs(pool: &PgPool) -> Result<Vec<(String, Value)>> {
    Ok(sqlx::query_as("SELECT h.run_id,s.manifest FROM project_hook_run h JOIN agent_run a ON a.id=h.run_id JOIN workspace_snapshot s ON s.run_id=a.id WHERE a.quiescent AND EXISTS (SELECT 1 FROM jsonb_array_elements(h.frozen->'value'->'hooks') AS hook WHERE hook->>'event'='after_run' AND hook->'roles' ? h.role AND NOT EXISTS (SELECT 1 FROM project_hook_invocation i WHERE i.run_id=h.run_id AND i.resource_id=h.resource_id AND i.event='after_run' AND i.hook_name=hook->>'name' AND (i.status='success' OR (i.status IN ('failed','timeout','cancelled','unknown') AND (COALESCE(hook->>'replay','never')<>'idempotent' OR i.attempt>=2) AND (i.status<>'unknown' OR i.stop_confirmed))))) ORDER BY a.run_sequence LIMIT 20")
        .fetch_all(pool).await?)
}

async fn finish_after_run(
    pool: &PgPool,
    root: &Path,
    broker: &crate::git_broker::GitBroker,
    run_id: &str,
    manifest: Value,
) -> Result<()> {
    let manifest: crate::workspace::Manifest = serde_json::from_value(manifest)?;
    broker.verify(&manifest)?;
    let result = event(pool, root, run_id, HookEvent::AfterRun, None).await;
    broker.verify(&manifest)?;
    log_auxiliary(run_id, result);
    require_auxiliary_stop(pool, run_id).await
}

fn log_auxiliary(run_id: &str, result: Result<bool>) {
    match result {
        Ok(false) => tracing::warn!(
            "auxiliary after_run failed for {}; preserved candidate retained",
            run_id
        ),
        Err(error) => {
            let detail = crate::operator_view::redact_text(&error.to_string());
            tracing::warn!(
                "auxiliary after_run for {} requires reconciliation: {}",
                run_id,
                detail
            );
        }
        Ok(true) => {}
    }
}

pub(crate) async fn require_auxiliary_stop(pool: &PgPool, run_id: &str) -> Result<()> {
    let paths: Vec<String> = sqlx::query_scalar("SELECT output_dir FROM project_hook_invocation WHERE run_id=$1 AND event='after_run' AND status IN ('intent','running','unknown')")
        .bind(run_id).fetch_all(pool).await?;
    if paths.iter().any(|path| {
        Path::new(path).join("identity.json").exists() && !verified_stop(Path::new(path))
    }) {
        return Err("after_run process stop proof missing".into());
    }
    Ok(())
}

pub async fn before_remove(
    pool: &PgPool,
    root: &Path,
    run_id: &str,
    resource_id: &str,
    path: &Path,
) -> Result<bool> {
    let reviewed_path: Option<String> =
        sqlx::query_scalar("SELECT workspace FROM project_hook_run WHERE run_id=$1")
            .bind(run_id)
            .fetch_optional(pool)
            .await?;
    if reviewed_path.as_deref() != Some(path.to_string_lossy().as_ref()) {
        return Ok(true);
    }
    event(
        pool,
        root,
        run_id,
        HookEvent::BeforeRemove,
        Some((resource_id, path)),
    )
    .await
}

enum StartDecision {
    Done(bool),
    Recover(String, PathBuf),
    Start(String, u32, PathBuf),
}

async fn one(
    pool: &PgPool,
    root: &Path,
    run_id: &str,
    saved: &Registration,
    resource: &str,
    workspace: &str,
    hook: &HookConfig,
) -> Result<bool> {
    crate::plugin_scope::admit(
        pool,
        &format!("hook:{}", hook.name),
        &format!("{run_id}:{resource}:{:?}", hook.event),
        saved.requirement,
        saved.revision,
    )
    .await?;
    match begin_invocation(pool, root, run_id, resource, hook).await? {
        StartDecision::Done(value) => Ok(value),
        StartDecision::Recover(id, directory) => Ok(reconcile(pool, &id, &directory, hook)
            .await?
            .unwrap_or(false)),
        StartDecision::Start(id, attempt, directory) => {
            execute_new(
                pool, run_id, saved, resource, workspace, hook, &id, attempt, &directory,
            )
            .await
        }
    }
}

async fn begin_invocation(
    pool: &PgPool,
    root: &Path,
    run_id: &str,
    resource: &str,
    hook: &HookConfig,
) -> Result<StartDecision> {
    let event = serde_json::to_value(&hook.event)?
        .as_str()
        .ok_or("hook event")?
        .to_owned();
    let mut tx = run_store::lock(pool).await?;
    let previous: Option<(String, i32, String, String)> = sqlx::query_as("SELECT invocation_id,attempt,status,output_dir FROM project_hook_invocation WHERE run_id=$1 AND resource_id=$2 AND event=$3 AND hook_name=$4 FOR UPDATE")
        .bind(run_id).bind(resource).bind(&event).bind(&hook.name).fetch_optional(&mut *tx).await?;
    let decision = if let Some(previous) = previous {
        existing_invocation(&mut tx, previous, hook).await?
    } else {
        create_invocation(&mut tx, root, run_id, resource, &event, hook).await?
    };
    tx.commit().await?;
    Ok(decision)
}

async fn existing_invocation(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    previous: (String, i32, String, String),
    hook: &HookConfig,
) -> Result<StartDecision> {
    let (id, attempt, status, directory) = previous;
    if status == "success" {
        return Ok(StartDecision::Done(true));
    }
    if !matches!(
        status.as_str(),
        "failed" | "unknown" | "timeout" | "cancelled"
    ) {
        return Ok(StartDecision::Recover(id, PathBuf::from(directory)));
    }
    let stopped = verified_stop(Path::new(&directory));
    if status == "unknown" && stopped {
        sqlx::query(
            "UPDATE project_hook_invocation SET stop_confirmed=true WHERE invocation_id=$1",
        )
        .bind(&id)
        .execute(&mut **tx)
        .await?;
    }
    if hook.replay != ReplayPolicy::Idempotent || attempt >= 2 || !stopped {
        return Ok(StartDecision::Done(false));
    }
    replay_invocation(tx, id, &directory).await
}

async fn replay_invocation(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: String,
    directory: &str,
) -> Result<StartDecision> {
    let next = Path::new(&directory)
        .parent()
        .ok_or("missing hook attempt parent")?
        .join("2");
    fs::create_dir_all(&next)?;
    sqlx::query("UPDATE project_hook_invocation SET attempt=2,status='intent',pid=NULL,process_identity=NULL,stop_confirmed=false,output_dir=$2,result=NULL,diagnostic=NULL WHERE invocation_id=$1")
        .bind(&id).bind(next.to_string_lossy().as_ref()).execute(&mut **tx).await?;
    Ok(StartDecision::Start(id, 2, next))
}

async fn create_invocation(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    root: &Path,
    run_id: &str,
    resource: &str,
    event: &str,
    hook: &HookConfig,
) -> Result<StartDecision> {
    let id = process::new_identity()?;
    let directory = process::run_directory(root, run_id)?
        .join("project-hooks")
        .join(&id)
        .join("1");
    fs::create_dir_all(&directory)?;
    sqlx::query("INSERT INTO project_hook_invocation(invocation_id,run_id,resource_id,event,hook_name,status,output_dir) VALUES($1,$2,$3,$4,$5,'intent',$6)")
        .bind(&id).bind(run_id).bind(resource).bind(event).bind(&hook.name)
        .bind(directory.to_string_lossy().as_ref()).execute(&mut **tx).await?;
    Ok(StartDecision::Start(id, 1, directory))
}

#[allow(clippy::too_many_arguments)]
async fn execute_new(
    pool: &PgPool,
    run_id: &str,
    saved: &Registration,
    resource: &str,
    workspace: &str,
    hook: &HookConfig,
    id: &str,
    attempt: u32,
    directory: &Path,
) -> Result<bool> {
    let identity = InvocationIdentity {
        protocol_version: 1,
        requirement_id: saved.requirement,
        revision: saved.revision,
        run_id: (hook.event != HookEvent::BeforeRemove).then(|| run_id.to_owned()),
        resource_id: resource.to_owned(),
        invocation_id: id.to_owned(),
        attempt,
        config_id: saved.frozen.config_id.clone(),
    };
    let deadline = SystemTime::now() + Duration::from_secs(hook.timeout_seconds as u64);
    let invocation = HookInvocation {
        identity: identity.clone(),
        event: hook.event.clone(),
        role: saved.role.clone(),
        workspace: workspace.to_owned(),
        output_dir: directory.to_string_lossy().into_owned(),
        deadline_at: chrono::DateTime::<chrono::Utc>::from(deadline).to_rfc3339(),
        context: serde_json::Map::new(),
    };
    invocation
        .validate(&saved.frozen, hook)
        .map_err(|e| format!("invalid hook invocation: {e:?}"))?;
    if serde_json::to_vec(&invocation)?.len() > crate::extension_contract::MAX_RESULT_BYTES {
        return Err("hook input exceeds protocol limit".into());
    }
    process::durable_write(&directory.join("input.json"), &invocation)?;
    let result = match verify_script(hook) {
        Ok(()) => run_script(pool, id, hook, directory, &identity).await,
        Err(error) => Err(error),
    };
    let (status, diagnostic, result_json) = classify_result(result, directory);
    sqlx::query("UPDATE project_hook_invocation SET status=$2,result=$3,diagnostic=$4,stop_confirmed=$5 WHERE invocation_id=$1 AND status IN ('intent','running')")
        .bind(id).bind(status).bind(result_json).bind(diagnostic)
        .bind(verified_stop(directory)).execute(pool).await?;
    Ok(status == "success")
}

fn classify_result(
    result: Result<(bool, Value)>,
    directory: &Path,
) -> (&'static str, Option<String>, Option<Value>) {
    match result {
        Ok((true, value)) => ("success", None, Some(value)),
        Ok((false, value)) => ("failed", None, Some(value)),
        Err(error) => {
            let status = failure_status(directory);
            (
                status,
                Some(crate::operator_view::redact_text(&error.to_string())),
                None,
            )
        }
    }
}

fn failure_status(directory: &Path) -> &'static str {
    let stopped = verified_stop(directory);
    if !stopped && directory.join("identity.json").exists() {
        "unknown"
    } else if directory.join("timeout.json").exists() && stopped {
        "timeout"
    } else if directory.join("cancelled.json").exists() && stopped {
        "cancelled"
    } else if stopped {
        "unknown"
    } else {
        "failed"
    }
}

fn verify_script(hook: &HookConfig) -> Result<()> {
    let path = Path::new(&hook.argv[0]);
    if !path.is_absolute() || !fs::symlink_metadata(path)?.file_type().is_file() {
        return Err("reviewed hook executable must be an absolute regular file".into());
    }
    let expected = format!("sha256:{:x}", Sha256::digest(fs::read(path)?));
    if hook.script_identity != expected {
        return Err("reviewed hook script identity changed".into());
    }
    Ok(())
}

async fn run_script(
    pool: &PgPool,
    id: &str,
    hook: &HookConfig,
    directory: &Path,
    identity: &InvocationIdentity,
) -> Result<(bool, Value)> {
    let key = hook_key(id);
    let child = spawn_hook(directory, hook, identity, &key)?;
    let receipt = await_identity(directory, &key).await?;
    sqlx::query("UPDATE project_hook_invocation SET status='running',pid=$2,process_identity=$3 WHERE invocation_id=$1 AND status='intent'")
        .bind(id).bind(receipt.process.pid as i32).bind(json!(receipt.process)).execute(pool).await?;
    process::durable_write(&directory.join("start.json"), &key)?;
    let (timed_out, cancelled) =
        wait_quiescent(pool, directory, identity, hook, &key, &receipt).await?;
    let _ = tokio::task::spawn_blocking(move || {
        let mut child = child;
        child.wait()
    })
    .await;
    if timed_out {
        return Err("hook timeout after complete descendant stop".into());
    }
    if cancelled {
        return Err("hook cancelled after complete descendant stop".into());
    }
    decode_exit(directory, identity, hook)
}

fn hook_key(id: &str) -> RunKey {
    RunKey {
        run_id: id.to_owned(),
        request_id: id.to_owned(),
        incarnation: id.to_owned(),
    }
}

fn verified_stop(directory: &Path) -> bool {
    let started = process::read::<crate::execution::Receipt>(&directory.join("identity.json"));
    let stopped = process::read::<crate::execution::Receipt>(&directory.join("quiescent.json"));
    matches!((started, stopped), (Ok(started), Ok(stopped)) if started == stopped)
}

fn spawn_hook(
    directory: &Path,
    hook: &HookConfig,
    identity: &InvocationIdentity,
    key: &RunKey,
) -> Result<std::process::Child> {
    let launch = Launch {
        key: key.clone(),
        workspace: directory.to_string_lossy().into_owned(),
        workspace_identity: identity.resource_id.clone(),
        program: hook.argv[0].clone(),
        args: hook.argv[1..].to_vec(),
    };
    process::durable_write(&directory.join("hook.json"), identity)?;
    process::durable_write(
        &directory.join("hook-limit.json"),
        &(hook.output_limit_bytes as u64),
    )?;
    let supervisor = std::env::var_os("SYMPHONY_SUPERVISOR")
        .map(PathBuf::from)
        .unwrap_or(std::env::current_exe()?);
    Ok(process::spawn(&supervisor, directory, &launch)?)
}

async fn await_identity(directory: &Path, key: &RunKey) -> Result<crate::execution::Receipt> {
    let started = Instant::now();
    loop {
        if let Ok(receipt) =
            process::read::<crate::execution::Receipt>(&directory.join("identity.json"))
        {
            return Ok(receipt);
        }
        if started.elapsed() > Duration::from_secs(15) {
            process::durable_write(&directory.join("stop.json"), &key)?;
            return Err("hook supervisor identity unavailable".into());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn wait_quiescent(
    pool: &PgPool,
    directory: &Path,
    identity: &InvocationIdentity,
    hook: &HookConfig,
    key: &RunKey,
    receipt: &crate::execution::Receipt,
) -> Result<(bool, bool)> {
    let deadline = Instant::now() + Duration::from_secs(hook.timeout_seconds as u64);
    let mut stopped_for_deadline = false;
    let mut stopped_for_cancel = false;
    let mut last_control_check = Instant::now() - Duration::from_secs(2);
    loop {
        if let Ok(stopped) =
            process::read::<crate::execution::Receipt>(&directory.join("quiescent.json"))
        {
            if stopped != *receipt {
                return Err("hook stop receipt identity mismatch".into());
            }
            break;
        }
        stopped_for_deadline |= check_deadline(directory, identity, key, deadline)?;
        if hook.event != HookEvent::AfterRun
            && hook.event != HookEvent::BeforeRemove
            && last_control_check.elapsed() >= Duration::from_secs(1)
        {
            last_control_check = Instant::now();
            stopped_for_cancel |= check_cancel(pool, directory, identity, key).await?;
        }
        if Instant::now() >= deadline + Duration::from_secs(20) {
            return Err("hook timeout without complete descendant stop proof".into());
        }
        process::durable_write(&directory.join("storage-heartbeat.json"), &key)?;
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    Ok((stopped_for_deadline, stopped_for_cancel))
}

fn check_deadline(
    directory: &Path,
    identity: &InvocationIdentity,
    key: &RunKey,
    deadline: Instant,
) -> Result<bool> {
    if Instant::now() < deadline {
        return Ok(false);
    }
    process::durable_write(&directory.join("timeout.json"), identity)?;
    process::durable_write(&directory.join("stop.json"), key)?;
    Ok(true)
}

async fn check_cancel(
    pool: &PgPool,
    directory: &Path,
    identity: &InvocationIdentity,
    key: &RunKey,
) -> Result<bool> {
    let cancelled: bool =
        sqlx::query_scalar("SELECT paused OR cancel_requested FROM requirement WHERE id=$1")
            .bind(identity.requirement_id)
            .fetch_one(pool)
            .await?;
    if cancelled {
        process::durable_write(&directory.join("cancelled.json"), identity)?;
        process::durable_write(&directory.join("stop.json"), key)?;
    }
    Ok(cancelled)
}

fn decode_exit(
    directory: &Path,
    identity: &InvocationIdentity,
    hook: &HookConfig,
) -> Result<(bool, Value)> {
    if directory.join("spawn-error.json").exists() {
        return Ok((false, json!({"spawn_error":true})));
    }
    let exit: Option<i32> = process::read(&directory.join("exit.json"))?;
    let Some(exit) = exit else {
        return Err("hook never started".into());
    };
    // waitpid status uses its low seven bits for signals and the next byte for exit.
    if exit & 0x7f != 0 || (exit >> 8) & 0xff != 0 {
        return Ok((false, json!({"exit_status":exit})));
    }
    parse_output(directory, identity, hook)
}

fn parse_output(
    directory: &Path,
    identity: &InvocationIdentity,
    hook: &HookConfig,
) -> Result<(bool, Value)> {
    let bytes = bounded_output(directory, hook.output_limit_bytes as u64)?;
    let result =
        parse_hook_result(&bytes, identity).map_err(|e| format!("invalid hook result: {e:?}"))?;
    if let HookOutcome::Success { artifacts } = &result.outcome {
        validate_artifacts(directory, artifacts, hook.output_limit_bytes as u64)?;
    }
    Ok((
        matches!(result.outcome, HookOutcome::Success { .. }),
        json!(result),
    ))
}

fn bounded_output(directory: &Path, configured: u64) -> Result<Vec<u8>> {
    if directory.join("truncated.json").exists() {
        return Err("hook output exceeded configured limit".into());
    }
    let limit = configured.min(crate::extension_contract::MAX_RESULT_BYTES as u64);
    let mut bytes = Vec::new();
    File::open(directory.join("stdout.json"))?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err("hook stdout exceeds limit".into());
    }
    if fs::metadata(directory.join("stderr.log"))?.len() > configured {
        return Err("hook stderr exceeds limit".into());
    }
    Ok(bytes)
}

fn validate_artifacts(
    directory: &Path,
    artifacts: &[crate::extension_contract::ArtifactRef],
    limit: u64,
) -> Result<()> {
    for artifact in artifacts {
        let mut path = directory.to_owned();
        for part in artifact.path.split('/') {
            path.push(part);
            let meta = fs::symlink_metadata(&path)?;
            if meta.file_type().is_symlink() {
                return Err("hook artifact symlink rejected".into());
            }
        }
        let meta = fs::metadata(&path)?;
        if !meta.is_file() || meta.len() > limit {
            return Err("hook artifact invalid or exceeds limit".into());
        }
    }
    Ok(())
}

async fn reconcile(
    pool: &PgPool,
    id: &str,
    directory: &Path,
    hook: &HookConfig,
) -> Result<Option<bool>> {
    let identity: InvocationIdentity = match process::read(directory.join("hook.json").as_path()) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    if !verified_stop(directory) {
        // A timed-out or restarted caller stops the registered supervisor. Its
        // heartbeat expires independently; absence of ECHILD remains unknown.
        let key = hook_key(id);
        process::durable_write(&directory.join("stop.json"), &key)?;
        record_reconciled(
            pool,
            id,
            "unknown",
            false,
            "stop receipt missing; reconcile side effects before replay",
        )
        .await?;
        return Ok(None);
    }
    reconcile_stopped(pool, id, directory, hook, &identity).await
}

async fn reconcile_stopped(
    pool: &PgPool,
    id: &str,
    directory: &Path,
    hook: &HookConfig,
    identity: &InvocationIdentity,
) -> Result<Option<bool>> {
    let exit: Option<i32> = process::read(&directory.join("exit.json")).unwrap_or(None);
    if directory.join("spawn-error.json").exists() {
        record_reconciled(
            pool,
            id,
            "failed",
            true,
            "reviewed hook executable could not start",
        )
        .await?;
        return Ok(Some(false));
    }
    if exit.is_some_and(|status| status != 0) {
        record_reconciled(
            pool,
            id,
            "failed",
            true,
            "hook exit nonzero after stop proof",
        )
        .await?;
        return Ok(Some(false));
    }
    if exit.is_none() {
        record_reconciled(
            pool,
            id,
            "unknown",
            true,
            "hook exit result missing after stop proof",
        )
        .await?;
        return Ok(Some(false));
    }
    persist_reconciled_output(pool, id, directory, identity, hook).await
}

async fn persist_reconciled_output(
    pool: &PgPool,
    id: &str,
    directory: &Path,
    identity: &InvocationIdentity,
    hook: &HookConfig,
) -> Result<Option<bool>> {
    let parsed = parse_output(directory, identity, hook);
    let (success, value) = match parsed {
        Ok(value) => value,
        Err(error) => {
            record_reconciled(
                pool,
                id,
                "unknown",
                true,
                &crate::operator_view::redact_text(&error.to_string()),
            )
            .await?;
            return Ok(Some(false));
        }
    };
    sqlx::query("UPDATE project_hook_invocation SET status=$2,result=$3,stop_confirmed=true WHERE invocation_id=$1")
        .bind(id)
        .bind(if success { "success" } else { "failed" })
        .bind(value)
        .execute(pool)
        .await?;
    Ok(Some(success))
}

async fn record_reconciled(
    pool: &PgPool,
    id: &str,
    status: &str,
    stopped: bool,
    diagnostic: &str,
) -> Result<()> {
    sqlx::query("UPDATE project_hook_invocation SET status=$2,stop_confirmed=$3,diagnostic=$4 WHERE invocation_id=$1")
        .bind(id).bind(status).bind(stopped).bind(diagnostic).execute(pool).await?;
    Ok(())
}

#[cfg(test)]
#[path = "../tests/unit/project_hooks.rs"]
mod failure_classification_tests;
