//! Explicit recovery of a closed, unmerged delivery on the original account.
use crate::{budget_store::require, linked_repair::Scope, run_store};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Transaction};
type Result<T> = std::result::Result<T, sqlx::Error>;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Decision {
    pub request_id: String,
    pub version: i64,
    pub revision: i64,
    pub merge_key: String,
    pub head: String,
    pub base: String,
    pub paths: Vec<String>,
    pub reason: String,
}

pub async fn view(pool: &PgPool, requirement: i64) -> Result<Value> {
    let recoveries: Vec<Value> = sqlx::query_scalar(
        "SELECT to_jsonb(p) FROM pre_merge_recovery p WHERE requirement_id=$1 ORDER BY created_at",
    )
    .bind(requirement)
    .fetch_all(pool)
    .await?;
    Ok(json!({"recoveries":recoveries}))
}

pub async fn decide(pool: &PgPool, requirement: i64, command: &Decision) -> Result<Value> {
    validate(command)?;
    let mut tx = run_store::lock(pool).await?;
    let input = json!({"pre_merge_recovery":requirement,"command":command});
    if let Some(saved) = replay(&mut tx, command, &input).await? {
        return Ok(saved);
    }
    let scope = eligible(&mut tx, requirement, command).await?;
    validate_paths(&scope, &command.paths)?;
    persist(tx, requirement, command, input).await
}

fn validate(command: &Decision) -> Result<()> {
    require(
        crate::contract::validate_request_id(&command.request_id).is_ok(),
        "invalid request identity",
    )?;
    require(
        command.version > 0 && command.revision > 0,
        "invalid recovery version",
    )?;
    require(
        oid(&command.head) && oid(&command.base),
        "exact head and base required",
    )?;
    require(
        !command.reason.trim().is_empty() && command.reason.len() <= 4096,
        "recovery reason required",
    )?;
    require(
        !command.paths.is_empty() && command.paths.len() <= 128,
        "explicit bounded paths required",
    )
}

fn oid(value: &str) -> bool {
    if value.len() != 40 {
        return false;
    }
    for byte in value.bytes() {
        if !byte.is_ascii_hexdigit() {
            return false;
        }
    }
    true
}

fn validate_paths(scope: &str, paths: &[String]) -> Result<()> {
    let scope = Scope::parse(scope).map_err(protocol)?;
    let mut seen = std::collections::BTreeSet::new();
    for path in paths {
        require(seen.insert(path), "duplicate recovery path")?;
        require(
            path_authorized(&scope, path),
            "recovery path outside original scope",
        )?;
    }
    Ok(())
}

fn path_authorized(scope: &Scope, path: &str) -> bool {
    for paths in scope.checks.values() {
        for allowed in paths {
            if allowed == path {
                return true;
            }
        }
    }
    false
}
fn protocol(value: &'static str) -> sqlx::Error {
    sqlx::Error::Protocol(value.into())
}

async fn replay(
    tx: &mut Transaction<'_, Postgres>,
    command: &Decision,
    input: &Value,
) -> Result<Option<Value>> {
    let saved: Option<(Value, Value)> =
        sqlx::query_as("SELECT input,result FROM business_request WHERE request_id=$1")
            .bind(&command.request_id)
            .fetch_optional(&mut **tx)
            .await?;
    if let Some((old, result)) = saved {
        require(old == *input, "recovery request identity conflict")?;
        return Ok(Some(result));
    }
    Ok(None)
}

async fn eligible(
    tx: &mut Transaction<'_, Postgres>,
    id: i64,
    command: &Decision,
) -> Result<String> {
    let row: Option<String> = sqlx::query_scalar("SELECT i.input#>>'{review,repair_scope}' FROM requirement r JOIN execution_control c ON c.requirement_id=r.id JOIN group_execution_item i ON i.requirement_id=r.id JOIN merge_operation m ON m.requirement_id=r.id JOIN delivery d ON d.action_key=m.delivery_key WHERE r.id=$1 AND r.version=$2 AND r.revision=$3 AND m.action_key=$4 AND r.state='Submitted' AND NOT r.cancel_requested AND (r.paused OR c.paused) AND c.recovery_complete AND m.state='blocked' AND NOT m.merge_started AND NOT m.pre_validation_started AND m.merged_sha IS NULL AND m.acceptance IS NULL AND d.head_sha=$5 AND d.superseded_by IS NULL AND i.input#>>'{child,kind}'='code_change' AND NOT EXISTS(SELECT 1 FROM agent_run WHERE NOT quiescent) AND NOT EXISTS(SELECT 1 FROM integration_validation WHERE NOT quiescent) AND NOT EXISTS(SELECT 1 FROM workspace_operation WHERE status<>'complete') AND NOT EXISTS(SELECT 1 FROM linked_failure f WHERE f.requirement_id=r.id AND f.state NOT IN ('complete','cancelled')) AND NOT EXISTS(SELECT 1 FROM pre_merge_recovery p WHERE p.merge_key=m.action_key AND (p.state<>'blocked' OR p.failure_id IS NOT NULL)) AND NOT EXISTS(SELECT 1 FROM delivery_action a WHERE a.action_key=d.action_key AND a.state NOT IN ('confirmed','withdrawn'))")
        .bind(id).bind(command.version).bind(command.revision).bind(&command.merge_key).bind(&command.head).fetch_optional(&mut **tx).await?;
    let scope = row.ok_or(sqlx::Error::RowNotFound)?;
    require(
        crate::group_queue_store::authorized(tx, id).await?,
        "original group authorization unavailable",
    )?;
    require(
        closed_identity(tx, &command.merge_key, &command.head, &command.base).await?,
        "close and reconcile the exact unmerged PR first",
    )?;
    Ok(scope)
}

