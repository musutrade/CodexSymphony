//! Host-only correction of a failed validator, never a source repair or resend.
use crate::{
    budget_store::require,
    local_delivery_store::Job,
    local_git,
    validation::{self, TrustedIdentity},
    validation_runner::Plan,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Transaction};
type Result<T> = crate::delivery_extension::Result<T>;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub request_id: String,
    pub requirement_id: i64,
    pub version: i64,
    pub delivery_key: String,
    pub previous_result_sha256: String,
    pub reason: String,
    pub plan: Plan,
}

pub async fn request(pool: &PgPool, command: &Command) -> Result<Value> {
    validate_command(command)?;
    let input = json!(command);
    let mut tx = crate::run_store::lock(pool).await?;
    if let Some(saved) = replay(&mut tx, command, &input).await? {
        return Ok(saved);
    }
    let (old_job, old_result, failure) = checked_previous(&mut tx, command).await?;
    register(&mut tx, command, &input, &old_job, &old_result, &failure).await?;
    reset_projection(&mut tx, command, &failure).await?;
    tx.commit().await?;
    Ok(acknowledgement(command))
}

fn validate_command(command: &Command) -> Result<()> {
    require(
        crate::contract::validate_request_id(&command.request_id).is_ok()
            && crate::runtime::text_valid(&command.reason, 4096),
        "invalid acceptance correction identity or reason",
    )?;
    Ok(())
}

async fn checked_previous(
    tx: &mut Transaction<'_, Postgres>,
    command: &Command,
) -> Result<(Value, Value, Value)> {
    let job = authorize(tx, command).await?;
    verify_target(&job)?;
    let (old_job, old_result, failure) = previous(tx, command).await?;
    require(
        validation::sha256(serde_json::to_vec(&old_result)?) == command.previous_result_sha256,
        "original failed acceptance identity changed",
    )?;
    let prior: crate::integration_process::Job = serde_json::from_value(old_job.clone())?;
    compatible(&prior.plan, &command.plan)?;
    Ok((old_job, old_result, failure))
}

fn acknowledgement(command: &Command) -> Value {
    json!({"accepted":true,"started":false,"delivery_key":command.delivery_key,"request_id":command.request_id,"model_calls_added":0,"delivery_updates_added":0})
}

async fn replay(
    tx: &mut Transaction<'_, Postgres>,
    command: &Command,
    input: &Value,
) -> Result<Option<Value>> {
    let saved: Option<Value> =
        sqlx::query_scalar("SELECT input FROM local_acceptance_recheck WHERE request_id=$1")
            .bind(&command.request_id)
            .fetch_optional(&mut **tx)
            .await?;
    if let Some(saved) = saved {
        require(saved == *input, "acceptance correction request conflict")?;
        return Ok(Some(acknowledgement(command)));
    }
    Ok(None)
}

async fn authorize(tx: &mut Transaction<'_, Postgres>, command: &Command) -> Result<Job> {
    let job: Job = sqlx::query_as("SELECT d.*,a.state,a.attempts FROM delivery d JOIN delivery_action a USING(action_key) JOIN requirement r ON r.id=d.requirement_id JOIN execution_control c ON c.requirement_id=r.id JOIN repository p ON p.id=d.internal_repository_id JOIN candidate_validation v ON v.id=d.validation_id WHERE d.action_key=$1 AND d.requirement_id=$2 AND r.version=$3 AND d.mode='local_git' AND a.kind='publish' AND a.attempts>0 AND a.state='blocked' AND r.state='Submitted' AND EXISTS(SELECT 1 FROM group_execution_item i WHERE i.requirement_id=r.id AND i.input#>>'{child,kind}'='code_change') AND r.revision=d.revision AND NOT r.cancel_requested AND (r.paused OR c.paused) AND c.recovery_complete AND NOT d.released AND d.local_acceptance_started AND d.local_acceptance_quiescent AND d.local_acceptance->>'passed'='false' AND NOT (p.document->>'revoked')::boolean AND p.version=(d.policy->>'repository_version')::bigint AND p.version>p.revoked_through_version AND plugin_scope_allows('delivery:local_git',p.id) AND plugin_scope_allows('validation:native',p.id) AND v.result='succeeded' AND v.superseded_by IS NULL AND v.candidate_sha=d.head_sha AND NOT EXISTS(SELECT 1 FROM agent_run WHERE NOT quiescent) AND NOT EXISTS(SELECT 1 FROM integration_validation WHERE NOT quiescent) AND NOT EXISTS(SELECT 1 FROM workspace_operation WHERE status<>'complete') AND NOT EXISTS(SELECT 1 FROM repair_reservation x JOIN linked_failure f ON f.id=x.linked_failure_id WHERE f.local_delivery=d.action_key)")
        .bind(&command.delivery_key).bind(command.requirement_id).bind(command.version)
        .fetch_one(&mut **tx).await?;
    require(
        crate::group_queue_store::authorized(tx, command.requirement_id).await?,
        "original group authorization unavailable",
    )?;
    Ok(job)
}

