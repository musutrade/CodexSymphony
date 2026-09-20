//! Delta authorization commits the reviewed revision and queue atomically.
use crate::{
    draft::{Document, Source},
    group_edit_store as edits,
    group_review::{RepositorySnapshot, Review},
    group_store::{self as store, Result, Tx},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[derive(serde::Deserialize)]
struct Pending {
    version: i64,
    document: Document,
    review: Review,
    affected: Vec<String>,
    repositories: Vec<RepositorySnapshot>,
}
pub async fn approve(tx: &mut Tx<'_>, id: &str, expected: i64, request: &str) -> Result<Value> {
    let pending = load(tx, id, expected).await?;
    let total = validate(tx, id, &pending).await?;
    persist_draft(tx, id, &pending.document, pending.review.parent_revision).await?;
    let version = persist_review(tx, id, &pending.review).await?;
    let authorization = authorize(tx, id, request, version, total, &pending).await?;
    apply_items(
        tx,
        id,
        authorization,
        &pending.document,
        &pending.review,
        &pending.repositories,
        &pending.affected,
    )
    .await?;
    let queue_version = finish(
        tx,
        id,
        authorization,
        pending.review.parent_revision,
        version,
    )
    .await?;
    Ok(edits::result(queue_version, &pending.affected))
}
async fn load(tx: &mut Tx<'_>, id: &str, expected: i64) -> Result<Pending> {
    let value = edits::pending(tx, id)
        .await?
        .ok_or(store::conflict("no pending queue changes"))?;
    let pending: Pending = serde_json::from_value(value).map_err(store::db)?;
    if pending.version != expected {
        return Err(store::conflict(
            "edit revision conflict; reload difference review",
        ));
    }
    edits::unstarted(tx, id, &pending.affected).await?;
    Ok(pending)
}
async fn validate(tx: &mut Tx<'_>, id: &str, pending: &Pending) -> Result<crate::budget::Amount> {
    let repositories = edits::repositories(tx, &pending.document).await?;
    if json!(repositories) != json!(pending.repositories) {
        return Err(store::conflict(
            "repository policy changed after difference review; propose again",
        ));
    }
    let total = crate::group_review::validate(
        &pending.document,
        pending.review.parent_revision,
        &pending.review,
        &repositories,
    )
    .map_err(store::invalid)?;
    store::balances(tx, id, &pending.review, total).await?;
    Ok(total)
}
async fn authorize(
    tx: &mut Tx<'_>,
    id: &str,
    request: &str,
    version: i64,
    total: crate::budget::Amount,
    pending: &Pending,
) -> Result<i64> {
    let snapshot = json!({"document":pending.document,"review":pending.review,"repositories":pending.repositories,"parent_revision":pending.review.parent_revision,"review_version":version,"group_budget":total,"reviewer":"local-user","affected":pending.affected,"scheduler_available":true,"business_complete":false});
    sqlx::query_scalar("INSERT INTO group_authorization(draft_id,review_version,request_id,input,snapshot) VALUES($1,$2,$3,$4,$5) RETURNING id")
        .bind(id).bind(version).bind(request).bind(json!({"edit_version":pending.version})).bind(snapshot).fetch_one(&mut **tx).await.map_err(store::db)
}
async fn finish(
    tx: &mut Tx<'_>,
    id: &str,
    authorization: i64,
    revision: i64,
    version: i64,
) -> Result<i64> {
    sqlx::query("UPDATE group_execution_item SET authorized_draft_revision=$2,authorized_review_version=$3 WHERE draft_id=$1")
        .bind(id).bind(revision).bind(version).execute(&mut **tx).await.map_err(store::db)?;
    sqlx::query(
        "UPDATE group_queue SET authorization_id=$2,state='waiting_scheduler' WHERE draft_id=$1",
    )
    .bind(id)
    .bind(authorization)
    .execute(&mut **tx)
    .await
    .map_err(store::db)?;
    sqlx::query("DELETE FROM group_edit WHERE draft_id=$1")
        .bind(id)
        .execute(&mut **tx)
        .await
        .map_err(store::db)?;
    edits::bump(tx, id).await
}
async fn persist_draft(
    tx: &mut Tx<'_>,
    id: &str,
    document: &Document,
    revision: i64,
) -> Result<()> {
    let source = Source {
        format: "json".into(),
        label: "local-user reviewed queue revision".into(),
        text: json!(document).to_string(),
    };
    let hash = format!("{:x}", Sha256::digest(source.text.as_bytes()));
    sqlx::query("INSERT INTO imported_draft_revision(draft_id,version,document,source,source_sha256) VALUES($1,$2,$3,$4,$5)")
        .bind(id).bind(revision).bind(json!(document)).bind(json!(source)).bind(&hash).execute(&mut **tx).await.map_err(store::db)?;
    sqlx::query("UPDATE imported_draft SET version=$2,document=$3,source=$4,source_sha256=$5,updated_at=now() WHERE id=$1")
        .bind(id).bind(revision).bind(json!(document)).bind(json!(source)).bind(hash).execute(&mut **tx).await.map_err(store::db)?;
    Ok(())
}
async fn persist_review(tx: &mut Tx<'_>, id: &str, review: &Review) -> Result<i64> {
    let version: i64 = sqlx::query_scalar("UPDATE group_review SET version=version+1,draft_revision=$2,document=$3 WHERE draft_id=$1 RETURNING version")
        .bind(id).bind(review.parent_revision).bind(json!(review)).fetch_one(&mut **tx).await.map_err(store::db)?;
    sqlx::query("INSERT INTO group_review_revision(draft_id,version,draft_revision,document) VALUES($1,$2,$3,$4)")
        .bind(id).bind(version).bind(review.parent_revision).bind(json!(review)).execute(&mut **tx).await.map_err(store::db)?;
    Ok(version)
}
async fn apply_items(
    tx: &mut Tx<'_>,
    id: &str,
    authorization: i64,
    document: &Document,
    review: &Review,
    repositories: &[RepositorySnapshot],
    affected: &[String],
) -> Result<()> {
    for name in affected {
        let Some(child) = document.children.iter().find(|c| &c.id == name) else {
            sqlx::query("UPDATE group_execution_item SET removed=true,frozen=true WHERE draft_id=$1 AND child_id=$2")
                .bind(id).bind(name).execute(&mut **tx).await.map_err(store::db)?;
            continue;
        };
        let item = review
            .items
            .iter()
            .find(|i| &i.child_id == name)
            .ok_or(store::invalid("review item missing"))?;
        let repository = repositories
            .iter()
            .find(|r| Some(r.id) == child.repository_id)
            .ok_or(store::invalid("repository missing"))?;
        let requirement =
            crate::group_edit_requirement::project(tx, id, child, item, repository, authorization)
                .await?;
        let input = json!({"child":child,"review":item,"parent_revision":review.parent_revision,"repository":repository});
        sqlx::query("INSERT INTO group_execution_item(draft_id,child_id,authorization_id,requirement_id,input,queue_order) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT(draft_id,child_id) DO UPDATE SET authorization_id=excluded.authorization_id,input=excluded.input,queue_order=excluded.queue_order,frozen=false,removed=false")
            .bind(id).bind(name).bind(authorization).bind(requirement).bind(input).bind(i64::from(child.order)).execute(&mut **tx).await.map_err(store::db)?;
    }
    edits::set_order(tx, id, document).await
}
