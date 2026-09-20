//! Explicit local-user queue edits and delta authorization.
use crate::{
    draft::Document,
    group_edit_store as edits,
    group_review::Review,
    group_store::{self as store, Result},
};
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::post,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgPool;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub request_id: String,
    pub version: i64,
    pub change: Change,
}
#[derive(Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Change {
    Reorder {
        order: Vec<String>,
    },
    Propose {
        document: Box<Document>,
        review: Review,
    },
    Approve {
        edit_version: i64,
    },
}
pub fn routes() -> Router<PgPool> {
    Router::new().route("/api/drafts/{id}/queue-edit", post(write))
}
async fn write(
    State(pool): State<PgPool>,
    Path(id): Path<String>,
    input: std::result::Result<Json<Request>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<Value>> {
    let input = decode(input)?;
    let mut tx = store::lock(&pool).await?;
    let identity = json!({"draft_id":id,"request":input});
    if let Some(result) = edits::replay(&mut tx, &input.request_id, &identity).await? {
        return Ok(Json(result));
    }
    prepare(&mut tx, &id, input.version).await?;
    let result = apply(&mut tx, &id, &input).await?;
    edits::audit(&mut tx, &id, &input.request_id, identity, &result).await?;
    tx.commit().await.map_err(store::db)?;
    Ok(Json(result))
}
fn decode(
    input: std::result::Result<Json<Request>, axum::extract::rejection::JsonRejection>,
) -> Result<Request> {
    let Json(input) = input.map_err(|_| store::invalid("invalid queue edit request"))?;
    crate::contract::validate_request_id(&input.request_id).map_err(store::invalid)?;
    Ok(input)
}
async fn prepare(tx: &mut store::Tx<'_>, id: &str, version: i64) -> Result<()> {
    edits::version(tx, id, version).await?;
    crate::group_queue_store::materialize_tx(tx)
        .await
        .map_err(store::db)
}
async fn apply(tx: &mut store::Tx<'_>, id: &str, input: &Request) -> Result<Value> {
    match &input.change {
        Change::Reorder { order } => reorder(tx, id, order).await,
        Change::Propose { document, review } => propose(tx, id, document, review).await,
        Change::Approve { edit_version } => {
            crate::group_edit_apply::approve(tx, id, *edit_version, &input.request_id).await
        }
    }
}
async fn reorder(tx: &mut store::Tx<'_>, id: &str, order: &[String]) -> Result<Value> {
    edits::no_pending(tx, id).await?;
    let (_, mut document) = store::draft(tx, id).await?;
    edits::ordered(tx, id, &mut document).await?;
    let next = crate::group_edit::reorder(&document, order).map_err(store::invalid)?;
    let changed: Vec<_> = document
        .children
        .iter()
        .zip(&next.children)
        .filter(|(old, new)| old.order != new.order)
        .map(|(old, _)| old.id.clone())
        .collect();
    edits::unstarted(tx, id, &changed).await?;
    edits::set_order(tx, id, &next).await?;
    Ok(edits::result(edits::bump(tx, id).await?, &[]))
}
async fn propose(
    tx: &mut store::Tx<'_>,
    id: &str,
    document: &Document,
    review: &Review,
) -> Result<Value> {
    validate_plan(document)?;
    let (before, affected) = difference(tx, id, document, review).await?;
    guard_changes(tx, id, &before, document, &affected).await?;
    let repositories = edits::repositories(tx, document).await?;
    let version = edits::bump(tx, id).await?;
    sqlx::query("INSERT INTO group_edit(draft_id,version,document,review,affected,repositories) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT(draft_id) DO UPDATE SET version=excluded.version,document=excluded.document,review=excluded.review,affected=excluded.affected,repositories=excluded.repositories")
        .bind(id).bind(version).bind(json!(document)).bind(json!(review)).bind(json!(affected)).bind(json!(repositories)).execute(&mut **tx).await.map_err(store::db)?;
    sqlx::query("UPDATE group_execution_item SET frozen=child_id=ANY($2) WHERE draft_id=$1")
        .bind(id)
        .bind(&affected)
        .execute(&mut **tx)
        .await
        .map_err(store::db)?;
    Ok(edits::result(version, &affected))
}

fn validate_plan(document: &Document) -> Result<()> {
    crate::draft::validate(document).map_err(store::invalid)?;
    crate::group_review::validate_order(document).map_err(store::invalid)?;
    Ok(())
}
async fn difference(
    tx: &mut store::Tx<'_>,
    id: &str,
    document: &Document,
    review: &Review,
) -> Result<(Document, Vec<String>)> {
    let (revision, mut before) = store::draft(tx, id).await?;
    edits::ordered(tx, id, &mut before).await?;
    let (_, old) = store::review(tx, id)
        .await?
        .ok_or(store::conflict("group review missing"))?;
    if review.parent_revision != revision + 1 {
        return Err(store::conflict(
            "change must reference the next exact draft revision",
        ));
    }
    let affected: Vec<_> = crate::group_edit::affected(&before, &old, document, review)
        .into_iter()
        .collect();
    if affected.is_empty() {
        return Err(store::invalid("no content changes; use queue reorder"));
    }
    Ok((before, affected))
}
async fn guard_changes(
    tx: &mut store::Tx<'_>,
    id: &str,
    before: &Document,
    document: &Document,
    affected: &[String],
) -> Result<()> {
    edits::unstarted(tx, id, affected).await?;
    // Reordering within a content edit cannot move a claimed item either.
    let moved: Vec<_> = before
        .children
        .iter()
        .filter(|c| {
            document
                .children
                .iter()
                .any(|n| n.id == c.id && n.order != c.order)
        })
        .map(|c| c.id.clone())
        .collect();
    edits::unstarted(tx, id, &moved).await?;
    Ok(())
}
