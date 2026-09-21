//! Persist before rerun; after a lost response observe the remote attempt first.
use crate::{github::Policy, github_http::AppClient};
use serde_json::{Value, json};
use sqlx::PgPool;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub async fn tick(pool: &PgPool, client: &mut AppClient, now: i64) -> Result<()> {
    crate::recovery_retry::expire(pool, now).await?;
    let rows: Vec<(String,String,Value,i64)> = sqlx::query_as("SELECT t.event_key,t.state,t.remote,t.remote_attempt FROM recovery_retry t JOIN recovery_failure f USING(event_key) WHERE f.phase IN ('ci','review') AND t.state IN ('pending','unknown') AND t.next_attempt_at<=$1 AND t.remote IS NOT NULL AND t.remote_attempt IS NOT NULL ORDER BY f.created_at LIMIT 1")
        .bind(now).fetch_all(pool).await?;
    for (event, state, remote, attempt) in rows {
        if let Err(error) = reconcile(pool, client, now, &event, &state, &remote, attempt).await {
            let retry_after = error
                .downcast_ref::<crate::github_http::Error>()
                .and_then(|e| {
                    // Keep the native closure boundary in the source inventory.
                    e.retry_after_seconds
                });
            query_failed(pool, &event, now, retry_after, &error.to_string()).await?;
        }
    }
    Ok(())
}

async fn reconcile(
    pool: &PgPool,
    client: &mut AppClient,
    now: i64,
    event: &str,
    state: &str,
    remote: &Value,
    attempt: i64,
) -> Result<()> {
    let policy: Policy = serde_json::from_value(remote["policy"].clone())?;
    let path = format!(
        "/repos/{}/actions/runs/{}",
        policy.repository, remote["run"]
    );
    let run = client.get(&policy, &path, now).await?;
    if run["head_sha"] != remote["head"] {
        return block(pool, event, "remote run identity changed").await;
    }
    if run["run_attempt"]
        .as_i64()
        .is_some_and(|current| current > attempt)
    {
        sqlx::query(
            "UPDATE recovery_retry SET state='complete',receipts=receipts||$2 WHERE event_key=$1",
        )
        .bind(event)
        .bind(json!([{"observed_run":run}]))
        .execute(pool)
        .await?;
        return Ok(());
    }
    if state == "unknown" {
        sqlx::query(
            "UPDATE recovery_retry SET next_attempt_at=$2 WHERE event_key=$1 AND state='unknown'",
        )
        .bind(event)
        .bind(now.saturating_add(30))
        .execute(pool)
        .await?;
        return Ok(());
    }
    dispatch(pool, client, now, event, remote, &policy, &path).await
}
async fn dispatch(
    pool: &PgPool,
    client: &mut AppClient,
    now: i64,
    event: &str,
    remote: &Value,
    policy: &Policy,
    path: &str,
) -> Result<()> {
    if !policy.delivery.as_ref().is_some_and(|contract| {
        // Only this explicitly reviewed remote action can be dispatched.
        contract.actions.rerun_actions
    }) {
        return block(pool, event, "repository did not authorize Actions rerun").await;
    }
    let pr = client
        .get(
            policy,
            &format!("/repos/{}/pulls/{}", policy.repository, remote["pr"]),
            now,
        )
        .await?;
    if pr["head"]["sha"] != remote["head"] || pr["state"] != "open" {
        return block(
            pool,
            event,
            "PR head/state changed; preserve original failure",
        )
        .await;
    }
    if !admit(pool, event, policy, now).await? {
        return Ok(());
    }
    let result = client
        .write(
            policy,
            reqwest::Method::POST,
            &format!("{path}/rerun-failed-jobs"),
            json!({}),
            now,
        )
        .await;
    save_receipt(pool, event, now, result).await
}
async fn save_receipt(
    pool: &PgPool,
    event: &str,
    now: i64,
    result: std::result::Result<Value, crate::github_http::Error>,
) -> Result<()> {
    let (receipt, retry_after, rejected) = match result {
        Ok(value) => (json!({"response":value}), None, false),
        Err(error) => (
            json!({"code":error.code,"http_status":error.status,"retry_after_seconds":error.retry_after_seconds}),
            error.retry_after_seconds,
            error.status == Some(429),
        ),
    };
    sqlx::query(
        "UPDATE recovery_retry SET receipts=receipts||$2,next_attempt_at=$3,state=CASE WHEN $4 AND attempts<2 THEN 'pending' ELSE state END WHERE event_key=$1",
    )
    .bind(event)
    .bind(json!([receipt]))
    .bind(
        now.saturating_add(
            i64::try_from(retry_after.unwrap_or(0))
                .unwrap_or(i64::MAX)
                .max(if rejected { 120 } else { 30 }),
        ),
    )
    .bind(rejected)
    .execute(pool)
    .await?;
    Ok(())
}

async fn query_failed(
    pool: &PgPool,
    event: &str,
    now: i64,
    retry_after: Option<u64>,
    reason: &str,
) -> Result<()> {
    let mut tx = crate::run_store::lock(pool).await?;
    let (failures, deadline): (i32, i64) =
        sqlx::query_as("SELECT probe_failures,deadline FROM recovery_retry WHERE event_key=$1")
            .bind(event)
            .fetch_one(&mut *tx)
            .await?;
    let next = crate::bounded_recovery::next_retry(now, deadline, failures as u32, retry_after);
    sqlx::query("UPDATE recovery_retry SET probe_failures=probe_failures+1,next_attempt_at=$2,state=CASE WHEN $3 THEN state ELSE 'blocked' END,receipts=receipts||$4 WHERE event_key=$1")
        .bind(event).bind(next.unwrap_or(deadline)).bind(next.is_some()).bind(json!([{"query_error":reason}])).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

async fn admit(pool: &PgPool, event: &str, policy: &Policy, now: i64) -> Result<bool> {
    let mut tx = crate::run_store::lock(pool).await?;
    let allowed: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM recovery_failure f JOIN requirement r ON r.id=f.requirement_id JOIN requirement_revision v ON v.requirement_id=r.id AND v.revision=r.revision JOIN repository p ON p.id=COALESCE((v.document->>'repository_id')::bigint,1) JOIN execution_control c ON c.requirement_id=r.id JOIN github_repository g ON g.repository_id=(v.document#>>'{repository,github_repository_id}')::bigint WHERE f.event_key=$1 AND NOT r.paused AND NOT r.cancel_requested AND NOT c.paused AND c.recovery_complete AND NOT (p.document->>'revoked')::boolean AND (v.document->>'repository_version')::bigint>p.revoked_through_version AND g.policy=$2 AND NOT g.stale AND NOT (SELECT blocked FROM storage_guard WHERE id=1))")
        .bind(event).bind(json!(policy)).fetch_one(&mut *tx).await?;
    if !allowed {
        return Ok(false);
    }
    let result = sqlx::query("UPDATE recovery_retry SET state='unknown',attempts=attempts+1 WHERE event_key=$1 AND state='pending' AND attempts<2 AND deadline>=$2 AND next_attempt_at<=$2")
        .bind(event).bind(now).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(result.rows_affected() == 1)
}

async fn block(pool: &PgPool, event: &str, reason: &str) -> Result<()> {
    let mut tx = crate::run_store::lock(pool).await?;
    sqlx::query("UPDATE recovery_retry SET state='blocked' WHERE event_key=$1")
        .bind(event)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE recovery_failure SET decision='blocked',reason=$2 WHERE event_key=$1")
        .bind(event)
        .bind(reason)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}
