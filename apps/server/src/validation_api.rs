use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use serde_json::{Value, json};
use sqlx::PgPool;

pub fn routes() -> Router<PgPool> {
    Router::new().route("/api/validations/{id}", get(get_validation))
}

async fn get_validation(
    State(pool): State<PgPool>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    match crate::validation_store::status(&pool, &id).await {
        Ok(Some(value)) => Ok(Json(value)),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(json!({"error":"validation not found"})),
        )),
        Err(_) => Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error":"database unavailable"})),
        )),
    }
}
