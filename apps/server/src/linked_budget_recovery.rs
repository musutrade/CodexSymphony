//! Explicit, paused recovery after a retained original-account budget grant.
use crate::{
    budget::Amount,
    budget_store::{decode, require},
    run_store,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Transaction};
type Result<T> = std::result::Result<T, sqlx::Error>;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub request_id: String,
    pub requirement_id: i64,
    pub version: i64,
    pub failure_id: String,
    pub budget_version: i64,
    pub authorization_request_id: String,
}

pub async fn recheck(pool: &PgPool, command: &Command) -> Result<Value> {
    require(
        crate::contract::validate_request_id(&command.request_id).is_ok(),
        "invalid recovery request identity",
    )?;
    let input = json!({"linked_budget_recovery":command});
    let mut tx = run_store::lock(pool).await?;
    if let Some(result) = replay(&mut tx, command, &input).await? {
        return Ok(result);
    }
    authorize(&mut tx, command).await?;
    let result = json!({"accepted":true,"started":false,"failure_id":command.failure_id,"budget_version":command.budget_version,"previous_blocker":"shared item or parent repair budget exhausted"});
    sqlx::query("UPDATE linked_failure SET state='observed',blocker=NULL WHERE id=$1")
        .bind(&command.failure_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO business_request(request_id,input,result) VALUES($1,$2,$3)")
        .bind(&command.request_id)
        .bind(input)
        .bind(&result)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(result)
}

async fn replay(
    tx: &mut Transaction<'_, Postgres>,
    command: &Command,
    input: &Value,
) -> Result<Option<Value>> {
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

async fn authorize(tx: &mut Transaction<'_, Postgres>, command: &Command) -> Result<()> {
    let delta: Option<Value> = sqlx::query_scalar("SELECT a.delta FROM linked_failure f JOIN requirement r ON r.id=f.requirement_id JOIN execution_control c ON c.requirement_id=r.id JOIN requirement_budget b ON b.requirement_id=r.id JOIN budget_authorization a ON a.requirement_id=b.requirement_id AND a.version=b.version WHERE f.id=$1 AND r.id=$2 AND r.version=$3 AND r.revision=f.revision AND r.state IN ('Running','Submitted') AND NOT r.cancel_requested AND (r.paused OR c.paused) AND c.recovery_complete AND f.state='blocked' AND f.blocker='shared item or parent repair budget exhausted' AND f.baseline IS NOT NULL AND b.version=$4 AND a.request_id=$5 AND a.created_at>f.created_at AND NOT EXISTS(SELECT 1 FROM repair_reservation WHERE linked_failure_id=f.id) AND NOT EXISTS(SELECT 1 FROM agent_run WHERE NOT quiescent) AND NOT EXISTS(SELECT 1 FROM integration_validation WHERE NOT quiescent) AND NOT EXISTS(SELECT 1 FROM workspace_operation WHERE status<>'complete') AND NOT EXISTS(SELECT 1 FROM business_request q WHERE q.input#>>'{linked_budget_recovery,failure_id}'=f.id AND q.input#>>'{linked_budget_recovery,authorization_request_id}'=a.request_id)")
        .bind(&command.failure_id).bind(command.requirement_id).bind(command.version).bind(command.budget_version).bind(&command.authorization_request_id).fetch_optional(&mut **tx).await?;
    let delta: Amount = decode(delta.ok_or(sqlx::Error::RowNotFound)?)?;
    require(
        delta.nonnegative() && delta != Amount::default(),
        "positive original-account grant required",
    )?;
    require(
        crate::group_queue_store::authorized(tx, command.requirement_id).await?,
        "original group authorization unavailable",
    )
}

#[cfg(test)]
#[path = "../tests/unit/linked_budget_recovery.rs"]
mod tests;
