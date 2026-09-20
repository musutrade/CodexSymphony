//! Atomic group review persistence; no coordinator/runtime dispatch occurs here.
use crate::{
    budget::Amount,
    draft::Document,
    group_review::{self, RepositorySnapshot, Review},
};
use axum::{Json, http::StatusCode};
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Transaction};
pub type Error = (StatusCode, Json<Value>);
pub type Result<T> = std::result::Result<T, Error>;
pub type Tx<'a> = Transaction<'a, Postgres>;
pub fn error(status: StatusCode, message: impl ToString) -> Error {
    (status, Json(json!({"error":message.to_string()})))
}
pub fn invalid(message: impl ToString) -> Error {
    error(StatusCode::UNPROCESSABLE_ENTITY, message)
}
pub fn conflict(message: impl ToString) -> Error {
    error(StatusCode::CONFLICT, message)
}
pub fn db(_: impl std::fmt::Display) -> Error {
    error(
        StatusCode::SERVICE_UNAVAILABLE,
        "group persistence unavailable; no partial authorization committed",
    )
}
pub async fn lock(pool: &PgPool) -> Result<Tx<'_>> {
    let mut tx = pool.begin().await.map_err(db)?;
    // Same repository/revocation/review lock as the existing control plane.
    sqlx::query("SELECT pg_advisory_xact_lock(13002)")
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    Ok(tx)
}
pub async fn draft(tx: &mut Tx<'_>, id: &str) -> Result<(i64, Document)> {
    let row: Option<(i64, Value)> =
        sqlx::query_as("SELECT version,document FROM imported_draft WHERE id=$1 FOR UPDATE")
            .bind(id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(db)?;
    let (version, document) = row.ok_or_else(|| error(StatusCode::NOT_FOUND, "draft not found"))?;
    Ok((version, serde_json::from_value(document).map_err(db)?))
}
pub async fn repositories(tx: &mut Tx<'_>) -> Result<Vec<RepositorySnapshot>> {
    let rows: Vec<(i64, i64, Value)> =
        sqlx::query_as("SELECT id::bigint,version,document FROM repository ORDER BY id")
            .fetch_all(&mut **tx)
            .await
            .map_err(db)?;
    rows.into_iter()
        .map(|(id, version, document)| {
            Ok(RepositorySnapshot {
                id,
                version,
                repository: serde_json::from_value(document).map_err(db)?,
            })
        })
        .collect()
}
pub async fn review(tx: &mut Tx<'_>, id: &str) -> Result<Option<(i64, Review)>> {
    let row: Option<(i64, Value)> =
        sqlx::query_as("SELECT version,document FROM group_review WHERE draft_id=$1")
            .bind(id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(db)?;
    row.map(decode_review).transpose()
}
pub async fn view(tx: &mut Tx<'_>, id: &str, revision: i64, document: &Document) -> Result<Value> {
    let mut document = document.clone();
    crate::group_edit_store::ordered(tx, id, &mut document).await?;
    let pending_edit = crate::group_edit_store::pending(tx, id).await?;
    let saved = review(tx, id).await?;
    let queue: Option<Value> = sqlx::query_scalar("SELECT jsonb_build_object('version',version,'authorization_id',authorization_id,'state',state) FROM group_queue WHERE draft_id=$1").bind(id).fetch_optional(&mut **tx).await.map_err(db)?;
    let budgets: Vec<Value> = sqlx::query_scalar("SELECT jsonb_build_object('item_id',item_id,'limits',limits,'used',used,'reserved',reserved) FROM group_budget WHERE draft_id=$1 ORDER BY item_id").bind(id).fetch_all(&mut **tx).await.map_err(db)?;
    let authorizations: Vec<Value> = sqlx::query_scalar("SELECT jsonb_build_object('id',id,'snapshot',snapshot) FROM group_authorization WHERE draft_id=$1 ORDER BY id").bind(id).fetch_all(&mut **tx).await.map_err(db)?;
    let execution = crate::group_queue_view::view(tx, id, &document)
        .await
        .map_err(db)?;
    let (version, review) = saved.map(review_json).unwrap_or((0, Value::Null));
    Ok(
        json!({"draft_id":id,"draft_revision":revision,"document":document,"version":version,"review":review,"repositories":repositories(tx).await?,"budgets":budgets,"queue":queue,"authorizations":authorizations,"scheduler_available":true,"business_complete":false,"execution":execution,"pending_edit":pending_edit}),
    )
}
pub async fn balances(tx: &mut Tx<'_>, id: &str, review: &Review, total: Amount) -> Result<()> {
    balance(tx, id, "", total).await?;
    for item in &review.items {
        balance(tx, id, &item.child_id, item.budget).await?;
    }
    Ok(())
}
async fn balance(tx: &mut Tx<'_>, id: &str, item: &str, limit: Amount) -> Result<()> {
    let row: Option<(Value, Value)> = sqlx::query_as(
        "SELECT used,reserved FROM group_budget WHERE draft_id=$1 AND item_id=$2 FOR UPDATE",
    )
    .bind(id)
    .bind(item)
    .fetch_optional(&mut **tx)
    .await
    .map_err(db)?;
    if let Some((used, reserved)) = row {
        group_review::check_balance(
            serde_json::from_value(used).map_err(db)?,
            serde_json::from_value(reserved).map_err(db)?,
            limit,
        )
        .map_err(invalid)?;
    }
    sqlx::query("INSERT INTO group_budget(draft_id,item_id,limits) VALUES($1,$2,$3) ON CONFLICT(draft_id,item_id) DO UPDATE SET limits=excluded.limits")
        .bind(id).bind(item).bind(json!(limit)).execute(&mut **tx).await.map_err(db)?;
    Ok(())
}
pub async fn invalidate(tx: &mut Tx<'_>, id: &str) -> Result<()> {
    crate::group_queue_store::require_editable(tx, id)
        .await
        .map_err(|_| {
            conflict("group inputs already bound; use queue-edit for unstarted changes")
        })?;
    sqlx::query("UPDATE group_queue SET state='needs_review',version=version+1 WHERE draft_id=$1")
        .bind(id)
        .execute(&mut **tx)
        .await
        .map_err(db)?;
    Ok(())
}

fn review_json((version, review): (i64, Review)) -> (i64, Value) {
    (version, json!(review))
}

fn decode_review((version, document): (i64, Value)) -> Result<(i64, Review)> {
    let review = serde_json::from_value(document).map_err(db)?;
    Ok((version, review))
}
