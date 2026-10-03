//! Host-only paused recheck of the same retained integration failure and account.
use crate::{
    budget_store::{decode, require},
    run_store,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Transaction};
type Result<T> = std::result::Result<T, sqlx::Error>;
type Tx<'a> = Transaction<'a, Postgres>;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub request_id: String,
    pub requirement_id: i64,
    pub version: i64,
    pub revision: i64,
    pub failure_id: String,
    pub budget_version: i64,
    pub configuration_sha256: String,
    pub reason: String,
}

pub async fn recheck(pool: &PgPool, command: &Command) -> Result<Value> {
    validate(command)?;
    let input = json!({"linked_source_recovery":command});
    let mut tx = run_store::lock(pool).await?;
    if let Some(result) = replay(&mut tx, command, &input).await? {
        return Ok(result);
    }
    authorize(&mut tx, command).await?;
    let result = persist(&mut tx, command, input).await?;
    tx.commit().await?;
    Ok(result)
}

async fn persist(tx: &mut Tx<'_>, command: &Command, input: Value) -> Result<Value> {
    let version: i64 = sqlx::query_scalar(
        "UPDATE requirement SET version=version+1 WHERE id=$1 RETURNING version",
    )
    .bind(command.requirement_id)
    .fetch_one(&mut **tx)
    .await?;
    let result = json!({"accepted":true,"started":false,"version":version,"failure_id":command.failure_id,"previous_blocker":"failure outside authorized repair checks"});
    sqlx::query("UPDATE linked_failure SET state='observed',blocker=NULL,source_receipts=source_receipts||jsonb_build_array($2::jsonb) WHERE id=$1")
        .bind(&command.failure_id)
        .bind(json!({"action":"paused_source_recheck","actor":"host_operator","command":command,"previous_blocker":"failure outside authorized repair checks","started":false}))
        .execute(&mut **tx).await?;
    sqlx::query("INSERT INTO business_request(request_id,input,result) VALUES($1,$2,$3)")
        .bind(&command.request_id)
        .bind(input)
        .bind(&result)
        .execute(&mut **tx)
        .await?;
    Ok(result)
}

fn validate(command: &Command) -> Result<()> {
    require(
        crate::contract::validate_request_id(&command.request_id).is_ok(),
        "invalid recovery request identity",
    )?;
    require(
        !command.reason.trim().is_empty() && command.reason.len() <= 4096,
        "recovery reason required",
    )?;
    require(
        command.configuration_sha256.len() == 64,
        "configuration digest required",
    )?;
    for byte in command.configuration_sha256.bytes() {
        require(byte.is_ascii_hexdigit(), "invalid configuration digest")?;
    }
    Ok(())
}

async fn replay(tx: &mut Tx<'_>, command: &Command, input: &Value) -> Result<Option<Value>> {
    let saved: Option<(Value, Value)> =
        sqlx::query_as("SELECT input,result FROM business_request WHERE request_id=$1")
            .bind(&command.request_id)
            .fetch_optional(&mut **tx)
            .await?;
    if let Some((previous, result)) = saved {
        require(previous == *input, "recovery request identity conflict")?;
        return Ok(Some(result));
    }
    Ok(None)
}

async fn authorize(tx: &mut Tx<'_>, command: &Command) -> Result<()> {
    let row: Option<(Value, Value, Value, Value)> = sqlx::query_as("SELECT i.input,v.binding,f.evidence,f.required_steps FROM linked_failure f JOIN requirement r ON r.id=f.requirement_id JOIN execution_control c ON c.requirement_id=r.id JOIN requirement_budget b ON b.requirement_id=r.id JOIN group_execution_item i ON i.requirement_id=r.id JOIN integration_validation v ON v.id=f.integration_id WHERE f.id=$1 AND r.id=$2 AND r.version=$3 AND r.revision=$4 AND f.revision=r.revision AND r.state='Running' AND NOT r.cancel_requested AND c.paused AND c.recovery_complete AND f.state='blocked' AND f.blocker='failure outside authorized repair checks' AND f.baseline IS NULL AND f.repository_id IS NULL AND f.source_run IS NULL AND b.version=$5 AND NOT b.exhausted AND v.state='failed' AND v.quiescent AND f.evidence=v.result->'evidence' AND v.revision=r.revision AND v.authorization_id=i.authorization_id AND (v.binding->>'requirement')::bigint=r.id AND (v.binding->>'revision')::bigint=r.revision AND (v.binding->>'authorization')::bigint=i.authorization_id AND v.binding#>>'{trusted,config_sha256}'=$6 AND i.input#>>'{review,integration,configuration_sha256}'=$6 AND NOT i.frozen AND NOT i.removed AND NOT (SELECT blocked FROM storage_guard WHERE id=1) AND NOT EXISTS(SELECT 1 FROM repair_reservation WHERE requirement_id=r.id) AND NOT EXISTS(SELECT 1 FROM agent_run WHERE NOT quiescent) AND NOT EXISTS(SELECT 1 FROM integration_validation WHERE NOT quiescent) AND NOT EXISTS(SELECT 1 FROM workspace_operation WHERE status<>'complete')")
        .bind(&command.failure_id).bind(command.requirement_id).bind(command.version).bind(command.revision).bind(command.budget_version).bind(&command.configuration_sha256).fetch_optional(&mut **tx).await?;
    let (input, binding, evidence, required) = row.ok_or(sqlx::Error::RowNotFound)?;
    require(
        crate::group_queue_store::authorized(tx, command.requirement_id).await?,
        "original group authorization unavailable",
    )?;
    let binding = decode(binding)?;
    let evidence = decode(evidence)?;
    let required: Vec<String> = decode(required)?;
    crate::linked_repair_source::recheck_integration_scope(&input, &binding, &evidence, &required)
        .map_err(scope_error)
}

fn scope_error(_: Box<dyn std::error::Error + Send + Sync>) -> sqlx::Error {
    sqlx::Error::Protocol(
        "original integration repair scope still rejects retained evidence".into(),
    )
}

#[cfg(test)]
#[path = "../tests/unit/linked_source_recovery.rs"]
mod tests;
