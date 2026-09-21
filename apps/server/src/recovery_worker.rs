//! Prepare one classified failure using the existing repair execution path.
use crate::{git_broker::GitBroker, runtime_service::Config};
use serde_json::Value;
use sqlx::PgPool;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub async fn plan(
    pool: &PgPool,
    broker: &GitBroker,
    incarnation: &str,
    config: &Config,
) -> Result<()> {
    reconcile_unstarted(pool, broker, incarnation, config).await?;
    let row: Option<(String,i64,i64,String,Value)> = sqlx::query_as("SELECT f.event_key,f.requirement_id,v.revision,v.source_run_id,s.manifest FROM recovery_failure f JOIN candidate_validation v ON v.id=f.source_validation_id JOIN workspace_snapshot s ON s.run_id=v.source_run_id JOIN execution_control c ON c.requirement_id=f.requirement_id WHERE f.decision='code' AND NOT EXISTS(SELECT 1 FROM repair_reservation p WHERE p.requirement_id=f.requirement_id AND p.status IN ('reserved','started')) ORDER BY f.created_at,f.event_key LIMIT 1")
        .fetch_optional(pool).await?;
    let Some((event, id, revision, source, manifest)) = row else {
        return Ok(());
    };
    let job = crate::runtime_resume::create(
        source,
        serde_json::from_value(manifest)?,
        broker,
        incarnation,
        id,
        revision,
        &config.launcher()?,
    )?;
    crate::recovery_store::reserve(pool, &event, &job, config.settings.reservation).await?;
    Ok(())
}

async fn reconcile_unstarted(
    pool: &PgPool,
    broker: &GitBroker,
    incarnation: &str,
    config: &Config,
) -> Result<()> {
    let mut tx = crate::run_store::lock(pool).await?;
    let row: Option<(i64,i64,String,Value,Value,Value,i64)> = sqlx::query_as("SELECT p.requirement_id,v.revision,v.source_run_id,s.manifest,p.launch,p.workspace,p.ordinal FROM repair_reservation p JOIN candidate_validation v ON v.id=p.source_validation_id JOIN workspace_snapshot s ON s.run_id=v.source_run_id WHERE p.status='reserved' AND p.launch IS NOT NULL AND p.launch#>>'{key,incarnation}'<>$1 AND NOT EXISTS(SELECT 1 FROM agent_run a WHERE a.id=p.launch#>>'{key,run_id}') ORDER BY p.requirement_id LIMIT 1")
        .bind(incarnation).fetch_optional(&mut *tx).await?;
    let Some((id, revision, source, manifest, old_launch, old_workspace, ordinal)) = row else {
        return Ok(());
    };
    if !crate::recovery_store::allowed(&mut tx, id, revision, incarnation).await? {
        return Ok(());
    }
    let job = replacement(source, manifest, broker, incarnation, id, revision, config)?;
    if !crate::storage_service::reserve_workspace(&mut tx, &job.workspace).await? {
        return Ok(());
    }
    save_replacement(tx, id, ordinal, old_launch, old_workspace, &job).await
}
fn replacement(
    source: String,
    manifest: Value,
    broker: &GitBroker,
    incarnation: &str,
    id: i64,
    revision: i64,
    config: &Config,
) -> Result<crate::runtime_resume::Job> {
    crate::runtime_resume::create(
        source,
        serde_json::from_value(manifest)?,
        broker,
        incarnation,
        id,
        revision,
        &config.launcher()?,
    )
}
async fn save_replacement(
    mut tx: sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
    ordinal: i64,
    old_launch: Value,
    old_workspace: Value,
    job: &crate::runtime_resume::Job,
) -> Result<()> {
    sqlx::query("INSERT INTO repair_intent_history(requirement_id,ordinal,launch,workspace,reason) VALUES($1,$2,$3,$4,'cold-start barrier complete; no Run was dispatched')")
        .bind(id).bind(ordinal).bind(old_launch).bind(old_workspace).execute(&mut *tx).await?;
    sqlx::query("UPDATE repair_reservation SET launch=$3,workspace=$4 WHERE requirement_id=$1 AND ordinal=$2")
        .bind(id).bind(ordinal).bind(serde_json::json!(job.launch)).bind(serde_json::json!(job.workspace)).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}
