//! Durable recovery work; restoring paid work never reuses an old RPC or process.
use crate::{
    execution::Launch,
    git_broker::GitBroker,
    process, run_store, runtime_questions, runtime_store,
    workspace::{Manifest, Workspace},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::path::Path;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Clone, Serialize, Deserialize)]
pub struct Job {
    pub source: String,
    pub launch: Launch,
    pub workspace: Workspace,
    pub manifest: Manifest,
}

pub async fn next(
    pool: &PgPool,
    broker: &GitBroker,
    incarnation: &str,
    launcher: &[String],
) -> Result<Option<Job>> {
    let mut tx = run_store::lock(pool).await?;
    let Some((source, requirement, revision)) = eligible(&mut tx, incarnation).await? else {
        return Ok(None);
    };
    if let Some(saved) = saved(&mut tx, &source, incarnation).await? {
        return Ok(saved);
    }
    let job = allocate(
        &mut tx,
        source,
        broker,
        incarnation,
        requirement,
        revision,
        launcher,
    )
    .await?;
    commit_restore(tx, pool, broker, job).await
}
async fn commit_restore(
    mut tx: sqlx::Transaction<'_, sqlx::Postgres>,
    pool: &PgPool,
    broker: &GitBroker,
    job: Job,
) -> Result<Option<Job>> {
    if !crate::storage_service::reserve_workspace(&mut tx, &job.workspace).await? {
        tx.rollback().await?;
        crate::storage_service::block(
            pool,
            "recovery storage allocation unavailable; retain source checkpoint",
        )
        .await?;
        return Ok(None);
    }
    tx.commit().await?;
    restore(pool, broker, job).await.map(Some)
}
async fn eligible(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    incarnation: &str,
) -> Result<Option<(String, i64, i64)>> {
    let sources: Vec<String> = sqlx::query_scalar("SELECT a.id FROM agent_run a WHERE a.quiescent AND (a.user_paused OR a.storage_resume_requested OR EXISTS(SELECT 1 FROM runtime_question q WHERE q.run_id=a.id AND q.resume_state='pending')) ORDER BY a.run_sequence").fetch_all(&mut **tx).await?;
    for source in sources {
        if let Some((id, revision)) = runtime_questions::resumable(tx, &source, incarnation).await?
            && runtime_questions::budget_available(tx, id).await?
        {
            return Ok(Some((source, id, revision)));
        }
    }
    Ok(None)
}
async fn saved(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    source: &str,
    incarnation: &str,
) -> Result<Option<Option<Job>>> {
    let saved: Option<(Value, String)> =
        sqlx::query_as("SELECT job,status FROM runtime_resume WHERE source_run=$1")
            .bind(source)
            .fetch_optional(&mut **tx)
            .await?;
    let Some((job, status)) = saved else {
        return Ok(None);
    };
    let job: Job = serde_json::from_value(job)?;
    if status == "prepared" && job.launch.key.incarnation == incarnation {
        return Ok(Some(Some(job)));
    }
    if status == "prepared" && retire_prepared(tx, &job).await? {
        return Ok(None);
    }
    Ok(Some(None))
}
async fn retire_prepared(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    job: &Job,
) -> Result<bool> {
    // No Run/process/model was dispatched. Preserve the old job, preparation
    // records and files; a fresh identity restores the authoritative source
    // checkpoint and must pass preparation in the new incarnation again.
    let retired = sqlx::query("WITH retired AS (DELETE FROM runtime_resume WHERE source_run=$1 AND status='prepared' AND NOT EXISTS(SELECT 1 FROM agent_run WHERE id=$2) RETURNING job,status) INSERT INTO runtime_resume_history(source_run,run_id,job,status,reason) SELECT $1,$2,job,status,'controller incarnation changed before dispatch' FROM retired")
        .bind(&job.source).bind(&job.launch.key.run_id).execute(&mut **tx).await?;
    Ok(retired.rows_affected() == 1)
}
async fn allocate(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    source: String,
    broker: &GitBroker,
    incarnation: &str,
    requirement: i64,
    revision: i64,
    launcher: &[String],
) -> Result<Job> {
    let manifest: Value =
        sqlx::query_scalar("SELECT manifest FROM workspace_snapshot WHERE run_id=$1")
            .bind(&source)
            .fetch_one(&mut **tx)
            .await?;
    let job = create(
        source,
        serde_json::from_value(manifest)?,
        broker,
        incarnation,
        requirement,
        revision,
        launcher,
    )?;
    sqlx::query("INSERT INTO runtime_resume(source_run,job,status) VALUES($1,$2,'restoring')")
        .bind(&job.source)
        .bind(json!(job))
        .execute(&mut **tx)
        .await?;
    Ok(job)
}

