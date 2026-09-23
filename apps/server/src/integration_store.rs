//! Control checks and atomic child/parent completion under the coordinator lock.
use crate::{
    budget_store::require,
    integration_process::{Job, Outcome},
    run_store,
};
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Transaction};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
type Tx<'a> = Transaction<'a, Postgres>;

pub(crate) async fn allowed(tx: &mut Tx<'_>, job: &Job) -> Result<bool> {
    let b = &job.binding;
    let current: Option<Value> = sqlx::query_scalar("SELECT i.input FROM group_execution_item i JOIN requirement r ON r.id=i.requirement_id JOIN execution_control c ON c.requirement_id=r.id WHERE r.id=$1 AND r.revision=$2 AND i.authorization_id=$3 AND NOT r.cancel_requested AND NOT r.paused AND NOT c.paused AND c.recovery_complete AND NOT (SELECT blocked FROM storage_guard WHERE id=1)")
        .bind(b.requirement).bind(b.revision).bind(b.authorization).fetch_optional(&mut **tx).await?;
    let Some(input) = current else {
        return Ok(false);
    };
    if !input_current(tx, b, &input).await? {
        return Ok(false);
    }
    if !repositories_allowed(tx, &b.versions).await? {
        return Ok(false);
    }
    Ok(crate::group_budget::prepaid_fits(tx, b.requirement).await?)
}
async fn input_current(
    tx: &mut Tx<'_>,
    b: &crate::integration::Binding,
    input: &Value,
) -> Result<bool> {
    Ok(
        crate::validation::sha256(serde_json::to_vec(input)?) == b.input_sha256
            && crate::group_queue_store::authorized(tx, b.requirement).await?,
    )
}
async fn repositories_allowed(
    tx: &mut Tx<'_>,
    versions: &[crate::integration::Version],
) -> Result<bool> {
    for v in versions {
        let valid: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM repository WHERE id=$1 AND version=$2 AND (document->>'github_repository_id')::bigint=$3 AND NOT (document->>'revoked')::boolean AND version>revoked_through_version)")
            .bind(v.repository_id).bind(v.repository_version).bind(v.github_repository_id).fetch_one(&mut **tx).await?;
        if !valid {
            return Ok(false);
        }
    }
    Ok(true)
}
pub(crate) async fn finish(pool: &PgPool, id: &str, job: &Job, outcome: &Outcome) -> Result<()> {
    let mut tx = run_store::lock(pool).await?;
    save_result(&mut tx, id, outcome).await?;
    if allowed(&mut tx, job).await? {
        finish_allowed(&mut tx, id, job, outcome).await?;
    }
    tx.commit().await?;
    Ok(())
}
async fn save_result(tx: &mut Tx<'_>, id: &str, outcome: &Outcome) -> Result<()> {
    sqlx::query(
        "UPDATE integration_validation SET result=$2 WHERE id=$1 AND (result IS NULL OR result=$2)",
    )
    .bind(id)
    .bind(json!(outcome))
    .execute(&mut **tx)
    .await?;
    let saved: Value =
        sqlx::query_scalar("SELECT result FROM integration_validation WHERE id=$1 AND quiescent")
            .bind(id)
            .fetch_one(&mut **tx)
            .await?;
    require(
        saved == json!(outcome),
        "integration result identity conflict",
    )?;
    Ok(())
}
async fn finish_allowed(tx: &mut Tx<'_>, id: &str, job: &Job, outcome: &Outcome) -> Result<()> {
    let passed = outcome
        .evidence
        .as_ref()
        .is_some_and(|e| crate::integration::verify(&job.binding, &job.binding.versions, e));
    if !passed {
        if let Some(evidence) = &outcome.evidence
            && crate::linked_repair::failed_code(evidence, &job.binding.required)
        {
            crate::linked_failure_store::record(
                tx,
                &format!("integration:{id}"),
                job.binding.requirement,
                job.binding.revision,
                None,
                Some(id),
                evidence,
                &job.binding.required,
            )
            .await?;
        }
        sqlx::query("UPDATE integration_validation SET state='failed',blocker='integration validation failed; preserve original owner and classified recovery evidence' WHERE id=$1").bind(id).execute(&mut **tx).await?;
        return Ok(());
    }
    sqlx::query("UPDATE linked_failure SET state='complete',final_version=jsonb_build_object('binding',$2::jsonb,'validation_id',$3::text) WHERE requirement_id=$1 AND state='merged'")
        .bind(job.binding.requirement).bind(json!(job.binding)).bind(id).execute(&mut **tx).await?;
    complete(tx, id, job, outcome).await?;
    parent(tx, id, job, outcome).await?;
    sqlx::query(
        "UPDATE execution_control SET requirement_id=NULL WHERE id=1 AND requirement_id=$1",
    )
    .bind(job.binding.requirement)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
async fn complete(tx: &mut Tx<'_>, id: &str, job: &Job, outcome: &Outcome) -> Result<()> {
    let b = &job.binding;
    let fact = json!({"source":"platform-integration-validation/v1","validation_id":id,"binding":b,"evidence_sha256":crate::validation::sha256(serde_json::to_vec(outcome)?)});
    sqlx::query("INSERT INTO group_completion(requirement_id,authorization_id,fact) VALUES($1,$2,$3) ON CONFLICT DO NOTHING")
        .bind(b.requirement).bind(b.authorization).bind(&fact).execute(&mut **tx).await?;
    let old: Value =
        sqlx::query_scalar("SELECT fact FROM group_completion WHERE requirement_id=$1")
            .bind(b.requirement)
            .fetch_one(&mut **tx)
            .await?;
    require(old == fact, "completed child fact cannot be rewritten")?;
    sqlx::query(
        "UPDATE integration_validation SET state='passed',blocker=NULL WHERE id=$1 AND quiescent",
    )
    .bind(id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "UPDATE requirement SET state='Done',version=version+1 WHERE id=$1 AND state<>'Done'",
    )
    .bind(b.requirement)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
async fn parent(tx: &mut Tx<'_>, id: &str, job: &Job, outcome: &Outcome) -> Result<()> {
    let (draft, authorization, revision, review_version, document, review): (String,i64,i64,i64,Value,Value) = sqlx::query_as("SELECT i.draft_id,q.authorization_id,d.version,g.version,d.document,g.document FROM group_execution_item i JOIN group_queue q USING(draft_id) JOIN imported_draft d ON d.id=i.draft_id JOIN group_review g ON g.draft_id=i.draft_id WHERE i.requirement_id=$1")
        .bind(job.binding.requirement).fetch_one(&mut **tx).await?;
    let unfinished: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM group_execution_item i LEFT JOIN requirement r ON r.id=i.requirement_id WHERE i.draft_id=$1 AND NOT i.removed AND (i.frozen OR COALESCE(r.cancel_requested,true) OR NOT EXISTS(SELECT 1 FROM group_completion c WHERE c.requirement_id=i.requirement_id AND c.authorization_id=i.authorization_id))) OR EXISTS(SELECT 1 FROM group_edit WHERE draft_id=$1)")
        .bind(&draft).fetch_one(&mut **tx).await?;
    if unfinished {
        return Ok(());
    }
    let (review, final_child) = coverage(tx, revision, document, review).await?;
    let final_id: Option<i64> = sqlx::query_scalar(
        "SELECT requirement_id FROM group_execution_item WHERE draft_id=$1 AND child_id=$2",
    )
    .bind(&draft)
    .bind(&final_child.id)
    .fetch_one(&mut **tx)
    .await?;
    if final_id != Some(job.binding.requirement) || final_child.kind != "validation_only" {
        return Ok(());
    }
    sqlx::query("INSERT INTO group_acceptance(draft_id,authorization_id,draft_revision,review_version,validation_id,evidence) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT DO NOTHING")
        .bind(draft).bind(authorization).bind(revision).bind(review_version).bind(id).bind(json!({"binding":job.binding,"outcome":outcome,"coverage":review.coverage})).execute(&mut **tx).await?;
    Ok(())
}

async fn coverage(
    tx: &mut Tx<'_>,
    revision: i64,
    document: Value,
    review: Value,
) -> Result<(crate::group_review::Review, crate::draft::Child)> {
    let document: crate::draft::Document = serde_json::from_value(document)?;
    let review: crate::group_review::Review = serde_json::from_value(review)?;
    let repositories = match crate::group_store::repositories(tx).await {
        Ok(repositories) => repositories,
        Err(_) => return Err("repository snapshots unavailable".into()),
    };
    crate::group_review::validate(&document, revision, &review, &repositories)?;
    let mut final_child = document
        .children
        .first()
        .ok_or("final integration item absent")?;
    for child in &document.children {
        if child.order > final_child.order {
            final_child = child;
        }
    }
    Ok((review, final_child.clone()))
}
