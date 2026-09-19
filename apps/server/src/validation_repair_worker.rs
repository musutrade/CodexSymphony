//! Repair launch preparation reuses the Runtime preflight and budget boundary.
use crate::{
    git_broker::GitBroker, runtime_resume::Job, runtime_service::Config,
    validation_repair as repair,
};
use serde_json::Value;
use sqlx::PgPool;
use std::path::Path;
type PendingRepair = (i64, i64, String, Value, Option<Value>, Option<Value>);
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub async fn tick(
    pool: &PgPool,
    root: &Path,
    broker: &GitBroker,
    incarnation: &str,
    config: &Config,
) -> Result<()> {
    let Some(job) = job(pool, broker, incarnation, &config.launcher()?).await? else {
        return Ok(());
    };
    if !crate::storage_service::admit_workspace(pool, &job.workspace).await? {
        return Ok(());
    }
    restore(broker, &job)?;
    if repair::bind(pool, &job.launch).await? {
        return Ok(());
    }
    prepare(pool, root, broker, config, &job).await
}
async fn prepare(
    pool: &PgPool,
    root: &Path,
    broker: &GitBroker,
    config: &Config,
    job: &Job,
) -> Result<()> {
    if crate::preparation_service::prepare(
        pool,
        crate::preparation_service::Request {
            launch: &job.launch,
            requirement: job.workspace.requirement,
            revision: job.workspace.revision,
            phase: "code_repair",
            now: crate::runtime_client::now(),
            adapter: &config.preparation_adapter,
            control_directory: root,
            broker,
            workspace: &job.workspace,
            config: config.preparation.clone(),
        },
    )
    .await?
    {
        repair::bind(pool, &job.launch).await?;
    }
    Ok(())
}
fn restore(broker: &GitBroker, job: &Job) -> Result<()> {
    if !Path::new(&job.workspace.path).exists() {
        broker.restore_candidate(&job.workspace, &job.manifest)?;
    }
    if broker.head(&job.workspace)? != job.manifest.head {
        return Err("repair must begin at fixed failed candidate".into());
    }
    Ok(())
}

async fn job(
    pool: &PgPool,
    broker: &GitBroker,
    incarnation: &str,
    launcher: &[String],
) -> Result<Option<Job>> {
    let row:Option<PendingRepair>=sqlx::query_as("SELECT p.requirement_id,v.revision,v.source_run_id,s.manifest,p.launch,p.workspace FROM repair_reservation p JOIN candidate_validation v ON v.id=p.source_validation_id JOIN workspace_snapshot s ON s.run_id=v.source_run_id WHERE p.status='reserved' ORDER BY p.requirement_id LIMIT 1").fetch_optional(pool).await?;
    let Some((id, revision, source, manifest, launch, workspace)) = row else {
        return Ok(None);
    };
    let manifest = serde_json::from_value(manifest)?;
    if let (Some(launch), Some(workspace)) = (launch, workspace) {
        let job = saved(source, manifest, launch, workspace, incarnation)?;
        return Ok(Some(job));
    }
    let job = crate::runtime_resume::create(
        source,
        manifest,
        broker,
        incarnation,
        id,
        revision,
        launcher,
    )?;
    if repair::plan(pool, id, &job.launch, &job.workspace).await? {
        Ok(Some(job))
    } else {
        Ok(None)
    }
}

fn saved(
    source: String,
    manifest: crate::workspace::Manifest,
    launch: Value,
    workspace: Value,
    incarnation: &str,
) -> Result<Job> {
    let job = Job {
        source,
        manifest,
        launch: serde_json::from_value(launch)?,
        workspace: serde_json::from_value(workspace)?,
    };
    if job.launch.key.incarnation != incarnation {
        return Err("repair launch belongs to previous incarnation; reconcile".into());
    }
    Ok(job)
}
