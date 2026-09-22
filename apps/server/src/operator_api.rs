//! Protected localhost operator read and command endpoints.
use crate::{operator_control, operator_view};
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
        .route(
            "/api/requirements/{id}/operations",
            get(detail).post(control),
        )
        .route(
            "/api/requirements/{id}/evidence/{run}/{channel}",
            get(evidence),
        )
        .route("/api/inbox", get(inbox))
        .route("/api/operator/questions/{id}/answer", post(answer))
}
fn error(error: sqlx::Error) -> (StatusCode, Json<Value>) {
    match error {
        sqlx::Error::RowNotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({"error":"object not found"})),
        ),
        sqlx::Error::Protocol(_) => (
            StatusCode::CONFLICT,
            Json(json!({"error":"state changed; refresh before acting"})),
        ),
        _ => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error":"storage unavailable"})),
        ),
    }
}
async fn detail(State(pool): State<PgPool>, Path(id): Path<i64>) -> Result<Json<Value>> {
    operator_view::detail(&pool, id)
        .await
        .map(Json)
        .map_err(error)
}
async fn control(
    State(pool): State<PgPool>,
    Path(id): Path<i64>,
    input: std::result::Result<
        Json<operator_control::Command>,
        axum::extract::rejection::JsonRejection,
    >,
) -> Result<Json<Value>> {
    let Json(command) = input.map_err(|_| {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error":"invalid control request"})),
        )
    })?;
    operator_control::execute(&pool, id, &command)
        .await
        .map(Json)
        .map_err(error)
}
async fn evidence(
    State(pool): State<PgPool>,
    Path((id, run, channel)): Path<(i64, String, String)>,
) -> Result<Json<Value>> {
    operator_view::evidence(&pool, id, &run, &channel)
        .await
        .map(Json)
        .map_err(error)
}
async fn inbox(State(pool): State<PgPool>) -> Result<Json<Value>> {
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT r.id FROM requirement r WHERE
         (r.cancel_requested AND NOT r.cleanup_complete) OR
         EXISTS(SELECT 1 FROM merge_operation m WHERE m.requirement_id=r.id AND m.state='blocked') OR
         (EXISTS(SELECT 1 FROM storage_attempt s WHERE s.requirement_id=r.id) AND
          EXISTS(SELECT 1 FROM storage_guard g WHERE g.blocked OR (g.scan_retry->>'todo')::boolean OR (g.measured->>'classification_todo')::bigint>0)) OR
         EXISTS(SELECT 1 FROM storage_material m JOIN storage_attempt a ON a.run_id=m.run_id
           WHERE a.requirement_id=r.id AND ((m.retry->>'todo')::boolean OR m.protection IN ('partial archive; reconcile','unknown identity','unknown preparation identity','partial/pending requires reconciliation'))) OR
         EXISTS(SELECT 1 FROM runtime_evidence e JOIN agent_run a ON a.id=e.run_id
           WHERE a.requirement_id=r.id AND (e.cleanup_retry->>'todo')::boolean) OR
         (NOT r.cancel_requested AND (
           r.paused OR r.state='Failed' OR
           EXISTS(SELECT 1 FROM recovery_failure f WHERE f.requirement_id=r.id AND f.decision='blocked') OR
           EXISTS(SELECT 1 FROM runtime_question q WHERE q.requirement_id=r.id
             AND q.revision=r.revision AND q.resume_state IN ('waiting','pending')) OR
           (SELECT a.blocker IS NOT NULL FROM agent_run a WHERE a.requirement_id=r.id
             AND a.revision=r.revision ORDER BY a.created_at DESC,a.id DESC LIMIT 1) OR
           EXISTS(SELECT 1 FROM preparation_record p WHERE p.requirement_id=r.id
             AND p.revision=r.revision AND NOT p.ready AND (p.retry->>'todo')::boolean) OR
           EXISTS(SELECT 1 FROM delivery d JOIN delivery_action a USING(action_key)
             WHERE d.requirement_id=r.id AND NOT d.released AND a.state='blocked') OR
           EXISTS(SELECT 1 FROM candidate_validation v WHERE v.requirement_id=r.id
             AND v.revision=r.revision AND v.result='blocked' AND v.stage<>'done')
         )) ORDER BY r.id",
    )
    .fetch_all(&pool)
    .await
    .map_err(error)?;
    Ok(Json(json!({"requirement_ids":ids})))
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Answer {
    version: i64,
    answers: Vec<AnswerItem>,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct AnswerItem {
    id: String,
    text: String,
}
async fn answer(
    State(pool): State<PgPool>,
    Path(id): Path<String>,
    input: std::result::Result<Json<Answer>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<Value>> {
    let Json(input) = input.map_err(|_| {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error":"invalid answer"})),
        )
    })?;
    let count = input.answers.len();
    let answers: serde_json::Map<String, Value> =
        input.answers.into_iter().map(answer_item).collect();
    if count != answers.len() {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error":"duplicate answer identity"})),
        ));
    }
    let command = crate::runtime_questions::Answer {
        version: input.version,
        answer: json!({"answers":answers}),
    };
    crate::runtime_questions::answer(&pool, &id, &command, crate::runtime_client::now())
        .await
        .map_err(error)?;
    Ok(Json(json!({"saved":true})))
}

fn answer_item(item: AnswerItem) -> (String, Value) {
    (item.id, json!({"answers":[item.text]}))
}
