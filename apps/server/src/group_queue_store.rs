//! Group authorization projects children into the existing Requirement/Run model.
use crate::{
    budget_store::{decode, require},
    draft::Document,
    group_review::{RepositorySnapshot, Review},
    run_store,
};
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Transaction};
pub(crate) type Tx<'a> = Transaction<'a, Postgres>;
pub(crate) type Result<T> = std::result::Result<T, sqlx::Error>;

pub async fn materialize(pool: &PgPool) -> Result<()> {
    let mut tx = run_store::lock(pool).await?;
    let rows: Vec<(String, i64, Value)> = sqlx::query_as("SELECT q.draft_id,q.authorization_id,a.snapshot FROM group_queue q JOIN group_authorization a ON a.id=q.authorization_id WHERE q.state='waiting_scheduler' AND NOT EXISTS(SELECT 1 FROM group_execution_item i WHERE i.draft_id=q.draft_id) ORDER BY q.created_at,q.draft_id")
        .fetch_all(&mut *tx).await?;
    for (draft, authorization, snapshot) in rows {
        project(&mut tx, &draft, authorization, snapshot).await?;
    }
    tx.commit().await
}
async fn project(tx: &mut Tx<'_>, draft: &str, authorization: i64, snapshot: Value) -> Result<()> {
    let mut document: Document = decode(snapshot["document"].clone())?;
    let review: Review = decode(snapshot["review"].clone())?;
    let repositories: Vec<RepositorySnapshot> = decode(snapshot["repositories"].clone())?;
    document.children.sort_by_key(|child| child.order);
    for child in &document.children {
        let item = review
            .items
            .iter()
            .find(|i| i.child_id == child.id)
            .ok_or_else(|| sqlx::Error::Protocol("missing authorized child".into()))?;
        let repository = repositories
            .iter()
            .find(|r| Some(r.id) == child.repository_id)
            .ok_or_else(|| sqlx::Error::Protocol("missing authorized repository".into()))?;
        let input = json!({"child":child,"review":item,"parent_revision":review.parent_revision,"repository":repository});
        let requirement = if child.kind == "code_change" {
            Some(project_requirement(tx, child, item, repository, authorization).await?)
        } else {
            None
        };
        sqlx::query("INSERT INTO group_execution_item(draft_id,child_id,authorization_id,requirement_id,input) VALUES($1,$2,$3,$4,$5)")
            .bind(draft).bind(&child.id).bind(authorization).bind(requirement).bind(input).execute(&mut **tx).await?;
    }
    Ok(())
}
async fn project_requirement(
    tx: &mut Tx<'_>,
    child: &crate::draft::Child,
    item: &crate::group_review::Item,
    repository: &RepositorySnapshot,
    authorization: i64,
) -> Result<i64> {
    let contract =
        crate::group_review::verification_contract(child, item).map_err(sqlx::Error::Protocol)?;
    let id: i64 = sqlx::query_scalar("INSERT INTO requirement(version,state,contract,repository_id,revision) VALUES(1,'Ready',$1,$2,1) RETURNING id")
        .bind(json!(contract)).bind(repository.id).fetch_one(&mut **tx).await?;
    let ac_ids: Vec<_> = child.acceptance_criteria.iter().map(|ac| &ac.id).collect();
    let snapshot = json!({"repository_id":repository.id,"revision":1,"contract":contract,"ac_ids":ac_ids,"repository_version":repository.version,"repository":repository.repository,"reviewer":"local-user","group_authorization_id":authorization,"child_revision":item.revision});
    sqlx::query("INSERT INTO requirement_revision VALUES($1,1,$2)")
        .bind(id)
        .bind(snapshot)
        .execute(&mut **tx)
        .await?;
    let mut policy = repository.repository.policy.clone();
    policy.token_limit = item.budget.tokens;
    policy.turn_limit = item.budget.turns;
    policy.model_work_seconds = item.budget.model_seconds;
    crate::budget_store::freeze(tx, id, &policy).await?;
    Ok(id)
}

