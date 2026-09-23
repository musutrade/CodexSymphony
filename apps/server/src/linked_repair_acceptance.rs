//! A repair merge updates this item's versions; it never edits another child's fact.
use crate::{automatic_merge::Intent, validation::ValidationEvidence};
use serde_json::json;
use sqlx::{Postgres, Transaction};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
pub(crate) async fn finish(
    tx: &mut Transaction<'_, Postgres>,
    intent: &Intent,
    evidence: &ValidationEvidence,
) -> Result<bool> {
    let linked: Option<String> = sqlx::query_scalar("UPDATE linked_failure SET state='merged',final_version=$2 WHERE repair_delivery=$1 AND state='reserved' RETURNING id")
        .bind(&intent.delivery_key).bind(json!({"candidate":evidence.candidate,"evidence":evidence,"pr":intent.pr,"merge_key":intent.action_key()})).fetch_optional(&mut **tx).await?;
    let Some(_) = linked else {
        return Ok(false);
    };
    let integration: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM group_execution_item WHERE requirement_id=$1 AND input#>>'{child,kind}'='validation_only')").bind(intent.requirement).fetch_one(&mut **tx).await?;
    if integration {
        finish_integration(tx, intent).await?;
    } else {
        sqlx::query("UPDATE linked_failure SET state='complete',final_version=$2 WHERE requirement_id=$1 AND state IN ('merged','reserved')")
            .bind(intent.requirement).bind(json!({"candidate":evidence.candidate,"evidence":evidence,"pr":intent.pr,"merge_key":intent.action_key()})).execute(&mut **tx).await?;
    }
    Ok(integration)
}

async fn finish_integration(tx: &mut Transaction<'_, Postgres>, intent: &Intent) -> Result<()> {
    sqlx::query("UPDATE merge_operation SET state='complete',blocker=NULL WHERE action_key=$1 AND state='merged'").bind(intent.action_key()).execute(&mut **tx).await?;
    sqlx::query("UPDATE requirement SET state='Running' WHERE id=$1 AND state='Submitted'")
        .bind(intent.requirement)
        .execute(&mut **tx)
        .await?;
    sqlx::query("UPDATE candidate_validation SET stage='done' WHERE id=$1 AND result='succeeded'")
        .bind(&intent.validation_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
#[cfg(test)]
#[path = "../tests/unit/linked_repair_acceptance.rs"]
mod tests;
