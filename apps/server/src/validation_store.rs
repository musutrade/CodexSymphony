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
    if !candidate.immutable || candidate.sha.is_empty() || candidate.tree.is_empty() {
        return Ok(false);
    }
    let mut tx = run_store::lock(pool).await?;
    let contract: Option<Value> = sqlx::query_scalar("SELECT v.document->'contract' FROM agent_run a JOIN requirement_revision v ON v.requirement_id=a.requirement_id AND v.revision=a.revision JOIN workspace_snapshot s ON s.run_id=a.id WHERE a.id=$1 AND a.requirement_id=$2 AND a.revision=$3 AND a.state='Succeeded' AND a.quiescent AND s.candidate AND s.manifest->>'head'=$4")
        .bind(source_run).bind(requirement).bind(revision).bind(&candidate.sha).fetch_optional(&mut *tx).await?;
    let Some(contract) = contract else {
        return Ok(false);
    };
    let required = required_steps(&contract)?;
    let result = sqlx::query("INSERT INTO candidate_validation (id,requirement_id,revision,source_run_id,candidate_sha,candidate_tree,trusted,required_steps,source_before,source_after,entry_before,entry_after,stage,result) VALUES ($1,$2,$3,$4,$5,$6,$7,$10,$8,$8,$9,$9,'declaration','pending') ON CONFLICT DO NOTHING")
        .bind(id).bind(requirement).bind(revision).bind(source_run).bind(&candidate.sha).bind(&candidate.tree).bind(json!(trusted)).bind(source).bind(entry).bind(json!(required)).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(result.rows_affected() == 1)
}

fn required_steps(contract: &Value) -> Result<Vec<String>> {
    let contract: crate::contract::Contract =
        serde_json::from_value(contract.clone()).map_err(decode)?;
    crate::contract::validate_contract(&contract).map_err(protocol)?;
    Ok(contract
        .validation_plan
        .into_iter()
        .map(|step| {
            // Preserve an explicit closure span for native coverage.
            step.id
        })
        .collect())
}
fn protocol(error: &'static str) -> sqlx::Error {
    sqlx::Error::Protocol(error.into())
}
fn decode(error: serde_json::Error) -> sqlx::Error {
    sqlx::Error::Decode(Box::new(error))
}

pub async fn begin(pool: &PgPool, id: &str) -> Result<bool> {
    let mut tx = run_store::lock(pool).await?;
    let result = sqlx::query("UPDATE candidate_validation v SET stage='validation' WHERE v.id=$1 AND v.stage='declaration' AND v.result='pending' AND (NOT EXISTS(SELECT 1 FROM repair_authorization auth WHERE auth.requirement_id=v.requirement_id AND auth.policy='bounded_v1') OR EXISTS(SELECT 1 FROM requirement r JOIN execution_control c ON c.requirement_id=r.id JOIN requirement_revision rev ON rev.requirement_id=r.id AND rev.revision=r.revision JOIN repository repo ON repo.id=COALESCE((rev.document->>'repository_id')::bigint,1) WHERE r.id=v.requirement_id AND r.revision=v.revision AND NOT r.paused AND NOT r.cancel_requested AND NOT c.paused AND c.recovery_complete AND NOT (repo.document->>'revoked')::boolean AND (rev.document->>'repository_version')::bigint>repo.revoked_through_version AND NOT (SELECT blocked FROM storage_guard WHERE id=1)))")
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
        "SELECT jsonb_build_object('id',step_id,'exit_code',exit_code,'output_sha256',output_sha256,'log_ref',log_ref,'consumer',consumer,'status',status,'bytes',octet_length(output),'recorded_at',recorded_at,'availability','available') FROM validation_step WHERE validation_id=$1 ORDER BY step_id",
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
    if !matches!(status, "succeeded" | "failed" | "unknown") {
        return Ok(false);
    }
    let mut tx = run_store::lock(pool).await?;
    let active: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM candidate_validation WHERE id=$1 AND stage='validation' AND result='pending')")
        .bind(validation_id).fetch_one(&mut *tx).await?;
    if !active {
        return Ok(false);
    }
    let result = sqlx::query("INSERT INTO validation_step(validation_id,step_id,command,exit_code,output,output_sha256,log_ref,consumer,code_failure,status) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) ON CONFLICT (validation_id,step_id) DO NOTHING")
        .bind(validation_id).bind(&step.id).bind(json!(step.command)).bind(step.exit_code).bind(&step.output).bind(&step.output_sha256).bind(&step.log_ref).bind(&step.consumer).bind(step.code_failure).bind(status).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(result.rows_affected() == 1)
}

async fn saved_steps(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: &str,
) -> Result<Vec<StepEvidence>> {
    let rows: Vec<Value> = sqlx::query_scalar("SELECT jsonb_build_object('id',step_id,'command',command,'exit_code',exit_code,'output',output,'output_sha256',output_sha256,'log_ref',log_ref,'consumer',consumer,'code_failure',code_failure) FROM validation_step WHERE validation_id=$1 ORDER BY step_id")
        .bind(id).fetch_all(&mut **tx).await?;
    rows.into_iter()
        .map(|row| {
            // Decode each retained step independently.
            serde_json::from_value(row).map_err(decode)
        })
        .collect()
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
    let Some(evidence) = load_evidence(&mut tx, id).await? else {
        return Ok(false);
    };
    let (saved_required, terminal) = requirements(&mut tx, id).await?;
    let stored = &evidence.steps;
    let mut supplied = steps.to_vec();
    supplied.sort_by(|a, b| {
        // Evidence ordering is canonical.
        a.id.cmp(&b.id)
    });
    let bound =
        supplied == *stored && matches_binding(required, &saved_required, source, entry, &evidence);
    let result = outcome(
        bound,
        terminal,
        &evidence,
        candidate,
        trusted,
        &saved_required,
    );
    save_outcome(&mut tx, id, result, source, entry).await?;
    tx.commit().await?;
    Ok(result == "succeeded")
}

