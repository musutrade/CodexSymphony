//! Test-merge checkout validation retains its own identity and cannot certify merged SHA.
use crate::{
    automatic_merge::{self, Intent},
    github_contract::PreMergeSource,
    github_http::AppClient,
    merge_store, merge_validation,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::path::Path;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
pub(crate) async fn tick(
    pool: &PgPool,
    client: &mut AppClient,
    root: &Path,
    now: i64,
) -> Result<()> {
    let row: Option<Value>=sqlx::query_scalar("SELECT intent FROM merge_operation WHERE state='prepared' AND pre_validation IS NULL AND intent#>>'{policy,delivery,pre_merge,checkout}'='test_merge' ORDER BY created_at LIMIT 1")
        .fetch_optional(pool).await?;
    let Some(row) = row else { return Ok(()) };
    let intent: Intent = serde_json::from_value(row)?;
    if let Err(error) = validate(pool, client, root, &intent, now).await {
        merge_store::block(
            pool,
            &intent,
            &format!(
                "pre_merge checkout validation blocked: {}",
                crate::operator_view::redact_text(&error.to_string())
            ),
        )
        .await?;
    }
    Ok(())
}
async fn validate(
    pool: &PgPool,
    client: &mut AppClient,
    root: &Path,
    intent: &Intent,
    now: i64,
) -> Result<()> {
    let observation =
        crate::github_observe::observe(client, &intent.policy, intent.pr, now).await?;
    if !automatic_merge::eligible(intent, &observation, now) {
        return Ok(());
    }
    if !intent
        .policy
        .delivery
        .as_ref()
        .is_some_and(|delivery| delivery.pre_merge.checkout == PreMergeSource::TestMerge)
    {
        return Ok(());
    }
    let sha = observation
        .test_merge_sha
        .as_deref()
        .ok_or("test-merge SHA unavailable")?;
    validate_checkout(pool, client, root, intent, sha, now).await
}
async fn validate_checkout(
    pool: &PgPool,
    client: &mut AppClient,
    root: &Path,
    intent: &Intent,
    sha: &str,
    now: i64,
) -> Result<()> {
    if !validation_allowed(pool, intent).await? {
        return Ok(());
    }
    let plan = merge_validation::source_plan(pool, intent).await?;
    let (_, required) = merge_store::validation(pool, intent).await?;
    let checkout = merge_validation::checkout(pool, client, root, intent, sha, now).await?;
    let directory = root
        .join("validations")
        .join(format!("pre-merge-{}-{sha}", intent.action_key()));
    if !begin_validation(pool, intent).await? {
        return Ok(());
    }
    let evidence =
        merge_validation::execute(pool, intent, checkout, directory, plan, required).await?;
    save_validation(pool, intent, &evidence).await
}
async fn save_validation(
    pool: &PgPool,
    intent: &Intent,
    evidence: &crate::validation::ValidationEvidence,
) -> Result<()> {
    sqlx::query("UPDATE merge_operation SET pre_validation=$2 WHERE action_key=$1 AND state='prepared' AND pre_validation IS NULL")
        .bind(intent.action_key()).bind(json!(evidence)).execute(pool).await?;
    Ok(())
}

async fn validation_allowed(pool: &PgPool, intent: &Intent) -> Result<bool> {
    let mut tx = crate::run_store::lock(pool).await?;
    if !merge_store::allowed(&mut tx, intent).await? {
        return Ok(false);
    }
    tx.commit().await?;
    Ok(true)
}
async fn begin_validation(pool: &PgPool, intent: &Intent) -> Result<bool> {
    let mut tx = crate::run_store::lock(pool).await?;
    if !merge_store::allowed(&mut tx, intent).await? {
        return Ok(false);
    }
    sqlx::query("UPDATE merge_operation SET pre_validation_started=true WHERE action_key=$1 AND state='prepared'")
        .bind(intent.action_key()).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(true)
}

#[cfg(test)]
#[path = "../tests/unit/merge_prevalidation.rs"]
mod tests;
