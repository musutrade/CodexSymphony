//! Session/CSRF protected operator recovery; no arbitrary execution endpoints.
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use serde_json::{Value, json};
use sqlx::PgPool;
type Result<T> = std::result::Result<T, (StatusCode, Json<Value>)>;

pub fn routes() -> Router<PgPool> {
    Router::new().route(
        "/api/requirements/{id}/extension-recovery",
        get(detail).post(resolve),
    )
}
async fn detail(State(pool): State<PgPool>, Path(id): Path<i64>) -> Result<Json<Value>> {
    crate::extension_recovery::view(&pool, id)
        .await
        .map(Json)
        .map_err(error)
}
async fn resolve(
    State(pool): State<PgPool>,
    Path(id): Path<i64>,
    input: std::result::Result<
        Json<crate::extension_recovery::Decision>,
        axum::extract::rejection::JsonRejection,
    >,
) -> Result<Json<Value>> {
    let Json(command) = input.map_err(invalid)?;
    crate::extension_recovery::decide(&pool, id, &command)
        .await
        .map(Json)
        .map_err(error)
}
fn invalid(_: axum::extract::rejection::JsonRejection) -> (StatusCode, Json<Value>) {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({"error":"invalid recovery decision"})),
    )
}
fn error(error: sqlx::Error) -> (StatusCode, Json<Value>) {
    match error {
        sqlx::Error::RowNotFound | sqlx::Error::Protocol(_) => (
            StatusCode::CONFLICT,
            Json(json!({"error":"recovery identity, authorization or prerequisites changed"})),
        ),
        _ => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error":"recovery storage unavailable"})),
        ),
    }
}
