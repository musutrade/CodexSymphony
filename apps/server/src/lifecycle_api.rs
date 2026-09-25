//! Authenticated lifecycle history and bounded operator notification replay.
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgPool;
type Result<T> = std::result::Result<T, (StatusCode, Json<Value>)>;
#[derive(Deserialize)]
pub struct Page {
    pub after: Option<i64>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Replay {
    pub event_id: i64,
    pub plugin_id: String,
}

pub fn routes() -> Router<PgPool> {
    Router::new()
        .route("/api/requirements/{id}/lifecycle", get(history))
        .route("/api/requirements/{id}/notifications/replay", post(replay))
}
pub async fn events(pool: &PgPool, id: i64, after: i64) -> std::result::Result<Value, sqlx::Error> {
    let events: Vec<Value> = sqlx::query_scalar("SELECT jsonb_build_object('event_id',e.id,'revision',e.revision,'sequence',e.sequence,'phase',e.phase,'source_id',e.source_id,'facts',e.facts,'occurred_at',e.occurred_at,'deliveries',(SELECT COALESCE(jsonb_agg(jsonb_build_object('plugin_id',d.plugin_id,'state',d.state,'attempts',d.attempts,'last_result',d.last_result)),'[]') FROM notification_delivery d WHERE d.event_id=e.id)) FROM lifecycle_event e WHERE e.requirement_id=$1 AND e.sequence>$2 ORDER BY e.sequence LIMIT 100")
        .bind(id).bind(after).fetch_all(pool).await?;
    Ok(json!({"events":events}))
}
async fn history(
    State(pool): State<PgPool>,
    Path(id): Path<i64>,
    Query(page): Query<Page>,
) -> Result<Json<Value>> {
    events(&pool, id, page.after.unwrap_or(0))
        .await
        .map(Json)
        .map_err(error)
}
async fn replay(
    State(pool): State<PgPool>,
    Path(id): Path<i64>,
    Json(input): Json<Replay>,
) -> Result<Json<Value>> {
    let changed = sqlx::query("UPDATE notification_delivery d SET state='pending',attempt_limit=6,next_attempt_at=clock_timestamp(),deadline=clock_timestamp()+interval '10 minutes',last_result='operator_replay' FROM lifecycle_event e,notification_plugin p WHERE e.id=d.event_id AND e.requirement_id=$1 AND d.event_id=$2 AND d.plugin_id=$3 AND p.id=d.plugin_id AND p.enabled AND d.state='failed' AND d.attempt_limit=3")
        .bind(id).bind(input.event_id).bind(&input.plugin_id).execute(&pool).await.map_err(error)?;
    Ok(Json(
        json!({"accepted":changed.rows_affected()==1,"started":false}),
    ))
}
fn error(_: sqlx::Error) -> (StatusCode, Json<Value>) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({"error":"lifecycle storage unavailable"})),
    )
}
