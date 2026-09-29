//! Reconcile the closed PR and freeze its approved target before shared repair.
use crate::{github::Policy, github_http::AppClient, pre_merge_recovery::Decision};
use serde_json::Value;
use sqlx::PgPool;
use std::path::Path;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(sqlx::FromRow)]
pub(crate) struct Pending {
    pub id: String,
    pub requirement_id: i64,
    pub revision: i64,
    pub request: Value,
    pub intent: Value,
    pub manifest: Value,
    pub source: String,
    pub document: Value,
}

pub(crate) trait Remote: Send {
    fn branch(
        &mut self,
        policy: &Policy,
    ) -> impl std::future::Future<Output = Result<String>> + Send;
    fn fetch(
        &mut self,
        policy: &Policy,
        path: &Path,
        sha: &str,
    ) -> impl std::future::Future<Output = Result<()>> + Send;
}
struct Broker<'a> {
    client: &'a mut AppClient,
    now: i64,
}
impl Remote for Broker<'_> {
    async fn branch(&mut self, policy: &Policy) -> Result<String> {
        let value = self
            .client
            .get(
                policy,
                &format!(
                    "/repos/{}/git/ref/heads/{}",
                    policy.repository,
                    crate::github_observe::segment(&policy.default_branch)
                ),
                self.now,
            )
            .await?;
        branch_sha(&value)
    }
    async fn fetch(&mut self, policy: &Policy, path: &Path, sha: &str) -> Result<()> {
        self.client
            .fetch_commit(policy, path, sha, self.now)
            .await
            .map_err(Into::into)
    }
}
fn branch_sha(value: &Value) -> Result<String> {
    Ok(value["object"]["sha"]
        .as_str()
        .ok_or("target branch identity unavailable")?
        .to_owned())
}

pub(crate) async fn tick(
    pool: &PgPool,
    client: &mut AppClient,
    root: &Path,
    now: i64,
) -> Result<()> {
    advance(pool, &mut Broker { client, now }, root, now).await
}

async fn advance(pool: &PgPool, remote: &mut impl Remote, root: &Path, now: i64) -> Result<()> {
    expire_attempts(pool, now).await?;
    let Some(pending) = claim(pool, now).await? else {
        return Ok(());
    };
    if let Err(error) = prepare(pool, remote, root, &pending).await {
        failure(pool, &pending.id, now, &error.to_string()).await?;
    }
    Ok(())
}

async fn expire_attempts(pool: &PgPool, now: i64) -> Result<()> {
    let mut tx = crate::run_store::lock(pool).await?;
    sqlx::query("UPDATE pre_merge_recovery SET state='blocked',blocker='source preparation attempts exhausted; reconcile interrupted or deferred preparation',receipts=receipts||jsonb_build_array(jsonb_build_object('time',$1::bigint,'attempt',attempts,'error','source preparation attempts exhausted without a ready source')) WHERE state='pending' AND attempts>=3 AND next_attempt_at<=$1")
        .bind(now).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

async fn claim(pool: &PgPool, now: i64) -> Result<Option<Pending>> {
    let mut tx = crate::run_store::lock(pool).await?;
    let row: Option<Pending> = sqlx::query_as("SELECT p.id,p.requirement_id,p.revision,p.request,m.intent,d.manifest,v.source_run_id AS source,rr.document FROM pre_merge_recovery p JOIN requirement r ON r.id=p.requirement_id JOIN execution_control c ON c.requirement_id=r.id JOIN merge_operation m ON m.action_key=p.merge_key JOIN delivery d ON d.action_key=m.delivery_key JOIN candidate_validation v ON v.id=d.validation_id JOIN requirement_revision rr ON rr.requirement_id=r.id AND rr.revision=r.revision WHERE p.state='pending' AND p.attempts<3 AND p.next_attempt_at<=$1 AND r.revision=p.revision AND r.state='Submitted' AND NOT r.paused AND NOT r.cancel_requested AND NOT c.paused AND c.recovery_complete AND NOT m.merge_started AND NOT m.pre_validation_started AND m.merged_sha IS NULL AND m.state='blocked' AND d.superseded_by IS NULL ORDER BY p.created_at LIMIT 1")
        .bind(now).fetch_optional(&mut *tx).await?;
    let Some(row) = row else { return Ok(None) };
    if !authorized(&mut tx, &row).await? {
        return Ok(None);
    }
    sqlx::query(
        "UPDATE pre_merge_recovery SET attempts=attempts+1,next_attempt_at=$2+30 WHERE id=$1",
    )
    .bind(&row.id)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Some(row))
}

