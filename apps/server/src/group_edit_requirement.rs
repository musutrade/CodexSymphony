//! Preserve Requirement and budget identities across unstarted revisions.
use crate::{
    draft::Child,
    group_review::{Item, RepositorySnapshot},
    group_store::{self as store, Result, Tx},
};
use serde_json::{Value, json};

pub async fn project(
    tx: &mut Tx<'_>,
    draft: &str,
    child: &Child,
    item: &Item,
    repository: &RepositorySnapshot,
    authorization: i64,
) -> Result<Option<i64>> {
    let previous: Option<(Option<i64>, Value)> = sqlx::query_as(
        "SELECT requirement_id,input FROM group_execution_item WHERE draft_id=$1 AND child_id=$2",
    )
    .bind(draft)
    .bind(&child.id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(store::db)?;
    let Some((requirement, input)) = previous else {
        return create(tx, child, item, repository, authorization).await;
    };
    if input["child"]["kind"] != child.kind
        || input["child"]["repository_id"] != json!(child.repository_id)
    {
        return Err(store::invalid(
            "stable child identity cannot change kind or repository; add a reviewed child and retain the group ledger",
        ));
    }
    if requirement.is_none() && item.integration.is_some() {
        return create(tx, child, item, repository, authorization).await;
    }
    if let Some(id) = requirement {
        revise(tx, id, child, item, repository, authorization).await?;
    }
    Ok(requirement)
}
async fn create(
    tx: &mut Tx<'_>,
    child: &Child,
    item: &Item,
    repository: &RepositorySnapshot,
    authorization: i64,
) -> Result<Option<i64>> {
    if crate::group_queue_store::executable(child, item) {
        return Ok(Some(
            crate::group_queue_store::project_requirement(
                tx,
                child,
                item,
                repository,
                authorization,
            )
            .await
            .map_err(store::db)?,
        ));
    }
    Ok(None)
}
async fn revise(
    tx: &mut Tx<'_>,
    id: i64,
    child: &Child,
    item: &Item,
    repository: &RepositorySnapshot,
    authorization: i64,
) -> Result<()> {
    let contract =
        crate::group_review::verification_contract(child, item).map_err(store::invalid)?;
    let revision: i64 = sqlx::query_scalar("UPDATE requirement SET version=version+1,revision=revision+1,contract=$2 WHERE id=$1 RETURNING revision")
        .bind(id).bind(json!(contract)).fetch_one(&mut **tx).await.map_err(store::db)?;
    let ac_ids: Vec<_> = child
        .acceptance_criteria
        .iter()
        .map(|a| a.id.as_str())
        .collect();
    let snapshot = json!({"repository_id":repository.id,"revision":revision,"contract":contract,"ac_ids":ac_ids,"repository_version":repository.version,"repository":repository.repository,"reviewer":"local-user","group_authorization_id":authorization,"child_revision":item.revision});
    sqlx::query("INSERT INTO requirement_revision VALUES($1,$2,$3)")
        .bind(id)
        .bind(revision)
        .bind(snapshot)
        .execute(&mut **tx)
        .await
        .map_err(store::db)?;
    budget(tx, id, item, authorization).await
}
async fn budget(tx: &mut Tx<'_>, id: i64, item: &Item, authorization: i64) -> Result<()> {
    let balance = crate::budget_store::balance(tx, id)
        .await
        .map_err(store::db)?;
    if !balance.exposure.fits(item.budget) {
        return Err(store::invalid("budget below cumulative exposure"));
    }
    let version: i64 = sqlx::query_scalar("UPDATE requirement_budget SET limits=$2,version=version+1 WHERE requirement_id=$1 RETURNING version")
        .bind(id).bind(json!(item.budget)).fetch_one(&mut **tx).await.map_err(store::db)?;
    let delta = json!({"tokens":item.budget.tokens-balance.limits.tokens,"turns":item.budget.turns-balance.limits.turns,"model_seconds":item.budget.model_seconds-balance.limits.model_seconds});
    sqlx::query("INSERT INTO budget_authorization(requirement_id,version,request_id,actor,reason,delta,limits) VALUES($1,$2,$3,'local-user','reviewed group delta',$4,$5)")
        .bind(id).bind(version).bind(format!("group-edit:{authorization}:{id}")).bind(delta).bind(json!(item.budget)).execute(&mut **tx).await.map_err(store::db)?;
    Ok(())
}
