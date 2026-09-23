//! One durable infrastructure retry ledger, used before reserving a model Run.
use crate::{
    execution::Launch,
    preparation::{Evidence, Failure, Retry},
    run_store,
};
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Transaction};
type Result<T> = std::result::Result<T, sqlx::Error>;

/// Persist the attempt before invoking the probe. A crash leaves an unknown
/// attempt for explicit reconciliation, never an automatic duplicate execution.
pub async fn begin(
    pool: &PgPool,
    launch: &Launch,
    requirement: i64,
    revision: i64,
    phase: &str,
    now: i64,
) -> Result<bool> {
    let mut tx = run_store::lock(pool).await?;
    if !permitted(&mut tx, launch, requirement).await? {
        return Ok(false);
    }
    let Some(mut retry) =
        load_for_begin(&mut tx, launch, requirement, revision, phase, now).await?
    else {
        return Ok(false);
    };
    let began = retry.begin(now, false);
    save(
        &mut tx,
        &launch.key.run_id,
        &retry,
        now,
        json!({"attempt_started":began,"retry":retry}),
    )
    .await?;
    tx.commit().await?;
    Ok(began)
}

async fn permitted(
    tx: &mut Transaction<'_, Postgres>,
    launch: &Launch,
    requirement: i64,
) -> Result<bool> {
    Ok(!paused(tx, requirement).await?
        || crate::runtime_resume::preparation_allowed(tx, launch).await?
        || crate::validation_repair::preparation_allowed(tx, launch).await?
        || crate::linked_repair_worker::preparation_allowed(tx, launch).await?)
}

async fn load_for_begin(
    tx: &mut Transaction<'_, Postgres>,
    launch: &Launch,
    requirement: i64,
    revision: i64,
    phase: &str,
    now: i64,
) -> Result<Option<Retry>> {
    sqlx::query("INSERT INTO preparation_record(run_id,requirement_id,revision,launch,retry) VALUES($1,$2,$3,$4,$5) ON CONFLICT DO NOTHING")
        .bind(&launch.key.run_id).bind(requirement).bind(revision).bind(json!(launch)).bind(json!(Retry::new(phase, now)))
        .execute(&mut **tx).await?;
    let (saved, value, ready): (Value, Value, bool) = sqlx::query_as(
        "SELECT launch,retry,ready FROM preparation_record WHERE run_id=$1 AND requirement_id=$2 AND revision=$3 FOR UPDATE",
    )
    .bind(&launch.key.run_id)
    .bind(requirement)
    .bind(revision)
    .fetch_one(&mut **tx)
    .await?;
    if saved != json!(launch) {
        return Ok(None);
    }
    let mut retry: Retry = decode(value)?;
    if ready && !refresh_success(tx, launch, phase, now, &mut retry).await? {
        return Ok(None);
    }
    Ok(Some(retry))
}

fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T> {
    serde_json::from_value(value).map_err(decode_error)
}

fn decode_error(error: serde_json::Error) -> sqlx::Error {
    sqlx::Error::Decode(Box::new(error))
}

async fn paused(tx: &mut Transaction<'_, Postgres>, requirement: i64) -> Result<bool> {
    sqlx::query_scalar("SELECT c.paused OR NOT c.recovery_complete OR c.requirement_id IS NOT NULL OR (SELECT blocked FROM storage_guard WHERE id=1) OR r.paused OR r.state<>'Ready' FROM execution_control c CROSS JOIN requirement r WHERE c.id=1 AND r.id=$1")
        .bind(requirement).fetch_one(&mut **tx).await
}

async fn save(
    tx: &mut Transaction<'_, Postgres>,
    id: &str,
    retry: &Retry,
    now: i64,
    mut event: Value,
) -> Result<()> {
    let requirement: i64 =
        sqlx::query_scalar("SELECT requirement_id FROM preparation_record WHERE run_id=$1")
            .bind(id)
            .fetch_one(&mut **tx)
            .await?;
    let present: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM requirement_budget WHERE requirement_id=$1)",
    )
    .bind(requirement)
    .fetch_one(&mut **tx)
    .await?;
    if present {
        event["budget"] = json!(crate::budget_store::balance(tx, requirement).await?);
    }
    sqlx::query("UPDATE preparation_record SET retry=$2 WHERE run_id=$1")
        .bind(id)
        .bind(json!(retry))
        .execute(&mut **tx)
        .await?;
    sqlx::query("INSERT INTO preparation_history(run_id,recorded_at,event) VALUES($1,$2,$3)")
        .bind(id)
        .bind(now)
        .bind(event)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Completion cannot unpause or change business/Run state. Raw evidence and the
