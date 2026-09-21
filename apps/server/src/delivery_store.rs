//! Transactional outbox and immutable, attempt-bound receipts.
use crate::{delivery::Identity, run_store};
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Transaction};
type Result<T> = std::result::Result<T, sqlx::Error>;

pub async fn enqueue(tx: &mut Transaction<'_, Postgres>, validation: &str) -> Result<()> {
    let (requirement, revision, head, document, manifest): (i64,i64,String,Value,Value) = sqlx::query_as("SELECT v.requirement_id,v.revision,v.candidate_sha,r.document,s.manifest FROM candidate_validation v JOIN requirement_revision r ON r.requirement_id=v.requirement_id AND r.revision=v.revision JOIN workspace_snapshot s ON s.run_id=v.source_run_id WHERE v.id=$1 AND v.result='succeeded' AND s.candidate AND s.manifest->>'head'=v.candidate_sha")
        .bind(validation).fetch_one(&mut **tx).await?;
    let previous: Option<(String,String,String,i64)> = sqlx::query_as("WITH RECURSIVE lineage(id,depth) AS (SELECT $1::text,0 UNION ALL SELECT p.source_validation_id,l.depth+1 FROM lineage l JOIN candidate_validation v ON v.id=l.id JOIN repair_reservation p ON p.repair_run_id=v.source_run_id WHERE p.event_key IS NOT NULL AND l.depth<3) SELECT COALESCE(d.original_action_key,d.action_key),d.branch,d.head_sha,d.pr_number FROM lineage l JOIN delivery d ON d.validation_id=l.id WHERE l.depth>0 AND d.pr_number IS NOT NULL AND d.superseded_by IS NULL ORDER BY l.depth LIMIT 1")
        .bind(validation).fetch_optional(&mut **tx).await?;
    let identity = delivery_identity(
        requirement,
        revision,
        head,
        &document,
        &manifest,
        previous.as_ref().map(|p| p.1.clone()),
    )?;
    let key = identity.action_key();
    sqlx::query("INSERT INTO delivery(action_key,validation_id,requirement_id,revision,repository_id,repository,branch,base_branch,head_sha,manifest,policy) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11) ON CONFLICT(action_key) DO NOTHING")
        .bind(&key).bind(validation).bind(requirement).bind(revision).bind(identity.repository_id as i64).bind(&identity.repository).bind(&identity.branch).bind(&identity.base_branch).bind(&identity.head).bind(manifest).bind(document).execute(&mut **tx).await?;
    if let Some((original, _, expected, number)) = previous {
        sqlx::query("UPDATE delivery SET original_action_key=$2,expected_head=$3,pr_number=$4 WHERE action_key=$1")
            .bind(&key).bind(original).bind(expected).bind(number).execute(&mut **tx).await?;
    }
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
fn delivery_identity(
    requirement: i64,
    revision: i64,
    head: String,
    document: &Value,
    manifest: &Value,
    branch: Option<String>,
) -> Result<Identity> {
    Ok(Identity {
        requirement,
        revision,
        head,
        repository_id: document["repository"]["github_repository_id"]
            .as_u64()
            .ok_or_else(invalid)?,
        repository: field(&document["repository"], "remote")?,
        base_branch: field(&document["repository"], "base_branch")?,
        branch: branch.unwrap_or(field(&manifest["workspace"], "branch")?),
    })
}
fn field(value: &Value, name: &str) -> Result<String> {
    value[name]
        .as_str()
        .filter(nonempty)
        .map(str::to_owned)
        .ok_or_else(invalid)
}
fn invalid() -> sqlx::Error {
    sqlx::Error::Protocol("delivery identity missing".into())
}

#[derive(Clone, sqlx::FromRow)]
pub struct Pending {
    pub action_key: String,
    pub kind: String,
    pub state: String,
    pub attempts: i32,
    pub requirement_id: i64,
    pub revision: i64,
    pub repository_id: i64,
    pub repository: String,
    pub branch: String,
    pub base_branch: String,
    pub head_sha: String,
    pub manifest: Value,
    pub pr_number: Option<i64>,
    pub original_action_key: Option<String>,
    pub expected_head: Option<String>,
}
impl Pending {
    pub fn fact(&self, pr: &Value) -> crate::delivery::PrFact {
        let mut proof = pr.clone();
        if let Some(original) = &self.original_action_key {
            let marker = format!("<!-- codexsymphony-delivery:{original} -->");
            if !pr["body"]
                .as_str()
                .is_some_and(|body| body.contains(&marker))
            {
                return crate::delivery::PrFact::Conflict;
            }
            proof["body"] = json!(self.identity().marker());
        }
        crate::delivery::pr_fact(&self.identity(), &proof)
    }
    pub fn identity(&self) -> Identity {
        Identity {
            requirement: self.requirement_id,
            revision: self.revision,
            repository_id: self.repository_id as u64,
            repository: self.repository.clone(),
            branch: self.branch.clone(),
            base_branch: self.base_branch.clone(),
            head: self.head_sha.clone(),
        }
    }
}
pub async fn due(pool: &PgPool, now: i64) -> Result<Vec<Pending>> {
    sqlx::query_as("SELECT d.*,a.kind,a.state,a.attempts FROM delivery d JOIN delivery_action a USING(action_key) WHERE a.state IN ('pending','unknown','blocked') AND a.next_attempt_at<=$1 ORDER BY d.requirement_id,a.kind LIMIT 1").bind(now).fetch_all(pool).await
}

/// Persist an unknown outcome before the external call, under the control lock.
/// The final send boundary serializes with pause/cancel, and never retries a
/// write until the worker has read remote identity again.
pub async fn begin(pool: &PgPool, job: &Pending, operation: &str, now: i64) -> Result<Option<i64>> {
    let mut tx = run_store::lock(pool).await?;
    if !matches!(
        (job.kind.as_str(), operation),
        ("publish", "push" | "create") | ("close", "close")
    ) || !allowed(&mut tx, job).await?
    {
        return Ok(None);
    }
    let result = write_attempt(&mut tx, job, operation, now).await?;
    tx.commit().await?;
    Ok(result)
}
async fn write_attempt(
    tx: &mut Transaction<'_, Postgres>,
    job: &Pending,
    operation: &str,
    now: i64,
) -> Result<Option<i64>> {
    let ordinal: Option<i32> = sqlx::query_scalar("UPDATE delivery_action SET attempts=attempts+1,state='unknown',next_attempt_at=$3+30 WHERE action_key=$1 AND kind=$2 AND state IN ('pending','unknown') AND attempts<attempt_limit RETURNING attempts")
        .bind(&job.action_key).bind(&job.kind).bind(now).fetch_optional(&mut **tx).await?;
    let Some(ordinal) = ordinal else {
        sqlx::query("UPDATE delivery_action SET state='blocked',error=jsonb_build_object('code','delivery_retry_exhausted','phase',kind,'attempts',attempts) WHERE action_key=$1 AND kind=$2 AND state='unknown' AND attempts>=attempt_limit")
            .bind(&job.action_key).bind(&job.kind).execute(&mut **tx).await?;
        return Ok(None);
    };
    let id=sqlx::query_scalar("INSERT INTO delivery_attempt(action_key,kind,ordinal,operation) VALUES($1,$2,$3,$4) RETURNING id")
        .bind(&job.action_key).bind(&job.kind).bind(ordinal).bind(operation).fetch_one(&mut **tx).await?;
    Ok(Some(id))
}
async fn allowed(tx: &mut Transaction<'_, Postgres>, job: &Pending) -> Result<bool> {
    let safe: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM requirement r JOIN execution_control c ON c.requirement_id=r.id WHERE r.id=$1 AND r.revision=$2 AND EXISTS(SELECT 1 FROM delivery d JOIN candidate_validation v ON v.id=d.validation_id WHERE d.action_key=$4 AND d.head_sha=v.candidate_sha AND v.result='succeeded' AND d.requirement_id=r.id AND d.revision=r.revision) AND c.recovery_complete AND NOT (SELECT blocked FROM storage_guard WHERE id=1) AND NOT EXISTS(SELECT 1 FROM agent_run a WHERE a.requirement_id=r.id AND NOT a.quiescent) AND NOT EXISTS(SELECT 1 FROM workspace_operation o JOIN agent_run a ON a.id=o.run_id WHERE a.requirement_id=r.id AND o.status<>'complete') AND NOT EXISTS(SELECT 1 FROM run_workspace w JOIN agent_run a ON a.id=w.run_id LEFT JOIN workspace_snapshot s ON s.run_id=w.run_id WHERE a.requirement_id=r.id AND s.run_id IS NULL) AND (($3='close' AND r.cancel_requested) OR ($3='publish' AND NOT r.cancel_requested AND NOT r.paused AND NOT c.paused AND r.state='Running')))")
        .bind(job.requirement_id).bind(job.revision).bind(&job.kind).bind(&job.action_key).fetch_one(&mut **tx).await?;
    if !safe || job.kind == "close" {
        return Ok(safe);
    }
    let authorized: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM repository r JOIN requirement_revision v ON v.requirement_id=$1 AND v.revision=$2 WHERE r.id=COALESCE((v.document->>'repository_id')::bigint,1) AND NOT (r.document->>'revoked')::boolean AND (v.document->>'repository_version')::bigint>r.revoked_through_version)").bind(job.requirement_id).bind(job.revision).fetch_one(&mut **tx).await?;
    Ok(
        authorized
            && crate::github_store::claim_ready(tx, job.requirement_id, job.revision).await?,
    )
}
pub async fn receipt(pool: &PgPool, attempt: i64, result: &Value) -> Result<()> {
    sqlx::query("UPDATE delivery_attempt SET result=$2 WHERE id=$1 AND result IS NULL")
        .bind(attempt)
        .bind(result)
        .execute(pool)
        .await?;
    Ok(())
}
pub async fn failed(
    pool: &PgPool,
    job: &Pending,
    now: i64,
    error: &crate::github_http::Error,
) -> Result<()> {
    let attempts: i32 =
        sqlx::query_scalar("SELECT attempts FROM delivery_action WHERE action_key=$1 AND kind=$2")
            .bind(&job.action_key)
            .bind(&job.kind)
            .fetch_one(pool)
            .await?;
    let mut evidence = crate::delivery::failure(error.code, &job.kind, attempts, now, error.status);
    if let Some(seconds) = error.retry_after_seconds {
        let requested = now.saturating_add(i64::try_from(seconds).unwrap_or(i64::MAX));
        evidence["next_attempt_at"] =
            json!(requested.max(evidence["next_attempt_at"].as_i64().unwrap_or(now)));
    }
    sqlx::query("UPDATE delivery_action SET state=CASE WHEN attempts>=attempt_limit OR $3='github_identity_conflict' THEN 'blocked' ELSE 'unknown' END,next_attempt_at=$4,error=$5 WHERE action_key=$1 AND kind=$2")
        .bind(&job.action_key).bind(&job.kind).bind(error.code).bind(evidence["next_attempt_at"].as_i64()).bind(evidence).execute(pool).await?;
    Ok(())
}
pub async fn confirmed(pool: &PgPool, job: &Pending, pr: &Value) -> Result<()> {
    let fact = job.fact(pr);
    if matches!(
        fact,
        crate::delivery::PrFact::Conflict | crate::delivery::PrFact::Unknown
    ) {
        return Err(invalid());
    }
    let number = confirmed_number(job, pr)?;
    let mut tx = run_store::lock(pool).await?;
    link(&mut tx, job, number).await?;
    handoff(&mut tx, job).await?;
    terminal(&mut tx, job, &fact).await?;
    sqlx::query("INSERT INTO delivery_observation(action_key,kind,fact) VALUES($1,$2,$3)")
        .bind(&job.action_key)
        .bind(&job.kind)
        .bind(json!({"pr":pr}))
        .execute(&mut *tx)
        .await?;
    tx.commit().await
}

