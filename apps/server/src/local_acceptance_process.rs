//! Reuse the fixed-version validator and subreaper. An interrupted launch is
//! observed/stopped using its original identity, never launched a second time.
use crate::{
    delivery_extension::Result,
    execution::{Launch, RunKey},
    integration_process::{self, Job, Outcome},
    local_delivery_store::{self as store, Job as Delivery},
    process, validation_supervisor,
};
use sqlx::PgPool;
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub(crate) async fn execute(
    pool: &PgPool,
    root: &Path,
    delivery: &Delivery,
    job: &Job,
    claimed: bool,
) -> Result<Outcome> {
    let directory = root.join(&job.invocation);
    let key = RunKey {
        run_id: job.invocation.clone(),
        request_id: job.invocation.clone(),
        incarnation: job.invocation.clone(),
    };
    start_or_reconcile(&directory, job, &key, claimed)?;
    wait(pool, delivery, &directory, &key).await?;
    sqlx::query("UPDATE delivery SET local_acceptance_quiescent=true WHERE action_key=$1")
        .bind(&delivery.action_key)
        .execute(pool)
        .await?;
    if directory.join("stop.json").exists() || directory.join("truncated.json").exists() {
        return Err("local acceptance was stopped or truncated".into());
    }
    integration_process::reconcile(&directory, job)
}
fn start_or_reconcile(directory: &Path, job: &Job, key: &RunKey, claimed: bool) -> Result<()> {
    if claimed {
        spawn(directory, job, key)?;
    }
    if !claimed && !validation_supervisor::quiescent(directory, key)? {
        stop(directory, key)?;
        return Err("local acceptance outcome unknown; original invocation retained".into());
    }
    Ok(())
}

fn spawn(directory: &Path, job: &Job, key: &RunKey) -> Result<()> {
    let supervisor = match std::env::var_os("SYMPHONY_SUPERVISOR") {
        Some(path) => PathBuf::from(path),
        None => std::env::current_exe()?,
    };
    let launch = Launch {
        key: key.clone(),
        workspace: job.checkouts[0].to_string_lossy().into_owned(),
        workspace_identity: job.binding.input_sha256.clone(),
        program: supervisor.to_string_lossy().into_owned(),
        args: Vec::from([
            "--integration-validation".into(),
            directory.to_string_lossy().into_owned(),
        ]),
    };
    let child = process::spawn(&supervisor, directory, &launch)?;
    tokio::spawn(reap(child));
    process::durable_write(&directory.join("job.json"), job)?;
    Ok(())
}
async fn reap(mut child: std::process::Child) {
    while let Ok(None) = child.try_wait() {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
fn stop(directory: &Path, key: &RunKey) -> Result<()> {
    std::fs::create_dir_all(directory)?;
    process::durable_write(&directory.join("stop.json"), key)?;
    Ok(())
}
async fn wait(pool: &PgPool, job: &Delivery, directory: &Path, key: &RunKey) -> Result<()> {
    let startup = Instant::now();
    let mut stopping = None;
    loop {
        if validation_supervisor::quiescent(directory, key)? {
            return Ok(());
        }
        if !allowed(pool, job).await.unwrap_or(false)
            || startup.elapsed() >= Duration::from_secs(10)
                && !directory.join("identity.json").exists()
        {
            stop(directory, key)?;
            if stopping.is_none() {
                stopping = Some(Instant::now());
            }
        }
        heartbeat(directory, key, stopping)?;
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
fn heartbeat(directory: &Path, key: &RunKey, stopping: Option<Instant>) -> Result<()> {
    if let Some(started) = stopping
        && started.elapsed() >= Duration::from_secs(20)
    {
        return Err("local acceptance stop unknown; retain owner and original process".into());
    }
    process::durable_write(&directory.join("storage-heartbeat.json"), key)?;
    if directory.join("identity.json").exists() {
        process::durable_write(&directory.join("start.json"), key)?;
    }
    Ok(())
}
async fn allowed(pool: &PgPool, job: &Delivery) -> Result<bool> {
    let mut tx = crate::run_store::lock(pool).await?;
    store::allowed(&mut tx, job).await
}

pub(crate) async fn reconcile_started(
    pool: &PgPool,
    root: &Path,
    delivery: &Delivery,
) -> Result<()> {
    if !delivery.local_acceptance_started {
        return Ok(());
    }
    let value: serde_json::Value =
        sqlx::query_scalar("SELECT local_acceptance_job FROM delivery WHERE action_key=$1")
            .bind(&delivery.action_key)
            .fetch_one(pool)
            .await?;
    let job: Job = serde_json::from_value(value)?;
    let directory = root.join(&job.invocation);
    let key = RunKey {
        run_id: job.invocation.clone(),
        request_id: job.invocation.clone(),
        incarnation: job.invocation.clone(),
    };
    if !validation_supervisor::quiescent(&directory, &key)? {
        stop(&directory, &key)?;
        return Err("local acceptance process not stopped; reconcile original invocation".into());
    }
    sqlx::query("UPDATE delivery SET local_acceptance_quiescent=true WHERE action_key=$1")
        .bind(&delivery.action_key)
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
#[path = "../tests/unit/local_acceptance_process.rs"]
mod tests;
