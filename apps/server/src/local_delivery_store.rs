//! Local delivery reuses the publication outbox and freezes a reviewed target.
use crate::{
    delivery_extension::Result,
    local_git::{self, Binding},
    run_store,
};
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Transaction};
type Tx<'a> = Transaction<'a, Postgres>;

pub async fn configured(
    tx: &mut Tx<'_>,
    requirement: i64,
    revision: i64,
) -> Result<Option<Binding>> {
    let document: Value = sqlx::query_scalar(
        "SELECT document FROM execution_revision WHERE requirement_id=$1 AND revision=$2",
    )
    .bind(requirement)
    .bind(revision)
    .fetch_one(&mut **tx)
    .await?;
    if document["repository"]["delivery"] != "local_git" {
        return Ok(None);
    }
    Ok(Some(resolve_document(&document)?))
}

pub fn resolve_document(document: &Value) -> Result<Binding> {
    let repository = &document["repository"];
    local_git::resolve(
        &local_git::installed()?,
        repository["remote"]
            .as_str()
            .ok_or("local reference missing")?,
        document["repository_id"]
            .as_i64()
            .ok_or("local repository missing")?,
        document["repository_version"]
            .as_i64()
            .ok_or("local repository version missing")?,
        repository["base_branch"]
            .as_str()
            .ok_or("local branch missing")?,
    )
}

pub async fn claim_ready(tx: &mut Tx<'_>, requirement: i64, revision: i64) -> Result<bool> {
    let Some(binding) = configured(tx, requirement, revision).await? else {
        return Ok(crate::github_store::claim_ready(tx, requirement, revision).await?);
    };
    let allowed: bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM repository r WHERE id=$1 AND version=$2 AND NOT (document->>'revoked')::boolean AND version>revoked_through_version AND plugin_scope_allows('delivery:local_git',id))")
        .bind(binding.target.repository_id).bind(binding.target.repository_version).fetch_one(&mut **tx).await?;
    Ok(allowed && local_git::head(&binding).is_ok())
}