pub(crate) fn create(
    source: String,
    manifest: Manifest,
    broker: &GitBroker,
    incarnation: &str,
    requirement: i64,
    revision: i64,
    launcher: &[String],
) -> Result<Job> {
    let (program, args) = launcher.split_first().ok_or("Runtime launcher missing")?;
    let id = process::new_identity()?;
    let key = crate::execution::RunKey {
        run_id: id.clone(),
        request_id: format!("answer-{id}"),
        incarnation: incarnation.into(),
    };
    let workspace = Workspace {
        key: key.clone(),
        identity: process::new_identity()?,
        requirement,
        revision,
        phase: "execution".into(),
        baseline: manifest.head.clone(),
        branch: format!("ai/req-{requirement}-{id}"),
        path: broker.path(&id)?.to_string_lossy().into_owned(),
    };
    let mut args = args.to_vec();
    args.push("app-server".into());
    let launch = Launch {
        key,
        workspace: workspace.path.clone(),
        workspace_identity: workspace.identity.clone(),
        program: program.clone(),
        args,
    };
    Ok(Job {
        source,
        launch,
        workspace,
        manifest,
    })
}
async fn restore(pool: &PgPool, broker: &GitBroker, job: Job) -> Result<Job> {
    let mut tx = run_store::lock(pool).await?;
    runtime_store::require(
        runtime_questions::resumable(&mut tx, &job.source, &job.launch.key.incarnation)
            .await?
            .is_some(),
        "resume authorization changed",
    )?;
    let worker = broker.clone();
    let copy = job.clone();
    tokio::task::spawn_blocking(move || worker.restore(&copy.workspace, &copy.manifest)).await??;
    sqlx::query(
        "UPDATE runtime_resume SET status='prepared' WHERE source_run=$1 AND status='restoring'",
    )
    .bind(&job.source)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(job)
}

/// Preparation calls this under the shared pause lock. An answer alone never
/// grants filesystem or model execution permission.
pub(crate) async fn preparation_allowed(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    launch: &Launch,
) -> std::result::Result<bool, sqlx::Error> {
    let source: Option<String> = sqlx::query_scalar(
        "SELECT source_run FROM runtime_resume WHERE job->'launch'=$1 AND status='prepared'",
    )
    .bind(json!(launch))
    .fetch_optional(&mut **tx)
    .await?;
    let Some(source) = source else {
        return Ok(false);
    };
    Ok(
        runtime_questions::resumable(tx, &source, &launch.key.incarnation)
            .await?
            .is_some(),
    )
}

pub async fn prepare(
    pool: &PgPool,
    root: &Path,
    broker: &GitBroker,
    job: &Job,
    adapter: &Path,
    config: Value,
) -> Result<bool> {
    let mut tx = run_store::lock(pool).await?;
    if crate::preparation_store::claim_ready(
        &mut tx,
        &job.launch,
        job.workspace.requirement,
        job.workspace.revision,
    )
    .await?
    {
        return Ok(true);
    }
    tx.commit().await?;
    crate::preparation_service::prepare(
        pool,
        crate::preparation_service::Request {
            launch: &job.launch,
            requirement: job.workspace.requirement,
            revision: job.workspace.revision,
            phase: "answer_resume",
            now: crate::runtime_client::now(),
            adapter,
            control_directory: root,
            broker,
            workspace: &job.workspace,
            config,
        },
    )
    .await
}

pub async fn bind(pool: &PgPool, job: &Job) -> Result<bool> {
    Ok(runtime_questions::reserve_resume(pool, &job.source, &job.launch).await?)
}
