//! Question answers persist independently of the originating RPC connection.
use crate::runtime_questions::{self, Answer};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use serde_json::{Value, json};
use sqlx::PgPool;
type Result<T> = std::result::Result<T, (StatusCode, Json<Value>)>;

pub fn routes() -> Router<PgPool> {
    Router::new()
        .route("/api/requirements/{id}/questions", get(list))
        .route("/api/questions/{id}/answer", post(answer))
}
fn error(error: sqlx::Error) -> (StatusCode, Json<Value>) {
    match error {
        sqlx::Error::RowNotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({"error":"question not found"})),
        ),
        sqlx::Error::Protocol(_) => (
            StatusCode::CONFLICT,
            Json(json!({"error":"question version, answer or authorization changed"})),
        ),
        _ => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error":"question storage unavailable"})),
        ),
    }
}
async fn list(State(pool): State<PgPool>, Path(id): Path<i64>) -> Result<Json<Value>> {
    let questions = runtime_questions::list(&pool, id).await.map_err(error)?;
    Ok(Json(json!({"questions":questions})))
}
async fn answer(
    State(pool): State<PgPool>,
    Path(id): Path<String>,
    input: std::result::Result<Json<Answer>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<Value>> {
    let Json(answer) = input.map_err(|_| {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error":"invalid answer"})),
        )
    })?;
    let question = runtime_questions::answer(&pool, &id, &answer, crate::runtime_client::now())
        .await
        .map_err(error)?;
    Ok(Json(json!({"question":question})))
}