fn matches_binding(
    required: &[String],
    saved: &[String],
    source: &str,
    entry: &str,
    evidence: &ValidationEvidence,
) -> bool {
    required == saved && source == evidence.source_before && entry == evidence.entry_before
}

async fn load_evidence(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: &str,
) -> Result<Option<ValidationEvidence>> {
    let saved: Option<(String,String,Value,String,String,String,String)> = sqlx::query_as("SELECT candidate_sha,candidate_tree,trusted,source_before,source_after,entry_before,entry_after FROM candidate_validation WHERE id=$1 AND stage='validation' AND result='pending' FOR UPDATE")
        .bind(id).fetch_optional(&mut **tx).await?;
    let Some((sha, tree, trusted_json, before, after, entry_before, entry_after)) = saved else {
        return Ok(None);
    };
    let saved_candidate = Candidate {
        sha,
        tree,
        immutable: true,
    };
    let saved_trusted: TrustedIdentity = serde_json::from_value(trusted_json).map_err(decode)?;
    let stored = saved_steps(tx, id).await?;
    let evidence = ValidationEvidence {
        candidate: saved_candidate,
        trusted: saved_trusted,
        source_before: before,
        source_after: after,
        entry_before,
        entry_after,
        steps: stored.clone(),
    };
    Ok(Some(evidence))
}
async fn requirements(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: &str,
) -> Result<(Vec<String>, bool)> {
    let saved_required: Value =
        sqlx::query_scalar("SELECT required_steps FROM candidate_validation WHERE id=$1")
            .bind(id)
            .fetch_one(&mut **tx)
            .await?;
    let saved_required: Vec<String> = serde_json::from_value(saved_required).map_err(decode)?;
    let terminal: bool = sqlx::query_scalar("SELECT NOT EXISTS(SELECT 1 FROM validation_step WHERE validation_id=$1 AND status<>'succeeded')").bind(id).fetch_one(&mut **tx).await?;
    Ok((saved_required, terminal))
}

fn outcome(
    bound: bool,
    terminal: bool,
    evidence: &ValidationEvidence,
    candidate: &Candidate,
    trusted: &TrustedIdentity,
    required: &[String],
) -> &'static str {
    if !bound {
        return "blocked";
    }
    let mut checked = evidence.clone();
    for step in &mut checked.steps {
        if step.exit_code.is_some() {
            step.exit_code = Some(0);
        }
    }
    if verify(&checked, candidate, trusted, required).is_err() {
        return "blocked";
    }
    if terminal && verify(evidence, candidate, trusted, required).is_ok() {
        "succeeded"
    } else {
        "gate_failed"
    }
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
    if kind != FailureKind::Code || ordinal != 1 {
        return Ok(false);
    }
    let mut tx = run_store::lock(pool).await?;
    let allowed: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM candidate_validation v WHERE v.id=$1 AND v.requirement_id=$2 AND v.result='gate_failed' AND NOT EXISTS(SELECT 1 FROM repair_authorization a WHERE a.requirement_id=$2 AND a.policy='bounded_v1') AND EXISTS(SELECT 1 FROM validation_step s WHERE s.validation_id=v.id AND s.status='failed' AND s.code_failure AND s.exit_code IS NOT NULL AND s.exit_code<>0))")
        .bind(source).bind(requirement).fetch_one(&mut *tx).await?;
    if !allowed {
        return Ok(false);
    }
    let result = sqlx::query("INSERT INTO repair_reservation(requirement_id,ordinal,source_validation_id,failure,status) VALUES($1,$2,$3,$4,'reserved') ON CONFLICT DO NOTHING")
        .bind(requirement).bind(ordinal).bind(source).bind(failure).execute(&mut *tx).await?;
    if result.rows_affected() == 1 {
        sqlx::query(
            "UPDATE candidate_validation SET stage='repair_reservation',repair_count=1 WHERE id=$1",
        )
        .bind(source)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(result.rows_affected() == 1)
}

async fn save_outcome(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: &str,
    result: &str,
    source: &str,
    entry: &str,
) -> Result<()> {
    sqlx::query("UPDATE candidate_validation SET stage=CASE WHEN $2='succeeded' THEN 'handoff' ELSE 'validation' END,result=$2,source_after=$3,entry_after=$4 WHERE id=$1")
        .bind(id).bind(result).bind(source).bind(entry).execute(&mut **tx).await?;
    sqlx::query("UPDATE repair_reservation p SET status=CASE WHEN $2='succeeded' THEN 'succeeded' ELSE 'failed' END FROM candidate_validation v WHERE v.id=$1 AND p.repair_run_id=v.source_run_id AND (p.status='started' OR ($2='succeeded' AND p.status='failed'))").bind(id).bind(result).execute(&mut **tx).await?;
    sqlx::query("UPDATE recovery_failure f SET decision='repaired' FROM repair_reservation p JOIN candidate_validation v ON v.source_run_id=p.repair_run_id WHERE v.id=$1 AND f.event_key=p.event_key AND $2='succeeded'")
        .bind(id).bind(result).execute(&mut **tx).await?;
    if result == "succeeded" {
        crate::delivery_store::enqueue(tx, id).await?;
    }
    Ok(())
}