/// actual phase remain queryable even after the single aggregated todo is set.
pub async fn finish(
    pool: &PgPool,
    launch: &Launch,
    expected: &str,
    evidence: &Evidence,
    now: i64,
) -> Result<bool> {
    let mut tx = run_store::lock(pool).await?;
    let (value, document, ready): (Value,Value,bool) = sqlx::query_as("SELECT p.retry,v.document,p.ready FROM preparation_record p JOIN requirement_revision v ON v.requirement_id=p.requirement_id AND v.revision=p.revision WHERE p.run_id=$1 AND p.launch=$2 FOR UPDATE OF p")
        .bind(&launch.key.run_id).bind(json!(launch)).fetch_one(&mut *tx).await?;
    let mut retry: Retry = decode(value)?;
    if ready || !retry.active() {
        return Ok(false);
    }
    let requested: Vec<String> = decode(
        document
            .pointer("/contract/network_access")
            .cloned()
            .unwrap_or(json!([])),
    )?;
    retry.complete(evidence.failure(expected, &requested), now);
    let ready = retry.last_failure.is_none();
    sqlx::query("UPDATE preparation_record SET ready=$2,evidence=$3,checked_at=$4 WHERE run_id=$1")
        .bind(&launch.key.run_id)
        .bind(ready)
        .bind(json!(evidence))
        .bind(now)
        .execute(&mut *tx)
        .await?;
    save(
        &mut tx,
        &launch.key.run_id,
        &retry,
        now,
        json!({"evidence":evidence,"retry":retry}),
    )
    .await?;
    tx.commit().await?;
    Ok(ready)
}

/// A caller must present explicit user recovery, after reconciling any unknown
/// probe. Total attempts and history survive, and pause remains independent.
pub async fn authorize_retry(pool: &PgPool, id: &str, now: i64, reason: &str) -> Result<()> {
    if reason.trim().is_empty() {
        return Err(sqlx::Error::Protocol("recovery reason required".into()));
    }
    let mut tx = run_store::lock(pool).await?;
    authorize_retry_in(&mut tx, id, now, reason).await?;
    tx.commit().await
}
pub(crate) async fn authorize_retry_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: &str,
    now: i64,
    reason: &str,
) -> Result<()> {
    let value: Value = sqlx::query_scalar(
        "SELECT retry FROM preparation_record p WHERE run_id=$1 AND NOT EXISTS(SELECT 1 FROM agent_run a WHERE a.id=p.run_id) FOR UPDATE",
    )
    .bind(id)
    .fetch_one(&mut **tx)
    .await?;
    let mut retry: Retry = decode(value)?;
    retry.authorize_retry_group(now);
    sqlx::query("UPDATE preparation_record SET ready=false WHERE run_id=$1")
        .bind(id)
        .execute(&mut **tx)
        .await?;
    save(
        tx,
        id,
        &retry,
        now,
        json!({"authorized_recovery":reason,"retry":retry}),
    )
    .await?;
    Ok(())
}

pub(crate) async fn claim_ready(
    tx: &mut Transaction<'_, Postgres>,
    launch: &Launch,
    id: i64,
    revision: i64,
) -> Result<bool> {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM preparation_record WHERE run_id=$1 AND requirement_id=$2 AND revision=$3 AND launch=$4 AND ready AND checked_at >= extract(epoch FROM now())::bigint-60)")
        .bind(&launch.key.run_id).bind(id).bind(revision).bind(json!(launch)).fetch_one(&mut **tx).await
}

pub async fn failed_probe(
    pool: &PgPool,
    launch: &Launch,
    failure: Failure,
    now: i64,
) -> Result<()> {
    let mut tx = run_store::lock(pool).await?;
    let (value, ready): (Value, bool) = sqlx::query_as(
        "SELECT retry,ready FROM preparation_record WHERE run_id=$1 AND launch=$2 FOR UPDATE",
    )
    .bind(&launch.key.run_id)
    .bind(json!(launch))
    .fetch_one(&mut *tx)
    .await?;
    let mut retry: Retry = decode(value)?;
    if ready || !retry.active() {
        return Ok(());
    }
    retry.fail(failure, now);
    save(
        &mut tx,
        &launch.key.run_id,
        &retry,
        now,
        json!({"retry":retry}),
    )
    .await?;
    tx.commit().await
}

async fn refresh_success(
    tx: &mut Transaction<'_, Postgres>,
    launch: &Launch,
    phase: &str,
    now: i64,
    retry: &mut Retry,
) -> Result<bool> {
    if phase != "answer_resume" {
        return Ok(false);
    }
    let stale: bool =
        sqlx::query_scalar("SELECT checked_at < $2-60 FROM preparation_record WHERE run_id=$1")
            .bind(&launch.key.run_id)
            .bind(now)
            .fetch_one(&mut **tx)
            .await?;
    if !stale {
        return Ok(false);
    }
    // Revalidate successful evidence using the original bounded ledger.
    // A pause cannot mint a fresh retry group or reset any attempt counter.
    retry.next_attempt_at = Some(now);
    sqlx::query("UPDATE preparation_record SET ready=false WHERE run_id=$1")
        .bind(&launch.key.run_id)
        .execute(&mut **tx)
        .await?;
    Ok(true)
}
