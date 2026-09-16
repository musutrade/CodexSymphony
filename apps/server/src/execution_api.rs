//! Localhost control adapter; all routes inherit Host/Origin/CSRF protection.
use crate::{execution::CODING_BLOCKER, run_store};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgPool;

type Error = (StatusCode, Json<Value>);
type Result<T> = std::result::Result<T, Error>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pause {
    pause: bool,
}

pub fn routes() -> Router<PgPool> {
    Router::new()
        .route("/api/execution", get(status))
        .route("/api/execution/pause", post(pause_global))
        .route("/api/requirements/{id}/pause", post(pause_requirement))
        .route("/api/requirements/{id}/cancel", post(cancel))
        .route("/api/requirements/{id}/delivery", get(delivery))
}

fn unavailable(_: sqlx::Error) -> Error {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({"error":"database unavailable"})),
    )
}

async fn status(State(pool): State<PgPool>) -> Result<Json<Value>> {
    let (owner, paused, recovery_complete): (Option<i64>, bool, bool) = sqlx::query_as(
        "SELECT requirement_id,paused,recovery_complete FROM execution_control WHERE id=1",
    )
    .fetch_one(&pool)
    .await
    .map_err(unavailable)?;
    Ok(Json(
        json!({"requirement_id":owner,"paused":paused,"recovery_complete":recovery_complete,"coding_ready":false,"coding_blocker":CODING_BLOCKER}),
    ))
}

fn valid(
    input: std::result::Result<Json<Pause>, axum::extract::rejection::JsonRejection>,
) -> Result<bool> {
    match input {
        Ok(Json(Pause { pause })) => Ok(pause),
        _ => Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error":"invalid JSON request"})),
        )),
    }
}

async fn pause_global(
    State(pool): State<PgPool>,
    input: std::result::Result<Json<Pause>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<Value>> {
    let paused = valid(input)?;
    control_pause(&pool, None, paused).await?;
    Ok(Json(json!({"paused":paused})))
}

async fn pause_requirement(
    State(pool): State<PgPool>,
    Path(id): Path<i64>,
    input: std::result::Result<Json<Pause>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<Value>> {
    let paused = valid(input)?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM requirement WHERE id=$1)")
        .bind(id)
        .fetch_one(&pool)
        .await
        .map_err(unavailable)?;
    if !exists {
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({"error":"requirement not found"})),
        ));
    }
    control_pause(&pool, Some(id), paused).await?;
    Ok(Json(json!({"paused":paused})))
}
async fn control_pause(pool: &PgPool, id: Option<i64>, paused: bool) -> Result<()> {
    if paused {
        run_store::pause(pool, id).await.map_err(unavailable)?;
    } else if !crate::delivery_control::resume(pool, id)
        .await
        .map_err(unavailable)?
    {
        return Err((
            StatusCode::CONFLICT,
            Json(json!({"error":"stop and preservation must finish before resume"})),
        ));
    }
    Ok(())
}
async fn cancel(State(pool): State<PgPool>, Path(id): Path<i64>) -> Result<Json<Value>> {
    if !crate::delivery_control::cancel(&pool, id)
        .await
        .map_err(unavailable)?
    {
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({"error":"requirement not found"})),
        ));
    }
    Ok(Json(
        json!({"cancel_requested":true,"cleanup_complete":false}),
    ))
}

async fn delivery(State(pool): State<PgPool>, Path(id): Path<i64>) -> Result<Json<Value>> {
    let deliveries: Vec<Value> = sqlx::query_scalar("SELECT jsonb_build_object('action_key',d.action_key,'validation_id',d.validation_id,'requirement_id',d.requirement_id,'revision',d.revision,'repository_id',d.repository_id,'repository',d.repository,'branch',d.branch,'head_sha',d.head_sha,'pr_number',d.pr_number,'consumer',d.consumer,'merged',d.released,'actions',(SELECT jsonb_agg(to_jsonb(a)) FROM delivery_action a WHERE a.action_key=d.action_key),'attempts',(SELECT jsonb_agg(to_jsonb(t)) FROM delivery_attempt t WHERE t.action_key=d.action_key)) FROM delivery d WHERE requirement_id=$1 ORDER BY revision")
        .bind(id).fetch_all(&pool).await.map_err(unavailable)?;
    Ok(Json(json!({"deliveries":deliveries})))
}