async fn authorized(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, row: &Pending) -> Result<bool> {
    if !crate::group_queue_store::authorized(tx, row.requirement_id).await? {
        return Ok(false);
    }
    let command: Decision = serde_json::from_value(row.request.clone())?;
    Ok(crate::pre_merge_recovery::closed_identity(
        tx,
        &command.merge_key,
        &command.head,
        &command.base,
    )
    .await?)
}

async fn prepare(
    pool: &PgPool,
    remote: &mut impl Remote,
    root: &Path,
    pending: &Pending,
) -> Result<()> {
    let command: Decision = serde_json::from_value(pending.request.clone())?;
    let intent: crate::automatic_merge::Intent = serde_json::from_value(pending.intent.clone())?;
    let actual = remote.branch(&intent.policy).await?;
    crate::budget_store::require(
        actual == command.base,
        "target baseline changed; explicit reconciliation required",
    )?;
    let broker = crate::git_broker::GitBroker::open(&root.join("workspaces"))?;
    let manifest = serde_json::from_value(pending.manifest.clone())?;
    remote
        .fetch(
            &intent.policy,
            &broker.delivery_repository(&manifest)?,
            &actual,
        )
        .await?;
    freeze(pool, &broker, pending, &command).await
}

async fn freeze(
    pool: &PgPool,
    broker: &crate::git_broker::GitBroker,
    pending: &Pending,
    command: &Decision,
) -> Result<()> {
    let mut tx = crate::run_store::lock(pool).await?;
    if !authorized(&mut tx, pending).await? {
        return Ok(());
    }
    if !still_pending(&mut tx, pending).await? {
        return Ok(());
    }
    crate::linked_repair_source::freeze_pre_merge(tx, broker, pending, command).await
}

async fn still_pending(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    pending: &Pending,
) -> Result<bool> {
    Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pre_merge_recovery p JOIN requirement r ON r.id=p.requirement_id JOIN execution_control c ON c.requirement_id=r.id JOIN repository repo ON repo.id=($4->>'repository_id')::bigint WHERE p.id=$1 AND p.state='pending' AND p.request=$2 AND p.revision=$3 AND r.revision=$3 AND r.state='Submitted' AND NOT r.paused AND NOT r.cancel_requested AND NOT c.paused AND c.recovery_complete AND repo.version=($4->>'repository_version')::bigint AND repo.document=$4->'repository' AND NOT (SELECT blocked FROM storage_guard WHERE id=1) AND NOT EXISTS(SELECT 1 FROM agent_run WHERE NOT quiescent) AND NOT EXISTS(SELECT 1 FROM workspace_operation WHERE status<>'complete'))")
        .bind(&pending.id).bind(&pending.request).bind(pending.revision).bind(&pending.document).fetch_one(&mut **tx).await?)
}

async fn failure(pool: &PgPool, id: &str, now: i64, reason: &str) -> Result<()> {
    let reason = crate::operator_view::redact_text(reason);
    sqlx::query("UPDATE pre_merge_recovery SET state=CASE WHEN attempts>=3 THEN 'blocked' ELSE state END,blocker=$2,receipts=receipts||jsonb_build_array(jsonb_build_object('time',$3::bigint,'attempt',attempts,'error',$2::text)) WHERE id=$1 AND state='pending'")
        .bind(id).bind(reason).bind(now).execute(pool).await?;
    Ok(())
}

#[cfg(test)]
#[path = "../tests/unit/pre_merge_recovery_worker.rs"]
mod tests;
