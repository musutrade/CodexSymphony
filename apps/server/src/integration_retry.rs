//! Finite infrastructure recovery; original invocation identities stay immutable.
use crate::{
    execution::Launch,
    integration_process::{Job, Outcome},
    integration_store, process, run_store,
};
use serde_json::Value;
use sqlx::PgPool;
use std::path::Path;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
pub(crate) async fn resume(
    pool: &PgPool,
    root: &Path,
    supervisor: &Path,
    incarnation: &str,
    id: &str,
    job: &Job,
    launch: &Launch,
) -> Result<()> {
    let mut tx = run_store::lock(pool).await?;
    if !integration_store::allowed(&mut tx, job).await? {
        return Ok(());
    }
    if !ready(&mut tx, root, id, job).await? {
        tx.commit().await?;
        return Ok(());
    }
    replace(&mut tx, root, supervisor, incarnation, id, job, launch).await?;
    tx.commit().await?;
    Ok(())
}
async fn ready(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    root: &Path,
    id: &str,
    job: &Job,
) -> Result<bool> {
    let (state, result, next): (String, Option<Value>, Option<i64>) = sqlx::query_as(
        "SELECT state,result,next_attempt_at FROM integration_validation WHERE id=$1 AND quiescent",
    )
    .bind(id)
    .fetch_one(&mut **tx)
    .await?;
    if matches!(state.as_str(), "passed" | "cancelled" | "interrupted") {
        return Ok(false);
    }
    if !retryable(result, root.join(id).join("stop.json").is_file())? {
        return Ok(false);
    }
    scheduled(tx, id, job, next).await
}
async fn scheduled(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: &str,
    job: &Job,
    next: Option<i64>,
) -> Result<bool> {
    let now = crate::runtime_client::now();
    let (count, first): (i64, i64) = sqlx::query_as("SELECT count(*),extract(epoch FROM min(created_at))::bigint FROM integration_validation WHERE requirement_id=$1")
        .bind(job.binding.requirement).fetch_one(&mut **tx).await?;
    let Some(next) = next else {
        schedule(tx, id, now, first, count).await?;
        return Ok(false);
    };
    Ok(next <= now && count <= 2 && now <= first.saturating_add(600))
}
async fn schedule(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: &str,
    now: i64,
    first: i64,
    count: i64,
) -> Result<()> {
    let next = crate::bounded_recovery::next_retry(
        now,
        first.saturating_add(600),
        (count - 1) as u32,
        None,
    );
    sqlx::query("UPDATE integration_validation SET next_attempt_at=$2,blocker=$3 WHERE id=$1")
        .bind(id)
        .bind(next)
        .bind(if next.is_some() {
            "known stopped infrastructure invocation; bounded validation retry scheduled"
        } else {
            "validation infrastructure retry budget exhausted; explicit recovery required"
        })
        .execute(&mut **tx)
        .await?;
    Ok(())
}
async fn replace(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    root: &Path,
    supervisor: &Path,
    incarnation: &str,
    id: &str,
    job: &Job,
    launch: &Launch,
) -> Result<()> {
    if crate::integration_process::versions(job)? != job.binding.versions
        || job.plan.identity()? != job.binding.trusted
    {
        return Err("recovery version or policy identity changed".into());
    }
    let new_id = format!("integration-{}", process::new_identity()?);
    let mut new_launch = launch.clone();
    new_launch.key.run_id = new_id.clone();
    new_launch.key.request_id = new_id.clone();
    new_launch.key.incarnation = incarnation.into();
    new_launch.program = supervisor.to_string_lossy().into_owned();
    new_launch.args = Vec::from([
        "--integration-validation".into(),
        root.join(&new_id).to_string_lossy().into_owned(),
    ]);
    sqlx::query("UPDATE integration_validation SET state='interrupted' WHERE id=$1")
        .bind(id)
        .execute(&mut **tx)
        .await?;
    let mut new_job = job.clone();
    new_job.invocation = new_id.clone();
    crate::integration_claim::persist(tx, &new_id, &new_job, &new_launch).await?;
    Ok(())
}
fn retryable(result: Option<Value>, stopped: bool) -> Result<bool> {
    let Some(result) = result else {
        return Ok(stopped);
    };
    let outcome: Outcome = serde_json::from_value(result)?;
    let Some(evidence) = outcome.evidence else {
        return Ok(stopped);
    };
    Ok(evidence.steps.iter().any(|s| {
        s.exit_code != Some(0)
            && matches!(
                crate::bounded_recovery::native_failure(&s.output),
                "service_unavailable" | "transport_timeout" | "runner_unavailable"
            )
    }))
}
