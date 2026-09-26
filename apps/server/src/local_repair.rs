//! Local delivered-version failures use the existing original-item repair quota,
//! reviewed paths and version-combination revalidation, without PR/merge records.
use crate::{
    delivery_extension::Result, local_delivery_store::Job, validation::ValidationEvidence,
};
use serde_json::json;
type Tx<'a> = sqlx::Transaction<'a, sqlx::Postgres>;

pub(crate) async fn failed(
    tx: &mut Tx<'_>,
    job: &Job,
    evidence: &ValidationEvidence,
) -> Result<()> {
    let required: serde_json::Value =
        sqlx::query_scalar("SELECT required_steps FROM candidate_validation WHERE id=$1")
            .bind(&job.validation_id)
            .fetch_one(&mut **tx)
            .await?;
    let checks: Vec<String> = serde_json::from_value(required.clone())?;
    if !crate::linked_repair::failed_code(evidence, &checks) {
        return Ok(());
    }
    let grouped: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM group_execution_item WHERE requirement_id=$1)",
    )
    .bind(job.requirement_id)
    .fetch_one(&mut **tx)
    .await?;
    if grouped {
        record_failure(tx, job, evidence, &required).await?;
    }
    Ok(())
}
async fn record_failure(
    tx: &mut Tx<'_>,
    job: &Job,
    evidence: &ValidationEvidence,
    required: &serde_json::Value,
) -> Result<()> {
    sqlx::query("UPDATE linked_failure SET state='merged',final_version=$2 WHERE repair_delivery=$1 AND state='reserved'").bind(&job.action_key).bind(json!({"candidate":evidence.candidate,"failed_evidence":evidence,"local_delivery":job.action_key})).execute(&mut **tx).await?;
    let id = format!("post-local:{}", job.action_key);
    sqlx::query("INSERT INTO linked_failure(id,requirement_id,revision,local_delivery,evidence,required_steps) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT(id) DO NOTHING").bind(&id).bind(job.requirement_id).bind(job.revision).bind(&job.action_key).bind(json!(evidence)).bind(required).execute(&mut **tx).await?;
    let same: bool=sqlx::query_scalar("SELECT requirement_id=$2 AND revision=$3 AND local_delivery=$4 AND evidence=$5 AND required_steps=$6 FROM linked_failure WHERE id=$1").bind(&id).bind(job.requirement_id).bind(job.revision).bind(&job.action_key).bind(json!(evidence)).bind(required).fetch_one(&mut **tx).await?;
    crate::budget_store::require(same, "local failure identity changed")?;
    Ok(())
}

pub(crate) async fn accepted(
    tx: &mut Tx<'_>,
    job: &Job,
    evidence: &ValidationEvidence,
) -> Result<bool> {
    let linked: Option<String>=sqlx::query_scalar("UPDATE linked_failure SET state='merged',final_version=$2 WHERE repair_delivery=$1 AND state='reserved' RETURNING id").bind(&job.action_key).bind(json!({"candidate":evidence.candidate,"evidence":evidence,"local_delivery":job.action_key})).fetch_optional(&mut **tx).await?;
    if linked.is_none() {
        return Ok(false);
    }
    let integration: bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM group_execution_item WHERE requirement_id=$1 AND input#>>'{child,kind}'='validation_only')").bind(job.requirement_id).fetch_one(&mut **tx).await?;
    finish_repair(tx, job, integration).await?;
    Ok(integration)
}
async fn finish_repair(tx: &mut Tx<'_>, job: &Job, integration: bool) -> Result<()> {
    sqlx::query("UPDATE delivery SET released=true WHERE action_key=$1 OR action_key IN (SELECT local_delivery FROM linked_failure WHERE requirement_id=$2 AND state IN ('merged','complete'))").bind(&job.action_key).bind(job.requirement_id).execute(&mut **tx).await?;
    if integration {
        sqlx::query("UPDATE requirement SET state='Running' WHERE id=$1 AND state='Submitted'")
            .bind(job.requirement_id)
            .execute(&mut **tx)
            .await?;
    } else {
        sqlx::query(
            "UPDATE linked_failure SET state='complete' WHERE requirement_id=$1 AND state='merged'",
        )
        .bind(job.requirement_id)
        .execute(&mut **tx)
        .await?;
    }
    sqlx::query("UPDATE candidate_validation SET stage='done' WHERE id=$1")
        .bind(&job.validation_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