pub async fn enqueue(
    tx: &mut Tx<'_>,
    validation: &str,
    requirement: i64,
    revision: i64,
    document: &Value,
    manifest: &Value,
) -> Result<()> {
    let (binding, saved) = frozen_target(tx, validation, requirement, revision, document).await?;
    let head = manifest["head"].as_str().ok_or("local candidate absent")?;
    let branch = manifest["workspace"]["branch"]
        .as_str()
        .ok_or("local source branch absent")?;
    let key = format!("local-{}", crate::validation::sha256(validation));
    sqlx::query("INSERT INTO delivery(action_key,validation_id,requirement_id,revision,repository_id,repository,branch,base_branch,head_sha,manifest,policy,mode,internal_repository_id,local_binding) VALUES($1,$2,$3,$4,0,$5,$6,$7,$8,$9,$10,'local_git',$11,$12) ON CONFLICT(action_key) DO NOTHING")
        .bind(&key).bind(validation).bind(requirement).bind(revision).bind(&binding.target.reference).bind(branch).bind(&binding.target.branch).bind(head).bind(manifest).bind(document).bind(binding.target.repository_id).bind(saved).execute(&mut **tx).await?;
    sqlx::query("UPDATE linked_failure f SET repair_delivery=$2 FROM candidate_validation v JOIN linked_run_input i ON i.run_id=v.source_run_id WHERE v.id=$1 AND f.id=i.failure_id AND f.state='reserved'").bind(validation).bind(&key).execute(&mut **tx).await?;
    sqlx::query(
        "INSERT INTO delivery_action(action_key,kind) VALUES($1,'publish') ON CONFLICT DO NOTHING",
    )
    .bind(&key)
    .execute(&mut **tx)
    .await?;
    sqlx::query("UPDATE validation_step SET consumer=$2 WHERE validation_id=$1")
        .bind(validation)
        .bind(format!("outbox:{key}"))
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn frozen_target(
    tx: &mut Tx<'_>,
    validation: &str,
    requirement: i64,
    revision: i64,
    document: &Value,
) -> Result<(Binding, Value)> {
    let binding = resolve_document(document)?;
    let saved: Value = sqlx::query_scalar(
        "SELECT COALESCE((SELECT f.local_binding FROM linked_failure f JOIN linked_run_input i ON i.failure_id=f.id JOIN candidate_validation v ON v.source_run_id=i.run_id WHERE v.id=$3),(SELECT local_binding FROM initial_run WHERE requirement_id=$1 AND revision=$2))",
    )
    .bind(requirement)
    .bind(revision)
    .bind(validation)
    .fetch_one(&mut **tx)
    .await?;
    if saved != json!(binding) {
        return Err("local target changed since execution preparation".into());
    }
    Ok((binding, saved))
}

#[derive(Clone, Debug, PartialEq, sqlx::FromRow)]
pub struct Job {
    pub action_key: String,
    pub validation_id: String,
    pub requirement_id: i64,
    pub revision: i64,
    pub head_sha: String,
    pub manifest: Value,
    pub policy: Value,
    pub local_binding: Value,
    pub local_acceptance_started: bool,
    pub local_acceptance: Option<Value>,
    pub state: String,
    pub attempts: i32,
}
impl Job {
    pub fn binding(&self) -> Result<Binding> {
        Ok(serde_json::from_value(self.local_binding.clone())?)
    }
    pub fn baseline(&self) -> Result<&str> {
        self.manifest["workspace"]["baseline"]
            .as_str()
            .ok_or("local baseline absent".into())
    }
}

pub async fn pending(pool: &PgPool) -> Result<Option<Job>> {
    Ok(sqlx::query_as("SELECT d.*,a.state,a.attempts FROM delivery d JOIN delivery_action a USING(action_key) WHERE d.mode='local_git' AND a.kind='publish' AND NOT d.released AND a.state<>'withdrawn' AND (NOT EXISTS(SELECT 1 FROM linked_failure f WHERE f.local_delivery=d.action_key) OR EXISTS(SELECT 1 FROM requirement r WHERE r.id=d.requirement_id AND r.cancel_requested)) AND a.next_attempt_at<=extract(epoch FROM now())::bigint ORDER BY d.requirement_id LIMIT 1")
        .fetch_optional(pool).await?)
}

pub async fn allowed(tx: &mut Tx<'_>, job: &Job) -> Result<bool> {
    let safe:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM delivery d JOIN candidate_validation v ON v.id=d.validation_id JOIN requirement r ON r.id=d.requirement_id JOIN execution_control c ON c.requirement_id=r.id JOIN repository repo ON repo.id=d.internal_repository_id WHERE d.action_key=$1 AND NOT d.released AND NOT d.local_storage_blocked AND r.revision=d.revision AND r.state IN ('Running','Submitted') AND NOT r.cancel_requested AND NOT r.paused AND NOT c.paused AND c.recovery_complete AND NOT (SELECT blocked FROM storage_guard WHERE id=1) AND NOT (repo.document->>'revoked')::boolean AND repo.version=(d.policy->>'repository_version')::bigint AND repo.version>repo.revoked_through_version AND plugin_scope_allows('delivery:local_git',repo.id) AND plugin_scope_allows('validation:native',repo.id) AND v.result='succeeded' AND v.superseded_by IS NULL AND v.candidate_sha=d.head_sha AND NOT EXISTS(SELECT 1 FROM agent_run a WHERE a.requirement_id=r.id AND NOT a.quiescent) AND NOT EXISTS(SELECT 1 FROM workspace_operation o JOIN agent_run a ON a.id=o.run_id WHERE a.requirement_id=r.id AND o.status<>'complete') AND NOT EXISTS(SELECT 1 FROM run_workspace w JOIN agent_run a ON a.id=w.run_id LEFT JOIN workspace_snapshot s ON s.run_id=w.run_id WHERE a.requirement_id=r.id AND s.run_id IS NULL) AND NOT EXISTS(SELECT 1 FROM linked_failure f WHERE f.requirement_id=r.id AND f.state NOT IN ('complete','cancelled','merged') AND f.repair_delivery IS DISTINCT FROM d.action_key))")
        .bind(&job.action_key).fetch_one(&mut **tx).await?;
    if !safe {
        return Ok(false);
    }
    let balance = crate::budget_store::balance(tx, job.requirement_id).await?;
    Ok(!balance.exhausted
        && balance.exposure.fits(balance.limits)
        && crate::group_queue_store::authorized(tx, job.requirement_id).await?
        && crate::group_budget::prepaid_fits(tx, job.requirement_id).await?)
}

