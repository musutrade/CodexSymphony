//! Local infrastructure retries preserve every validation attempt and its logs.
use crate::{
    bounded_recovery::{Failure, next_retry},
    git_broker::GitBroker,
    validation_runner::Plan,
};
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Transaction};
use std::path::Path;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub(crate) async fn schedule(
    tx: &mut Transaction<'_, Postgres>,
    event: &str,
    id: i64,
    failure: &Failure,
) -> std::result::Result<(), sqlx::Error> {
    let now = crate::runtime_client::now();
    let deadline = now.saturating_add(600);
    let next = next_retry(now, deadline, 0, failure.retry_after_seconds);
    sqlx::query("INSERT INTO recovery_retry(event_key,deadline,next_attempt_at,state) SELECT $1,$2,$3,$4 WHERE NOT EXISTS(SELECT 1 FROM recovery_retry t JOIN recovery_failure f USING(event_key) WHERE f.requirement_id=$5 AND f.phase=$6 AND f.facts->>'candidate_sha'=$7 AND f.facts->>'step'=$8)")
        .bind(event).bind(deadline).bind(next.unwrap_or(deadline)).bind(if next.is_some() { "pending" } else { "blocked" }).bind(id).bind(&failure.phase).bind(&failure.candidate_sha).bind(&failure.step).execute(&mut **tx).await?;
    Ok(())
}

pub async fn local(pool: &PgPool, root: &Path, broker: &GitBroker, plan: &Plan) -> Result<bool> {
    let now = crate::runtime_client::now();
    expire(pool, now).await?;
    let Some((event, validation)) = claim(pool, now).await? else {
        return Ok(false);
    };
    let result = execute_local(pool, root, broker, plan, &validation).await;
    match result {
        Ok(passed) => complete(pool, &event, &validation, passed, now).await?,
        Err(error) => {
            sqlx::query("UPDATE recovery_failure SET reason=$2 WHERE event_key=$1")
                .bind(&event)
                .bind(format!(
                    "validation process/output reconciliation required: {error}"
                ))
                .execute(pool)
                .await?;
        }
    }
    Ok(true)
}

