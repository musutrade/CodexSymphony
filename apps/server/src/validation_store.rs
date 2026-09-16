//! Durable validation ledger.  Every mutation is serialized with the existing
//! execution lock, so recovery can continue an in-flight phase without creating
//! a second verifier or repair reservation.
use crate::{
    run_store,
    validation::{
        Candidate, FailureKind, StepEvidence, TrustedIdentity, ValidationEvidence, verify,
    },
};
use serde_json::{Value, json};
use sqlx::PgPool;

type Result<T> = std::result::Result<T, sqlx::Error>;
#[allow(clippy::too_many_arguments)]
pub async fn create(
    pool: &PgPool,
    id: &str,
    requirement: i64,
    revision: i64,
    source_run: &str,
    candidate: &Candidate,
    trusted: &TrustedIdentity,
    source: &str,
    entry: &str,
) -> Result<bool> {
    let mut tx = run_store::lock(pool).await?;
    let result = sqlx::query("INSERT INTO candidate_validation (id,requirement_id,revision,source_run_id,candidate_sha,candidate_tree,trusted,source_before,source_after,entry_before,entry_after,stage,result) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$8,$9,$9,'declaration','pending') ON CONFLICT DO NOTHING")
        .bind(id).bind(requirement).bind(revision).bind(source_run).bind(&candidate.sha).bind(&candidate.tree).bind(json!(trusted)).bind(source).bind(entry).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(result.rows_affected() == 1)
}

pub async fn begin(pool: &PgPool, id: &str) -> Result<bool> {
    let mut tx = run_store::lock(pool).await?;
    let result = sqlx::query("UPDATE candidate_validation SET stage='validation' WHERE id=$1 AND stage IN ('declaration','validation') AND result='pending'")
        .bind(id).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(result.rows_affected() == 1)
}

pub async fn status(pool: &PgPool, id: &str) -> Result<Option<Value>> {
    let record: Option<(String, i64, String, String, String, String)> = sqlx::query_as(
        "SELECT stage, requirement_id, candidate_sha, candidate_tree, result, trusted::text FROM candidate_validation WHERE id=$1",
    ).bind(id).fetch_optional(pool).await?;
    let Some((stage, requirement, sha, tree, result, trusted)) = record else {
        return Ok(None);
    };
    let steps: Vec<Value> = sqlx::query_scalar(
        "SELECT jsonb_build_object('id',step_id,'exit_code',exit_code,'output_sha256',output_sha256,'log_ref',log_ref,'consumer',consumer,'status',status) FROM validation_step WHERE validation_id=$1 ORDER BY step_id",
    ).bind(id).fetch_all(pool).await?;
    Ok(Some(
        json!({"id":id,"requirement_id":requirement,"candidate_sha":sha,"candidate_tree":tree,"stage":stage,"result":result,"trusted":serde_json::from_str::<Value>(&trusted).unwrap_or(Value::Null),"steps":steps}),
    ))
}

pub async fn record_step(
    pool: &PgPool,
    validation_id: &str,
    step: &StepEvidence,
    status: &str,
) -> Result<bool> {
    let mut tx = run_store::lock(pool).await?;
    let result = sqlx::query("INSERT INTO validation_step(validation_id,step_id,command,exit_code,output,output_sha256,log_ref,consumer,code_failure,status) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) ON CONFLICT (validation_id,step_id) DO UPDATE SET command=EXCLUDED.command,exit_code=EXCLUDED.exit_code,output=EXCLUDED.output,output_sha256=EXCLUDED.output_sha256,log_ref=EXCLUDED.log_ref,consumer=EXCLUDED.consumer,code_failure=EXCLUDED.code_failure,status=EXCLUDED.status")
        .bind(validation_id).bind(&step.id).bind(json!(step.command)).bind(step.exit_code).bind(&step.output).bind(&step.output_sha256).bind(&step.log_ref).bind(&step.consumer).bind(step.code_failure).bind(status).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(result.rows_affected() == 1)
}

#[allow(clippy::too_many_arguments)]
pub async fn finish(
    pool: &PgPool,
    id: &str,
    candidate: &Candidate,
    trusted: &TrustedIdentity,
    source: &str,
    entry: &str,
    steps: &[StepEvidence],
    required: &[String],
) -> Result<bool> {
    let mut tx = run_store::lock(pool).await?;
    let saved: Option<(String,String,Value,String,String,String,String)> = sqlx::query_as("SELECT candidate_sha,candidate_tree,trusted,source_before,source_after,entry_before,entry_after FROM candidate_validation WHERE id=$1 AND stage='validation' FOR UPDATE")
        .bind(id).fetch_optional(&mut *tx).await?;
    let Some((sha, tree, trusted_json, before, after, entry_before, entry_after)) = saved else {
        tx.commit().await?;
        return Ok(false);
    };
    let saved_candidate = Candidate {
        sha,
        tree,
        immutable: true,
    };
    let saved_trusted: TrustedIdentity =
        serde_json::from_value(trusted_json).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
    let evidence = ValidationEvidence {
        candidate: saved_candidate,
        trusted: saved_trusted,
        source_before: before,
        source_after: after,
        entry_before,
        entry_after,
        steps: steps.to_vec(),
    };
    let passed = verify(&evidence, candidate, trusted, required).is_ok()
        && source == evidence.source_before
        && entry == evidence.entry_before;
    sqlx::query("UPDATE candidate_validation SET stage=CASE WHEN $2 THEN 'handoff' ELSE 'validation' END,result=CASE WHEN $2 THEN 'succeeded' ELSE 'gate_failed' END,source_after=$3,entry_after=$4 WHERE id=$1")
        .bind(id).bind(passed).bind(source).bind(entry).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(passed)
}

/// Only classified code failures may reserve the single repair ordinal. The
/// unique requirement key makes retries and crash recovery idempotent.
pub async fn reserve_repair(
    pool: &PgPool,
    requirement: i64,
    ordinal: i64,
    source: &str,
    failure: &Value,
    kind: FailureKind,
) -> Result<bool> {
    if kind != FailureKind::Code {
        return Ok(false);
    }
    let mut tx = run_store::lock(pool).await?;
    let result = sqlx::query("INSERT INTO repair_reservation(requirement_id,ordinal,source_validation_id,failure,status) VALUES($1,$2,$3,$4,'reserved') ON CONFLICT (requirement_id) DO NOTHING")
        .bind(requirement).bind(ordinal).bind(source).bind(failure).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(result.rows_affected() == 1)
}