pub async fn begin(pool: &PgPool, job: &Job) -> Result<Option<i64>> {
    let mut tx = run_store::lock(pool).await?;
    if !allowed(&mut tx, job).await? {
        return Ok(None);
    }
    let ordinal:Option<i32>=sqlx::query_scalar("UPDATE delivery_action SET state='unknown',attempts=attempts+1 WHERE action_key=$1 AND kind='publish' AND state IN ('pending','blocked') AND attempts=0 RETURNING attempts")
        .bind(&job.action_key).fetch_optional(&mut *tx).await?;
    let Some(ordinal) = ordinal else {
        return Ok(None);
    };
    let id=sqlx::query_scalar("INSERT INTO delivery_attempt(action_key,kind,ordinal,operation) VALUES($1,'publish',$2,'local_update') RETURNING id")
        .bind(&job.action_key).bind(ordinal).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(Some(id))
}

pub async fn observed(pool: &PgPool, job: &Job, fact: &local_git::Observation) -> Result<()> {
    let mut tx = run_store::lock(pool).await?;
    sqlx::query("INSERT INTO delivery_observation(action_key,kind,fact) SELECT $1,'local_git',$2 WHERE NOT EXISTS(SELECT 1 FROM delivery_observation WHERE action_key=$1 AND kind='local_git' AND fact=$2)")
        .bind(&job.action_key)
        .bind(json!({"target":job.local_binding,"candidate":job.head_sha,"observation":fact}))
        .execute(&mut *tx)
        .await?;
    if *fact == local_git::Observation::Delivered {
        sqlx::query("UPDATE delivery_action SET state='confirmed',error=NULL WHERE action_key=$1 AND kind='publish'").bind(&job.action_key).execute(&mut *tx).await?;
        sqlx::query("UPDATE requirement SET state='Submitted',version=version+1 WHERE id=$1 AND state='Running' AND NOT cancel_requested").bind(job.requirement_id).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn blocked(pool: &PgPool, job: &Job, reason: &str) -> Result<()> {
    sqlx::query("UPDATE delivery_action SET state='blocked',error=jsonb_build_object('code','local_delivery_blocked','reason',$2::text),next_attempt_at=extract(epoch FROM now())::bigint+30 WHERE action_key=$1 AND kind='publish'")
        .bind(&job.action_key).bind(reason).execute(pool).await?;
    Ok(())
}

pub async fn cancel_unsent(pool: &PgPool, job: &Job) -> Result<bool> {
    let mut tx = run_store::lock(pool).await?;
    let cancelled: bool =
        sqlx::query_scalar("SELECT cancel_requested FROM requirement WHERE id=$1")
            .bind(job.requirement_id)
            .fetch_one(&mut *tx)
            .await?;
    if !cancelled {
        return Ok(false);
    }
    // Started updates require their atomic receipt to establish delivery. An
    // unconfirmed outcome cannot release the owner or authorize another send.
    sqlx::query("UPDATE delivery_action SET state='withdrawn' WHERE action_key=$1 AND kind='publish' AND attempts=0").bind(&job.action_key).execute(&mut *tx).await?;
    sqlx::query("UPDATE delivery SET released=true WHERE action_key=$1 AND (NOT local_acceptance_started OR local_acceptance_quiescent) AND EXISTS(SELECT 1 FROM delivery_action a WHERE a.action_key=$1 AND a.state IN ('withdrawn','confirmed'))").bind(&job.action_key).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(true)
}

pub async fn project_repositories(
    pool: &PgPool,
    entries: &mut [Value],
) -> std::result::Result<(), sqlx::Error> {
    for entry in entries {
        if entry["repository"]["delivery"] != "local_git" {
            continue;
        }
        let document = json!({"repository_id":entry["id"],"repository_version":entry["version"],"repository":entry["repository"]});
        let scoped: bool =
            sqlx::query_scalar("SELECT plugin_scope_allows('delivery:local_git',$1)")
                .bind(entry["id"].as_i64())
                .fetch_one(pool)
                .await?;
        let blocker = capability_blocker(&document, scoped);
        entry["delivery_ready"] = json!(blocker.is_none());
        entry["capability_stale"] = json!(false);
        entry["capability_checked_at"] = json!(crate::runtime_client::now());
        entry["capability_blockers"] = json!(blocker.into_iter().collect::<Vec<_>>());
    }
    Ok(())
}
fn capability_blocker(document: &Value, scoped: bool) -> Option<String> {
    if !scoped || document["repository"]["revoked"] == true {
        return Some("local delivery not authorized for repository".into());
    }
    match resolve_document(document).and_then(capability_head) {
        Ok(_) => None,
        Err(error) => Some(error.to_string()),
    }
}
fn capability_head(binding: Binding) -> Result<String> {
    local_git::head(&binding)
}