/// Reserve the next observation time even when a paused or blocked action cannot
/// write. Background reconciliation stays bounded and never wakes the model.
pub async fn observed(pool: &PgPool, job: &Pending, now: i64) -> Result<()> {
    sqlx::query("UPDATE delivery_action SET next_attempt_at=$3+60 WHERE action_key=$1 AND kind=$2")
        .bind(&job.action_key)
        .bind(&job.kind)
        .bind(now)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn withdraw(pool: &PgPool, job: &Pending, head: Option<&str>) -> Result<bool> {
    let mut tx = run_store::lock(pool).await?;
    let cancelled: bool =
        sqlx::query_scalar("SELECT cancel_requested FROM requirement WHERE id=$1")
            .bind(job.requirement_id)
            .fetch_one(&mut *tx)
            .await?;
    if !cancelled {
        return Ok(false);
    }
    // A late create cannot be disproved by an empty list. A late push is resolved
    // only by the exact candidate head, or by proof that no push was attempted.
    sqlx::query("UPDATE delivery_action a SET state='withdrawn',error=NULL WHERE a.action_key=$1 AND a.kind='publish' AND NOT EXISTS(SELECT 1 FROM delivery_attempt t WHERE t.action_key=a.action_key AND t.operation='create') AND ($2 OR NOT EXISTS(SELECT 1 FROM delivery_attempt t WHERE t.action_key=a.action_key))")
        .bind(&job.action_key).bind(head == Some(job.head_sha.as_str())).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(true)
}

fn confirmed_number(job: &Pending, pr: &Value) -> Result<i64> {
    let number = pr["number"].as_i64().ok_or_else(invalid)?;
    if job.pr_number.is_some_and(|saved| saved != number) {
        return Err(invalid());
    }
    Ok(number)
}
async fn link(tx: &mut Transaction<'_, Postgres>, job: &Pending, number: i64) -> Result<()> {
    sqlx::query("INSERT INTO github_pr(repository_id,number,requirement_id) VALUES($1,$2,$3) ON CONFLICT DO NOTHING").bind(job.repository_id).bind(number).bind(job.requirement_id).execute(&mut **tx).await?;
    let owner: i64 = sqlx::query_scalar(
        "SELECT requirement_id FROM github_pr WHERE repository_id=$1 AND number=$2",
    )
    .bind(job.repository_id)
    .bind(number)
    .fetch_one(&mut **tx)
    .await?;
    if owner != job.requirement_id {
        return Err(invalid());
    }
    let linked = sqlx::query("UPDATE delivery SET pr_number=$2,consumer=$3 WHERE action_key=$1 AND (pr_number IS NULL OR pr_number=$2)").bind(&job.action_key).bind(number).bind(format!("pr:{}:{number}:{}",job.repository_id,job.head_sha)).execute(&mut **tx).await?;
    if linked.rows_affected() != 1 {
        return Err(invalid());
    }
    Ok(())
}
async fn handoff(tx: &mut Transaction<'_, Postgres>, job: &Pending) -> Result<()> {
    if let Some(expected) = &job.expected_head {
        sqlx::query("UPDATE delivery SET superseded_by=$1 WHERE requirement_id=$2 AND repository_id=$3 AND branch=$4 AND head_sha=$5 AND action_key<>$1 AND superseded_by IS NULL")
            .bind(&job.action_key).bind(job.requirement_id).bind(job.repository_id).bind(&job.branch).bind(expected).execute(&mut **tx).await?;
        sqlx::query("UPDATE delivery_action SET state='withdrawn' WHERE kind='close' AND action_key IN(SELECT action_key FROM delivery WHERE superseded_by=$1)")
            .bind(&job.action_key).execute(&mut **tx).await?;
    }
    sqlx::query("UPDATE validation_step s SET consumer=d.consumer FROM delivery d WHERE d.action_key=$1 AND s.validation_id=d.validation_id").bind(&job.action_key).execute(&mut **tx).await?;
    sqlx::query("UPDATE delivery_action SET state='confirmed',error=NULL WHERE action_key=$1 AND kind='publish'").bind(&job.action_key).execute(&mut **tx).await?;
    sqlx::query("UPDATE requirement SET state='Submitted',version=version+1 WHERE id=$1 AND state='Running' AND NOT cancel_requested").bind(job.requirement_id).execute(&mut **tx).await?;
    sqlx::query("INSERT INTO delivery_action(action_key,kind) SELECT $1,'close' FROM requirement WHERE id=$2 AND cancel_requested ON CONFLICT DO NOTHING").bind(&job.action_key).bind(job.requirement_id).execute(&mut **tx).await?;
    Ok(())
}
async fn terminal(
    tx: &mut Transaction<'_, Postgres>,
    job: &Pending,
    fact: &crate::delivery::PrFact,
) -> Result<()> {
    if matches!(
        fact,
        crate::delivery::PrFact::Closed | crate::delivery::PrFact::Merged
    ) {
        sqlx::query("UPDATE delivery_action SET state='confirmed',error=NULL WHERE action_key=$1 AND kind='close'").bind(&job.action_key).execute(&mut **tx).await?;
    }
    if *fact == crate::delivery::PrFact::Merged {
        sqlx::query("UPDATE delivery SET released=true WHERE action_key=$1")
            .bind(&job.action_key)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

fn nonempty(value: &&str) -> bool {
    !value.is_empty()
}
