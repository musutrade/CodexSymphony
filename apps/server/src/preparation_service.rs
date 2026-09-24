//! Platform-owned execution adapter, run after worktree/Broker preparation.
use crate::{
    execution::Launch,
    git_broker::GitBroker,
    preparation::{CODEX_VERSION, CORE_SHA256, CORE_VERSION, Evidence, Failure},
    preparation_store, process, storage,
    workspace::Workspace,
};
use serde_json::Value;
use sqlx::PgPool;
use std::{
    fs::{self, File},
    io::Read,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Deployment config and adapter path are loaded by the platform, never from
/// an Agent tool argument. Nothing here changes /etc/codex/requirements.toml.
pub struct Request<'a> {
    pub launch: &'a Launch,
    pub requirement: i64,
    pub revision: i64,
    pub phase: &'a str,
    pub now: i64,
    pub adapter: &'a Path,
    pub control_directory: &'a Path,
    pub broker: &'a GitBroker,
    pub workspace: &'a Workspace,
    pub config: Value,
}

pub async fn prepare(pool: &PgPool, request: Request<'_>) -> Result<bool> {
    let started = Instant::now();
    let Some(expected) = admit(pool, &request).await? else {
        return Ok(false);
    };
    if !environment_ready(pool, &request, started).await? {
        return Ok(false);
    }
    let (directory, output_limit) = preparation_directory(pool, &request).await?;
    let Some(config) = project_config(pool, &request).await? else {
        return Ok(false);
    };
    let result = run_probe(&request, config, &directory, output_limit).await?;
    let now = request
        .now
        .saturating_add(started.elapsed().as_secs() as i64);
    finish_preparation(pool, &request, &expected, result, &directory, now).await
}

async fn environment_ready(pool: &PgPool, request: &Request<'_>, started: Instant) -> Result<bool> {
    if let Err(error) = crate::environment_service::admit(
        pool,
        request.requirement,
        request.revision,
        request.phase,
        Some(Path::new(&request.launch.workspace)),
    )
    .await
    {
        let failure = Failure::new(
            "environment_mismatch",
            &error.to_string(),
            "environment_observation",
        );
        preparation_store::failed_probe(
            pool,
            request.launch,
            failure,
            request
                .now
                .saturating_add(started.elapsed().as_secs() as i64),
        )
        .await?;
        return Ok(false);
    }
    Ok(true)
}

async fn admit(pool: &PgPool, request: &Request<'_>) -> Result<Option<String>> {
    validate_launch(request)?;
    let expected = request.config["deployment_identity"]
        .as_str()
        .filter(nonempty_identity)
        .ok_or("deployment identity required")?;
    if !storage_ready(pool, request).await {
        return Ok(None);
    }
    if !preparation_store::begin(
        pool,
        request.launch,
        request.requirement,
        request.revision,
        request.phase,
        request.now,
    )
    .await?
    {
        return Ok(None);
    }
    Ok(Some(expected.to_owned()))
}

async fn project_config(pool: &PgPool, request: &Request<'_>) -> Result<Option<Value>> {
    let mut config = request.config.clone();
    config["workspace"] = Value::String(request.launch.workspace.clone());
    config["tool_lock"] = serde_json::json!({"core_version":CORE_VERSION,
        "core_sha256":CORE_SHA256, "codex_version":CODEX_VERSION});
    config["environment_extension_verified"] = Value::Bool(false);
    if crate::environment_service::plan(pool, request.requirement, request.revision)
        .await?
        .is_some()
    {
        // Environment admission already ran using the immutable reviewed plan.
        // The legacy adapter still checks the Agent, cwd and writable paths.
        config["environment_extension_verified"] = Value::Bool(true);
        config["dependencies"] = serde_json::json!([]);
    }
    // The legacy probe records a mismatched Broker as a scoped failure. Do
    // not freeze that invalid identity into a hook Run before the check.
    if (request.workspace.requirement, request.workspace.revision)
        != (request.requirement, request.revision)
        || broker_ready(request.broker, request.workspace, request.launch).is_err()
    {
        return Ok(Some(config));
    }
    prepare_project_hooks(pool, request, config).await
}

async fn prepare_project_hooks(
    pool: &PgPool,
    request: &Request<'_>,
    mut config: Value,
) -> Result<Option<Value>> {
    let role = match request.phase {
        "code_repair" | "linked_code_repair" => crate::extension_contract::HookRole::Repair,
        _ => crate::extension_contract::HookRole::Coding,
    };
    let Some(project_preparation) = register_project(pool, request, role).await? else {
        return Ok(None);
    };
    if project_preparation {
        // Reviewed project dependencies run in before_run. Legacy projects keep
        // the existing deployment dependency probes unchanged.
        config["dependencies"] = serde_json::json!([]);
    }
    if request.phase == "preparation"
        && !hook_step(
            pool,
            request,
            crate::extension_contract::HookEvent::AfterCreate,
            request.now,
        )
        .await?
    {
        return Ok(None);
    }
    Ok(Some(config))
}

