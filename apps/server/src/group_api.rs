//! Explicit local-user group review. One confirmation, one durable group queue entry.
use crate::{
    contract,
    group_review::{self, Review},
    group_store::{self as store, Result},
};
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgPool;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Write {
    pub version: i64,
    pub draft_revision: i64,
    pub review: Review,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Confirm {
    pub request_id: String,
    pub version: i64,
    pub draft_revision: i64,
}
pub fn routes() -> Router<PgPool> {
    Router::new()
        .route("/api/drafts/{id}/review", get(read).put(save))
        .route("/api/drafts/{id}/authorize", post(confirm))
}
fn decode<T>(
    input: std::result::Result<Json<T>, axum::extract::rejection::JsonRejection>,
) -> Result<T> {
    match input {
        Ok(Json(value)) => Ok(value),
        Err(_) => Err(store::invalid("invalid group review request")),
    }
}
async fn read(State(pool): State<PgPool>, Path(id): Path<String>) -> Result<Json<Value>> {
    let mut tx = store::lock(&pool).await?;
    let (revision, document) = store::draft(&mut tx, &id).await?;
    let result = store::view(&mut tx, &id, revision, &document).await?;
    tx.commit().await.map_err(store::db)?;
    Ok(Json(result))
}
async fn save(
    State(pool): State<PgPool>,
    Path(id): Path<String>,
    input: std::result::Result<Json<Write>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<Value>> {
    let input = decode(input)?;
    let mut tx = store::lock(&pool).await?;
    let (revision, document) = store::draft(&mut tx, &id).await?;
    write_review(&mut tx, &id, revision, &input).await?;
    let result = store::view(&mut tx, &id, revision, &document).await?;
    tx.commit().await.map_err(store::db)?;
    Ok(Json(result))
}
async fn write_review(
    tx: &mut store::Tx<'_>,
    id: &str,
    revision: i64,
    input: &Write,
) -> Result<()> {
    let version = store::review(tx, id).await?.map_or(0, review_version);
    check_versions(revision, input.draft_revision, version, input.version)?;
    if input.review.parent_revision != revision {
        return Err(store::conflict("review references stale parent revision"));
    }
    let next = version
        .checked_add(1)
        .ok_or(store::invalid("review version exhausted"))?;
    let review = json!(input.review);
    sqlx::query("INSERT INTO group_review(draft_id,version,draft_revision,document) VALUES($1,$2,$3,$4) ON CONFLICT(draft_id) DO UPDATE SET version=excluded.version,draft_revision=excluded.draft_revision,document=excluded.document")
        .bind(id).bind(next).bind(revision).bind(&review).execute(&mut **tx).await.map_err(store::db)?;
    sqlx::query("INSERT INTO group_review_revision(draft_id,version,draft_revision,document) VALUES($1,$2,$3,$4)")
        .bind(id).bind(next).bind(revision).bind(review).execute(&mut **tx).await.map_err(store::db)?;
    store::invalidate(tx, id).await?;
    Ok(())
}
fn check_versions(
    actual_draft: i64,
    expected_draft: i64,
    actual_review: i64,
    expected_review: i64,
) -> Result<()> {
    if (actual_draft, actual_review) != (expected_draft, expected_review) {
        return Err(store::conflict(
            "draft/review version conflict; reload and reconcile changes",
        ));
    }
    Ok(())
}
async fn confirm(
    State(pool): State<PgPool>,
    Path(id): Path<String>,
    input: std::result::Result<Json<Confirm>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<Value>> {
    let input = decode(input)?;
    contract::validate_request_id(&input.request_id).map_err(store::invalid)?;
    let mut tx = store::lock(&pool).await?;
    let identity = json!({"draft_id":id,"confirmation":input});
    if let Some(result) = replay(&mut tx, &input.request_id, &identity).await? {
        return Ok(Json(result));
    }
    let authorization = authorize(&mut tx, &id, &input, identity).await?;
    tx.commit().await.map_err(store::db)?;
    Ok(Json(confirmation(authorization)))
}
async fn authorize(
    tx: &mut store::Tx<'_>,
    id: &str,
    input: &Confirm,
    identity: Value,
) -> Result<i64> {
    let (revision, document) = store::draft(tx, id).await?;
    let (version, review) = store::review(tx, id)
        .await?
        .ok_or_else(|| store::conflict("save group review first"))?;
    check_versions(revision, input.draft_revision, version, input.version)?;
    ensure_unapproved(tx, id, version).await?;
    let snapshot = prepare_snapshot(tx, id, revision, version, &document, &review).await?;
    persist_authorization(tx, id, input, identity, snapshot).await
}
async fn persist_authorization(
    tx: &mut store::Tx<'_>,
    id: &str,
    input: &Confirm,
    identity: Value,
    snapshot: Value,
) -> Result<i64> {
    let authorization:i64 = sqlx::query_scalar("INSERT INTO group_authorization(draft_id,review_version,request_id,input,snapshot) VALUES($1,$2,$3,$4,$5) RETURNING id")
        .bind(id).bind(input.version).bind(&input.request_id).bind(identity).bind(snapshot).fetch_one(&mut **tx).await.map_err(store::db)?;
    sqlx::query("INSERT INTO group_queue(draft_id,authorization_id,state) VALUES($1,$2,'waiting_scheduler') ON CONFLICT(draft_id) DO UPDATE SET authorization_id=excluded.authorization_id,state=excluded.state,version=group_queue.version+1")
        .bind(id).bind(authorization).execute(&mut **tx).await.map_err(store::db)?;
    Ok(authorization)
}
async fn ensure_unapproved(tx: &mut store::Tx<'_>, id: &str, version: i64) -> Result<()> {
    let previous: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM group_authorization WHERE draft_id=$1 AND review_version=$2)",
    )
    .bind(id)
    .bind(version)
    .fetch_one(&mut **tx)
    .await
    .map_err(store::db)?;
    if previous {
        return Err(store::conflict(
            "review already authorized; reload existing authorization",
        ));
    }
    Ok(())
}
async fn prepare_snapshot(
    tx: &mut store::Tx<'_>,
    id: &str,
    revision: i64,
    version: i64,
    document: &crate::draft::Document,
    review: &Review,
) -> Result<Value> {
    let repositories = store::repositories(tx).await?;
    let total = group_review::validate(document, revision, review, &repositories)
        .map_err(store::invalid)?;
    store::balances(tx, id, review, total).await?;
    let used_repositories: Vec<_> = repositories
        .into_iter()
        .filter(|r| {
            document
                .children
                .iter()
                .any(|c| c.repository_id == Some(r.id))
        })
        .collect();
    let snapshot = json!({"parent_revision":revision,"document":document,"review_version":version,"review":review,"repositories":used_repositories,"group_budget":total,"reviewer":"local-user","scheduler_available":false,"business_complete":false});
    Ok(snapshot)
}
fn confirmation(id: i64) -> Value {
    json!({"authorization_id":id,"state":"waiting_scheduler","scheduler_available":false,"business_complete":false})
}
async fn replay(tx: &mut store::Tx<'_>, key: &str, input: &Value) -> Result<Option<Value>> {
    let row: Option<(i64, Value)> =
        sqlx::query_as("SELECT id,input FROM group_authorization WHERE request_id=$1")
            .bind(key)
            .fetch_optional(&mut **tx)
            .await
            .map_err(store::db)?;
    match row {
        Some((id, previous)) if previous == *input => Ok(Some(confirmation(id))),
        Some(_) => Err(store::conflict(
            "request_id already bound to different group confirmation",
        )),
        None => Ok(None),
    }
}

fn review_version((version, _): (i64, Review)) -> i64 {
    version
}
