//! Dispatch one explicitly approved validation generation without rerunning Agent.
use crate::{git_broker::GitBroker, validation_runner::Plan, workspace::Manifest};
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Transaction};
use std::path::{Path, PathBuf};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
type Pending = (String, String, i64, i64, String, Value, Value);
type Tx<'a> = Transaction<'a, Postgres>;

pub async fn tick(
    pool: &PgPool,
    root: &Path,
    broker: &GitBroker,
    plan: &Plan,
    hooks: &Value,
) -> Result<bool> {
    let row: Option<Pending> = sqlx::query_as("SELECT f.event_key,v.id,v.requirement_id,v.revision,v.source_run_id,s.manifest,f.resolution FROM recovery_failure f JOIN candidate_validation v ON v.id=f.source_validation_id JOIN workspace_snapshot s ON s.run_id=v.source_run_id JOIN execution_control c ON c.requirement_id=v.requirement_id JOIN requirement r ON r.id=v.requirement_id AND r.revision=v.revision WHERE f.resolution_state IN ('pending','running') ORDER BY f.created_at LIMIT 1").fetch_optional(pool).await?;
    let Some(row) = row else {
        return Ok(false);
    };
    if !crate::validation_context::allowed(pool, row.2, row.3).await? {
        return Ok(false);
    }
    let event = row.0.clone();
    let result = execute(pool, root, broker, plan, hooks, row).await;
    settle(pool, &event, result).await?;
    Ok(true)
}

async fn settle(pool: &PgPool, event: &str, result: Result<bool>) -> Result<()> {
    match result {
        Ok(passed) => {
            sqlx::query(
                "UPDATE recovery_failure SET resolution_state=$2,decision=$3 WHERE event_key=$1",
            )
            .bind(event)
            .bind(if passed { "complete" } else { "failed" })
            .bind(if passed { "recovered" } else { "continued" })
            .execute(pool)
            .await?;
        }
        Err(error) => {
            fail_successor(pool, event).await?;
            sqlx::query("UPDATE recovery_failure SET resolution_state='blocked',reason=$2 WHERE event_key=$1")
                .bind(event).bind(crate::operator_view::redact_text(&error.to_string())).execute(pool).await?;
        }
    }
    Ok(())
}

struct Prepared {
    validation: String,
    source_run: String,
    requirement: i64,
    revision: i64,
    manifest: Manifest,
    checkout: PathBuf,
    candidate: crate::validation::Candidate,
}

async fn execute(
    pool: &PgPool,
    root: &Path,
    broker: &GitBroker,
    plan: &Plan,
    hooks: &Value,
    row: Pending,
) -> Result<bool> {
    check_plan(&row.6, plan)?;
    let prepared = prepare(pool, root, broker, plan, hooks, row).await?;
    let result = crate::validation_service::validate(
        pool,
        crate::validation_service::Request {
            id: &prepared.validation,
            source_run: &prepared.source_run,
            requirement: prepared.requirement,
            revision: prepared.revision,
            checkout: &prepared.checkout,
            directory: &root.join(&prepared.validation),
            candidate: &prepared.candidate,
            plan,
        },
    )
    .await;
    // after_run is only valid after the execution is known stopped.
    if result.is_ok() {
        crate::validation_worker::finish_validation_hook(
            pool,
            root,
            broker,
            &prepared.validation,
            &prepared.manifest,
        )
        .await?;
    }
    result
}

fn check_plan(resolution: &Value, plan: &Plan) -> Result<()> {
    if resolution["command"]["action"]["plan_digest"] != plan.identity()?.config_sha256 {
        return Err("approved recovery plan differs from deployed validation route".into());
    }
    Ok(())
}

async fn prepare(
    pool: &PgPool,
    root: &Path,
    broker: &GitBroker,
    plan: &Plan,
    hooks: &Value,
    row: Pending,
) -> Result<Prepared> {
    let (event, source_validation, requirement, revision, source_run, manifest, _) = row;
    let validation = claim(pool, &event, &source_validation, plan).await?;
    let manifest = serde_json::from_value(manifest)?;
    let checkout = crate::validation_worker::restore(broker, &source_run, &manifest)?;
    crate::environment_service::admit(pool, requirement, revision, "validation", Some(&checkout))
        .await?;
    let candidate = crate::validation_runner::candidate(&checkout)?;
    if !crate::validation_worker::prepare_validation_hook(
        pool,
        root,
        &validation,
        &manifest,
        &checkout,
        hooks,
    )
    .await?
    {
        return Err("revalidation preparation hook did not pass".into());
    }
    Ok(Prepared {
        validation,
        source_run,
        requirement,
        revision,
        manifest,
        checkout,
        candidate,
    })
}

