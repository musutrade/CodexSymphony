//! Trusted adapter from the actual merged checkout to child business completion.
use crate::{
    automatic_merge::Intent,
    github::{CheckState, Observation},
    github_http::AppClient,
    merge_store as store, merge_validation,
    validation::{self, ValidationEvidence},
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
    sqlx::query("UPDATE merge_operation m SET state='cancelled' FROM requirement r WHERE r.id=m.requirement_id AND r.cancel_requested AND m.state='merged' AND (NOT m.acceptance_started OR m.acceptance IS NOT NULL)").execute(pool).await?;
    let row: Option<(Value,String,i64)> = sqlx::query_as("SELECT intent,merged_sha,COALESCE(merged_at,created_at) FROM merge_operation WHERE state='merged' AND next_attempt_at<=$1 ORDER BY created_at LIMIT 1")
        .bind(now).fetch_optional(pool).await?;
    let Some((intent, sha, started)) = row else {
        return Ok(());
    };
    let intent: Intent = serde_json::from_value(intent)?;
    let mut tx = crate::run_store::lock(pool).await?;
    if !store::allowed(&mut tx, &intent).await? {
        return Ok(());
    }
    tx.commit().await?;
    if let Err(error) = accept(pool, client, root, &intent, &sha, started, now).await {
        if let Some(remote) = error.downcast_ref::<crate::github_http::Error>()
            && now.saturating_sub(started) < post_wait(&intent)
        {
            let delay = i64::try_from(remote.retry_after_seconds.unwrap_or(30)).unwrap_or(i64::MAX);
            store::receipt(
                pool,
                &intent,
                json!({"post_merge_observation_error":remote.to_string()}),
                now,
                delay,
            )
            .await?;
            return Ok(());
        }
        store::receipt(pool,&intent,json!({"post_merge_failure":crate::operator_view::redact_text(&error.to_string()),"artifact_directory":root.join("validations").join(format!("post-merge-{}",intent.action_key()))}),now,30).await?;
        store::block(
            pool,
            &intent,
            &format!(
                "post_merge acceptance requires original-item recovery (M3-5 unavailable): {}",
                crate::operator_view::redact_text(&error.to_string())
            ),
        )
        .await?;
    }
    Ok(())
}

async fn accept(
    pool: &PgPool,
    client: &mut AppClient,
    root: &Path,
    intent: &Intent,
    sha: &str,
    started: i64,
    now: i64,
) -> Result<()> {
    let observation =
        crate::github_observe::observe(client, &intent.policy, intent.pr, now).await?;
    if crate::automatic_merge::confirmed(intent, &observation).as_deref() != Some(sha) {
        return Err("confirmed merge identity changed".into());
    }
    let phase = post_phase(&observation)?;
    if phase
        .checks
        .iter()
        .any(|check| check.state == CheckState::Failure)
    {
        return Err("post_merge required check failed".into());
    }
    if !phase
        .checks
        .iter()
        .all(|check| check.state == CheckState::Success)
    {
        if now.saturating_sub(started) >= phase.wait_seconds as i64 {
            return Err("post_merge check deadline reached".into());
        }
        crate::merge_dispatch::dispatch(pool, client, intent, sha, now).await?;
        return Ok(store::receipt(pool, intent, json!({"post_merge":phase}), now, 30).await?);
    }
    let plan = merge_validation::plan(pool, intent).await?;
    let (_, required) = store::validation(pool, intent).await?;
    let checkout = merge_validation::checkout(pool, client, root, intent, sha, now).await?;
    let directory = root
        .join("validations")
        .join(format!("post-merge-{}", intent.action_key()));
    let mut tx = crate::run_store::lock(pool).await?;
    if !store::allowed(&mut tx, intent).await? {
        return Ok(());
    }
    sqlx::query(
        "UPDATE merge_operation SET acceptance_started=true WHERE action_key=$1 AND state='merged'",
    )
    .bind(intent.action_key())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    let evidence =
        merge_validation::execute(pool, intent, checkout, directory, plan, required.clone())
            .await?;
    // Collection can take longer than the observation TTL. Re-read policy and
    // the exact PR after execution, before recording business completion.
    let capability = crate::github_observe::preflight(
        client,
        &intent.policy,
        intent.pr,
        crate::github_service::now(),
    )
    .await?;
    if !capability.blockers.is_empty() {
        return Err("post_merge capability changed".into());
    }
    crate::github_store::save_capability(pool, &capability).await?;
    let now = crate::github_service::now();
    let observation =
        crate::github_observe::observe(client, &intent.policy, intent.pr, now).await?;
    if crate::automatic_merge::confirmed(intent, &observation).as_deref() != Some(sha) {
        return Err("merged source changed during acceptance".into());
    }
    let mut phase = post_phase(&observation)?;
    phase
        .bind_validation(
            evidence.clone(),
            &evidence.candidate,
            &evidence.trusted,
            &required,
        )
        .map_err(|error| format!("post_merge binding rejected: {error:?}"))?;
    if phase.state(started, now) != CheckState::Success {
        return Err("post_merge evidence incomplete".into());
    }
    crate::github_store::save_observation(pool, &observation).await?;
    finish(pool, intent, &evidence, &observation).await
}

