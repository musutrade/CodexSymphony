//! Durable native validation failure identity; no fabricated Run or validation.
use crate::{run_store, validation::ValidationEvidence};
use serde_json::json;
use sqlx::{PgPool, Postgres, Transaction};
type Result<T> = std::result::Result<T, sqlx::Error>;

pub(crate) async fn post_merge(
    pool: &PgPool,
    intent: &crate::automatic_merge::Intent,
    evidence: &ValidationEvidence,
    required: &[String],
) -> Result<()> {
    let mut tx = run_store::lock(pool).await?;
    if !crate::merge_store::allowed(&mut tx, intent).await? {
        return Ok(());
    }
    let eligible: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM merge_operation WHERE action_key=$1 AND merged_sha=$2 AND (state='merged' OR (state='blocked' AND blocker='post_merge acceptance needs original-item review: post_merge binding rejected: ExitFailed' AND acceptance IS NULL)))")
        .bind(intent.action_key()).bind(&evidence.candidate.sha).fetch_one(&mut *tx).await?;
    if !eligible {
        return Ok(());
    }
    sqlx::query("UPDATE linked_failure SET state='merged',final_version=$2 WHERE repair_delivery=$1 AND state='reserved'")
        .bind(&intent.delivery_key).bind(json!({"candidate":evidence.candidate,"failed_evidence":evidence,"pr":intent.pr})).execute(&mut *tx).await?;
    let id = format!("post-merge:{}", intent.action_key());
    record(
        &mut tx,
        &id,
        intent.requirement,
        intent.revision,
        Some(&intent.action_key()),
        None,
        evidence,
        required,
    )
    .await?;
    // The merge remains a real merged fact. Its failed invocation is retained
    // verbatim and cannot be replaced by a later passing checkout.
    sqlx::query("UPDATE merge_operation SET state='blocked',acceptance=$2,blocker='original-item linked repair pending' WHERE action_key=$1 AND (state='merged' OR (state='blocked' AND blocker='post_merge acceptance needs original-item review: post_merge binding rejected: ExitFailed'))")
        .bind(intent.action_key()).bind(json!({"failed_evidence":evidence})).execute(&mut *tx).await?;
    tx.commit().await
}
#[allow(clippy::too_many_arguments)]
pub(crate) async fn record(
    tx: &mut Transaction<'_, Postgres>,
    id: &str,
    requirement: i64,
    revision: i64,
    merge: Option<&str>,
    integration: Option<&str>,
    evidence: &ValidationEvidence,
    required: &[String],
) -> Result<()> {
    sqlx::query("INSERT INTO linked_failure(id,requirement_id,revision,merge_key,integration_id,evidence,required_steps) VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT(id) DO NOTHING")
        .bind(id).bind(requirement).bind(revision).bind(merge).bind(integration).bind(json!(evidence)).bind(json!(required)).execute(&mut **tx).await?;
    let same: bool = sqlx::query_scalar("SELECT requirement_id=$2 AND revision=$3 AND merge_key IS NOT DISTINCT FROM $4 AND integration_id IS NOT DISTINCT FROM $5 AND evidence=$6 AND required_steps=$7 FROM linked_failure WHERE id=$1")
        .bind(id).bind(requirement).bind(revision).bind(merge).bind(integration).bind(json!(evidence)).bind(json!(required)).fetch_one(&mut **tx).await?;
    crate::budget_store::require(same, "linked failure identity conflict")
}
