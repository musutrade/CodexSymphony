//! Validation uses the existing subreaper. A completed script is insufficient:
//! descendants must be stopped before its evidence can enter the ledger.
use crate::{
    execution::{Launch, Receipt, RunKey},
    process,
    validation::{Candidate, StepEvidence},
    validation_context::Context,
    validation_runner::Plan,
};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Serialize, Deserialize)]
pub struct Job {
    pub context: Context,
    pub candidate: Candidate,
    pub plan: Plan,
    pub limit: u64,
}

pub fn run(directory: &Path) -> Result<()> {
    let job: Job = process::read(&directory.join("input.json"))?;
    if directory != job.context.directory {
        return Err("validation process directory differs".into());
    }
    crate::validation_hook::execute_limited(
        &job.context.checkout,
        directory,
        &job.candidate,
        &job.plan,
        &job.context.call,
        job.limit,
    )?;
    Ok(())
}

pub async fn execute(pool: &PgPool, job: Job, claimed: bool) -> Result<Vec<StepEvidence>> {
    let directory = &job.context.directory;
    let id = &job.context.call.identity;
    let key = RunKey {
        run_id: id.invocation_id.clone(),
        request_id: id.invocation_id.clone(),
        incarnation: id.invocation_id.clone(),
    };
    settle(pool, &job, &key, claimed).await?;
    require_complete(directory, &key)?;
    crate::validation_context::check_current(pool, &job.context, "validation").await?;
    let (_, steps) = crate::validation_hook::execute_limited(
        &job.context.checkout,
        directory,
        &job.candidate,
        &job.plan,
        &job.context.call,
        job.limit,
    )?;
    Ok(steps)
}

async fn settle(pool: &PgPool, job: &Job, key: &RunKey, claimed: bool) -> Result<()> {
    let directory = &job.context.directory;
    if claimed {
        spawn(job, key)?;
        wait(pool, &job.context, key).await?;
    } else {
        // Do not revive or repeat an interrupted invocation after restart.
        if !quiescent(directory, key)? {
            process::durable_write(&directory.join("stop.json"), key)?;
            return Err("validation process outcome unknown; retain original invocation".into());
        }
    }
    Ok(())
}

fn spawn(job: &Job, key: &RunKey) -> Result<()> {
    let directory = &job.context.directory;
    std::fs::create_dir_all(directory)?;
    if directory.join("input.json").exists() {
        return Err("validation launch already has a durable intent".into());
    }
    process::durable_write(&directory.join("input.json"), job)?;
    process::durable_write(&directory.join("hook.json"), &job.context.call)?;
    process::durable_write(&directory.join("hook-limit.json"), &job.limit)?;
    let supervisor = std::env::var_os("SYMPHONY_SUPERVISOR")
        .map(PathBuf::from)
        .unwrap_or(std::env::current_exe()?);
    let launch = Launch {
        key: key.clone(),
        workspace: job.context.checkout.to_string_lossy().into_owned(),
        workspace_identity: job.context.call.policy_digest.clone(),
        program: supervisor.to_string_lossy().into_owned(),
        args: Vec::from([
            "--validation-hook".into(),
            directory.to_string_lossy().into_owned(),
        ]),
    };
    let mut child = process::spawn(&supervisor, directory, &launch)?;
    tokio::task::spawn_blocking(move || child.wait());
    Ok(())
}

async fn wait(pool: &PgPool, context: &Context, key: &RunKey) -> Result<()> {
    let directory = &context.directory;
    let mut stop_started = None;
    let startup = Instant::now();
    loop {
        if quiescent(directory, key)? {
            return Ok(());
        }
        check_startup(directory, key, startup)?;
        let allowed = invocation_allowed(pool, context).await;
        if !allowed {
            process::durable_write(&directory.join("stop.json"), key)?;
            stop_started.get_or_insert_with(Instant::now);
        }
        if stop_started.is_some_and(|started| {
            Instant::now().duration_since(started) >= Duration::from_secs(20)
        }) {
            return Err("validation stop unknown; retain process and evidence".into());
        }
        heartbeat(directory, key)?;
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

async fn invocation_allowed(pool: &PgPool, context: &Context) -> bool {
    let id = &context.call.identity;
    crate::runtime_client::now().saturating_mul(1000) < context.call.deadline_unix_ms
        && crate::validation_context::allowed(pool, id.requirement_id, id.revision)
            .await
            .unwrap_or(false)
}

fn check_startup(directory: &Path, key: &RunKey, startup: Instant) -> Result<()> {
    if startup.elapsed() >= Duration::from_secs(10) && !directory.join("identity.json").exists() {
        process::durable_write(&directory.join("stop.json"), key)?;
        return Err("validation supervisor startup identity unknown".into());
    }
    Ok(())
}

pub fn quiescent(directory: &Path, key: &RunKey) -> Result<bool> {
    if !directory.join("identity.json").exists() {
        return Ok(false);
    }
    let identity: Receipt = process::read(&directory.join("identity.json"))?;
    if identity.key != *key {
        return Err("validation supervisor identity differs".into());
    }
    if !directory.join("quiescent.json").exists() {
        return Ok(false);
    }
    let stopped: Receipt = process::read(&directory.join("quiescent.json"))?;
    if identity != stopped {
        return Err("validation supervisor stop identity differs".into());
    }
    Ok(true)
}

pub fn require_complete(directory: &Path, key: &RunKey) -> Result<()> {
    if !quiescent(directory, key)?
        || directory.join("stop.json").exists()
        || directory.join("truncated.json").exists()
    {
        return Err("validation stopped or quiescence unproven".into());
    }
    let exit: Option<i32> = process::read(&directory.join("exit.json"))?;
    if exit != Some(0) {
        return Err("validation supervisor did not complete normally".into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "../tests/unit/validation_supervisor.rs"]
mod tests;