async fn claim(pool: &PgPool, event: &str, source: &str, plan: &Plan) -> Result<String> {
    let mut tx = crate::run_store::lock(pool).await?;
    authorize(&mut tx, source).await?;
    let saved: Option<String> = sqlx::query_scalar("SELECT successor_validation FROM recovery_failure WHERE event_key=$1 AND resolution_state IN ('pending','running')").bind(event).fetch_one(&mut *tx).await?;
    if let Some(saved) = saved {
        return Ok(saved);
    }
    let validation = create_successor(&mut tx, event, source, plan).await?;
    tx.commit().await?;
    Ok(validation)
}

async fn authorize(tx: &mut Tx<'_>, source: &str) -> Result<()> {
    let (id, revision, incarnation): (i64,i64,String) = sqlx::query_as("SELECT v.requirement_id,v.revision,c.incarnation FROM candidate_validation v JOIN execution_control c ON c.requirement_id=v.requirement_id WHERE v.id=$1").bind(source).fetch_one(&mut **tx).await?;
    crate::budget_store::require(
        crate::recovery_store::allowed(tx, id, revision, &incarnation).await?,
        "revalidation authority changed",
    )?;
    let current: bool = sqlx::query_scalar("SELECT NOT EXISTS(SELECT 1 FROM candidate_validation v JOIN agent_run original ON original.id=v.source_run_id JOIN agent_run newer ON newer.requirement_id=v.requirement_id AND newer.run_sequence>original.run_sequence WHERE v.id=$1)").bind(source).fetch_one(&mut **tx).await?;
    crate::budget_store::require(current, "source Run superseded")?;
    Ok(())
}

async fn source_stopped(tx: &mut Tx<'_>, source: &str) -> Result<()> {
    let context: Option<Value> = sqlx::query_scalar(
        "SELECT hook_context FROM candidate_validation WHERE id=$1 AND superseded_by IS NULL",
    )
    .bind(source)
    .fetch_one(&mut **tx)
    .await?;
    crate::extension_recovery::stopped(context)?;
    Ok(())
}

async fn create_successor(
    tx: &mut Tx<'_>,
    event: &str,
    source: &str,
    plan: &Plan,
) -> Result<String> {
    source_stopped(tx, source).await?;
    let validation = format!("revalidate-{}", crate::process::new_identity()?);
    let trusted = plan.identity()?;
    sqlx::query("INSERT INTO candidate_validation(id,requirement_id,revision,source_run_id,candidate_sha,candidate_tree,trusted,required_steps,source_before,source_after,entry_before,entry_after,stage,result,retry_of,approved_plan,hook_required) SELECT $1,requirement_id,revision,source_run_id,candidate_sha,candidate_tree,$3,required_steps,source_before,source_before,$4,$4,'declaration','pending',id,$5,hook_required FROM candidate_validation WHERE id=$2 AND result IN ('blocked','gate_failed') AND superseded_by IS NULL")
        .bind(&validation).bind(source).bind(json!(trusted)).bind(&trusted.protected_entry_sha256).bind(json!(plan)).execute(&mut **tx).await?;
    sqlx::query(
        "UPDATE candidate_validation SET superseded_by=$2,hook_invalidated=true WHERE id=$1",
    )
    .bind(source)
    .bind(&validation)
    .execute(&mut **tx)
    .await?;
    sqlx::query("UPDATE recovery_failure SET successor_validation=$2,resolution_state='running' WHERE event_key=$1").bind(event).bind(&validation).execute(&mut **tx).await?;
    Ok(validation)
}

async fn fail_successor(pool: &PgPool, event: &str) -> Result<()> {
    let successor: Option<String> =
        sqlx::query_scalar("SELECT successor_validation FROM recovery_failure WHERE event_key=$1")
            .bind(event)
            .fetch_one(pool)
            .await?;
    if let Some(id) = successor {
        sqlx::query("UPDATE candidate_validation SET result='blocked',failure='revalidation interrupted; reconcile execution and preparation evidence' WHERE id=$1 AND result='pending'").bind(&id).execute(pool).await?;
        crate::extension_failure::crashed(pool, &id).await?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "../tests/unit/extension_revalidation.rs"]
mod tests;
