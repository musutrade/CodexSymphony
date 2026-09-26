//! Runtime identity evidence bound to the original Run and reviewed selection.
use crate::{
    execution::RunKey,
    model_selection::{Frozen, Registration},
    runtime_store,
};
use serde_json::{Value, json};
use sqlx::PgPool;
type Result<T> = std::result::Result<T, sqlx::Error>;

pub async fn load(pool: &PgPool, key: &RunKey) -> Result<Option<Frozen>> {
    let value: Option<Value> = sqlx::query_scalar("SELECT v.document->'frozen_model' FROM agent_run a JOIN execution_revision v ON v.requirement_id=a.requirement_id AND v.revision=a.revision WHERE a.id=$1 AND a.request_id=$2 AND a.incarnation=$3")
        .bind(&key.run_id).bind(&key.request_id).bind(&key.incarnation).fetch_one(pool).await?;
    match value {
        Some(value) => Ok(Some(serde_json::from_value(value).map_err(decode_error)?)),
        None => Ok(None),
    }
}

fn decode_error(_: serde_json::Error) -> sqlx::Error {
    runtime_store::invalid("invalid frozen model selection")
}

pub async fn admit(
    pool: &PgPool,
    key: &RunKey,
    frozen: &Frozen,
    registration: Option<&Registration>,
) -> Result<()> {
    let registration =
        registration.ok_or(runtime_store::invalid("model capability unavailable"))?;
    runtime_store::require(
        registration.version == frozen.capability_version,
        "model capability version changed; review again",
    )?;
    let repository:i64=sqlx::query_scalar("SELECT plugin_scope_repository(requirement_id,revision) FROM agent_run WHERE id=$1 AND request_id=$2 AND incarnation=$3")
        .bind(&key.run_id).bind(&key.request_id).bind(&key.incarnation).fetch_one(pool).await?;
    registration
        .admit(repository, &frozen.selection.config)
        .map_err(runtime_store::invalid)
}

pub async fn record(pool: &PgPool, key: &RunKey, frozen: &Frozen, response: &Value) -> Result<()> {
    let checked = crate::model_selection::check_response(frozen, response);
    let actual = json!({"model":response["model"],"provider":response["modelProvider"],"effort":response["reasoningEffort"]});
    sqlx::query(
        "INSERT INTO runtime_model_identity(run_id,frozen,actual,matched) VALUES($1,$2,$3,$4)",
    )
    .bind(&key.run_id)
    .bind(json!(frozen))
    .bind(actual)
    .bind(checked.is_ok())
    .execute(pool)
    .await?;
    checked.map_err(runtime_store::invalid)
}

pub async fn view(pool: &PgPool, requirement: i64) -> Result<Value> {
    let identities: Vec<Value> = sqlx::query_scalar("SELECT jsonb_build_object('run_id',a.id,'revision',a.revision,'frozen',COALESCE(i.frozen,v.document->'frozen_model'),'actual',i.actual,'matched',i.matched,'usage',COALESCE((SELECT jsonb_agg(jsonb_build_object('call_id',c.turn_id,'usage',c.usage,'reserved',c.reserved) ORDER BY c.created_at,c.turn_id) FROM model_call c WHERE c.run_id=a.id),'[]'::jsonb)) FROM agent_run a JOIN execution_revision v ON v.requirement_id=a.requirement_id AND v.revision=a.revision LEFT JOIN runtime_model_identity i ON i.run_id=a.id WHERE a.requirement_id=$1 ORDER BY a.run_sequence")
        .bind(requirement).fetch_all(pool).await?;
    Ok(json!({"runs":identities}))
}

/// Reject unsupported choices before opening a Runtime process or reserving usage.
pub async fn prepare(
    pool: &PgPool,
    key: &RunKey,
    registration: Option<&Registration>,
) -> Result<Option<Frozen>> {
    let frozen = load(pool, key).await?;
    if let Some(selection) = &frozen
        && let Err(error) = admit(pool, key, selection, registration).await
    {
        sqlx::query("UPDATE agent_run SET blocker='model configuration unavailable or unauthorized; reconcile deployment',stop_requested=true WHERE id=$1 AND request_id=$2 AND incarnation=$3 AND NOT quiescent")
            .bind(&key.run_id).bind(&key.request_id).bind(&key.incarnation).execute(pool).await?;
        return Err(error);
    }
    Ok(frozen)
}