pub(crate) async fn closed_identity(
    tx: &mut Transaction<'_, Postgres>,
    merge: &str,
    head: &str,
    base: &str,
) -> Result<bool> {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM merge_operation m JOIN delivery d ON d.action_key=m.delivery_key JOIN github_pr p ON p.repository_id=d.repository_id AND p.number=d.pr_number AND p.requirement_id=d.requirement_id WHERE m.action_key=$1 AND NOT p.stale AND p.last_synced_at BETWEEN extract(epoch FROM now())::bigint-59 AND extract(epoch FROM now())::bigint AND p.observation->>'head'=$2 AND p.observation->>'base'=$3 AND p.observation->>'head_ref'=d.branch AND p.observation->>'base_ref'=d.base_branch AND p.observation->>'closed'='true' AND p.observation->>'merge'='Unmerged' AND m.merged_sha IS NULL AND NOT m.merge_started)")
        .bind(merge).bind(head).bind(base).fetch_one(&mut **tx).await
}

async fn persist(
    mut tx: Transaction<'_, Postgres>,
    id: i64,
    command: &Decision,
    input: Value,
) -> Result<Value> {
    let recovery = format!("pre-merge:{}", command.merge_key);
    sqlx::query("INSERT INTO pre_merge_recovery(id,requirement_id,revision,merge_key,request) VALUES($1,$2,$3,$4,$5) ON CONFLICT(id) DO UPDATE SET request=excluded.request,state='pending',attempts=0,next_attempt_at=0,blocker=NULL,receipts=pre_merge_recovery.receipts||jsonb_build_array(jsonb_build_object('previous_request',pre_merge_recovery.request,'previous_attempts',pre_merge_recovery.attempts,'previous_blocker',pre_merge_recovery.blocker)) WHERE pre_merge_recovery.state='blocked' AND pre_merge_recovery.failure_id IS NULL")
        .bind(&recovery).bind(id).bind(command.revision).bind(&command.merge_key).bind(json!(command)).execute(&mut *tx).await?;
    let version: i64 = sqlx::query_scalar(
        "UPDATE requirement SET version=version+1 WHERE id=$1 RETURNING version",
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    let result = json!({"accepted":true,"started":false,"version":version,"recovery_id":recovery});
    sqlx::query("INSERT INTO business_request(request_id,input,result) VALUES($1,$2,$3)")
        .bind(&command.request_id)
        .bind(input)
        .bind(&result)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(result)
}

/// A stale/reopened/merged original PR cannot authorize replacement work.
pub(crate) async fn admitted(tx: &mut Transaction<'_, Postgres>, failure: &str) -> Result<bool> {
    let row: Option<(String, Value, String)> = sqlx::query_as(
        "SELECT merge_key,request,state FROM pre_merge_recovery WHERE failure_id=$1",
    )
    .bind(failure)
    .fetch_optional(&mut **tx)
    .await?;
    let Some((merge, request, state)) = row else {
        return Ok(true);
    };
    if state != "ready" {
        return Ok(false);
    }
    let command: Decision = serde_json::from_value(request).map_err(decode_error)?;
    closed_identity(tx, &merge, &command.head, &command.base).await
}
fn decode_error(_: serde_json::Error) -> sqlx::Error {
    protocol("invalid retained recovery decision")
}

pub(crate) async fn delivery_allowed(
    tx: &mut Transaction<'_, Postgres>,
    delivery: &str,
) -> Result<bool> {
    let failure: Option<String> = sqlx::query_scalar("SELECT failure_id FROM pre_merge_recovery p JOIN linked_failure f ON f.id=p.failure_id WHERE f.repair_delivery=$1")
        .bind(delivery).fetch_optional(&mut **tx).await?;
    match failure {
        Some(id) => admitted(tx, &id).await,
        None => Ok(true),
    }
}

pub(crate) async fn supersede(tx: &mut Transaction<'_, Postgres>, replacement: &str) -> Result<()> {
    sqlx::query("UPDATE delivery d SET superseded_by=$1 FROM pre_merge_recovery p JOIN linked_failure f ON f.id=p.failure_id JOIN merge_operation m ON m.action_key=p.merge_key WHERE f.repair_delivery=$1 AND d.action_key=m.delivery_key AND d.superseded_by IS NULL AND NOT m.merge_started AND m.merged_sha IS NULL")
        .bind(replacement).execute(&mut **tx).await?;
    Ok(())
}

#[cfg(test)]
#[path = "../tests/unit/pre_merge_recovery.rs"]
pub(crate) mod tests;