async fn register_project(
    pool: &PgPool,
    request: &Request<'_>,
    role: crate::extension_contract::HookRole,
) -> Result<Option<bool>> {
    match crate::project_hooks::register(
        pool,
        request.launch,
        request.workspace,
        role,
        &request.config,
    )
    .await
    {
        Ok(value) => Ok(Some(value)),
        Err(error) => {
            record_hook_error(
                pool,
                request.launch,
                "project_hook_registration",
                &error.to_string(),
                request.now,
            )
            .await?;
            Ok(None)
        }
    }
}

async fn run_probe(
    request: &Request<'_>,
    config: Value,
    directory: &Path,
    output_limit: u64,
) -> Result<Result<Evidence>> {
    let adapter = request.adapter.to_owned();
    let artifacts = directory.to_owned();
    let broker = request.broker.clone();
    let workspace = request.workspace.clone();
    let launch = request.launch.clone();
    let identity = (request.requirement, request.revision);
    Ok(tokio::task::spawn_blocking(move || {
        if (workspace.requirement, workspace.revision) != identity {
            return Err("Broker requirement revision mismatch".into());
        }
        broker_ready(&broker, &workspace, &launch)?;
        probe(&adapter, &config, &artifacts, output_limit)
    })
    .await?)
}

async fn finish_preparation(
    pool: &PgPool,
    request: &Request<'_>,
    expected: &str,
    result: Result<Evidence>,
    directory: &Path,
    now: i64,
) -> Result<bool> {
    // Persist the actual result directly. A second inventory cannot establish
    // whether this evidence commit will succeed and can discard useful failures.
    let requested: Vec<String> = serde_json::from_value(
        request
            .config
            .get("network_urls")
            .cloned()
            .unwrap_or_else(|| serde_json::json!([])),
    )?;
    if let Ok(evidence) = &result
        && evidence.failure(expected, &requested).is_none()
        && !hook_step(
            pool,
            request,
            crate::extension_contract::HookEvent::BeforeRun,
            now,
        )
        .await?
    {
        return Ok(false);
    }
    record_result(pool, request.launch, expected, result, directory, now).await
}

async fn hook_step(
    pool: &PgPool,
    request: &Request<'_>,
    event: crate::extension_contract::HookEvent,
    now: i64,
) -> Result<bool> {
    match crate::project_hooks::event(
        pool,
        request.control_directory,
        &request.launch.key.run_id,
        event.clone(),
        None,
    )
    .await
    {
        Ok(true) => Ok(true),
        Ok(false) => {
            record_hook_error(
                pool,
                request.launch,
                "project_hook_failed",
                &format!("{event:?} did not complete; inspect project_hook_invocation"),
                now,
            )
            .await?;
            Ok(false)
        }
        Err(error) => {
            record_hook_error(
                pool,
                request.launch,
                "project_hook_error",
                &error.to_string(),
                now,
            )
            .await?;
            Ok(false)
        }
    }
}

async fn record_hook_error(
    pool: &PgPool,
    launch: &Launch,
    code: &str,
    detail: &str,
    now: i64,
) -> Result<()> {
    let detail = crate::operator_view::redact_text(detail);
    preparation_store::failed_probe(
        pool,
        launch,
        Failure::new(code, &detail, "project_hook_invocation"),
        now,
    )
    .await?;
    Ok(())
}

async fn preparation_directory(pool: &PgPool, request: &Request<'_>) -> Result<(PathBuf, u64)> {
    let directory = request.control_directory.join(format!(
        ".preparation-{}-{}",
        request.launch.key.run_id,
        process::new_identity()?
    ));
    fs::create_dir(&directory)?;
    crate::storage_service::preparation(pool, request.workspace, &directory).await?;
    Ok((directory, crate::storage_service::entry_limit(pool).await?))
}

fn nonempty_identity(value: &&str) -> bool {
    !value.is_empty()
}

fn validate_launch(request: &Request<'_>) -> Result<()> {
    let mut command: Vec<String> = serde_json::from_value(request.config["launcher"].clone())?;
    command.push("app-server".into());
    let mut actual = Vec::new();
    actual.push(request.launch.program.clone());
    actual.extend(request.launch.args.clone());
    if command != actual {
        return Err("probe launcher must match the intended app-server command".into());
    }
    Ok(())
}

