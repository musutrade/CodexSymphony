//! Merge outbox shares the coordinator lock and the existing delivery identity.
use crate::{
    automatic_merge::Intent,
    budget_store::{decode, require},
    run_store,
};
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Transaction};
pub(crate) type Result<T> = std::result::Result<T, sqlx::Error>;
type Tx<'a> = Transaction<'a, Postgres>;
type CandidateRow = (
    String,
    i64,
    i64,
    Option<i64>,
    Value,
    i64,
    String,
    String,
    String,
    Value,
);

pub(crate) async fn candidate(pool: &PgPool) -> Result<Option<Intent>> {
    let row: Option<CandidateRow> = sqlx::query_as("SELECT d.action_key,d.requirement_id,d.revision,i.authorization_id,g.policy,d.pr_number,d.head_sha,d.branch,d.validation_id,COALESCE(b.dependencies,'[]'::jsonb) FROM delivery d JOIN requirement r ON r.id=d.requirement_id JOIN execution_control c ON c.requirement_id=r.id JOIN github_repository g ON g.repository_id=d.repository_id LEFT JOIN group_execution_item i ON i.requirement_id=r.id LEFT JOIN group_claim_input b ON b.requirement_id=r.id WHERE r.state='Submitted' AND d.superseded_by IS NULL AND d.pr_number IS NOT NULL AND g.policy#>>'{delivery,actions,merge}'='true' AND NOT EXISTS(SELECT 1 FROM merge_operation m WHERE m.delivery_key=d.action_key AND m.state<>'invalidated') ORDER BY d.requirement_id LIMIT 1").fetch_optional(pool).await?;
    let Some((
        delivery_key,
        requirement,
        revision,
        authorization,
        policy,
        pr,
        head,
        branch,
        validation_id,
        dependencies,
    )) = row
    else {
        return Ok(None);
    };
    Ok(Some(Intent {
        delivery_key,
        requirement,
        revision,
        authorization,
        policy: decode(policy)?,
        pr: pr as u64,
        head,
        base: String::new(),
        checkout_sha: None,
        branch,
        validation_id,
        dependencies,
    }))
}

pub(crate) async fn allowed(tx: &mut Tx<'_>, intent: &Intent) -> Result<bool> {
    let allowed: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM delivery d JOIN requirement r ON r.id=d.requirement_id JOIN requirement_revision v ON v.requirement_id=r.id AND v.revision=r.revision JOIN repository p ON p.id=COALESCE((v.document->>'repository_id')::bigint,1) JOIN execution_control c ON c.requirement_id=r.id JOIN github_repository g ON g.repository_id=d.repository_id JOIN candidate_validation cv ON cv.id=d.validation_id WHERE d.action_key=$1 AND d.superseded_by IS NULL AND r.revision=$2 AND r.state='Submitted' AND NOT r.paused AND NOT r.cancel_requested AND NOT c.paused AND c.recovery_complete AND NOT (p.document->>'revoked')::boolean AND p.version=(v.document->>'repository_version')::bigint AND p.version=g.repository_version AND g.policy=$3 AND d.policy->>'repository_version'=v.document->>'repository_version' AND (SELECT authorization_id FROM group_execution_item WHERE requirement_id=r.id) IS NOT DISTINCT FROM $4 AND NOT g.stale AND cv.result='succeeded' AND cv.candidate_sha=d.head_sha AND NOT (SELECT blocked FROM storage_guard WHERE id=1) AND NOT EXISTS(SELECT 1 FROM agent_run WHERE NOT quiescent) AND NOT EXISTS(SELECT 1 FROM workspace_operation WHERE status<>'complete') AND NOT EXISTS(SELECT 1 FROM delivery_action a WHERE a.action_key=d.action_key AND a.state NOT IN ('confirmed','withdrawn')) AND NOT EXISTS(SELECT 1 FROM repair_reservation x WHERE x.requirement_id=r.id AND x.status IN ('reserved','started')) AND NOT EXISTS(SELECT 1 FROM recovery_failure f WHERE f.requirement_id=r.id AND f.decision IN ('code','reserved','blocked','infrastructure')))")
        .bind(&intent.delivery_key).bind(intent.revision).bind(json!(intent.policy)).bind(intent.authorization).fetch_one(&mut **tx).await?;
    if !allowed || !crate::group_queue_store::authorized(tx, intent.requirement).await? {
        return Ok(false);
    }
    let Some(dependencies) = crate::group_completion::dependencies(tx, intent.requirement).await?
    else {
        return Ok(false);
    };
    // Merge finishes already-admitted work; it does not reserve a new model
    // turn. The final authorized turn may complete at the reservation ceiling.
    let balance = crate::budget_store::balance(tx, intent.requirement).await?;
    Ok(json!(dependencies) == intent.dependencies
        && !balance.exhausted
        && balance.exposure.fits(balance.limits)
        && crate::group_budget::prepaid_fits(tx, intent.requirement).await?)
}

