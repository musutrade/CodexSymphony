//! Read-only progress: PR publication never means parent completion.
use crate::{
    draft::Document,
    group_queue_store::{Result, Tx},
};
use serde_json::{Value, json};
pub async fn view(tx: &mut Tx<'_>, draft: &str, document: &Document) -> Result<Value> {
    let owner: Option<i64> =
        sqlx::query_scalar("SELECT requirement_id FROM execution_control WHERE id=1")
            .fetch_one(&mut **tx)
            .await?;
    let paused: bool = sqlx::query_scalar("SELECT paused FROM execution_control WHERE id=1")
        .fetch_one(&mut **tx)
        .await?;
    let mut children = document.children.iter().collect::<Vec<_>>();
    children.sort_by_key(|child| crate::group_queue_store::child_order(child));
    let mut items = Vec::new();
    let mut completed = 0;
    for child in children {
        let saved:Option<Value>=sqlx::query_scalar("SELECT jsonb_build_object('frozen',i.frozen,'requirement_id',i.requirement_id,'state',r.state,'paused',COALESCE(r.paused,false),'cancelled',COALESCE(r.cancel_requested,false),'complete',NOT COALESCE(r.cancel_requested,false) AND EXISTS(SELECT 1 FROM group_completion c WHERE c.requirement_id=i.requirement_id AND c.authorization_id=i.authorization_id)) FROM group_execution_item i LEFT JOIN requirement r ON r.id=i.requirement_id WHERE i.draft_id=$1 AND i.child_id=$2")
            .bind(draft).bind(&child.id).fetch_optional(&mut **tx).await?;
        let saved = saved.unwrap_or(Value::Null);
        let requirement = saved["requirement_id"].as_i64();
        let reason = reason(tx, child, &saved, paused).await?;
        completed += usize::from(saved["complete"] == true);
        items.push(json!({"child_id":child.id,"kind":child.kind,"order":child.order,"depends_on":child.depends_on,"repository_id":child.repository_id,"requirement_id":requirement,"state":saved["state"].as_str().unwrap_or("Queued"),"owner":requirement.is_some() && owner==requirement,"complete":saved["complete"]==true,"waiting_reason":reason}));
    }
    Ok(
        json!({"owner":owner,"paused":paused,"completed":completed,"total":document.children.len(),"parent_state":"waiting_business_acceptance","items":items}),
    )
}
async fn reason(
    tx: &mut Tx<'_>,
    child: &crate::draft::Child,
    saved: &Value,
    paused: bool,
) -> Result<&'static str> {
    if saved["frozen"] == true {
        return Ok("needs_review");
    }
    if saved["complete"] == true {
        return Ok("confirmed_merge_and_acceptance");
    }
    if child.kind == "validation_only" {
        return Ok("waiting_validation_only_execution_not_implemented");
    }
    if paused || saved["paused"] == true {
        return Ok("paused");
    }
    if saved["cancelled"] == true {
        return Ok("cancelled_not_success");
    }
    let Some(id) = saved["requirement_id"].as_i64() else {
        return Ok("waiting_authorization_or_scheduler");
    };
    eligible_reason(tx, id, saved["state"].as_str().unwrap_or("")).await
}
async fn eligible_reason(tx: &mut Tx<'_>, id: i64, state: &str) -> Result<&'static str> {
    if !crate::group_queue_store::authorized(tx, id).await? {
        return Ok("needs_review");
    }
    if state == "Submitted" {
        return Ok("waiting_confirmed_merge_and_applicable_acceptance");
    }
    if state != "Ready" {
        return Ok("occupied_execution_or_blocker");
    }
    ready_reason(tx, id).await
}
async fn ready_reason(tx: &mut Tx<'_>, id: i64) -> Result<&'static str> {
    if crate::group_completion::dependencies(tx, id)
        .await?
        .is_none()
    {
        return Ok("waiting_dependency_completion");
    }
    if !crate::github_store::claim_ready(tx, id, 1).await? {
        return Ok("repository_unavailable");
    }
    if !crate::group_queue_store::head(tx)
        .await?
        .is_some_and(|r| r.0 == id)
    {
        return Ok("waiting_queue_order");
    }
    Ok("waiting_repository_baseline_or_preparation")
}
