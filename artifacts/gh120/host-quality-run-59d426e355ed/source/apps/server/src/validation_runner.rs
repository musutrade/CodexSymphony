//! Fixed candidate checks in the trusted development environment. The host
//! supplies the reviewed plan; neither Agent output nor HTTP can choose it.
use crate::{
    process,
    validation::{Candidate, StepEvidence, TrustedIdentity, sha256},
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Read,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
const LIMIT: u64 = 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub id: String,
    pub command: Vec<String>,
    pub timeout_seconds: u64,
    pub code_failure: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub steps: Vec<Step>,
    pub entry: PathBuf,
    pub entry_sha256: String,
}
impl Plan {
    pub fn identity(&self) -> Result<TrustedIdentity> {
        self.validate()?;
        Ok(TrustedIdentity {
            command_sha256: sha256(serde_json::to_vec(&self.steps)?),
            config_sha256: sha256(serde_json::to_vec(self)?),
            protected_entry: self.entry.to_string_lossy().into_owned(),
            protected_entry_sha256: self.entry_sha256.clone(),
            tool: "trusted-development-process".into(),
            tool_version: "1".into(),
        })
    }
    fn validate(&self) -> Result<()> {
        if self.steps.is_empty() || !self.entry.is_absolute() {
            return Err("invalid validation plan".into());
        }
        if digest(&self.entry)? != self.entry_sha256 {
            return Err("protected validation tool changed".into());
        }
        let mut ids = std::collections::HashSet::new();
        for step in &self.steps {
            validate_step(step)?;
            if !ids.insert(&step.id) {
                return Err("duplicate approved validation check".into());
            }
        }
        Ok(())
    }
}
fn validate_step(step: &Step) -> Result<()> {
    if step.id.is_empty()
        || step.command.first().map(String::as_str) != Some("/gate-entry")
        || !(1..=3600).contains(&step.timeout_seconds)
    {
        return Err("invalid approved step".into());
    }
    Ok(())
}
#[derive(Clone, Copy)]
struct Control<'a> {
    limit: u64,
    cancelled: Option<&'a AtomicBool>,
}
impl Control<'_> {
    fn stopped(self) -> bool {
        self.cancelled
            .is_some_and(|flag| flag.load(Ordering::Acquire))
    }
    fn check(self) -> Result<()> {
        if self.stopped() {
            return Err("validation stopped by current control intent".into());
        }
        Ok(())
    }
}

fn digest(path: &Path) -> Result<String> {
    Ok(sha256(fs::read(path)?))
}
fn git(root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()?;
    if !output.status.success() {
        return Err("candidate Git identity unavailable".into());
    }
    Ok(String::from_utf8(output.stdout)?.trim().into())
}
pub fn candidate(root: &Path) -> Result<Candidate> {
    if !git(root, &["status", "--porcelain", "--untracked-files=all"])?.is_empty() {
        return Err("candidate checkout is dirty".into());
    }
    Ok(Candidate {
        sha: git(root, &["rev-parse", "HEAD"])?,
        tree: git(root, &["rev-parse", "HEAD^{tree}"])?,
        immutable: true,
    })
}

/// Invocation directory is platform-owned and unique for this validation ID.
/// A lost invocation is UNKNOWN; an existing result is reconciled, never rerun.
pub fn execute(
    root: &Path,
    directory: &Path,
    expected: &Candidate,
    plan: &Plan,
) -> Result<Vec<StepEvidence>> {
    execute_limited(root, directory, expected, plan, LIMIT)
}
pub fn execute_limited(
    root: &Path,
    directory: &Path,
    expected: &Candidate,
    plan: &Plan,
    limit: u64,
) -> Result<Vec<StepEvidence>> {
    execute_controlled(
        root,
        directory,
        expected,
        plan,
        Control {
            limit: limit.min(LIMIT),
            cancelled: None,
        },
    )
}

