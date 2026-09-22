//! Completion protocol used by the trusted merged-checkout acceptance adapter.
//! Protocol fixtures remain distinct from real M3 business acceptance.
use crate::{
    budget_store::{decode, require},
    group_queue_store::{Result, Tx},
    run_store,
};
use serde_json::{Value, json};
use sqlx::PgPool;

pub use crate::group_dependency::{Fact, Verifier, validate};
pub async fn record(pool: &PgPool, verifier: &impl Verifier, fact: &Fact) -> Result<()> {
    let mut tx = run_store::lock(pool).await?;
    record_in(&mut tx, verifier, fact).await?;
    tx.commit().await
}
pub(crate) async fn record_in(
    tx: &mut Tx<'_>,
    verifier: &impl Verifier,
    fact: &Fact,
) -> Result<()> {
    require(
        validate(fact) && verifier.verify(fact),
        "untrusted or inapplicable completion evidence",
    )?;
    bind(tx, fact).await?;
    sqlx::query("INSERT INTO group_completion(requirement_id,authorization_id,fact) VALUES($1,$2,$3) ON CONFLICT DO NOTHING")
        .bind(fact.requirement_id).bind(fact.authorization_id).bind(json!(fact)).execute(&mut **tx).await?;
    let saved: Value =
        sqlx::query_scalar("SELECT fact FROM group_completion WHERE requirement_id=$1")
            .bind(fact.requirement_id)
            .fetch_one(&mut **tx)
            .await?;
    require(saved == json!(fact), "completion identity conflict")?;
    Ok(())
}
async fn bind(tx: &mut Tx<'_>, fact: &Fact) -> Result<()> {
    let input: Value=sqlx::query_scalar("SELECT i.input FROM group_execution_item i JOIN requirement r ON r.id=i.requirement_id WHERE i.requirement_id=$1 AND i.authorization_id=$2 AND NOT r.cancel_requested AND r.state='Submitted'")
        .bind(fact.requirement_id).bind(fact.authorization_id).fetch_one(&mut **tx).await?;
    require(
        input["child"]["repository_id"] == fact.repository_id
            && input["review"]["revision"] == fact.child_revision
            && input["repository"]["repository"]["github_repository_id"]
                == fact.github_repository_id
            && input["review"]["verification"] == fact.acceptance_plan,
        "completion does not match authorized version and plan",
    )?;
    let merged: bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM delivery d JOIN github_pr p ON p.repository_id=d.repository_id AND p.number=d.pr_number AND p.requirement_id=d.requirement_id WHERE d.requirement_id=$1 AND d.repository_id=$2 AND d.pr_number=$3 AND d.head_sha=$4 AND p.observation->>'head'=d.head_sha AND p.observation->>'head_ref'=d.branch AND p.observation->>'base_ref'=d.base_branch AND p.observation->>'merge'='Merged' AND p.observation->>'merged_sha'=$5 AND NOT p.stale AND p.last_synced_at>extract(epoch FROM now())::bigint-60)")
        .bind(fact.requirement_id).bind(fact.github_repository_id).bind(fact.pr_number).bind(&fact.head_sha).bind(&fact.merged_sha).fetch_one(&mut **tx).await?;
    require(merged, "confirmed exact merge observation required")
}

pub(crate) async fn dependencies(tx: &mut Tx<'_>, id: i64) -> Result<Option<Vec<Fact>>> {
    let input: Option<(String, Value, i64)> =
        sqlx::query_as("SELECT draft_id,input,COALESCE(queue_order,(input#>>'{child,order}')::bigint) FROM group_execution_item WHERE requirement_id=$1")
            .bind(id)
            .fetch_optional(&mut **tx)
            .await?;
    let Some((draft, input, order)) = input else {
        return Ok(Some(Vec::new()));
    };
    let mut names: Vec<String> = decode(input["child"]["depends_on"].clone())?;
    let previous: Vec<String>=sqlx::query_scalar("SELECT child_id FROM group_execution_item WHERE draft_id=$1 AND input#>'{child,repository_id}'=$2 AND NOT removed AND COALESCE(queue_order,(input#>>'{child,order}')::bigint)<$3")
        .bind(&draft).bind(&input["child"]["repository_id"]).bind(order).fetch_all(&mut **tx).await?;
    names.extend(previous);
    names.sort();
    names.dedup();
    let mut facts = Vec::new();
    for name in names {
        let fact: Option<Value>=sqlx::query_scalar("SELECT c.fact FROM group_execution_item i JOIN group_completion c ON c.requirement_id=i.requirement_id AND c.authorization_id=i.authorization_id JOIN requirement r ON r.id=i.requirement_id WHERE i.draft_id=$1 AND i.child_id=$2 AND NOT r.cancel_requested AND r.state IN ('Submitted','Done')")
            .bind(&draft).bind(name).fetch_optional(&mut **tx).await?;
        let Some(fact) = fact else {
            return Ok(None);
        };
        facts.extend(dependency_fact(tx, fact).await?);
    }
    Ok(Some(facts))
}

async fn validation_dependency(tx: &mut Tx<'_>, fact: &Value) -> Result<bool> {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM integration_validation WHERE id=$1 AND state='passed' AND quiescent)")
        .bind(fact["validation_id"].as_str()).fetch_one(&mut **tx).await
}

async fn dependency_fact(tx: &mut Tx<'_>, fact: Value) -> Result<Option<Fact>> {
    if fact["source"] == "platform-integration-validation/v1" {
        require(
            validation_dependency(tx, &fact).await?,
            "validation completion identity unavailable",
        )?;
        return Ok(None);
    }
    Ok(Some(decode(fact)?))
}
