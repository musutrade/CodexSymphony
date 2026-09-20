//! Only this explicit POST admits advisory model work.
use crate::{
    generation::Request,
    generation_runtime::{self, Config},
    generation_store::{self as store, Result},
};
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};
use serde_json::Value;
use sqlx::PgPool;
pub fn routes() -> Router<PgPool> {
    Router::new()
        .route("/api/draft-generations", get(list).post(create))
        .route("/api/draft-generations/{id}", get(read))
}
async fn list(State(pool): State<PgPool>) -> Result<Json<Value>> {
    store::list(&pool).await.map(Json)
}
async fn read(State(pool): State<PgPool>, Path(id): Path<String>) -> Result<Json<Value>> {
    store::read(&pool, &id).await.map(Json)
}
async fn create(
    State(pool): State<PgPool>,
    input: std::result::Result<Json<Request>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<Value>> {
    let Json(request) = input.map_err(|e| store::error(422, e.body_text()))?;
    request.validate().map_err(|e| store::error(422, e))?;
    if let Some(record) = store::replay(&pool, &request).await? {
        return Ok(Json(record));
    }
    let config = Config::load()
        .map_err(|_| store::error(503, "operator generation configuration unavailable"))?
        .ok_or_else(|| {
            store::error(
                503,
                "draft generation is not configured; import and edit remain available",
            )
        })?;
    let (record, start) = store::admit(&pool, &request).await?;
    if start {
        tokio::spawn(generation_runtime::run(pool, config, request));
    }
    Ok(Json(record))
}
