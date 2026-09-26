//! An operator may bind one successful revalidation to post-merge execution.
//! Repository policy, protection, failed generations and budgets stay intact.
use crate::{
    automatic_merge::Intent,
    budget_store::{decode, require},
    extension_recovery::Action,
    github::Policy,
    github_contract::PostMerge,
    validation_runner::Plan,
};
use serde_json::Value;
use sqlx::{PgPool, Postgres, Transaction};

pub(crate) async fn authorize(
    tx: &mut Transaction<'_, Postgres>,
    requirement: i64,
    revision: i64,
    action: &Action,
) -> Result<(), sqlx::Error> {
    let Action::RevalidateDelivery { policy_digest, .. } = action else {
        return Ok(());
    };
    let value: Value = sqlx::query_scalar("SELECT g.policy FROM execution_revision v JOIN repository r ON r.id=(v.document->>'repository_id')::bigint JOIN github_repository g ON g.repository_id=(r.document->>'github_repository_id')::bigint WHERE v.requirement_id=$1 AND v.revision=$2 AND r.version=(v.document->>'repository_version')::bigint AND g.repository_version=r.version AND NOT (r.document->>'revoked')::boolean")
        .bind(requirement).bind(revision).fetch_one(&mut **tx).await?;
    check_policy(&decode(value)?, policy_digest)
}

fn check_policy(policy: &Policy, expected: &str) -> Result<(), sqlx::Error> {
    let bytes = serde_json::to_vec(policy).map_err(sqlx::Error::decode)?;
    require(
        crate::validation::sha256(bytes) == expected,
        "delivery recovery policy identity differs",
    )?;
    let contract = policy.delivery.as_ref().ok_or_else(missing_contract)?;
    require(
        matches!(contract.post_merge, PostMerge::FixedValidation { .. }),
        "delivery recovery requires a fixed post-merge validation plan",
    )
}

fn missing_contract() -> sqlx::Error {
    sqlx::Error::Protocol("delivery recovery contract unavailable".into())
}

pub(crate) async fn replacement(
    pool: &PgPool,
    intent: &Intent,
) -> Result<Option<Plan>, Box<dyn std::error::Error + Send + Sync>> {
    let row: Option<Value> = sqlx::query_scalar("SELECT f.resolution#>'{command,action}' FROM recovery_failure f JOIN candidate_validation v ON v.id=f.successor_validation WHERE f.successor_validation=$1 AND f.requirement_id=$2 AND v.requirement_id=$2 AND v.revision=$3 AND v.candidate_sha=$4 AND f.resolution#>>'{command,action,kind}'='revalidate_delivery' AND f.resolution_state='complete' AND f.decision='recovered' AND v.result='succeeded' AND v.superseded_by IS NULL")
        .bind(&intent.validation_id).bind(intent.requirement).bind(intent.revision).bind(&intent.head).fetch_optional(pool).await?;
    let Some(value) = row else {
        return Ok(None);
    };
    let action: Action = decode(value)?;
    let Action::RevalidateDelivery {
        policy_digest,
        plan_digest,
        ..
    } = action
    else {
        return Err("delivery recovery action differs".into());
    };
    check_policy(&intent.policy, &policy_digest)?;
    let plan = crate::merge_validation::source_plan(pool, intent).await?;
    require(
        plan.identity()?.config_sha256 == plan_digest,
        "delivery recovery validation plan identity differs",
    )?;
    Ok(Some(plan))
}

/// Only an explicitly reviewed, never-sent operation can receive fresh proof.
pub(crate) async fn prepare_pending(
    tx: &mut Transaction<'_, Postgres>,
    requirement: i64,
    validation: &str,
    action: &Action,
) -> Result<(), sqlx::Error> {
    check_pending(tx, requirement, validation, action).await?;
    if matches!(action, Action::RevalidateDelivery { .. }) {
        reconcile_interruption(tx, requirement, validation).await?;
    }
    Ok(())
}

async fn check_pending(
    tx: &mut Transaction<'_, Postgres>,
    requirement: i64,
    validation: &str,
    action: &Action,
) -> Result<(), sqlx::Error> {
    let pending: Vec<String> = sqlx::query_scalar(
        "SELECT action_key FROM delivery WHERE requirement_id=$1 AND NOT released FOR UPDATE",
    )
    .bind(requirement)
    .fetch_all(&mut **tx)
    .await?;
    if !pending.is_empty() {
        require(
            matches!(action, Action::RevalidateDelivery { .. }),
            "pending delivery requires explicit delivery revalidation",
        )?;
        require(
            pending.len() == 1,
            "multiple pending deliveries require reconciliation",
        )?;
        pending_unchanged(tx, &pending[0], validation).await?;
    }
    Ok(())
}

