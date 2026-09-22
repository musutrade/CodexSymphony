//! Consumer facts are read under the same lock used to start/restore/deliver.
use crate::{
    git_broker::GitBroker,
    storage_lifecycle::Protection,
    storage_store::{Deployment, Result, Tx},
    workspace::Manifest,
};
use serde_json::Value;

pub async fn protection(
    tx: &mut Tx<'_>,
    config: &Deployment,
    run: &str,
    kind: &str,
) -> Result<Protection> {
    let facts: (bool,bool,bool,bool,bool) = sqlx::query_as(
        "SELECT
          EXISTS(SELECT 1 FROM agent_run a WHERE a.requirement_id=s.requirement_id AND NOT a.quiescent),
          EXISTS(SELECT 1 FROM candidate_validation v WHERE v.requirement_id=s.requirement_id AND (v.result='pending' OR (v.result='succeeded' AND v.stage<>'done'))) OR
          EXISTS(SELECT 1 FROM repair_reservation p JOIN candidate_validation v ON v.id=p.source_validation_id WHERE v.source_run_id=s.run_id AND p.status IN ('reserved','started')) OR
          EXISTS(SELECT 1 FROM delivery d WHERE d.requirement_id=s.requirement_id AND NOT d.released) OR
          EXISTS(SELECT 1 FROM merge_operation m WHERE m.requirement_id=s.requirement_id AND m.state NOT IN ('complete','cancelled','invalidated')) OR
          EXISTS(SELECT 1 FROM runtime_resume r WHERE r.source_run=s.run_id AND r.status IN ('restoring','prepared')) OR
          EXISTS(SELECT 1 FROM runtime_question q WHERE q.run_id=s.run_id AND q.resume_state IN ('waiting','pending')) OR
          EXISTS(SELECT 1 FROM agent_run paused WHERE paused.id=s.run_id AND (paused.user_paused OR paused.storage_resume_requested) AND NOT EXISTS(SELECT 1 FROM runtime_resume restored WHERE restored.source_run=s.run_id AND restored.status='dispatched')),
          EXISTS(SELECT 1 FROM workspace_operation o WHERE o.run_id=s.run_id AND o.status<>'complete') OR
          EXISTS(SELECT 1 FROM preparation_record p WHERE p.run_id=s.run_id AND NOT p.ready AND NOT (p.retry->>'todo')::boolean AND p.retry->'next_attempt_at'='null'::jsonb),
          COALESCE(s.identity->>'repository','')='' OR
          (NOT EXISTS(SELECT 1 FROM agent_run a WHERE a.id=s.run_id) AND
           NOT EXISTS(SELECT 1 FROM preparation_record p WHERE p.run_id=s.run_id AND (p.retry->>'todo')::boolean)),
          s.resolved_by IS NULL AND EXISTS(SELECT 1 FROM candidate_validation v WHERE v.source_run_id=s.run_id AND v.result='succeeded')
         FROM storage_attempt s WHERE s.run_id=$1")
        .bind(run).fetch_one(&mut **tx).await?;
    let mut protection = Protection {
        active: facts.0,
        consumer: facts.1,
        unreconciled: facts.2,
        unknown: facts.3,
        current_success: kind == "retrospective" && facts.4,
        unique: false,
    };
    if kind == "recovery" {
        protection.unique = !rebuildable(tx, config, run).await?;
    }
    Ok(protection)
}

async fn rebuildable(tx: &mut Tx<'_>, config: &Deployment, run: &str) -> Result<bool> {
    let row: Option<(Value,String)> = sqlx::query_as(
        "SELECT w.manifest,d.head_sha FROM workspace_snapshot w JOIN storage_attempt s ON s.run_id=w.run_id JOIN candidate_validation v ON v.source_run_id=COALESCE(s.resolved_by,s.run_id) JOIN delivery d ON d.validation_id=v.id JOIN delivery_action a ON a.action_key=d.action_key AND a.kind='publish' AND a.state='confirmed' WHERE w.run_id=$1 AND d.released AND v.result='succeeded'")
        .bind(run).fetch_optional(&mut **tx).await?;
    let Some((manifest, pushed)) = row else {
        return Ok(false);
    };
    verify_reclaim(tx, config, run, manifest, &pushed).await
}
async fn verify_reclaim(
    tx: &mut Tx<'_>,
    config: &Deployment,
    run: &str,
    manifest: Value,
    pushed: &str,
) -> Result<bool> {
    let manifest: Manifest = serde_json::from_value(manifest)?;
    let broker = GitBroker::open(&config.execution.path.join("workspaces"))?;
    let deleted: Vec<String> =
        sqlx::query_scalar("SELECT id FROM storage_material WHERE run_id=$1 AND status='deleted'")
            .bind(run)
            .fetch_all(&mut **tx)
            .await?;
    let work_deleted = deleted.contains(&format!("{run}-work"));
    let snapshot_deleted = deleted.contains(&format!("{run}-snapshot"));
    use sha2::{Digest, Sha256};
    let proof = serde_json::json!({"pushed":pushed,"manifest_sha256":format!("{:x}",Sha256::digest(serde_json::to_vec(&manifest)?))});
    if !work_deleted && !snapshot_deleted {
        if !broker.rebuildable(&manifest, pushed)? {
            return Ok(false);
        }
        sqlx::query("UPDATE storage_attempt SET reclaim_proof=$2 WHERE run_id=$1")
            .bind(run)
            .bind(&proof)
            .execute(&mut **tx)
            .await?;
        return Ok(true);
    }
    verify_remaining(
        tx,
        run,
        &proof,
        &broker,
        &manifest,
        pushed,
        (work_deleted, snapshot_deleted),
    )
    .await
}
async fn verify_remaining(
    tx: &mut Tx<'_>,
    run: &str,
    proof: &Value,
    broker: &GitBroker,
    manifest: &Manifest,
    pushed: &str,
    deleted: (bool, bool),
) -> Result<bool> {
    let (work_deleted, snapshot_deleted) = deleted;
    let saved: Option<Value> =
        sqlx::query_scalar("SELECT reclaim_proof FROM storage_attempt WHERE run_id=$1")
            .bind(run)
            .fetch_one(&mut **tx)
            .await?;
    if saved.as_ref() != Some(proof) {
        return Ok(false);
    }
    if !snapshot_deleted {
        broker.verify(manifest)?;
    }
    if !work_deleted {
        return broker.rebuildable_source(manifest, pushed);
    }
    Ok(true)
}

pub async fn resolve(tx: &mut Tx<'_>) -> Result<()> {
    // Only persisted repair/recovery relationships supply predecessors.
    // No path/branch similarity, newer failure, post-merge or PR-only inference.
    let pairs: Vec<(String,String,Value,Value)> = sqlx::query_as(
        "WITH RECURSIVE edges(old,new) AS (
          SELECT v.source_run_id,p.repair_run_id FROM repair_reservation p JOIN candidate_validation v ON v.id=p.source_validation_id WHERE p.repair_run_id IS NOT NULL
          UNION SELECT source_run,job#>>'{launch,key,run_id}' FROM runtime_resume WHERE status='dispatched'
        ), chain(old,new,depth) AS (
          SELECT old,new,1 FROM edges UNION ALL SELECT c.old,e.new,c.depth+1 FROM chain c JOIN edges e ON e.old=c.new WHERE c.depth<64
        ) SELECT c.old,c.new,o.identity,n.identity FROM chain c
          JOIN storage_attempt o ON o.run_id=c.old JOIN storage_attempt n ON n.run_id=c.new
          JOIN candidate_validation v ON v.source_run_id=c.new JOIN delivery d ON d.validation_id=v.id JOIN delivery_action published ON published.action_key=d.action_key AND published.kind='publish' AND published.state='confirmed'
          LEFT JOIN candidate_validation prior ON prior.source_run_id=c.old
          WHERE v.result='succeeded' AND d.released AND o.sequence<n.sequence AND o.resolved_by IS NULL
            AND (prior.id IS NULL OR prior.trusted=v.trusted)
          ORDER BY n.sequence DESC")
        .fetch_all(&mut **tx).await?;
    for (older, newer, old, new) in pairs {
        let old = serde_json::from_value(old)?;
        let new = serde_json::from_value(new)?;
        if crate::storage_lifecycle::replaces(&old, &new, &older) {
            sqlx::query(
                "UPDATE storage_attempt SET resolved_by=$2 WHERE run_id=$1 AND resolved_by IS NULL",
            )
            .bind(older)
            .bind(newer)
            .execute(&mut **tx)
            .await?;
        }
    }
    Ok(())
}
