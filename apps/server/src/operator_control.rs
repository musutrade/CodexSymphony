//! Versioned operator intent shares the execution lock and existing stop/cleanup rules.
use crate::{delivery_control, run_store};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgPool;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub version: i64,
    pub request_id: String,
    pub action: Action,
}
#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Pause,
    Resume,
    Cancel,
    Recheck,
    StorageRecheck,
}
type Result<T> = std::result::Result<T, sqlx::Error>;
fn require(value: bool) -> Result<()> {
    if value {
        Ok(())
    } else {
        Err(sqlx::Error::Protocol(
            "control version or availability changed".into(),
        ))
    }
}
pub async fn execute(pool: &PgPool, id: i64, command: &Command) -> Result<Value> {
    require(crate::contract::validate_request_id(&command.request_id).is_ok())?;
    let mut tx = run_store::lock(pool).await?;
    let input = json!({"operator_requirement":id,"command":command});
    if let Some(result) = replay(&mut tx, &command.request_id, &input).await? {
        return Ok(result);
    }
    let version = apply_current(&mut tx, id, command).await?;
    let result = record(&mut tx, id, command, version, input).await?;
    tx.commit().await?;
    Ok(result)
}
async fn replay(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    request_id: &str,
    input: &Value,
) -> Result<Option<Value>> {
    let saved = sqlx::query_as::<_, (Value, Value)>(
        "SELECT input,result FROM business_request WHERE request_id=$1",
    )
    .bind(request_id)
    .fetch_optional(&mut **tx)
    .await?;
    if let Some((saved, result)) = saved {
        require(saved == *input)?;
        return Ok(Some(result));
    }
    Ok(None)
}
async fn apply_current(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
    command: &Command,
) -> Result<i64> {
    let (version, state, paused, cancelled): (i64, String, bool, bool) = sqlx::query_as(
        "SELECT version,state,paused,cancel_requested FROM requirement WHERE id=$1 FOR UPDATE",
    )
    .bind(id)
    .fetch_one(&mut **tx)
    .await?;
    require(
        version == command.version
            && (!cancelled || matches!(command.action, Action::StorageRecheck)),
    )?;
    require(allowed(&state, paused, command.action))?;
    apply(tx, id, command.action).await?;
    sqlx::query_scalar(
        "UPDATE requirement SET version=GREATEST(version,$2+1) WHERE id=$1 RETURNING version",
    )
    .bind(id)
    .bind(version)
    .fetch_one(&mut **tx)
    .await
}
async fn record(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
    command: &Command,
    version: i64,
    input: Value,
) -> Result<Value> {
    let result = json!({"version":version});
    sqlx::query("INSERT INTO business_event(object_id,kind,version) VALUES($1,$2,$3)")
        .bind(format!("requirement:{id}"))
        .bind(format!(
            "operator_{}",
            json!(command.action).as_str().unwrap()
        ))
        .bind(version)
        .execute(&mut **tx)
        .await?;
    sqlx::query("INSERT INTO business_request(request_id,input,result) VALUES($1,$2,$3)")
        .bind(&command.request_id)
        .bind(input)
        .bind(&result)
        .execute(&mut **tx)
        .await?;
    Ok(result)
}
pub fn allowed(state: &str, paused: bool, action: Action) -> bool {
    match action {
        Action::Pause => !paused && ["Ready", "Running", "Submitted"].contains(&state),
        Action::Resume => paused && ["Ready", "Running", "Submitted"].contains(&state),
        Action::Recheck => ["Ready", "Running"].contains(&state),
        Action::StorageRecheck => true,
        Action::Cancel => matches!(
            state,
            "Draft" | "Ready" | "Running" | "Submitted" | "Failed"
        ),
    }
}
async fn apply(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
    action: Action,
) -> Result<()> {
    match action {
        Action::Pause => run_store::pause_in(tx, Some(id)).await,
        Action::Resume => require(delivery_control::resume_in(tx, Some(id)).await?),
        Action::Cancel => require(delivery_control::cancel_in(tx, id).await?),
        Action::Recheck => recheck(tx, id).await,
        Action::StorageRecheck => storage_recheck(tx, id).await,
    }
}

async fn storage_recheck(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, id: i64) -> Result<()> {
    let config = crate::storage_store::deployment(tx)
        .await
        .map_err(storage_error)?
        .ok_or(sqlx::Error::Protocol(
            "storage configuration missing".into(),
        ))?;
    crate::storage_cleanup::authorize(tx, id, crate::runtime_client::now())
        .await
        .map_err(storage_error)?;
    crate::storage::recover_in(tx, &config.execution.path)
        .await
        .map_err(storage_error)?;
    sqlx::query(
        "INSERT INTO operator_intervention(requirement_id,reason) VALUES($1,'storage_recheck')",
    )
    .bind(id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
fn storage_error(_: Box<dyn std::error::Error + Send + Sync>) -> sqlx::Error {
    sqlx::Error::Protocol("storage recovery remains blocked; retain originals".into())
}

async fn recheck(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, id: i64) -> Result<()> {
    let run:String=sqlx::query_scalar("SELECT p.run_id FROM preparation_record p JOIN requirement r ON r.id=p.requirement_id AND r.revision=p.revision WHERE p.requirement_id=$1 AND NOT p.ready AND (p.retry->>'todo')::boolean AND NOT EXISTS(SELECT 1 FROM agent_run a WHERE a.id=p.run_id) ORDER BY p.checked_at DESC LIMIT 1")
        .bind(id).fetch_one(&mut **tx).await?;
    crate::preparation_store::authorize_retry_in(
        tx,
        &run,
        crate::runtime_client::now(),
        "explicit current-version operator recheck",
    )
    .await?;
    sqlx::query(
        "INSERT INTO operator_intervention(requirement_id,reason) VALUES($1,'preparation_recheck')",
    )
    .bind(id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
