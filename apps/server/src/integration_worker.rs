//! Serial validation supervision and restart reconciliation, without coding runs.
use crate::{
    execution::{Launch, Receipt},
    git_broker::GitBroker,
    integration_process::{self, Job},
    integration_store as store, process, run_store,
    validation_runner::Plan,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::path::Path;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
#[derive(sqlx::FromRow)]
struct Saved {
    id: String,
    state: String,
    job: Value,
    launch: Value,
    quiescent: bool,
}
async fn current(pool: &PgPool) -> Result<Option<Saved>> {
    Ok(sqlx::query_as("SELECT v.id,v.state,v.job,v.launch,v.quiescent FROM integration_validation v JOIN execution_control c ON c.requirement_id=v.requirement_id WHERE c.id=1 ORDER BY v.created_at DESC,v.id DESC LIMIT 1") .fetch_optional(pool).await?)
}
pub async fn tick(
    pool: &PgPool,
    root: &Path,
    supervisor: &Path,
    broker: &GitBroker,
    incarnation: &str,
    plan: &Plan,
) -> Result<bool> {
    if let Some(saved) = current(pool).await? {
        let repairing: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM linked_failure WHERE integration_id=$1 AND (state IN ('reserved','merged','complete') OR (state='observed' AND baseline IS NOT NULL)))")
            .bind(&saved.id).fetch_one(pool).await?;
        if repairing {
            return Ok(false);
        }
        advance(pool, root, supervisor, incarnation, saved).await?;
        return Ok(true);
    }
    if !crate::storage::permit(pool, root).await {
        return Ok(false);
    }
    crate::integration_claim::claim(pool, root, supervisor, broker, incarnation, plan).await
}
async fn advance(
    pool: &PgPool,
    root: &Path,
    supervisor: &Path,
    incarnation: &str,
    saved: Saved,
) -> Result<()> {
    let job: Job = serde_json::from_value(saved.job.clone())?;
    let mut launch: Launch = serde_json::from_value(saved.launch.clone())?;
    rebind_prepared(pool, incarnation, &saved, &mut launch).await?;
    let directory = root.join(&saved.id);
    if saved.quiescent {
        return completed(pool, root, supervisor, incarnation, &saved, &job, &launch).await;
    }
    if saved.state == "prepared" {
        start(pool, supervisor, &directory, &saved, &job, &launch).await?;
    }
    observe(pool, &directory, &saved, &job, &launch, false).await?;
    Ok(())
}
async fn rebind_prepared(
    pool: &PgPool,
    incarnation: &str,
    saved: &Saved,
    launch: &mut Launch,
) -> Result<()> {
    if saved.state == "prepared" && launch.key.incarnation != incarnation {
        launch.key.incarnation = incarnation.into();
        sqlx::query("UPDATE integration_validation SET launch=$2 WHERE id=$1 AND state='prepared' AND launch=$3")
            .bind(&saved.id).bind(json!(launch)).bind(&saved.launch).execute(pool).await?;
    }
    Ok(())
}
async fn completed(
    pool: &PgPool,
    root: &Path,
    supervisor: &Path,
    incarnation: &str,
    saved: &Saved,
    job: &Job,
    launch: &Launch,
) -> Result<()> {
    let directory = root.join(&saved.id);
    if directory.join("outcome.json").is_file() {
        let outcome = integration_process::reconcile(&directory, job)?;
        store::finish(pool, &saved.id, job, &outcome).await?;
    }
    crate::integration_retry::resume(pool, root, supervisor, incarnation, &saved.id, job, launch)
        .await
}
async fn start(
    pool: &PgPool,
    supervisor: &Path,
    directory: &Path,
    saved: &Saved,
    job: &Job,
    launch: &Launch,
) -> Result<()> {
    if !crate::storage_service::validation(pool, &saved.id).await? {
        return Ok(());
    }
    if !begin_start(pool, saved, job).await? {
        return Ok(());
    }
    // Commit first. A lost spawn response never authorizes a second process.
    let mut child = process::spawn(supervisor, directory, launch)?;
    tokio::task::spawn_blocking(move || child.wait());
    process::durable_write(&directory.join("job.json"), job)?;
    Ok(())
}
async fn begin_start(pool: &PgPool, saved: &Saved, job: &Job) -> Result<bool> {
    let mut tx = run_store::lock(pool).await?;
    if !store::allowed(&mut tx, job).await? {
        return Ok(false);
    }
    let changed = sqlx::query(
        "UPDATE integration_validation SET state='executing' WHERE id=$1 AND state='prepared'",
    )
    .bind(&saved.id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    if changed.rows_affected() != 1 {
        return Ok(false);
    }
    Ok(true)
}
async fn observe(
    pool: &PgPool,
    directory: &Path,
    saved: &Saved,
    job: &Job,
    launch: &Launch,
    stopping: bool,
) -> Result<bool> {
    let receipt: Option<Receipt> = process::read(&directory.join("identity.json")).ok();
    let Some(receipt) = receipt else {
        if saved.state == "prepared" {
            return Ok(true);
        }
        block(
            pool,
            &saved.id,
            "validation process identity unknown; reconcile original supervisor and output",
        )
        .await?;
        return Ok(false);
    };
    if quiescence(pool, directory, saved, launch, &receipt).await? {
        return Ok(true);
    }
    if process::identity(receipt.process.pid).ok().as_ref() != Some(&receipt.process) {
        block(
            pool,
            &saved.id,
            "validation supervisor lost; descendant quiescence unknown",
        )
        .await?;
        return Ok(false);
    }
    control(pool, directory, job, launch, stopping).await?;
    Ok(false)
}
async fn quiescence(
    pool: &PgPool,
    directory: &Path,
    saved: &Saved,
    launch: &Launch,
    receipt: &Receipt,
) -> Result<bool> {
    if receipt.key != launch.key {
        return Err("validation process identity conflict".into());
    }
    let saved_process = sqlx::query("UPDATE integration_validation SET process_identity=$2 WHERE id=$1 AND (process_identity IS NULL OR process_identity=$2)")
        .bind(&saved.id).bind(json!(receipt.process)).execute(pool).await?;
    crate::budget_store::require(
        saved_process.rows_affected() == 1,
        "saved validation process identity differs",
    )?;
    if let Ok(quiescent) = process::read::<Receipt>(&directory.join("quiescent.json")) {
        if quiescent != *receipt {
            return Err("validation stop receipt identity conflict".into());
        }
        sqlx::query(
            "UPDATE integration_validation SET quiescent=true WHERE id=$1 AND process_identity=$2",
        )
        .bind(&saved.id)
        .bind(json!(receipt.process))
        .execute(pool)
        .await?;
        return Ok(true);
    }
    Ok(false)
}
async fn control(
    pool: &PgPool,
    directory: &Path,
    job: &Job,
    launch: &Launch,
    stopping: bool,
) -> Result<()> {
    if control_allowed(pool, job, stopping).await? {
        renew(pool, directory, launch).await?;
    } else {
        process::durable_write(&directory.join("stop.json"), &true)?;
    }
    Ok(())
}
async fn control_allowed(pool: &PgPool, job: &Job, stopping: bool) -> Result<bool> {
    let permit = crate::storage::allowed(pool).await;
    let mut tx = run_store::lock(pool).await?;
    let allowed = !stopping && permit && store::allowed(&mut tx, job).await?;
    tx.commit().await?;
    Ok(allowed)
}
async fn renew(pool: &PgPool, directory: &Path, launch: &Launch) -> Result<()> {
    crate::storage::write(pool, &directory.join("storage-heartbeat.json"), &launch.key).await?;
    if directory.join("job.json").is_file() && !directory.join("stop.json").exists() {
        crate::storage::write(pool, &directory.join("start.json"), &launch.key).await?;
    }
    Ok(())
}
pub(crate) async fn recover(pool: &PgPool, root: &Path, incarnation: &str) -> Result<bool> {
    let Some(saved) = current(pool).await? else {
        return Ok(true);
    };
    if saved.quiescent || saved.state == "prepared" {
        return Ok(true);
    }
    let job: Job = serde_json::from_value(saved.job.clone())?;
    let launch: Launch = serde_json::from_value(saved.launch.clone())?;
    let old = launch.key.incarnation != incarnation;
    let stopped = observe(pool, &root.join(&saved.id), &saved, &job, &launch, old).await?;
    Ok(!old || stopped)
}
async fn block(pool: &PgPool, id: &str, reason: &str) -> Result<()> {
    sqlx::query("UPDATE integration_validation SET state='unknown',blocker=$2 WHERE id=$1 AND NOT quiescent").bind(id).bind(reason).execute(pool).await?;
    Ok(())
}
