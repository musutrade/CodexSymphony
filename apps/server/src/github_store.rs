//! Durable facts with compare-and-set policy identity; no queue release here.
use crate::{
    github::{Capability, Observation, Policy, retry_delay},
    github_http::Error,
};
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Transaction};

pub async fn configure(pool: &PgPool, policy: &Policy, probe_pr: u64) -> Result<bool, sqlx::Error> {
    let changed = sqlx::query("INSERT INTO github_repository(repository_id,repository_version,policy,probe_pr) SELECT $1,$2,$3,$4 FROM repository WHERE version=$2 AND (document->>'github_repository_id')::bigint=$1 AND document->>'remote'=$5 AND document->>'base_branch'=$6 AND NOT (document->>'revoked')::boolean ON CONFLICT(repository_id) DO UPDATE SET repository_version=$2,policy=$3,probe_pr=$4,stale=true,next_attempt_at=0 WHERE github_repository.policy IS DISTINCT FROM $3 OR github_repository.probe_pr<>$4")
        .bind(policy.repository_id as i64).bind(policy.version).bind(sqlx::types::Json(policy)).bind(probe_pr as i64)
        .bind(&policy.repository).bind(&policy.default_branch).execute(pool).await?;
    Ok(changed.rows_affected() == 1)
}
pub async fn link(
    pool: &PgPool,
    repository: u64,
    number: u64,
    requirement: i64,
) -> Result<bool, sqlx::Error> {
    let changed = sqlx::query("INSERT INTO github_pr(repository_id,number,requirement_id) SELECT $1,$2,r.id FROM requirement r JOIN requirement_revision v ON v.requirement_id=r.id AND v.revision=r.revision JOIN github_repository g ON (v.document->'repository'->>'github_repository_id')::bigint=g.repository_id WHERE r.id=$3 AND g.repository_id=$1 ON CONFLICT DO NOTHING")
        .bind(repository as i64).bind(number as i64).bind(requirement).execute(pool).await?;
    Ok(changed.rows_affected() == 1)
}
pub async fn claim_ready(
    tx: &mut Transaction<'_, Postgres>,
    requirement: i64,
    revision: i64,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM requirement_revision v JOIN repository r ON r.id=COALESCE((v.document->>'repository_id')::bigint,1) JOIN github_repository g ON g.repository_id=(r.document->>'github_repository_id')::bigint WHERE v.requirement_id=$1 AND v.revision=$2 AND g.repository_version=r.version AND (v.document->>'repository_version')::bigint=r.version AND NOT g.stale AND g.checked_at>extract(epoch FROM now())::bigint-60 AND g.capability->'blockers'='[]'::jsonb AND g.capability->'policy'=g.policy)")
        .bind(requirement).bind(revision).fetch_one(&mut **tx).await
}
pub async fn save_capability(pool: &PgPool, capability: &Capability) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE github_repository SET capability=$2,checked_at=$3,stale=false,failures=0,error=NULL,next_attempt_at=$3+60 WHERE repository_id=$1 AND policy=$4")
        .bind(capability.policy.repository_id as i64).bind(sqlx::types::Json(capability)).bind(capability.checked_at)
        .bind(sqlx::types::Json(&capability.policy)).execute(pool).await?;
    Ok(())
}
pub async fn save_observation(pool: &PgPool, observation: &Observation) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE github_pr SET observation=$3,last_synced_at=$4,stale=false,failures=0,error=NULL,next_attempt_at=$4+60 WHERE repository_id=$1 AND number=$2 AND EXISTS(SELECT 1 FROM github_repository g WHERE g.repository_id=$1 AND g.policy=$5)")
        .bind(observation.repository_id as i64).bind(observation.number as i64).bind(sqlx::types::Json(observation)).bind(observation.last_synced_at).bind(sqlx::types::Json(&observation.policy)).execute(pool).await?;
    Ok(())
}
pub async fn failed(
    pool: &PgPool,
    repo: u64,
    number: Option<u64>,
    failures: i32,
    now: i64,
    error: &Error,
) -> Result<(), sqlx::Error> {
    let next = now + retry_delay((failures + 1) as u32);
    let evidence = json!({"code":error.code,"phase":"github_observation","http_status":error.status,"attempts":failures+1,"next_attempt_at":next});
    match number {
        Some(number) => {
            sqlx::query("UPDATE github_pr SET stale=true,failures=failures+1,next_attempt_at=$3,error=$4 WHERE repository_id=$1 AND number=$2")
            .bind(repo as i64).bind(number as i64).bind(next).bind(evidence).execute(pool).await?;
        }
        None => {
            sqlx::query("UPDATE github_repository SET stale=true,failures=failures+1,next_attempt_at=$2,error=$3 WHERE repository_id=$1")
            .bind(repo as i64).bind(next).bind(evidence).execute(pool).await?;
        }
    }
    Ok(())
}
pub async fn due_repositories(
    pool: &PgPool,
    now: i64,
) -> Result<Vec<(Value, i64, i32)>, sqlx::Error> {
    sqlx::query_as("SELECT policy,probe_pr,failures FROM github_repository WHERE next_attempt_at<=$1 ORDER BY repository_id")
        .bind(now).fetch_all(pool).await
}
pub async fn due_prs(pool: &PgPool, now: i64) -> Result<Vec<(Value, i64, i32)>, sqlx::Error> {
    sqlx::query_as("SELECT g.policy,p.number,p.failures FROM github_pr p JOIN github_repository g USING(repository_id) WHERE p.next_attempt_at<=$1 ORDER BY p.repository_id,p.number")
        .bind(now).fetch_all(pool).await
}
