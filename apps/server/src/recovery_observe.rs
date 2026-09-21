//! Only identity-bound required checks may produce repair input. Review prose
//! and model hypotheses do not certify a root cause or authorize a code change.
use crate::{
    bounded_recovery::Failure,
    github::{Check, CheckState, Observation},
    github_http::AppClient,
};
use sqlx::PgPool;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub async fn observe(
    pool: &PgPool,
    client: &mut AppClient,
    observation: &Observation,
) -> Result<()> {
    if observation.closed || observation.merge != crate::github::MergeFact::Unmerged {
        return Ok(());
    }
    let Some(contract) = &observation.policy.delivery else {
        return Ok(());
    };
    let row: Option<(i64,String,i64)> = sqlx::query_as("SELECT d.requirement_id,d.validation_id,d.revision FROM delivery d JOIN repair_authorization a USING(requirement_id) JOIN execution_control c ON c.requirement_id=d.requirement_id WHERE d.repository_id=$1 AND d.pr_number=$2 AND d.head_sha=$3 AND a.policy='bounded_v1'")
        .bind(observation.repository_id as i64).bind(observation.number as i64).bind(&observation.head).fetch_optional(pool).await?;
    let Some((id, source, revision)) = row else {
        return Ok(());
    };
    let checks = observation
        .phases
        .as_ref()
        .and_then(|phases| phases.iter().find(|p| p.phase == "pre_merge"))
        .map(|p| p.checks.as_slice())
        .unwrap_or(&observation.checks);
    record_success(pool, id, observation, checks).await?;
    for check in checks
        .iter()
        .filter(|check| check.state == CheckState::Failure && check.evidence.len() == 1)
    {
        observe_check(
            pool,
            client,
            observation,
            check,
            (id, &source, revision),
            contract.actions.read_logs,
        )
        .await?;
    }
    Ok(())
}

