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
        for step in &self.steps {
            validate_step(step)?;
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
    fs::create_dir_all(directory)?;
    let _lock = process::InstanceLock::acquire(&directory.join("lock"))?;
    let identity = plan.identity()?;
    if candidate(root)? != *expected {
        return Err("fixed candidate mismatch".into());
    }
    let binding = serde_json::json!({"candidate":expected,"identity":identity});
    if let Some(evidence) = replay(directory, &binding)? {
        return Ok(evidence);
    }
    run_plan(root, directory, expected, plan, &identity, &binding)
}
fn replay(directory: &Path, binding: &serde_json::Value) -> Result<Option<Vec<StepEvidence>>> {
    if directory.join("binding.json").exists() {
        let saved: serde_json::Value = process::read(&directory.join("binding.json"))?;
        if &saved != binding {
            return Err("validation invocation identity changed".into());
        }
        let evidence: Vec<StepEvidence> = process::read(&directory.join("result.json"))?;
        for (index, step) in evidence.iter().enumerate() {
            if digest(&directory.join(format!("step-{index}.log")))? != step.output_sha256 {
                return Err("retained evidence changed".into());
            }
        }
        return Ok(Some(evidence));
    }
    Ok(None)
}
fn run_plan(
    root: &Path,
    directory: &Path,
    expected: &Candidate,
    plan: &Plan,
    identity: &TrustedIdentity,
    binding: &serde_json::Value,
) -> Result<Vec<StepEvidence>> {
    process::durable_write(&directory.join("binding.json"), &binding)?;
    let mut evidence = Vec::new();
    for (index, step) in plan.steps.iter().enumerate() {
        evidence.push(run_step(root, directory, index, step, plan)?);
    }
    if candidate(root)? != *expected || plan.identity()? != *identity {
        return Err("validation source or tool changed".into());
    }
    process::durable_write(&directory.join("result.json"), &evidence)?;
    Ok(evidence)
}
fn run_step(
    root: &Path,
    directory: &Path,
    index: usize,
    step: &Step,
    plan: &Plan,
) -> Result<StepEvidence> {
    let path = directory.join(format!("step-{index}.log"));
    let file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)?;
    let mut command = command(root, step, plan);
    command
        .stdout(Stdio::from(file.try_clone()?))
        .stderr(Stdio::from(file));
    let mut child = command.spawn()?;
    let exit = wait(&mut child, step.timeout_seconds, &path)?;
    evidence(directory, index, step, exit)
}
fn wait(child: &mut std::process::Child, timeout: u64, output: &Path) -> Result<Option<i32>> {
    let deadline = Instant::now() + Duration::from_secs(timeout);
    let exit = loop {
        if let Some(status) = child.try_wait()? {
            break status.code();
        }
        if Instant::now() >= deadline || fs::metadata(output)?.len() >= LIMIT {
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
    c.env_remove("GITHUB_TOKEN").env_remove("GH_TOKEN");
    c
}