fn post_phase(observation: &Observation) -> Result<crate::github_contract::PhaseEvidence> {
    observation
        .phases
        .as_ref()
        .and_then(|phases| phases.iter().find(|phase| phase.phase == "post_merge"))
        .cloned()
        .ok_or_else(|| "post_merge phase absent".into())
}

async fn finish(
    pool: &PgPool,
    intent: &Intent,
    evidence: &ValidationEvidence,
    observation: &Observation,
) -> Result<()> {
    let mut tx = crate::run_store::lock(pool).await?;
    // Preserve collected results even if pause/cancel won the completion race.
    sqlx::query("UPDATE merge_operation SET acceptance=$2 WHERE action_key=$1 AND state='merged'")
        .bind(intent.action_key())
        .bind(json!({"evidence":evidence,"observation":observation}))
        .execute(&mut *tx)
        .await?;
    if !store::allowed(&mut tx, intent).await? {
        tx.commit().await?;
        return Ok(());
    }
    let method: Option<String> =
        sqlx::query_scalar("SELECT merge_method FROM merge_operation WHERE action_key=$1")
            .bind(intent.action_key())
            .fetch_one(&mut *tx)
            .await?;
    if method.is_none() {
        sqlx::query("UPDATE merge_operation SET state='blocked',blocker='merged checkout accepted; actual merge method remains unconfirmed after lost response' WHERE action_key=$1").bind(intent.action_key()).execute(&mut *tx).await?;
        tx.commit().await?;
        return Ok(());
    }
    if let Some(authorization) = intent.authorization {
        let input: Value=sqlx::query_scalar("SELECT input FROM group_execution_item WHERE requirement_id=$1 AND authorization_id=$2")
            .bind(intent.requirement).bind(authorization).fetch_one(&mut *tx).await?;
        let fact = crate::group_dependency::Fact {
            requirement_id: intent.requirement,
            authorization_id: authorization,
            child_revision: input["review"]["revision"]
                .as_i64()
                .ok_or("child revision missing")?,
            repository_id: input["child"]["repository_id"]
                .as_i64()
                .ok_or("child repository missing")?,
            github_repository_id: intent.policy.repository_id as i64,
            pr_number: intent.pr as i64,
            head_sha: intent.head.clone(),
            merged_sha: evidence.candidate.sha.clone(),
            acceptance_sha: evidence.candidate.sha.clone(),
            acceptance_plan: input["review"]["verification"].clone(),
            source: "platform-merged-checkout-validation/v1".into(),
            evidence_sha256: validation::sha256(serde_json::to_vec(evidence)?),
            artifact: format!("merge-operation:{}:acceptance", intent.action_key()),
        };
        crate::group_completion::record_in(&mut tx, &Trusted { fact: fact.clone() }, &fact).await?;
    }
    sqlx::query("UPDATE merge_operation SET state='complete',blocker=NULL WHERE action_key=$1 AND state='merged'")
        .bind(intent.action_key()).execute(&mut *tx).await?;
    sqlx::query("UPDATE requirement SET state='Done',version=version+1 WHERE id=$1 AND state='Submitted' AND revision=$2 AND NOT cancel_requested")
        .bind(intent.requirement).bind(intent.revision).execute(&mut *tx).await?;
    sqlx::query("UPDATE candidate_validation SET stage='done' WHERE id=$1 AND result='succeeded'")
        .bind(&intent.validation_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// Constructed only after actual independent merged-checkout execution above.
struct Trusted {
    fact: crate::group_dependency::Fact,
}
impl crate::group_dependency::Verifier for Trusted {
    fn verify(&self, fact: &crate::group_dependency::Fact) -> bool {
        self.fact == *fact
    }
}

fn post_wait(intent: &Intent) -> i64 {
    let Some(contract) = &intent.policy.delivery else {
        return 0;
    };
    let wait = match &contract.post_merge {
        crate::github_contract::PostMerge::Checks { wait_seconds, .. }
        | crate::github_contract::PostMerge::FixedValidation { wait_seconds, .. } => *wait_seconds,
    };
    i64::try_from(wait).unwrap_or(i64::MAX)
}