/// Global ordering includes unsupported validation-only items; never skip them.
pub(crate) async fn head(tx: &mut Tx<'_>) -> Result<Option<(i64, i64, bool)>> {
    let row: Option<(Option<i64>, Option<i64>, bool)> = sqlx::query_as("SELECT requirement_id,revision,paused FROM execution_queue ORDER BY queued_at,queue_key,item_order LIMIT 1")
        .fetch_optional(&mut **tx).await?;
    Ok(match row {
        Some((Some(id), Some(revision), paused)) => Some((id, revision, paused)),
        _ => None,
    })
}
pub(crate) async fn authorized(tx: &mut Tx<'_>, id: i64) -> Result<bool> {
    let row: Option<bool> = sqlx::query_scalar("SELECT q.state='waiting_scheduler' AND q.authorization_id=i.authorization_id AND d.version=(i.input->>'parent_revision')::bigint AND g.version=a.review_version FROM group_execution_item i JOIN group_queue q USING(draft_id) JOIN imported_draft d ON d.id=i.draft_id JOIN group_review g ON g.draft_id=i.draft_id JOIN group_authorization a ON a.id=i.authorization_id WHERE i.requirement_id=$1")
        .bind(id).fetch_optional(&mut **tx).await?;
    Ok(row.unwrap_or(true))
}
pub(crate) async fn require_editable(tx: &mut Tx<'_>, draft: &str) -> Result<()> {
    let active: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM group_execution_item i JOIN requirement r ON r.id=i.requirement_id WHERE i.draft_id=$1)")
        .bind(draft).fetch_one(&mut **tx).await?;
    require(
        !active,
        "group execution inputs already bound; editing requires future group revision controls",
    )
}

pub(crate) async fn bind_baseline(
    tx: &mut Tx<'_>,
    broker: &crate::git_broker::GitBroker,
    id: i64,
    baseline: &str,
) -> Result<bool> {
    let group: Option<(i64,i64)> = sqlx::query_as("SELECT authorization_id,(input#>>'{child,repository_id}')::bigint FROM group_execution_item WHERE requirement_id=$1")
        .bind(id).fetch_optional(&mut **tx).await?;
    let Some((authorization, repository)) = group else {
        return Ok(true);
    };
    let Some(facts) = crate::group_completion::dependencies(tx, id).await? else {
        return Ok(false);
    };
    for fact in &facts {
        // Cross-repository facts bind confirmed versions/artifacts and the
        // approved verification plan; Git ancestry has no cross-repo meaning.
        if fact.repository_id == repository && !broker.contains_commit(baseline, &fact.merged_sha) {
            return Ok(false);
        }
    }
    sqlx::query("INSERT INTO group_claim_input(requirement_id,authorization_id,baseline,dependencies) VALUES($1,$2,$3,$4) ON CONFLICT DO NOTHING")
        .bind(id).bind(authorization).bind(baseline).bind(json!(facts)).execute(&mut **tx).await?;
    let matches: bool=sqlx::query_scalar("SELECT authorization_id=$2 AND baseline=$3 AND dependencies=$4 FROM group_claim_input WHERE requirement_id=$1")
        .bind(id).bind(authorization).bind(baseline).bind(json!(facts)).fetch_one(&mut **tx).await?;
    Ok(matches)
}
pub(crate) async fn claim_input_ready(
    tx: &mut Tx<'_>,
    id: i64,
    launch: &crate::execution::Launch,
) -> Result<bool> {
    let bound: bool=sqlx::query_scalar("SELECT NOT EXISTS(SELECT 1 FROM group_execution_item WHERE requirement_id=$1) OR EXISTS(SELECT 1 FROM group_execution_item i JOIN group_claim_input b ON b.requirement_id=i.requirement_id AND b.authorization_id=i.authorization_id JOIN initial_run n ON n.requirement_id=i.requirement_id WHERE i.requirement_id=$1 AND n.workspace->>'baseline'=b.baseline AND n.launch=$2)")
        .bind(id).bind(json!(launch)).fetch_one(&mut **tx).await?;
    Ok(bound && crate::group_budget::allowed(tx, id, crate::budget::Amount::default()).await?)
}
