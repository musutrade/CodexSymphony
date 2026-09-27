//! Protected product routes serve the same redacted copy as Runtime tools.
use crate::diagnostics::{Chunk, Page};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use serde::Deserialize;
use serde_json::json;
use sqlx::PgPool;
type Error = (StatusCode, Json<serde_json::Value>);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListArgs {
    pub after: i64,
}

pub fn routes() -> Router<PgPool> {
    Router::new()
        .route("/api/requirements/{id}/diagnostics/{after}", get(list))
        .route(
            "/api/requirements/{id}/diagnostic-artifacts/{artifact}/{offset}/{limit}",
            get(read),
        )
}
fn rejected(_: impl std::fmt::Display) -> Error {
    (
        StatusCode::CONFLICT,
        Json(
            json!({"error":"diagnostic unavailable, unauthorized, expired or invalid range; refresh manifest"}),
        ),
    )
}
async fn list(
    State(pool): State<PgPool>,
    Path((id, after)): Path<(i64, i64)>,
) -> std::result::Result<Json<Page>, Error> {
    let mut tx = crate::run_store::lock(&pool).await.map_err(rejected)?;
    crate::diagnostic_store::user_allowed(&mut tx, id)
        .await
        .map_err(rejected)?;
    let page = crate::diagnostic_store::list_in(&mut tx, id, None, after)
        .await
        .map_err(rejected)?;
    tx.commit().await.map_err(rejected)?;
    Ok(Json(page))
}
async fn content(
    pool: &PgPool,
    id: i64,
    artifact: &str,
) -> crate::diagnostics::Result<(crate::diagnostics::Artifact, Vec<u8>)> {
    let mut tx = crate::run_store::lock(pool).await?;
    crate::diagnostic_store::user_allowed(&mut tx, id).await?;
    let content = crate::diagnostic_store::load_in(&mut tx, id, None, artifact).await?;
    tx.commit().await?;
    Ok(content)
}
async fn read(
    State(pool): State<PgPool>,
    Path((id, artifact, offset, limit)): Path<(i64, String, u64, usize)>,
) -> std::result::Result<Json<Chunk>, Error> {
    let (manifest, bytes) = content(&pool, id, &artifact).await.map_err(rejected)?;
    crate::diagnostics::chunk(manifest, &bytes, offset, limit)
        .map(Json)
        .map_err(rejected)
}
