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
) -> Result<()> {
    match input {
        Ok(Json(Pause { pause: true })) => Ok(()),
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
    valid(input)?;
    run_store::pause(&pool, None).await.map_err(unavailable)?;
    Ok(Json(json!({"paused":true})))
}

async fn pause_requirement(
    State(pool): State<PgPool>,
    Path(id): Path<i64>,
    input: std::result::Result<Json<Pause>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<Value>> {
    valid(input)?;
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
    run_store::pause(&pool, Some(id))
        .await
        .map_err(unavailable)?;
    Ok(Json(json!({"paused":true})))
}