fn verify_target(job: &Job) -> Result<()> {
    let binding = job.binding()?;
    require(
        crate::local_delivery_store::resolve_document(&job.policy)? == binding,
        "original target registration changed",
    )?;
    require(
        local_git::head(&binding)? == job.head_sha
            && local_git::observe(&binding, &job.action_key, job.baseline()?, &job.head_sha)?
                == local_git::Observation::Delivered,
        "original local delivery is not the current confirmed target",
    )?;
    Ok(())
}

async fn previous(
    tx: &mut Transaction<'_, Postgres>,
    command: &Command,
) -> Result<(Value, Value, Value)> {
    Ok(sqlx::query_as("SELECT d.local_acceptance_job,d.local_acceptance,to_jsonb(f) FROM delivery d JOIN linked_failure f ON f.local_delivery=d.action_key WHERE d.action_key=$1 AND f.repair_delivery IS NULL AND f.state IN ('observed','blocked','cancelled') AND NOT EXISTS(SELECT 1 FROM linked_failure other WHERE other.requirement_id=d.requirement_id AND other.id<>f.id AND other.state NOT IN ('complete','cancelled','merged'))")
        .bind(&command.delivery_key).fetch_one(&mut **tx).await?)
}

fn compatible(previous: &Plan, replacement: &Plan) -> Result<()> {
    let before = previous.identity()?;
    let after = replacement.identity()?;
    compatible_entry(previous, replacement)?;
    for (old, new) in previous.steps.iter().zip(&replacement.steps) {
        compatible_step(old, new)?;
    }
    require(before != after, "validator prerequisites unchanged")?;
    Ok(())
}

fn compatible_entry(previous: &Plan, replacement: &Plan) -> Result<()> {
    require(
        previous.entry == replacement.entry
            && previous.entry_sha256 == replacement.entry_sha256
            && previous.steps.len() == replacement.steps.len(),
        "acceptance correction changed protected entry or required check set",
    )?;
    Ok(())
}

fn compatible_step(
    old: &crate::validation_runner::Step,
    new: &crate::validation_runner::Step,
) -> Result<()> {
    require(
        old.id == new.id
            && old.timeout_seconds == new.timeout_seconds
            && old.code_failure == new.code_failure,
        "acceptance correction changed check identity, deadline or classification",
    )?;
    Ok(())
}

async fn register(
    tx: &mut Transaction<'_, Postgres>,
    command: &Command,
    input: &Value,
    old_job: &Value,
    old_result: &Value,
    failure: &Value,
) -> Result<()> {
    let ordinal: i64 =
        sqlx::query_scalar("SELECT count(*)+1 FROM local_acceptance_recheck WHERE delivery_key=$1")
            .bind(&command.delivery_key)
            .fetch_one(&mut **tx)
            .await?;
    require(ordinal <= 3, "acceptance correction limit reached")?;
    sqlx::query("INSERT INTO local_acceptance_recheck(request_id,delivery_key,ordinal,input,previous_job,previous_result,previous_failure,plan) VALUES($1,$2,$3,$4,$5,$6,$7,$8)")
        .bind(&command.request_id).bind(&command.delivery_key).bind(ordinal as i32)
        .bind(input).bind(old_job).bind(old_result).bind(failure).bind(json!(command.plan))
        .execute(&mut **tx).await?;
    Ok(())
}

async fn reset_projection(
    tx: &mut Transaction<'_, Postgres>,
    command: &Command,
    failure: &Value,
) -> Result<()> {
    // Cancel only the mistaken code-repair proposal, not the Requirement. Its
    // original state/evidence are retained above before advancing the projection.
    sqlx::query("UPDATE linked_failure SET state='cancelled',blocker=$2 WHERE id=$1")
        .bind(
            failure["id"]
                .as_str()
                .ok_or("original failure identity missing")?,
        )
        .bind(format!(
            "operator validator correction: {}",
            command.request_id
        ))
        .execute(&mut **tx)
        .await?;
    sqlx::query("UPDATE delivery SET local_acceptance_started=false,local_acceptance_quiescent=true,local_acceptance_job=NULL,local_acceptance=NULL WHERE action_key=$1")
        .bind(&command.delivery_key).execute(&mut **tx).await?;
    sqlx::query(
        "UPDATE delivery_action SET next_attempt_at=0 WHERE action_key=$1 AND kind='publish'",
    )
    .bind(&command.delivery_key)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub(crate) async fn plan(
    pool: &PgPool,
    delivery: &str,
    original: Plan,
    trusted: TrustedIdentity,
    invocation: String,
) -> Result<(Plan, TrustedIdentity, String)> {
    let row: Option<(String, Value)> = sqlx::query_as("SELECT request_id,plan FROM local_acceptance_recheck WHERE delivery_key=$1 ORDER BY ordinal DESC LIMIT 1")
        .bind(delivery).fetch_optional(pool).await?;
    if let Some((request, value)) = row {
        let replacement: Plan = serde_json::from_value(value)?;
        compatible(&original, &replacement)?;
        let identity = replacement.identity()?;
        return Ok((
            replacement,
            identity,
            format!("local-acceptance-recheck-{}", validation::sha256(request)),
        ));
    }
    Ok((original, trusted, invocation))
}

pub(crate) async fn active(tx: &mut Transaction<'_, Postgres>, delivery: &str) -> Result<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM local_acceptance_recheck WHERE delivery_key=$1)",
    )
    .bind(delivery)
    .fetch_one(&mut **tx)
    .await?)
}
