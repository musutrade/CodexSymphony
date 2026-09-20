//! Durable call intents and cumulative settlement. All mutations share the
//! review/revocation lock; no model or remote action is performed here.
use crate::{
    budget::{Amount, Purpose, Usage, Waiting},
    contract::Policy,
    execution::RunKey,
    run_store,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Transaction};
type Result<T> = std::result::Result<T, sqlx::Error>;
type Tx<'a> = Transaction<'a, Postgres>;

pub(crate) fn require(ok: bool, message: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(sqlx::Error::Protocol(message.into()))
    }
}
pub(crate) fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T> {
    serde_json::from_value(value).map_err(decode_error)
}

fn decode_error(error: serde_json::Error) -> sqlx::Error {
    sqlx::Error::Decode(Box::new(error))
}
fn accounting_overflow() -> sqlx::Error {
    sqlx::Error::Protocol("budget accounting overflow".into())
}

pub(crate) async fn freeze(tx: &mut Tx<'_>, id: i64, policy: &Policy) -> Result<()> {
    let limits = Amount {
        tokens: policy.token_limit,
        turns: policy.turn_limit,
        model_seconds: policy.model_work_seconds,
    };
    sqlx::query("INSERT INTO requirement_budget(requirement_id,limits) VALUES($1,$2) ON CONFLICT DO NOTHING")
        .bind(id).bind(json!(limits)).execute(&mut **tx).await?;
    sqlx::query("INSERT INTO budget_authorization(requirement_id,version,request_id,actor,reason,delta,limits) SELECT requirement_id,1,'initial-budget:'||requirement_id,'local-user','initial reviewed authorization',limits,limits FROM requirement_budget WHERE requirement_id=$1 ON CONFLICT DO NOTHING")
        .bind(id).execute(&mut **tx).await?;
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Increase {
    pub request_id: String,
    pub requirement_id: i64,
    pub expected_version: i64,
    pub actor: String,
    pub reason: String,
    pub delta: Amount,
}
/// Explicit platform/user authorization boundary, like preparation recovery.
/// It grants only resources, never repair ordinals, process restart or unpause.
pub async fn increase(pool: &PgPool, input: &Increase) -> Result<()> {
    validate_increase(input)?;
    let mut tx = run_store::lock(pool).await?;
    if replay_increase(&mut tx, input).await? {
        return Ok(());
    }
    apply_increase(&mut tx, input).await?;
    tx.commit().await
}
fn validate_increase(input: &Increase) -> Result<()> {
    require(
        !input.request_id.is_empty()
            && !input.actor.trim().is_empty()
            && !input.reason.trim().is_empty(),
        "authorization identity and reason required",
    )?;
    require(
        input.delta.nonnegative() && input.delta != Amount::default(),
        "positive increment required",
    )?;
    Ok(())
}
async fn apply_increase(tx: &mut Tx<'_>, input: &Increase) -> Result<()> {
    let grouped: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM group_execution_item WHERE requirement_id=$1)",
    )
    .bind(input.requirement_id)
    .fetch_one(&mut **tx)
    .await?;
    require(!grouped, "group budget changes require group review")?;
    let (limits, version): (Value, i64) = sqlx::query_as(
        "SELECT limits,version FROM requirement_budget WHERE requirement_id=$1 FOR UPDATE",
    )
    .bind(input.requirement_id)
    .fetch_one(&mut **tx)
    .await?;
    require(
        version == input.expected_version,
        "authorization version conflict",
    )?;
    let limits = decode::<Amount>(limits)?
        .checked_add(input.delta)
        .ok_or_else(accounting_overflow)?;
    sqlx::query("INSERT INTO budget_authorization(requirement_id,version,request_id,actor,reason,delta,limits) VALUES($1,$2,$3,$4,$5,$6,$7)")
        .bind(input.requirement_id).bind(version + 1).bind(&input.request_id).bind(&input.actor).bind(&input.reason).bind(json!(input.delta)).bind(json!(limits)).execute(&mut **tx).await?;
    sqlx::query("UPDATE requirement_budget SET limits=$2,version=version+1,exhausted=false WHERE requirement_id=$1")
        .bind(input.requirement_id).bind(json!(limits)).execute(&mut **tx).await?;
    Ok(())
}
async fn replay_increase(tx: &mut Tx<'_>, input: &Increase) -> Result<bool> {
    let previous: Option<Value> = sqlx::query_scalar("SELECT jsonb_build_object('request_id',request_id,'requirement_id',requirement_id,'expected_version',version-1,'actor',actor,'reason',reason,'delta',delta) FROM budget_authorization WHERE request_id=$1")
        .bind(&input.request_id).fetch_optional(&mut **tx).await?;
    if let Some(previous) = previous {
        require(
            previous == json!(input),
            "authorization request identity conflict",
        )?;
        return Ok(true);
    }
    Ok(false)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallIntent {
    pub key: RunKey,
    pub turn_id: String,
    pub purpose: Purpose,
    pub reserve: Amount,
}
#[derive(Debug, PartialEq, Eq)]
pub enum Admission {
    Reserved,
    Existing,
    Blocked,
}

/// Only Reserved permits dispatch. Existing includes an UNKNOWN result after
/// crash: reconcile that intent; never resend it as a new model request.
pub async fn reserve(pool: &PgPool, intent: &CallIntent) -> Result<Admission> {
    validate_intent(intent)?;
    if !run_store::actions_allowed(pool, &intent.key).await? {
        return Ok(Admission::Blocked);
    }
    let mut tx = run_store::lock(pool).await?;
    let result = reserve_in_transaction(&mut tx, intent).await?;
    tx.commit().await?;
    Ok(result)
}
fn validate_intent(intent: &CallIntent) -> Result<()> {
    require(
        !intent.turn_id.is_empty() && intent.turn_id.len() <= 200,
        "turn identity required",
    )?;
    require(
        intent.reserve.positive() && intent.reserve.turns == 1,
        "one positive turn reservation required",
    )?;
    Ok(())
}
async fn reserve_in_transaction(tx: &mut Tx<'_>, intent: &CallIntent) -> Result<Admission> {
    if existing(tx, intent).await? {
        return Ok(Admission::Existing);
    }
    let Some(id) = admitted_run(tx, &intent.key).await? else {
        return Ok(Admission::Blocked);
    };
    let balance = balance(tx, id).await?;
    let fits = balance
        .exposure
        .checked_add(intent.reserve)
        .is_some_and(|total| total.fits(balance.limits));
    if balance.exhausted || !fits || !crate::group_budget::allowed(tx, id, intent.reserve).await? {
        stop(tx, id).await?;
        return Ok(Admission::Blocked);
    }
    sqlx::query("INSERT INTO model_call(run_id,turn_id,requirement_id,intent,reserved,usage) VALUES($1,$2,$3,$4,$5,$6)")
        .bind(&intent.key.run_id).bind(&intent.turn_id).bind(id).bind(json!(intent)).bind(json!(intent.reserve)).bind(json!(Usage::default())).execute(&mut **tx).await?;
    crate::group_budget::sync(tx, id).await?;
    Ok(Admission::Reserved)
}
async fn existing(tx: &mut Tx<'_>, intent: &CallIntent) -> Result<bool> {
    let saved: Option<Value> =
        sqlx::query_scalar("SELECT intent FROM model_call WHERE run_id=$1 AND turn_id=$2")
            .bind(&intent.key.run_id)
            .bind(&intent.turn_id)
            .fetch_optional(&mut **tx)
            .await?;
    if let Some(saved) = saved {
        require(saved == json!(intent), "call identity conflict")?;
        return Ok(true);
    }
    Ok(false)
}
async fn admitted_run(tx: &mut Tx<'_>, key: &RunKey) -> Result<Option<i64>> {
    sqlx::query_scalar("SELECT a.requirement_id FROM agent_run a JOIN requirement r ON r.id=a.requirement_id JOIN execution_control c ON c.requirement_id=r.id JOIN requirement_revision v ON v.requirement_id=r.id AND v.revision=a.revision CROSS JOIN repository p WHERE p.id=COALESCE((v.document->>'repository_id')::bigint,1) AND a.id=$1 AND a.request_id=$2 AND a.incarnation=$3 AND c.incarnation=a.incarnation AND c.recovery_complete AND NOT c.paused AND NOT r.paused AND NOT a.stop_requested AND NOT a.quiescent AND a.state IN ('Created','Running') AND a.model IS NOT NULL AND a.model<>'' AND NOT (p.document->>'revoked')::boolean AND (v.document->>'repository_version')::bigint>p.revoked_through_version AND NOT (SELECT blocked FROM storage_guard WHERE id=1) AND a.created_at>now()-interval '8 hours'")
        .bind(&key.run_id).bind(&key.request_id).bind(&key.incarnation).fetch_optional(&mut **tx).await
}

#[derive(Debug, Serialize)]
pub struct Balance {
    pub limits: Amount,
    /// Known usage lower bounds, including observations from unfinished calls.
    pub used: Amount,
    /// Actual + conservative outstanding reservations, without double charging.
    pub exposure: Amount,
    pub unresolved_calls: usize,
    pub exhausted: bool,
}
pub(crate) async fn balance(tx: &mut Tx<'_>, id: i64) -> Result<Balance> {
    let (limits, exhausted): (Value, bool) =
        sqlx::query_as("SELECT limits,exhausted FROM requirement_budget WHERE requirement_id=$1")
            .bind(id)
            .fetch_one(&mut **tx)
            .await?;
    let mut result = Balance {
        limits: decode(limits)?,
        used: Amount::default(),
        exposure: Amount::default(),
        unresolved_calls: 0,
        exhausted,
    };
    let rows: Vec<(Value, Value)> =
        sqlx::query_as("SELECT reserved,usage FROM model_call WHERE requirement_id=$1")
            .bind(id)
            .fetch_all(&mut **tx)
            .await?;
    for (reserved, usage) in rows {
        accumulate(&mut result, decode(reserved)?, decode(usage)?)?;
    }
    Ok(result)
}
fn accumulate(result: &mut Balance, reserved: Amount, usage: Usage) -> Result<()> {
    result.used = usage
        .actual()
        .and_then(|actual| result.used.checked_add(actual))
        .ok_or_else(accounting_overflow)?;
    result.exposure = usage
        .exposure(reserved)
        .and_then(|exposure| result.exposure.checked_add(exposure))
        .ok_or_else(accounting_overflow)?;
    result.unresolved_calls += usize::from(!usage.settled());
    Ok(())
}
pub async fn inspect(pool: &PgPool, id: i64) -> Result<Balance> {
    let mut tx = run_store::lock(pool).await?;
    balance(&mut tx, id).await
}
pub(crate) async fn stop(tx: &mut Tx<'_>, id: i64) -> Result<()> {
    sqlx::query("UPDATE requirement_budget SET exhausted=true WHERE requirement_id=$1")
        .bind(id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("UPDATE agent_run SET stop_requested=true,blocker='budget_exhausted: stop and preserve' WHERE requirement_id=$1 AND NOT quiescent")
        .bind(id).execute(&mut **tx).await?;
    Ok(())
}

/// Accounting accepts late events even from stopped/old Runs. Their exact key
/// must match the original call; it cannot alter a newer Run's lifecycle.
pub async fn settle(
    pool: &PgPool,
    key: &RunKey,
    turn: &str,
    event: &str,
    usage: &Usage,
) -> Result<()> {
    require(!event.is_empty() && usage.valid(), "invalid usage event")?;
    let mut tx = run_store::lock(pool).await?;
    let id = apply_usage(&mut tx, key, turn, event, usage).await?;
    crate::group_budget::sync(&mut tx, id).await?;
    let balance = balance(&mut tx, id).await?;
    if balance.used.execution_exhausted(balance.limits)
        || !balance.exposure.fits(balance.limits)
        || crate::group_budget::exhausted(&mut tx, id).await?
    {
        stop(&mut tx, id).await?;
        crate::group_budget::stop_group(&mut tx, id).await?;
    }
    tx.commit().await
}
async fn apply_usage(
    tx: &mut Tx<'_>,
    key: &RunKey,
    turn: &str,
    event: &str,
    usage: &Usage,
) -> Result<i64> {
    let (id, intent, previous): (i64, Value, Value) = sqlx::query_as("SELECT requirement_id,intent,usage FROM model_call WHERE run_id=$1 AND turn_id=$2 FOR UPDATE")
        .bind(&key.run_id).bind(turn).fetch_one(&mut **tx).await?;
    require(
        decode::<CallIntent>(intent)?.key == *key,
        "usage Run identity conflict",
    )?;
    archive_usage(tx, key, turn, event, usage).await?;
    let merged = decode::<Usage>(previous)?.merge(usage);
    require(merged.valid(), "inconsistent cumulative counters")?;
    sqlx::query("UPDATE model_call SET usage=$3 WHERE run_id=$1 AND turn_id=$2")
        .bind(&key.run_id)
        .bind(turn)
        .bind(json!(merged))
        .execute(&mut **tx)
        .await?;
    Ok(id)
}
async fn archive_usage(
    tx: &mut Tx<'_>,
    key: &RunKey,
    turn: &str,
    event: &str,
    usage: &Usage,
) -> Result<()> {
    sqlx::query("INSERT INTO model_usage_event(run_id,turn_id,event_id,usage) VALUES($1,$2,$3,$4) ON CONFLICT DO NOTHING")
        .bind(&key.run_id).bind(turn).bind(event).bind(json!(usage)).execute(&mut **tx).await?;
    let saved: Value = sqlx::query_scalar(
        "SELECT usage FROM model_usage_event WHERE run_id=$1 AND turn_id=$2 AND event_id=$3",
    )
    .bind(&key.run_id)
    .bind(turn)
    .bind(event)
    .fetch_one(&mut **tx)
    .await?;
    require(saved == json!(usage), "usage event identity conflict")
}

pub async fn record_waiting(pool: &PgPool, key: &RunKey, waiting: &Waiting) -> Result<()> {
    require(waiting.valid(), "negative waiting duration")?;
    let mut tx = run_store::lock(pool).await?;
    let old: Value = sqlx::query_scalar(
        "SELECT waiting FROM agent_run WHERE id=$1 AND request_id=$2 AND incarnation=$3 FOR UPDATE",
    )
    .bind(&key.run_id)
    .bind(&key.request_id)
    .bind(&key.incarnation)
    .fetch_one(&mut *tx)
    .await?;
    let merged = decode::<Waiting>(old)?.merge(waiting);
    sqlx::query("UPDATE agent_run SET waiting=$2 WHERE id=$1")
        .bind(&key.run_id)
        .bind(json!(merged))
        .execute(&mut *tx)
        .await?;
    tx.commit().await
}
/// Absolute database creation time includes all waits and survives restart.
pub async fn expire_runs(pool: &PgPool) -> Result<()> {
    sqlx::query("UPDATE agent_run SET stop_requested=true,blocker='runtime_timeout: absolute Run lifetime; stop and preserve' WHERE NOT quiescent AND created_at <= now()-($1 * interval '1 second')")
        .bind(crate::budget::RUN_LIFETIME_SECONDS).execute(pool).await?;
    Ok(())
}