pub async fn expire(pool: &PgPool, now: i64) -> Result<()> {
    let mut tx = crate::run_store::lock(pool).await?;
    sqlx::query("UPDATE recovery_retry SET state='blocked' WHERE state IN ('pending','unknown') AND deadline<$1").bind(now).execute(&mut *tx).await?;
    sqlx::query("UPDATE recovery_failure f SET decision='blocked',reason='infrastructure retry exhausted or outcome unresolved; inspect preserved evidence and restore the failed service before explicit recovery' FROM recovery_retry t WHERE t.event_key=f.event_key AND t.state='blocked' AND f.decision='infrastructure'").execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

async fn claim(pool: &PgPool, now: i64) -> Result<Option<(String, String)>> {
    let mut tx = crate::run_store::lock(pool).await?;
    let row: Option<(String,String,i32,String,Option<Value>)> = sqlx::query_as("SELECT t.event_key,f.source_validation_id,t.attempts,t.state,t.remote FROM recovery_retry t JOIN recovery_failure f USING(event_key) JOIN requirement r ON r.id=f.requirement_id JOIN execution_control c ON c.requirement_id=r.id JOIN candidate_validation v ON v.id=f.source_validation_id WHERE f.phase='local' AND t.state IN ('pending','unknown') AND t.next_attempt_at<=$1 AND t.deadline>=$1 AND NOT r.paused AND NOT r.cancel_requested AND NOT c.paused AND c.recovery_complete AND r.revision=v.revision AND NOT EXISTS(SELECT 1 FROM agent_run a WHERE a.requirement_id=r.id AND NOT a.quiescent) ORDER BY f.created_at LIMIT 1")
        .bind(now).fetch_optional(&mut *tx).await?;
    let Some((event, source, attempts, state, remote)) = row else {
        return Ok(None);
    };
    if state == "unknown" {
        return Ok(remote.and_then(|v| {
            v["validation"].as_str().map(|id| {
                // A recorded retry reuses its validation identity after restart.
                (event, id.to_owned())
            })
        }));
    }
    let validation = format!("{source}-infrastructure-{}", attempts + 1);
    sqlx::query("INSERT INTO candidate_validation(id,requirement_id,revision,source_run_id,candidate_sha,candidate_tree,trusted,required_steps,source_before,source_after,entry_before,entry_after,stage,result,retry_of) SELECT $1,requirement_id,revision,source_run_id,candidate_sha,candidate_tree,trusted,required_steps,source_before,source_before,entry_before,entry_before,'declaration','pending',id FROM candidate_validation WHERE id=$2")
        .bind(&validation).bind(&source).execute(&mut *tx).await?;
    sqlx::query("UPDATE recovery_retry SET state='unknown',attempts=attempts+1,remote=$2 WHERE event_key=$1 AND attempts<2")
        .bind(&event).bind(json!({"validation":validation})).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(Some((event, validation)))
}

async fn complete(
    pool: &PgPool,
    event: &str,
    validation: &str,
    passed: bool,
    now: i64,
) -> Result<()> {
    let mut tx = crate::run_store::lock(pool).await?;
    let (attempts, deadline): (i32, i64) =
        sqlx::query_as("SELECT attempts,deadline FROM recovery_retry WHERE event_key=$1")
            .bind(event)
            .fetch_one(&mut *tx)
            .await?;
    let infrastructure: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM recovery_failure WHERE source_validation_id=$1 AND decision='infrastructure')").bind(validation).fetch_one(&mut *tx).await?;
    let next = if infrastructure {
        next_retry(now, deadline, attempts as u32, None)
    } else {
        None
    };
    let state = retry_state(passed, next);
    sqlx::query("UPDATE recovery_retry SET state=$2,next_attempt_at=$3,receipts=receipts||$4 WHERE event_key=$1")
        .bind(event).bind(state).bind(next.unwrap_or(deadline)).bind(json!([{"validation":validation,"passed":passed}])).execute(&mut *tx).await?;
    let decision = recovery_decision(passed, infrastructure, next);
    sqlx::query("UPDATE recovery_failure f SET decision=$2,reason=CASE WHEN $2='blocked' THEN 'infrastructure retry budget exhausted; restore service and explicitly recover' ELSE f.reason END FROM recovery_failure original WHERE original.event_key=$1 AND f.requirement_id=original.requirement_id AND f.phase=original.phase AND f.facts->>'candidate_sha'=original.facts->>'candidate_sha' AND f.decision='infrastructure'")
        .bind(event).bind(decision).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

fn retry_state(passed: bool, next: Option<i64>) -> &'static str {
    if passed {
        "complete"
    } else if next.is_some() {
        "pending"
    } else {
        "blocked"
    }
}
fn recovery_decision(passed: bool, infrastructure: bool, next: Option<i64>) -> &'static str {
    if passed {
        "recovered"
    } else if !infrastructure {
        "reclassified"
    } else if next.is_none() {
        "blocked"
    } else {
        "infrastructure"
    }
}

async fn execute_local(
    pool: &PgPool,
    root: &Path,
    broker: &GitBroker,
    plan: &Plan,
    validation: &str,
) -> Result<bool> {
    let (source, id, revision, manifest): (String,i64,i64,Value) = sqlx::query_as("SELECT v.source_run_id,v.requirement_id,v.revision,s.manifest FROM candidate_validation v JOIN workspace_snapshot s ON s.run_id=v.source_run_id WHERE v.id=$1")
        .bind(validation).fetch_one(pool).await?;
    let manifest = serde_json::from_value(manifest)?;
    let checkout = crate::validation_worker::restore(broker, &source, &manifest)?;
    let candidate = crate::validation_runner::candidate(&checkout)?;
    crate::validation_service::validate(
        pool,
        crate::validation_service::Request {
            id: validation,
            source_run: &source,
            requirement: id,
            revision,
            checkout: &checkout,
            directory: &root.join(validation),
            candidate: &candidate,
            plan,
        },
    )
    .await
}