/// Control-plane cancellation stops the whole local command group and retains
/// the original invocation. Incomplete invocations are never silently replayed.
pub fn execute_cancellable(
    root: &Path,
    directory: &Path,
    expected: &Candidate,
    plan: &Plan,
    cancelled: &AtomicBool,
) -> Result<Vec<StepEvidence>> {
    execute_controlled(
        root,
        directory,
        expected,
        plan,
        Control {
            limit: LIMIT,
            cancelled: Some(cancelled),
        },
    )
}
fn execute_controlled(
    root: &Path,
    directory: &Path,
    expected: &Candidate,
    plan: &Plan,
    control: Control<'_>,
) -> Result<Vec<StepEvidence>> {
    control.check()?;
    fs::create_dir_all(directory)?;
    let checkout = fs::canonicalize(root)?;
    if plan.entry.starts_with(root)
        || directory.starts_with(root)
        || fs::canonicalize(&plan.entry)?.starts_with(&checkout)
        || fs::canonicalize(directory)?.starts_with(&checkout)
    {
        return Err("validation implementation and evidence must be outside candidate".into());
    }
    let _lock = process::InstanceLock::acquire(&directory.join("lock"))?;
    let identity = plan.identity()?;
    if candidate(root)? != *expected {
        return Err("fixed candidate mismatch".into());
    }
    let binding = serde_json::json!({"candidate":expected,"identity":identity});
    if let Some(evidence) = replay(directory, &binding, plan)? {
        return Ok(evidence);
    }
    run_plan(
        root, directory, expected, plan, &identity, &binding, control,
    )
}
fn replay(
    directory: &Path,
    binding: &serde_json::Value,
    plan: &Plan,
) -> Result<Option<Vec<StepEvidence>>> {
    if directory.join("binding.json").exists() {
        let saved: serde_json::Value = process::read(&directory.join("binding.json"))?;
        if &saved != binding {
            return Err("validation invocation identity changed".into());
        }
        let evidence: Vec<StepEvidence> = process::read(&directory.join("result.json"))?;
        if evidence.len() != plan.steps.len() {
            return Err("retained validation check set changed".into());
        }
        for (index, (step, approved)) in evidence.iter().zip(&plan.steps).enumerate() {
            let log = directory.join(format!("step-{index}.log"));
            verify_retained_step(step, approved, &log)?;
        }
        return Ok(Some(evidence));
    }
    Ok(None)
}
fn verify_retained_step(step: &StepEvidence, approved: &Step, log: &Path) -> Result<()> {
    if step.id != approved.id
        || step.command != approved.command
        || step.code_failure != approved.code_failure
        || step.log_ref != log.to_string_lossy()
        || sha256(&step.output) != step.output_sha256
        || digest(log)? != step.output_sha256
    {
        return Err("retained evidence changed".into());
    }
    Ok(())
}
fn run_plan(
    root: &Path,
    directory: &Path,
    expected: &Candidate,
    plan: &Plan,
    identity: &TrustedIdentity,
    binding: &serde_json::Value,
    control: Control<'_>,
) -> Result<Vec<StepEvidence>> {
    process::durable_write(&directory.join("binding.json"), &binding)?;
    let mut evidence = Vec::new();
    for (index, step) in plan.steps.iter().enumerate() {
        control.check()?;
        evidence.push(run_step(root, directory, index, step, plan, control)?);
    }
    verify_unchanged(root, expected, plan, identity)?;
    process::durable_write(&directory.join("result.json"), &evidence)?;
    Ok(evidence)
}
fn verify_unchanged(
    root: &Path,
    expected: &Candidate,
    plan: &Plan,
    identity: &TrustedIdentity,
) -> Result<()> {
    if candidate(root)? != *expected || plan.identity()? != *identity {
        return Err("validation source or tool changed".into());
    }
    Ok(())
}
fn run_step(
    root: &Path,
    directory: &Path,
    index: usize,
    step: &Step,
    plan: &Plan,
    control: Control<'_>,
) -> Result<StepEvidence> {
    let path = directory.join(format!("step-{index}.log"));
    let file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)?;
    let mut command = command(root, step, plan);
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let mut capture = crate::storage_output::Capture::default();
    capture.combined(
        child.stdout.take().ok_or("stdout unavailable")?,
        child.stderr.take().ok_or("stderr unavailable")?,
        file,
        control.limit,
    );
    let exit = wait(&mut child, step.timeout_seconds, &capture, control)?;
    finish_capture(capture, &path, control.limit)?;
    control.check()?;
    evidence(directory, index, step, exit)
}
fn finish_capture(capture: crate::storage_output::Capture, path: &Path, limit: u64) -> Result<()> {
    if capture.finish()? {
        process::durable_write(
            &path.with_extension("truncated.json"),
            &serde_json::json!({"kept_range":[0,limit],"reason":"raw output byte limit","complete":false}),
        )?;
        return Err("validation output limit reached".into());
    }
    Ok(())
}
fn wait(
    child: &mut std::process::Child,
    timeout: u64,
    output: &crate::storage_output::Capture,
    control: Control<'_>,
) -> Result<Option<i32>> {
    let deadline = Instant::now() + Duration::from_secs(timeout);
    let exit = loop {
        if let Some(status) = child.try_wait()? {
            break status.code();
        }
        if Instant::now() >= deadline || output.stopped() || control.stopped() {
            stop_group(child.id());
            child.wait()?;
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    stop_group(child.id());
    Ok(exit)
}
fn stop_group(pid: u32) {
    unsafe extern "C" {
        fn kill(pid: i32, signal: i32) -> i32;
    }
    unsafe {
        kill(-(pid as i32), 9);
    }
}
fn evidence(
    directory: &Path,
    index: usize,
    step: &Step,
    exit: Option<i32>,
) -> Result<StepEvidence> {
    let path = directory.join(format!("step-{index}.log"));
    // The command process group is stopped before reading retained output.
    let mut bytes = Vec::new();
    fs::File::open(&path)?
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 >= LIMIT {
        return Err("validation output limit reached".into());
    }
    let output = String::from_utf8(bytes)?;
    Ok(StepEvidence {
        id: step.id.clone(),
        command: step.command.clone(),
        exit_code: exit,
        output_sha256: sha256(&output),
        output,
        log_ref: path.to_string_lossy().into_owned(),
        consumer: "handoff".into(),
        code_failure: step.code_failure,
    })
}
fn command(root: &Path, step: &Step, plan: &Plan) -> Command {
    let mut c = Command::new(&plan.entry);
    c.args(&step.command[1..])
        .current_dir(root)
        .process_group(0)
        .stdin(Stdio::null());
    // Credentials belong to remote-action/signing services, not this worker.
    process::development_environment(&mut c);
    c
}