async fn reconcile_interruption(
    tx: &mut Transaction<'_, Postgres>,
    requirement: i64,
    validation: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO recovery_failure(event_key,requirement_id,source_validation_id,phase,facts,fingerprint,decision,reason) SELECT 'extension:'||id||':control',requirement_id,id,'local',jsonb_build_object('candidate_sha',candidate_sha,'phase','local','feedback',jsonb_build_object('verdict','unknown','source','host','fault',jsonb_build_object('class','authorization','code','validation_control_interruption','owner','operator','resume_condition','Approve a fresh validation generation before any external write'))),id,'blocked','control interruption invalidated prior proof; original successful results retained' FROM candidate_validation WHERE id=$1 AND requirement_id=$2 AND result='succeeded' AND hook_invalidated AND superseded_by IS NULL ON CONFLICT DO NOTHING")
            .bind(validation).bind(requirement).execute(&mut **tx).await?;
    Ok(())
}

async fn pending_unchanged(
    tx: &mut Transaction<'_, Postgres>,
    key: &str,
    validation: &str,
) -> Result<(), sqlx::Error> {
    let eligible: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM delivery d JOIN delivery_action a ON a.action_key=d.action_key AND a.kind='publish' JOIN candidate_validation v ON v.id=d.validation_id WHERE d.action_key=$1 AND v.id=$2 AND d.mode='github_pr' AND NOT d.released AND d.pr_number IS NULL AND d.original_action_key IS NULL AND v.result='succeeded' AND v.hook_invalidated AND a.state='pending' AND a.attempts=0 AND NOT EXISTS(SELECT 1 FROM delivery_attempt t WHERE t.action_key=d.action_key) AND NOT EXISTS(SELECT 1 FROM merge_operation m WHERE m.delivery_key=d.action_key))")
        .bind(key).bind(validation).fetch_one(&mut **tx).await?;
    require(
        eligible,
        "delivery may have external effects; reconcile original operation",
    )
}

/// Preserve operation/candidate identity and journal the authorized proof change.
pub(crate) async fn rebind_pending(
    tx: &mut Transaction<'_, Postgres>,
    validation: &str,
    key: &str,
) -> Result<(), sqlx::Error> {
    let prior: Option<String> =
        sqlx::query_scalar("SELECT validation_id FROM delivery WHERE action_key=$1 FOR UPDATE")
            .bind(key)
            .fetch_optional(&mut **tx)
            .await?;
    let Some(prior) = prior else {
        return Ok(());
    };
    if prior == validation {
        return Ok(());
    }
    pending_unchanged(tx, key, &prior).await?;
    let event: String = sqlx::query_scalar("SELECT f.event_key FROM recovery_failure f JOIN candidate_validation v ON v.id=f.successor_validation JOIN candidate_validation old ON old.id=f.source_validation_id JOIN delivery d ON d.validation_id=old.id WHERE d.action_key=$1 AND v.id=$2 AND old.id=$3 AND old.superseded_by=v.id AND v.retry_of=old.id AND v.source_run_id=old.source_run_id AND v.candidate_sha=d.head_sha AND v.candidate_tree=old.candidate_tree AND v.requirement_id=d.requirement_id AND v.revision=d.revision AND v.result='succeeded' AND NOT v.hook_invalidated AND f.resolution#>>'{command,action,kind}'='revalidate_delivery' AND f.resolution_state IN ('running','complete')")
        .bind(key).bind(validation).bind(&prior).fetch_one(&mut **tx).await?;
    sqlx::query("INSERT INTO delivery_observation(action_key,kind,fact) VALUES($1,'validation_rebound',jsonb_build_object('previous_validation',$2::text,'validation',$3::text,'recovery_event',$4::text,'external_attempts',0))")
        .bind(key).bind(&prior).bind(validation).bind(event).execute(&mut **tx).await?;
    sqlx::query("UPDATE delivery SET validation_id=$2 WHERE action_key=$1 AND validation_id=$3")
        .bind(key)
        .bind(validation)
        .bind(prior)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

#[cfg(test)]
#[path = "../tests/unit/extension_delivery_recovery.rs"]
mod tests;