async fn storage_ready(pool: &PgPool, request: &Request<'_>) -> bool {
    storage::permit(pool, request.control_directory).await
        && storage::permit(pool, Path::new(&request.launch.workspace)).await
}

async fn record_result(
    pool: &PgPool,
    launch: &Launch,
    expected: &str,
    result: Result<Evidence>,
    directory: &Path,
    now: i64,
) -> Result<bool> {
    match result {
        Ok(evidence) => record_evidence(pool, launch, expected, &evidence, now).await,
        Err(error) => {
            let failure = Failure::new(
                "preparation_capability_mismatch",
                &error.to_string(),
                &directory.to_string_lossy(),
            );
            preparation_store::failed_probe(pool, launch, failure, now).await?;
            Ok(false)
        }
    }
}

async fn record_evidence(
    pool: &PgPool,
    launch: &Launch,
    expected: &str,
    evidence: &Evidence,
    now: i64,
) -> Result<bool> {
    Ok(preparation_store::finish(pool, launch, expected, evidence, now).await?)
}

fn broker_ready(broker: &GitBroker, workspace: &Workspace, launch: &Launch) -> Result<()> {
    if workspace.key != launch.key
        || workspace.path != launch.workspace
        || workspace.identity != launch.workspace_identity
    {
        return Err("Broker workspace does not match intended Agent launch".into());
    }
    broker.head(workspace)?;
    Ok(())
}

/// Kill the entire probe process group before joining bounded output readers.
struct Probe(Child);
impl Drop for Probe {
    fn drop(&mut self) {
        unsafe extern "C" {
            fn kill(pid: i32, signal: i32) -> i32;
        }
        unsafe {
            kill(-(self.0.id() as i32), 9);
        }
        let _ = self.0.wait();
    }
}

fn probe(adapter: &Path, config: &Value, directory: &Path, limit: u64) -> Result<Evidence> {
    if serde_json::to_vec(config)?.len() as u64 > limit {
        return Err("preparation input entry limit".into());
    }
    process::durable_write(&directory.join("input.json"), config)?;
    let stdout = directory.join("stdout.json");
    let mut child = spawn_probe(adapter, directory)?;
    let capture = capture_probe(&mut child, directory, &stdout, limit)?;
    let result = wait_probe(&mut child, &capture);
    drop(child);
    let truncated = capture.finish()?;
    record_probe_quiescent(directory, truncated, limit)?;
    result?;
    decode_output(stdout)
}
fn record_probe_quiescent(directory: &Path, truncated: bool, limit: u64) -> Result<()> {
    process::durable_write(
        &directory.join("quiescent.json"),
        &serde_json::json!({"quiescent":true,"truncated":truncated,"limit":limit}),
    )?;
    if truncated {
        return Err(
            "preparation output truncated at configured entry limit (at most 1 MiB)".into(),
        );
    }
    Ok(())
}

fn capture_probe(
    child: &mut Probe,
    directory: &Path,
    stdout: &Path,
    limit: u64,
) -> Result<crate::storage_output::Capture> {
    let mut capture = crate::storage_output::Capture::default();
    capture.stream(
        child.0.stdout.take().ok_or("probe stdout unavailable")?,
        File::create(stdout)?,
        limit,
    );
    capture.stream(
        child.0.stderr.take().ok_or("probe stderr unavailable")?,
        File::create(directory.join("stderr.log"))?,
        limit,
    );
    Ok(capture)
}

fn wait_probe(child: &mut Probe, capture: &crate::storage_output::Capture) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(110);
    loop {
        if let Some(status) = child.0.try_wait()? {
            if !status.success() {
                return Err(format!(
                    "app-server preparation probe exited {status}; inspect retained stderr.log"
                )
                .into());
            }
            return Ok(());
        }
        if Instant::now() >= deadline || capture.stopped() {
            return Err("fixed preparation probe deadline exceeded".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn spawn_probe(adapter: &Path, directory: &Path) -> Result<Probe> {
    Ok(Probe(
        Command::new("python3")
            .arg(adapter)
            .process_group(0)
            .stdin(Stdio::from(File::open(directory.join("input.json"))?))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?,
    ))
}

fn decode_output(path: PathBuf) -> Result<Evidence> {
    let mut bytes = Vec::new();
    File::open(path)?.take(1_048_577).read_to_end(&mut bytes)?;
    if bytes.len() > 1_048_576 {
        return Err("probe output exceeds fixed 1 MiB limit".into());
    }
    Ok(serde_json::from_slice(&bytes)?)
}