pub(crate) async fn prepare(pool: &PgPool, intent: &Intent, now: i64) -> Result<bool> {
    let mut tx = run_store::lock(pool).await?;
    if !allowed(&mut tx, intent).await? {
        return Ok(false);
    }
    let changed = sqlx::query("INSERT INTO merge_operation(action_key,delivery_key,requirement_id,intent,state,created_at,next_attempt_at) VALUES($1,$2,$3,$4,'prepared',$5,$5) ON CONFLICT(action_key) DO UPDATE SET state='prepared',next_attempt_at=$5,blocker=NULL,receipts=merge_operation.receipts||jsonb_build_array(jsonb_build_object('readmitted_at',$5::bigint)) WHERE merge_operation.state='invalidated' AND NOT merge_operation.merge_started AND merge_operation.intent=$4")
        .bind(intent.action_key()).bind(&intent.delivery_key).bind(intent.requirement).bind(json!(intent)).bind(now).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(changed.rows_affected() == 1)
}

pub(crate) async fn due(pool: &PgPool, now: i64) -> Result<Option<(Intent, String, i64)>> {
    let row: Option<(Value,String,i64)> = sqlx::query_as("SELECT intent,state,created_at FROM merge_operation WHERE state IN ('prepared','unknown') AND next_attempt_at<=$1 ORDER BY created_at LIMIT 1")
        .bind(now).fetch_optional(pool).await?;
    row.map(|(intent, state, created)| Ok((decode(intent)?, state, created)))
        .transpose()
}

/// Commit before transport. No retry changes an unknown send back to prepared.
pub(crate) async fn begin(pool: &PgPool, intent: &Intent, now: i64) -> Result<bool> {
    let mut tx = run_store::lock(pool).await?;
    if !allowed(&mut tx, intent).await? {
        return Ok(false);
    }
    let changed = sqlx::query("UPDATE merge_operation SET state='unknown',merge_started=true,next_attempt_at=$2+30 WHERE action_key=$1 AND state='prepared'")
        .bind(intent.action_key()).bind(now).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(changed.rows_affected() == 1)
}

pub(crate) async fn receipt(
    pool: &PgPool,
    intent: &Intent,
    receipt: Value,
    now: i64,
    delay: i64,
) -> Result<()> {
    sqlx::query(
        "UPDATE merge_operation SET receipts=receipts||$2,next_attempt_at=$3 WHERE action_key=$1",
    )
    .bind(intent.action_key())
    .bind(json!([receipt]))
    .bind(now.saturating_add(delay.max(30)))
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn merged(
    pool: &PgPool,
    intent: &Intent,
    sha: &str,
    observation: &crate::github::Observation,
) -> Result<()> {
    let mut tx = run_store::lock(pool).await?;
    let saved: Option<String> =
        sqlx::query_scalar("SELECT merged_sha FROM merge_operation WHERE action_key=$1 FOR UPDATE")
            .bind(intent.action_key())
            .fetch_one(&mut *tx)
            .await?;
    require(
        saved.as_deref().is_none_or(|saved| saved == sha),
        "merged identity conflict",
    )?;
    sqlx::query("INSERT INTO github_evidence_history(repository_id,pr_number,observed_at,policy,evidence) VALUES($1,$2,$3,$4,$5)")
        .bind(intent.policy.repository_id as i64).bind(intent.pr as i64).bind(observation.last_synced_at).bind(json!(intent.policy)).bind(json!(observation)).execute(&mut *tx).await?;
    sqlx::query("UPDATE merge_operation SET state='merged',merged_sha=$2,merged_at=COALESCE(merged_at,$4),merge_method=CASE WHEN EXISTS(SELECT 1 FROM jsonb_array_elements(receipts) receipt WHERE receipt#>>'{merge_response,merged}'='true' AND receipt#>>'{merge_response,sha}'=$2) THEN intent#>>'{policy,delivery,actions,merge_method}' ELSE merge_method END,receipts=receipts||$3 WHERE action_key=$1 AND state IN ('prepared','unknown','merged')")
        .bind(intent.action_key()).bind(sha).bind(json!([{"confirmed_observation":observation}])).bind(observation.last_synced_at).execute(&mut *tx).await?;
    tx.commit().await
}

pub(crate) async fn block(pool: &PgPool, intent: &Intent, reason: &str) -> Result<()> {
    sqlx::query("UPDATE merge_operation SET state='blocked',blocker=$2 WHERE action_key=$1")
        .bind(intent.action_key())
        .bind(reason)
        .execute(pool)
        .await?;
    Ok(())
}

pub(crate) async fn validation(
    pool: &PgPool,
    intent: &Intent,
) -> Result<(crate::validation::ValidationEvidence, Vec<String>)> {
    let value: Value = sqlx::query_scalar("SELECT jsonb_build_object('candidate',jsonb_build_object('sha',candidate_sha,'tree',candidate_tree,'immutable',true),'trusted',trusted,'source_before',source_before,'source_after',source_after,'entry_before',entry_before,'entry_after',entry_after,'steps',(SELECT jsonb_agg(jsonb_build_object('id',step_id,'command',command,'exit_code',exit_code,'output',output,'output_sha256',output_sha256,'log_ref',log_ref,'consumer',consumer,'code_failure',code_failure) ORDER BY step_id) FROM validation_step WHERE validation_id=v.id)) FROM candidate_validation v WHERE id=$1 AND result='succeeded' AND candidate_sha=$2")
        .bind(&intent.validation_id).bind(&intent.head).fetch_one(pool).await?;
    let required: Value =
        sqlx::query_scalar("SELECT required_steps FROM candidate_validation WHERE id=$1")
            .bind(&intent.validation_id)
            .fetch_one(pool)
            .await?;
    Ok((decode(value)?, decode(required)?))
}
