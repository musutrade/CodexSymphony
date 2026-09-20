//! One immutable repair launch intent per root Requirement. A reservation is
//! not a running Agent: preparation and the existing cumulative budget still apply.
use crate::{execution::Launch, run_store};
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Transaction};
type Result<T> = std::result::Result<T, sqlx::Error>;

pub async fn plan(
    pool: &PgPool,
    requirement: i64,
    launch: &Launch,
    workspace: &crate::workspace::Workspace,
) -> Result<bool> {
    if workspace.key != launch.key
        || workspace.path != launch.workspace
        || workspace.identity != launch.workspace_identity
        || workspace.requirement != requirement
    {
        return Ok(false);
    }
    let mut tx = run_store::lock(pool).await?;
    if !eligible(&mut tx, requirement, &launch.key.incarnation).await? {
        return Ok(false);
    }
    let result=sqlx::query("UPDATE repair_reservation SET launch=$2,workspace=$3 WHERE requirement_id=$1 AND status='reserved' AND launch IS NULL AND EXISTS(SELECT 1 FROM candidate_validation v WHERE v.id=source_validation_id AND v.candidate_sha=$4 AND v.revision=$5)")
        .bind(requirement).bind(json!(launch)).bind(json!(workspace)).bind(&workspace.baseline).bind(workspace.revision).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(result.rows_affected() == 1)
}
async fn eligible(tx: &mut Transaction<'_, Postgres>, id: i64, incarnation: &str) -> Result<bool> {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM repair_reservation p JOIN candidate_validation v ON v.id=p.source_validation_id JOIN agent_run a ON a.id=v.source_run_id JOIN requirement r ON r.id=p.requirement_id JOIN requirement_revision rev ON rev.requirement_id=r.id AND rev.revision=v.revision JOIN execution_control c ON c.requirement_id=r.id CROSS JOIN repository repo WHERE p.requirement_id=$1 AND p.status='reserved' AND v.result='gate_failed' AND r.revision=v.revision AND r.state='Running' AND NOT r.paused AND NOT c.paused AND c.incarnation=$2 AND c.recovery_complete AND repo.id=COALESCE((rev.document->>'repository_id')::bigint,1) AND NOT (repo.document->>'revoked')::boolean AND (rev.document->>'repository_version')::bigint>repo.revoked_through_version AND a.quiescent AND a.state='Succeeded' AND NOT (SELECT blocked FROM storage_guard WHERE id=1) AND NOT EXISTS(SELECT 1 FROM agent_run WHERE requirement_id=r.id AND NOT quiescent) AND NOT EXISTS(SELECT 1 FROM runtime_blocker b JOIN agent_run old ON old.id=b.run_id WHERE old.requirement_id=r.id AND NOT b.resolved))")
        .bind(id).bind(incarnation).fetch_one(&mut **tx).await
}
pub(crate) async fn preparation_allowed(
    tx: &mut Transaction<'_, Postgres>,
    launch: &Launch,
) -> Result<bool> {
    let id: Option<i64> = sqlx::query_scalar(
        "SELECT requirement_id FROM repair_reservation WHERE launch=$1 AND status='reserved'",
    )
    .bind(json!(launch))
    .fetch_optional(&mut **tx)
    .await?;
    let Some(id) = id else {
        return Ok(false);
    };
    Ok(eligible(tx, id, &launch.key.incarnation).await?
        && crate::runtime_questions::budget_available(tx, id).await?)
}
pub async fn bind(pool: &PgPool, launch: &Launch) -> Result<bool> {
    let mut tx = run_store::lock(pool).await?;
    if !preparation_allowed(&mut tx, launch).await? {
        return Ok(false);
    }
    let (id,revision,candidate):(i64,i64,String)=sqlx::query_as("SELECT p.requirement_id,v.revision,v.candidate_sha FROM repair_reservation p JOIN candidate_validation v ON v.id=p.source_validation_id WHERE p.launch=$1 AND p.status='reserved'").bind(json!(launch)).fetch_one(&mut *tx).await?;
    if !crate::preparation_store::claim_ready(&mut tx, launch, id, revision).await? {
        return Ok(false);
    }
    let workspace: Option<Value> =
        sqlx::query_scalar("SELECT workspace FROM repair_reservation WHERE launch=$1")
            .bind(json!(launch))
            .fetch_optional(&mut *tx)
            .await?;
    if workspace.as_ref().and_then(baseline) != Some(candidate.as_str()) {
        return Ok(false);
    }
    commit(tx, id, revision, launch).await
}
async fn commit(
    mut tx: Transaction<'_, Postgres>,
    id: i64,
    revision: i64,
    launch: &Launch,
) -> Result<bool> {
    run_store::insert_run(&mut tx, id, revision, launch).await?;
    sqlx::query("INSERT INTO run_workspace(run_id,identity,restored_from) SELECT $1,workspace,v.source_run_id FROM repair_reservation p JOIN candidate_validation v ON v.id=p.source_validation_id WHERE p.requirement_id=$2").bind(&launch.key.run_id).bind(id).execute(&mut *tx).await?;
    sqlx::query(
        "UPDATE repair_reservation SET repair_run_id=$2,status='started' WHERE requirement_id=$1",
    )
    .bind(id)
    .bind(&launch.key.run_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(true)
}
fn baseline(value: &Value) -> Option<&str> {
    value["baseline"].as_str()
}
