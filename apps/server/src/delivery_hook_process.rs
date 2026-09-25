//! Credential-free auxiliary delivery checks using the existing subreaper.
use crate::{
    controlled_contract::{Call, Evaluation},
    execution::{Launch, RunKey},
    process,
    validation::Candidate,
    validation_runner::Plan,
};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
type Result<T> = crate::delivery_extension::Result<T>;

#[derive(Serialize, Deserialize)]
pub struct Job {
    pub call: Call,
    pub checkout: PathBuf,
    pub directory: PathBuf,
    pub candidate: Candidate,
    pub plan: Plan,
}

pub fn run(directory: &Path) -> Result<()> {
    let job: Job = process::read(&directory.join("input.json"))?;
    if directory != job.directory {
        return Err("delivery hook directory mismatch".into());
    }
    if job.call.candidate.as_ref().is_none_or(|source| {
        source.commit != job.candidate.sha || source.tree != job.candidate.tree
    }) || crate::runtime_client::now().saturating_mul(1000) >= job.call.deadline_unix_ms
        || job.call.implementation_digest != job.plan.entry_sha256
    {
        return Err("delivery hook candidate, implementation or deadline mismatch".into());
    }
    let steps = crate::validation_runner::execute_limited(
        &job.checkout,
        directory,
        &job.candidate,
        &job.plan,
        1024 * 1024,
    )?;
    let evaluation = crate::validation_hook::evaluation(&job.call, &steps);
    process::durable_write(&directory.join("evaluation.json"), &evaluation)?;
    Ok(())
}

pub async fn execute(pool: &PgPool, job: &Job, claimed: bool) -> Result<Evaluation> {
    let id = &job.call.identity.invocation_id;
    let key = RunKey {
        run_id: id.clone(),
        request_id: id.clone(),
        incarnation: id.clone(),
    };
    if claimed {
        spawn(job, &key)?;
        wait(pool, job, &key).await?;
    }
    // On restart consume only the original result with a complete stop proof;
    // absence is unknown, never permission to launch a second process.
    crate::validation_supervisor::require_complete(&job.directory, &key)?;
    if crate::validation_runner::candidate(&job.checkout)? != job.candidate {
        return Err("delivery hook changed the candidate".into());
    }
    replay(job)
}

fn replay(job: &Job) -> Result<Evaluation> {
    // Replay validates retained logs and protected implementation without
    // executing a completed Plan. Missing binding/result is not a new launch.
    if !job.directory.join("binding.json").is_file() || !job.directory.join("result.json").is_file()
    {
        return Err("delivery hook evidence incomplete".into());
    }
    let steps = crate::validation_runner::execute_limited(
        &job.checkout,
        &job.directory,
        &job.candidate,
        &job.plan,
        1024 * 1024,
    )?;
    let expected = crate::validation_hook::evaluation(&job.call, &steps);
    let saved: Evaluation = process::read(&job.directory.join("evaluation.json"))?;
    if saved != expected {
        return Err("delivery hook evaluation changed".into());
    }
    Ok(saved)
}

fn spawn(job: &Job, key: &RunKey) -> Result<()> {
    persist_launch(job)?;
    let directory = &job.directory;
    let supervisor = std::env::var_os("SYMPHONY_SUPERVISOR")
        .map(PathBuf::from)
        .unwrap_or(std::env::current_exe()?);
    let launch = Launch {
        key: key.clone(),
        workspace: job.checkout.to_string_lossy().into_owned(),
        workspace_identity: job.call.policy_digest.clone(),
        program: supervisor.to_string_lossy().into_owned(),
        args: Vec::from([
            "--delivery-hook".into(),
            directory.to_string_lossy().into_owned(),
        ]),
    };
    let child = process::spawn(&supervisor, directory, &launch)?;
    tokio::spawn(reap(child));
    Ok(())
}

fn persist_launch(job: &Job) -> Result<()> {
    let directory = &job.directory;
    std::fs::create_dir_all(directory)?;
    if directory.join("input.json").exists() {
        return Err("delivery hook already has intent".into());
    }
    if serde_json::to_vec(job)?.len() > 64 * 1024 {
        return Err("delivery hook input exceeds limit".into());
    }
    process::durable_write(&directory.join("input.json"), job)?;
    process::durable_write(&directory.join("hook.json"), &job.call)?;
    process::durable_write(&directory.join("hook-limit.json"), &(1024_u64 * 1024))?;
    Ok(())
}

async fn reap(mut child: std::process::Child) {
    while let Ok(None) = child.try_wait() {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn wait(pool: &PgPool, job: &Job, key: &RunKey) -> Result<()> {
    let started = Instant::now();
    let mut stopping = None;
    loop {
        if crate::validation_supervisor::quiescent(&job.directory, key)? {
            return Ok(());
        }
        check_startup(&job.directory, key, started)?;
        if !allowed(pool, job).await {
            process::durable_write(&job.directory.join("stop.json"), key)?;
            stopping.get_or_insert_with(Instant::now);
        }
        check_stop(stopping)?;
        heartbeat(&job.directory, key)?;
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn heartbeat(directory: &Path, key: &RunKey) -> Result<()> {
    process::durable_write(&directory.join("storage-heartbeat.json"), key)?;
    if directory.join("identity.json").exists() {
        process::durable_write(&directory.join("start.json"), key)?;
    }
    Ok(())
}

fn check_startup(directory: &Path, key: &RunKey, started: Instant) -> Result<()> {
    if !directory.join("identity.json").exists() && started.elapsed() >= Duration::from_secs(10) {
        process::durable_write(&directory.join("stop.json"), key)?;
        return Err("delivery hook startup unknown".into());
    }
    Ok(())
}
async fn allowed(pool: &PgPool, job: &Job) -> bool {
    let id = &job.call.identity;
    crate::runtime_client::now().saturating_mul(1000) < job.call.deadline_unix_ms
        && crate::validation_context::allowed(pool, id.requirement_id, id.revision)
            .await
            .unwrap_or(false)
}
fn check_stop(stopping: Option<Instant>) -> Result<()> {
    let expired = match stopping {
        Some(at) => at.elapsed() >= Duration::from_secs(20),
        None => false,
    };
    if expired {
        return Err("delivery hook stop unknown".into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "../tests/unit/delivery_hook_process.rs"]
mod tests;
