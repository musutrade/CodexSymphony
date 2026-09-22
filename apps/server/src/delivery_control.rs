//! User control intent is durable before stopping or reconciling external work.
use crate::run_store;
use sqlx::PgPool;
type Result<T> = std::result::Result<T, sqlx::Error>;
pub async fn cancel(pool: &PgPool, id: i64) -> Result<bool> {
    let mut tx = run_store::lock(pool).await?;
    let result = cancel_in(&mut tx, id).await?;
    tx.commit().await?;
    Ok(result)
}
pub(crate) async fn cancel_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
) -> Result<bool> {
    let changed=sqlx::query("UPDATE requirement SET cancel_requested=true,state='Cancelled',version=version+CASE WHEN cancel_requested THEN 0 ELSE 1 END WHERE id=$1").bind(id).execute(&mut **tx).await?;
    sqlx::query(
        "UPDATE agent_run SET stop_requested=true WHERE requirement_id=$1 AND NOT quiescent",
    )
    .bind(id)
    .execute(&mut **tx)
    .await?;
    sqlx::query("INSERT INTO delivery_action(action_key,kind) SELECT action_key,'close' FROM delivery WHERE requirement_id=$1 AND pr_number IS NOT NULL AND superseded_by IS NULL ON CONFLICT DO NOTHING").bind(id).execute(&mut **tx).await?;
    Ok(changed.rows_affected() == 1)
}
pub async fn resume(pool: &PgPool, id: Option<i64>) -> Result<bool> {
    let mut tx = run_store::lock(pool).await?;
    let result = resume_in(&mut tx, id).await?;
    tx.commit().await?;
    Ok(result)
}
pub(crate) async fn resume_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: Option<i64>,
) -> Result<bool> {
    let safe:bool=sqlx::query_scalar("SELECT recovery_complete AND NOT (SELECT blocked FROM storage_guard WHERE id=1) AND NOT EXISTS(SELECT 1 FROM agent_run WHERE NOT quiescent AND stop_requested) AND NOT EXISTS(SELECT 1 FROM workspace_operation WHERE status<>'complete') FROM execution_control WHERE id=1").fetch_one(&mut **tx).await?;
    if !safe {
        return Ok(false);
    }
    match id {
        Some(id) => {
            let changed = sqlx::query(
                "UPDATE requirement SET paused=false WHERE id=$1 AND NOT cancel_requested",
            )
            .bind(id)
            .execute(&mut **tx)
            .await?;
            if changed.rows_affected() != 1 {
                return Ok(false);
            }
        }
        None => {
            sqlx::query("UPDATE execution_control SET paused=false WHERE id=1")
                .execute(&mut **tx)
                .await?;
        }
    }
    Ok(true)
}
pub async fn settle(pool: &PgPool) -> Result<()> {
    let mut tx = run_store::lock(pool).await?;
    // A cancelled action with no PR request can be withdrawn. An unknown PR
    // creation remains occupied even after a negative read (a late server-side
    // request might still complete). Read reconciliation continues independently.
    sqlx::query("UPDATE delivery_action a SET state='withdrawn' FROM delivery d JOIN requirement r ON r.id=d.requirement_id WHERE a.action_key=d.action_key AND a.kind='publish' AND r.cancel_requested AND (d.pr_number IS NULL OR d.original_action_key IS NOT NULL) AND NOT EXISTS(SELECT 1 FROM delivery_attempt t WHERE t.action_key=d.action_key )").execute(&mut *tx).await?;
    sqlx::query("UPDATE delivery_action a SET state='withdrawn' FROM delivery d WHERE a.action_key=d.action_key AND a.kind='close' AND d.original_action_key IS NOT NULL AND EXISTS(SELECT 1 FROM delivery_action p WHERE p.action_key=d.action_key AND p.kind='publish' AND p.state='withdrawn') AND NOT EXISTS(SELECT 1 FROM delivery_attempt t WHERE t.action_key=d.action_key)").execute(&mut *tx).await?;
    // Mark exact merge facts from the existing read-only observer. A nonempty
    // merge SHA, a stale snapshot, or a closed PR alone never satisfies this.
    sqlx::query("UPDATE delivery d SET released=true FROM github_pr p WHERE p.repository_id=d.repository_id AND p.number=d.pr_number AND p.requirement_id=d.requirement_id AND NOT p.stale AND p.last_synced_at>extract(epoch FROM now())::bigint-60 AND p.observation->>'merge'='Merged' AND p.observation->>'head'=d.head_sha AND p.observation->>'head_ref'=d.branch AND p.observation->>'base_ref'=d.base_branch").execute(&mut *tx).await?;
    sqlx::query("UPDATE requirement r SET cleanup_complete=true WHERE r.cancel_requested AND NOT EXISTS(SELECT 1 FROM integration_validation v WHERE v.requirement_id=r.id AND NOT v.quiescent AND v.state<>'prepared') AND NOT EXISTS(SELECT 1 FROM merge_operation m WHERE m.requirement_id=r.id AND (m.state IN ('unknown','merged') OR (m.state='prepared' AND m.pre_validation_started AND m.pre_validation IS NULL) OR (m.state='blocked' AND (m.merge_started OR m.pre_validation_started)))) AND NOT EXISTS(SELECT 1 FROM agent_run a WHERE a.requirement_id=r.id AND NOT a.quiescent) AND NOT EXISTS(SELECT 1 FROM workspace_operation o JOIN agent_run a ON a.id=o.run_id WHERE a.requirement_id=r.id AND o.status<>'complete') AND NOT EXISTS(SELECT 1 FROM run_workspace w JOIN agent_run a ON a.id=w.run_id LEFT JOIN workspace_snapshot s ON s.run_id=w.run_id WHERE a.requirement_id=r.id AND s.run_id IS NULL) AND NOT EXISTS(SELECT 1 FROM delivery d WHERE d.requirement_id=r.id AND d.superseded_by IS NULL AND NOT d.released AND NOT EXISTS(SELECT 1 FROM delivery_action a WHERE a.action_key=d.action_key AND ((a.kind='close' AND a.state='confirmed') OR (a.kind='publish' AND a.state='withdrawn'))))").execute(&mut *tx).await?;
    sqlx::query("UPDATE execution_control c SET requirement_id=NULL FROM requirement r WHERE c.requirement_id=r.id AND ((r.cancel_requested AND r.cleanup_complete) OR (NOT r.cancel_requested AND NOT r.paused AND NOT c.paused AND NOT EXISTS(SELECT 1 FROM recovery_failure f WHERE f.requirement_id=r.id AND f.decision IN ('code','reserved','blocked','infrastructure')) AND (NOT EXISTS(SELECT 1 FROM group_execution_item i WHERE i.requirement_id=r.id) OR EXISTS(SELECT 1 FROM group_completion f WHERE f.requirement_id=r.id)) AND NOT EXISTS(SELECT 1 FROM merge_operation m WHERE m.requirement_id=r.id AND (m.state IN ('unknown','merged') OR (m.state='prepared' AND m.pre_validation_started AND m.pre_validation IS NULL) OR (m.state='blocked' AND (m.merge_started OR m.pre_validation_started)))) AND NOT EXISTS(SELECT 1 FROM delivery d JOIN github_repository g ON g.repository_id=d.repository_id WHERE d.requirement_id=r.id AND g.policy#>>'{delivery,actions,merge}'='true' AND g.repository_version=(d.policy->>'repository_version')::bigint AND NOT EXISTS(SELECT 1 FROM merge_operation m WHERE m.delivery_key=d.action_key AND m.state='complete')) AND EXISTS(SELECT 1 FROM delivery d WHERE d.requirement_id=r.id) AND NOT EXISTS(SELECT 1 FROM delivery d WHERE d.requirement_id=r.id AND d.superseded_by IS NULL AND NOT d.released))) AND NOT EXISTS(SELECT 1 FROM agent_run a WHERE a.requirement_id=r.id AND NOT a.quiescent)").execute(&mut *tx).await?;
    tx.commit().await
}
