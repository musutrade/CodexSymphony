//! Versioned queue operations use the scheduler's transaction lock.
use crate::{
    draft::Document,
    group_store::{self as store, Result, Tx},
};
use serde_json::{Value, json};

pub async fn replay(tx: &mut Tx<'_>, key: &str, identity: &Value) -> Result<Option<Value>> {
    let row: Option<(Value, Value)> =
        sqlx::query_as("SELECT input,result FROM group_queue_event WHERE request_id=$1")
            .bind(key)
            .fetch_optional(&mut **tx)
            .await
            .map_err(store::db)?;
    match row {
        Some((input, result)) if input == *identity => Ok(Some(result)),
        Some(_) => Err(store::conflict(
            "request_id already used for a different queue operation",
        )),
        None => Ok(None),
    }
}
pub async fn audit(
    tx: &mut Tx<'_>,
    id: &str,
    key: &str,
    input: Value,
    result: &Value,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO group_queue_event(request_id,draft_id,input,result) VALUES($1,$2,$3,$4)",
    )
    .bind(key)
    .bind(id)
    .bind(input)
    .bind(result)
    .execute(&mut **tx)
    .await
    .map_err(store::db)?;
    Ok(())
}
pub async fn version(tx: &mut Tx<'_>, id: &str, expected: i64) -> Result<()> {
    let actual: Option<i64> =
        sqlx::query_scalar("SELECT version FROM group_queue WHERE draft_id=$1 AND state='waiting_scheduler' FOR UPDATE")
            .bind(id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(store::db)?;
    if actual != Some(expected) {
        return Err(store::conflict(
            "queue version conflict; reload and reconcile changes",
        ));
    }
    Ok(())
}
pub async fn bump(tx: &mut Tx<'_>, id: &str) -> Result<i64> {
    sqlx::query_scalar(
        "UPDATE group_queue SET version=version+1 WHERE draft_id=$1 RETURNING version",
    )
    .bind(id)
    .fetch_one(&mut **tx)
    .await
    .map_err(store::db)
}
pub async fn ordered(tx: &mut Tx<'_>, id: &str, document: &mut Document) -> Result<()> {
    let rows: Vec<(String,i64)> = sqlx::query_as("SELECT child_id,COALESCE(queue_order,(input#>>'{child,order}')::bigint) FROM group_execution_item WHERE draft_id=$1 AND NOT removed")
        .bind(id).fetch_all(&mut **tx).await.map_err(store::db)?;
    for (id, order) in rows {
        if let Some(child) = document.children.iter_mut().find(|c| c.id == id) {
            child.order = order as u32;
        }
    }
    Ok(())
}
pub async fn unstarted(tx: &mut Tx<'_>, id: &str, children: &[String]) -> Result<()> {
    let bound: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM group_execution_item i LEFT JOIN requirement r ON r.id=i.requirement_id WHERE i.draft_id=$1 AND i.child_id=ANY($2) AND (r.state<>'Ready' OR r.cancel_requested OR EXISTS(SELECT 1 FROM initial_run n WHERE n.requirement_id=i.requirement_id) OR EXISTS(SELECT 1 FROM agent_run a WHERE a.requirement_id=i.requirement_id) OR EXISTS(SELECT 1 FROM group_completion c WHERE c.requirement_id=i.requirement_id)))")
        .bind(id).bind(children).fetch_one(&mut **tx).await.map_err(store::db)?;
    if bound {
        return Err(store::conflict(
            "item already claimed or terminal; stop and preserve through existing controls, then review a linked revision; Run input cannot be edited",
        ));
    }
    Ok(())
}
pub async fn no_pending(tx: &mut Tx<'_>, id: &str) -> Result<()> {
    let pending: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM group_edit WHERE draft_id=$1)")
            .bind(id)
            .fetch_one(&mut **tx)
            .await
            .map_err(store::db)?;
    if pending {
        return Err(store::conflict(
            "pending changes require review before reordering",
        ));
    }
    Ok(())
}
pub async fn set_order(tx: &mut Tx<'_>, id: &str, document: &Document) -> Result<()> {
    for child in &document.children {
        sqlx::query(
            "UPDATE group_execution_item SET queue_order=$3 WHERE draft_id=$1 AND child_id=$2",
        )
        .bind(id)
        .bind(&child.id)
        .bind(i64::from(child.order))
        .execute(&mut **tx)
        .await
        .map_err(store::db)?;
    }
    Ok(())
}
pub async fn pending(tx: &mut Tx<'_>, id: &str) -> Result<Option<Value>> {
    sqlx::query_scalar("SELECT jsonb_build_object('version',version,'document',document,'review',review,'affected',affected,'repositories',repositories) FROM group_edit WHERE draft_id=$1")
        .bind(id).fetch_optional(&mut **tx).await.map_err(store::db)
}
pub fn result(version: i64, affected: &[String]) -> Value {
    json!({"version":version,"affected":affected})
}

pub async fn repositories(
    tx: &mut Tx<'_>,
    document: &Document,
    review: &crate::group_review::Review,
) -> Result<Vec<crate::group_review::RepositorySnapshot>> {
    let repositories = store::repositories(tx).await?;
    Ok(repositories
        .into_iter()
        .filter(|r| {
            document
                .children
                .iter()
                .any(|c| c.repository_id == Some(r.id))
                || review
                    .items
                    .iter()
                    .filter_map(|i| i.integration.as_ref())
                    .any(|a| a.repositories.iter().any(|p| p.repository_id == r.id))
        })
        .collect())
}
