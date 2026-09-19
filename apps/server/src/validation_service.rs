//! Platform validation entrypoint. Source Run success and Gate success remain
//! separate; durable claims prevent replay after lost process/output evidence.
use crate::{
    validation::{Candidate, FailureKind, StepEvidence},
    validation_runner::{self, Plan},
    validation_store as store,
};
use sqlx::PgPool;
use std::path::Path;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub struct Request<'a> {
    pub id: &'a str,
    pub source_run: &'a str,
    pub requirement: i64,
    pub revision: i64,
    pub checkout: &'a Path,
    pub directory: &'a Path,
    pub candidate: &'a Candidate,
    pub plan: &'a Plan,
}

pub async fn validate(pool: &PgPool, r: Request<'_>) -> Result<bool> {
    let trusted = r.plan.identity()?;
    let required: Vec<String> = r.plan.steps.iter().map(step_id).collect();
    create(pool, &r, &trusted).await?;
    if !pending(pool, &r, &trusted).await? {
        if !store::status(pool, r.id).await?.is_some_and(succeeded) {
            return Ok(false);
        }
        if !r.directory.join("binding.json").exists() {
            return Err("accepted validation evidence unavailable".into());
        }
        return reconcile(r).await;
    }
    execute(pool, r, &trusted, &required).await
}
async fn reconcile(r: Request<'_>) -> Result<bool> {
    let checkout = r.checkout.to_owned();
    let directory = r.directory.to_owned();
    let candidate = r.candidate.clone();
    let plan = r.plan.clone();
    tokio::task::spawn_blocking(move || {
        validation_runner::execute(&checkout, &directory, &candidate, &plan)
    })
    .await??;
    Ok(true)
}
fn succeeded(status: serde_json::Value) -> bool {
    status["result"] == "succeeded"
}
async fn create(
    pool: &PgPool,
    r: &Request<'_>,
    trusted: &crate::validation::TrustedIdentity,
) -> Result<()> {
    store::create(
        pool,
        r.id,
        r.requirement,
        r.revision,
        r.source_run,
        r.candidate,
        trusted,
        &r.candidate.tree,
        &trusted.protected_entry_sha256,
    )
    .await?;
    Ok(())
}
async fn pending(
    pool: &PgPool,
    r: &Request<'_>,
    trusted: &crate::validation::TrustedIdentity,
) -> Result<bool> {
    let Some(status) = store::status(pool, r.id).await? else {
        return Ok(false);
    };
    if status["candidate_sha"] != r.candidate.sha
        || status["candidate_tree"] != r.candidate.tree
        || status["trusted"] != serde_json::json!(trusted)
    {
        return Err("saved validation identity differs".into());
    }
    if status["result"] != "pending" {
        return Ok(false);
    }
    Ok(true)
}
async fn execute(
    pool: &PgPool,
    r: Request<'_>,
    trusted: &crate::validation::TrustedIdentity,
    required: &[String],
) -> Result<bool> {
    let claimed = store::begin(pool, r.id).await?;
    if !claimed && !r.directory.join("binding.json").exists() {
        return Err("validation claim has unknown process/output; reconcile before retry".into());
    }
    let checkout = r.checkout.to_owned();
    let directory = r.directory.to_owned();
    let candidate = r.candidate.clone();
    let plan = r.plan.clone();
    let limit = crate::storage_service::entry_limit(pool).await?;
    let steps = tokio::task::spawn_blocking(move || {
        validation_runner::execute_limited(&checkout, &directory, &candidate, &plan, limit)
    })
    .await??;
    finish(pool, r, trusted, required, &steps).await
}
async fn finish(
    pool: &PgPool,
    r: Request<'_>,
    trusted: &crate::validation::TrustedIdentity,
    required: &[String],
    steps: &[StepEvidence],
) -> Result<bool> {
    for step in steps {
        store::record_step(pool, r.id, step, step_status(step)).await?;
    }
    let passed = store::finish(
        pool,
        r.id,
        r.candidate,
        trusted,
        &r.candidate.tree,
        &trusted.protected_entry_sha256,
        steps,
        required,
    )
    .await?;
    if !passed {
        reserve(pool, &r, steps).await?;
    }
    Ok(passed)
}
fn step_id(step: &validation_runner::Step) -> String {
    step.id.clone()
}
fn step_status(step: &StepEvidence) -> &'static str {
    match step.exit_code {
        Some(0) => "succeeded",
        Some(_) => "failed",
        None => "unknown",
    }
}
async fn reserve(pool: &PgPool, r: &Request<'_>, steps: &[StepEvidence]) -> Result<()> {
    for step in steps {
        if step_status(step) == "failed" && step.code_failure {
            let failure = serde_json::json!({"candidate":r.candidate,"step":step,"remaining_acceptance":r.plan.steps});
            store::reserve_repair(pool, r.requirement, 1, r.id, &failure, FailureKind::Code)
                .await?;
            break;
        }
    }
    Ok(())
}