async fn bind_attempt(
    pool: &PgPool,
    event: &str,
    run: i64,
    attempt: i64,
    observation: &Observation,
) -> Result<()> {
    let mut tx = crate::run_store::lock(pool).await?;
    let row:Option<(String,i32,i64,String)> = sqlx::query_as("SELECT t.event_key,t.attempts,t.deadline,t.state FROM recovery_retry t JOIN recovery_failure f USING(event_key) JOIN recovery_failure incoming ON incoming.event_key=$1 WHERE f.requirement_id=incoming.requirement_id AND f.phase=incoming.phase AND f.facts->>'candidate_sha'=incoming.facts->>'candidate_sha' AND f.facts->>'step'=incoming.facts->>'step' AND ((t.remote IS NULL AND t.state='pending') OR (t.state='complete' AND (t.remote_attempt<$2 OR t.remote->>'run'<>$3)))")
        .bind(event).bind(attempt).bind(run.to_string()).fetch_optional(&mut *tx).await?;
    let Some((key, attempts, deadline, state)) = row else {
        return Ok(());
    };
    let next = crate::bounded_recovery::next_retry(
        observation.last_synced_at,
        deadline,
        attempts as u32,
        None,
    );
    sqlx::query("UPDATE recovery_retry SET remote_attempt=$2,remote=$3,state=$4,next_attempt_at=CASE WHEN $6='complete' THEN $5 ELSE next_attempt_at END WHERE event_key=$1")
        .bind(key).bind(attempt).bind(serde_json::json!({"policy":observation.policy,"run":run,"pr":observation.number,"head":observation.head})).bind(if next.is_some(){"pending"}else{"blocked"}).bind(next.unwrap_or(deadline)).bind(state).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

async fn logs(
    client: &mut AppClient,
    observation: &Observation,
    check: &Check,
    readable: bool,
) -> (String, String) {
    let proof = &check.evidence[0];
    if let Some(job) = proof["workflow_job"]["id"].as_u64() {
        let path = format!(
            "/repos/{}/actions/jobs/{job}/logs",
            observation.policy.repository
        );
        if readable {
            return match client
                .job_log(&observation.policy, job, observation.last_synced_at)
                .await
            {
                Ok(raw) => (raw, path),
                Err(error) => (
                    format!(
                        "raw log unavailable: {error}; failure summary cannot certify root cause"
                    ),
                    path,
                ),
            };
        }
        return ("raw log authorization missing".into(), path);
    }
    let raw = format!(
        "{}\n{}",
        proof["output"]["summary"].as_str().unwrap_or(""),
        proof["output"]["text"].as_str().unwrap_or("")
    );
    (
        raw,
        format!(
            "/repos/{}/check-runs/{}",
            observation.policy.repository, proof["id"]
        ),
    )
}

async fn record_success(
    pool: &PgPool,
    id: i64,
    observation: &Observation,
    checks: &[Check],
) -> Result<()> {
    for check in checks
        .iter()
        .filter(|check| check.state == CheckState::Success)
    {
        sqlx::query("UPDATE recovery_failure SET decision='recovered' WHERE requirement_id=$1 AND phase IN ('ci','review') AND facts->>'candidate_sha'=$2 AND facts->>'step'=$3 AND decision IN ('infrastructure','blocked')")
            .bind(id).bind(&observation.head).bind(&check.selector.name).execute(pool).await?;
    }
    Ok(())
}
async fn observe_check(
    pool: &PgPool,
    client: &mut AppClient,
    observation: &Observation,
    check: &Check,
    (id, source, revision): (i64, &str, i64),
    read_logs: bool,
) -> Result<()> {
    let proof = &check.evidence[0];
    let event = format!(
        "github:{}:{}:{}:{}",
        observation.repository_id,
        observation.number,
        proof["id"],
        proof["workflow_run"]["run_attempt"]
    );
    let Some((event, raw, log_ref)) =
        new_evidence(pool, client, observation, check, read_logs, &event).await?
    else {
        return Ok(());
    };
    let failure = Failure {
        phase: if check.selector.name.to_ascii_lowercase().contains("review") {
            "review"
        } else {
            "ci"
        }
        .into(),
        step: check.selector.name.clone(),
        candidate_sha: observation.head.clone(),
        pr_head: Some(observation.head.clone()),
        input_identity: format!("{id}:{revision}"),
        environment_identity: serde_json::to_string(&check.selector)?,
        command: Vec::from([check.selector.name.clone()]),
        native_code: crate::bounded_recovery::native_failure(&raw).into(),
        raw,
        log_ref,
        exit_code: None,
        authorized_code_check: matches!(
            check.selector.source,
            crate::github::Source::Actions { .. }
        ),
        retry_after_seconds: None,
    };
    crate::recovery_store::record(pool, id, source, &event, &failure).await?;
    if let (Some(run), Some(attempt)) = (
        proof["workflow_run"]["id"].as_i64(),
        proof["workflow_run"]["run_attempt"].as_i64(),
    ) {
        bind_attempt(pool, &event, run, attempt, observation).await?;
    }
    Ok(())
}

async fn new_evidence(
    pool: &PgPool,
    client: &mut AppClient,
    observation: &Observation,
    check: &Check,
    read_logs: bool,
    event: &str,
) -> Result<Option<(String, String, String)>> {
    let old: Option<serde_json::Value> =
        sqlx::query_scalar("SELECT facts FROM recovery_failure WHERE event_key=$1")
            .bind(event)
            .fetch_optional(pool)
            .await?;
    if old
        .as_ref()
        .is_some_and(|facts| facts["native_code"] != "unknown")
    {
        return Ok(None);
    }
    let (raw, path) = logs(client, observation, check, read_logs).await;
    let key = if old.is_some() {
        format!("{event}:evidence:{}", crate::validation::sha256(&raw))
    } else {
        event.to_owned()
    };
    if old.as_ref().is_some_and(|facts| facts["raw"] == raw) {
        return Ok(None);
    }
    Ok(Some((key, raw, path)))
}
