//! Shared quota/resource reservation and ordinary Runtime preparation for repairs.
use crate::{
    execution::Launch, git_broker::GitBroker, runtime_resume::Job, runtime_service::Config,
};
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Transaction};
use std::path::Path;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub(crate) async fn tick(
    pool: &PgPool,
    root: &Path,
    broker: &GitBroker,
    incarnation: &str,
    config: &Config,
) -> Result<()> {
    reserve(pool, broker, incarnation, config).await?;
    let Some((id, mut job)) = pending_job(pool).await? else {
        return Ok(());
    };
    if job.launch.key.incarnation != incarnation {
        job = rebind(pool, broker, incarnation, config, &id, job).await?;
    }
    prepare_job(pool, root, broker, config, &job).await
}
async fn pending_job(pool: &PgPool) -> Result<Option<(String, Job)>> {
    let row: Option<(String, Value, Value, Value, String)> = sqlx::query_as("SELECT f.id,p.launch,p.workspace,f.manifest,f.source_run FROM repair_reservation p JOIN linked_failure f ON f.id=p.linked_failure_id WHERE p.status='reserved' ORDER BY f.created_at LIMIT 1").fetch_optional(pool).await?;
    let Some((id, launch, workspace, manifest, source)) = row else {
        return Ok(None);
    };
    Ok(Some((
        id,
        serde_json::from_value(
            json!({"source":source,"launch":launch,"workspace":workspace,"manifest":manifest}),
        )?,
    )))
}
async fn prepare_job(
    pool: &PgPool,
    root: &Path,
    broker: &GitBroker,
    config: &Config,
    job: &Job,
) -> Result<()> {
    if !restore_allowed(pool, job).await? {
        return Ok(());
    }
    restore(broker, job)?;
    if bind(pool, &job.launch).await? {
        return Ok(());
    }
    if crate::preparation_service::prepare(
        pool,
        crate::preparation_service::Request {
            launch: &job.launch,
            requirement: job.workspace.requirement,
            revision: job.workspace.revision,
            phase: "linked_code_repair",
            now: crate::runtime_client::now(),
            adapter: &config.preparation_adapter,
            control_directory: root,
            broker,
            workspace: &job.workspace,
            config: config.preparation.clone(),
        },
    )
    .await?
    {
        bind(pool, &job.launch).await?;
    }
    Ok(())
}
async fn restore_allowed(pool: &PgPool, job: &Job) -> Result<bool> {
    let mut tx = crate::run_store::lock(pool).await?;
    if !preparation_allowed(&mut tx, &job.launch).await? {
        return Ok(false);
    }
    tx.commit().await?;
    crate::storage_service::admit_workspace(pool, &job.workspace).await
}
fn restore(broker: &GitBroker, job: &Job) -> Result<()> {
    if !Path::new(&job.workspace.path).exists() {
        broker.restore_candidate(&job.workspace, &job.manifest)?;
    }
    crate::budget_store::require(
        broker.head(&job.workspace)? == job.workspace.baseline,
        "linked repair baseline drift",
    )?;
    Ok(())
}
type Source = (String, i64, i64, String, Value);
async fn reserve(
    pool: &PgPool,
    broker: &GitBroker,
    incarnation: &str,
    config: &Config,
) -> Result<()> {
    let mut tx = crate::run_store::lock(pool).await?;
    let row:Option<Source>=sqlx::query_as("SELECT f.id,f.requirement_id,f.revision,f.source_run,f.manifest FROM linked_failure f WHERE f.state='observed' AND f.baseline IS NOT NULL ORDER BY f.created_at LIMIT 1").fetch_optional(&mut *tx).await?;
    let Some(source) = row else {
        return Ok(());
    };
    if !allowed(&mut tx, &source.0, incarnation).await? {
        return Ok(());
    }
    reserve_admitted(tx, source, broker, incarnation, config).await
}
async fn reserve_admitted(
    mut tx: Transaction<'_, Postgres>,
    source: Source,
    broker: &GitBroker,
    incarnation: &str,
    config: &Config,
) -> Result<()> {
    let Some(ordinal) = quota(&mut tx, &source.0, source.1, config.settings.reservation).await?
    else {
        tx.commit().await?;
        return Ok(());
    };
    let job = make_job(&source, broker, incarnation, config)?;
    commit_reservation(tx, &source.0, ordinal, &job, config).await
}
fn make_job(
    source: &Source,
    broker: &GitBroker,
    incarnation: &str,
    config: &Config,
) -> Result<Job> {
    crate::runtime_resume::create(
        source.3.clone(),
        serde_json::from_value(source.4.clone())?,
        broker,
        incarnation,
        source.1,
        source.2,
        &config.launcher()?,
    )
}
async fn quota(
    tx: &mut Transaction<'_, Postgres>,
    id: &str,
    requirement: i64,
    resources: crate::budget::Amount,
) -> Result<Option<i64>> {
    let ordinal: i64 = sqlx::query_scalar(
        "SELECT COALESCE(max(ordinal),0)+1 FROM repair_reservation WHERE requirement_id=$1",
    )
    .bind(requirement)
    .fetch_one(&mut **tx)
    .await?;
    let limit:Option<i64>=sqlx::query_scalar("SELECT repair_limit FROM repair_authorization WHERE requirement_id=$1 AND policy='bounded_v1'").bind(requirement).fetch_optional(&mut **tx).await?;
    if limit.is_none_or(|limit| ordinal > limit)
        || !crate::budget_store::reservation_allowed(tx, requirement, resources).await?
    {
        sqlx::query("UPDATE linked_failure SET state='blocked',blocker='shared item or parent repair budget exhausted' WHERE id=$1").bind(id).execute(&mut **tx).await?;
        return Ok(None);
    }
    Ok(Some(ordinal))
}
async fn commit_reservation(
    mut tx: Transaction<'_, Postgres>,
    id: &str,
    ordinal: i64,
    job: &Job,
    config: &Config,
) -> Result<()> {
    if !crate::storage_service::reserve_workspace(&mut tx, &job.workspace).await? {
        return Ok(());
    }
    sqlx::query("INSERT INTO repair_reservation(requirement_id,ordinal,linked_failure_id,failure,status,launch,workspace,resources) SELECT $2,$3,id,jsonb_build_object('original_failure',id,'evidence',evidence,'authorized_paths',paths,'baseline',baseline,'instruction','Repair only the authorized paths in this repository. Preserve the original task. Report a blocker if another scope is needed.'),'reserved',$4,$5,$6 FROM linked_failure WHERE id=$1")
        .bind(id).bind(job.workspace.requirement).bind(ordinal).bind(json!(job.launch)).bind(json!(job.workspace)).bind(json!(config.settings.reservation)).execute(&mut *tx).await?;
    sqlx::query("UPDATE linked_failure SET state='reserved' WHERE id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE requirement SET state='Running' WHERE id=$1 AND state='Submitted'")
        .bind(job.workspace.requirement)
        .execute(&mut *tx)
        .await?;

    dependency_input(&mut tx, job.workspace.requirement, &job.workspace.baseline).await?;
    crate::group_budget::sync(&mut tx, job.workspace.requirement).await?;
    tx.commit().await?;
    Ok(())
}
async fn dependency_input(
    tx: &mut Transaction<'_, Postgres>,
    requirement: i64,
    baseline: &str,
) -> Result<()> {
    let dependencies = crate::group_completion::dependencies(tx, requirement)
        .await?
        .ok_or("original dependencies unavailable")?;
    sqlx::query("INSERT INTO group_claim_input(requirement_id,authorization_id,baseline,dependencies) SELECT requirement_id,authorization_id,$2,$3 FROM group_execution_item WHERE requirement_id=$1 ON CONFLICT DO NOTHING")
        .bind(requirement).bind(baseline).bind(json!(dependencies)).execute(&mut **tx).await?;

    Ok(())
}
pub(crate) async fn allowed(
    tx: &mut Transaction<'_, Postgres>,
    id: &str,
    incarnation: &str,
) -> std::result::Result<bool, sqlx::Error> {
    let requirement: Option<i64> = sqlx::query_scalar("SELECT f.requirement_id FROM linked_failure f JOIN requirement r ON r.id=f.requirement_id JOIN execution_control c ON c.requirement_id=r.id JOIN repository repo ON repo.id=f.repository_id WHERE f.id=$1 AND f.state IN ('observed','reserved') AND r.revision=f.revision AND r.state IN ('Running','Submitted') AND NOT r.paused AND NOT r.cancel_requested AND NOT c.paused AND c.recovery_complete AND c.incarnation=$2 AND repo.version=(f.document->>'repository_version')::bigint AND repo.document=f.document->'repository' AND NOT (repo.document->>'revoked')::boolean AND NOT (SELECT blocked FROM storage_guard WHERE id=1) AND NOT EXISTS(SELECT 1 FROM agent_run WHERE NOT quiescent) AND NOT EXISTS(SELECT 1 FROM integration_validation WHERE NOT quiescent) AND NOT EXISTS(SELECT 1 FROM workspace_operation WHERE status<>'complete') AND NOT EXISTS(SELECT 1 FROM runtime_blocker b JOIN agent_run a ON a.id=b.run_id WHERE a.requirement_id=r.id AND NOT b.resolved) AND NOT EXISTS(SELECT 1 FROM repair_reservation p WHERE p.requirement_id=r.id AND p.status IN ('reserved','started') AND p.linked_failure_id IS DISTINCT FROM f.id)")
        .bind(id).bind(incarnation).fetch_optional(&mut **tx).await?;
    let Some(requirement) = requirement else {
        return Ok(false);
    };
    let balance = crate::budget_store::balance(tx, requirement).await?;
    Ok(!balance.exhausted
        && balance.exposure.fits(balance.limits)
        && crate::group_queue_store::authorized(tx, requirement).await?
        && crate::group_budget::prepaid_fits(tx, requirement).await?)
}
pub(crate) async fn preparation_allowed(
    tx: &mut Transaction<'_, Postgres>,
    launch: &Launch,
) -> std::result::Result<bool, sqlx::Error> {
    let id: Option<String> = sqlx::query_scalar("SELECT linked_failure_id FROM repair_reservation WHERE launch=$1 AND status='reserved' AND linked_failure_id IS NOT NULL").bind(json!(launch)).fetch_optional(&mut **tx).await?;
    match id {
        Some(id) => allowed(tx, &id, &launch.key.incarnation).await,
        None => Ok(false),
    }
}
async fn bind(pool: &PgPool, launch: &Launch) -> Result<bool> {
    let mut tx = crate::run_store::lock(pool).await?;
    if !preparation_allowed(&mut tx, launch).await? {
        return Ok(false);
    }
    let (id, revision): (i64,i64) = sqlx::query_as("SELECT p.requirement_id,f.revision FROM repair_reservation p JOIN linked_failure f ON f.id=p.linked_failure_id WHERE p.launch=$1 AND p.status='reserved'").bind(json!(launch)).fetch_one(&mut *tx).await?;
    if !crate::preparation_store::claim_ready(&mut tx, launch, id, revision).await? {
        return Ok(false);
    }
    bind_ready(tx, id, revision, launch).await
}
async fn bind_ready(
    mut tx: Transaction<'_, Postgres>,
    id: i64,
    revision: i64,
    launch: &Launch,
) -> Result<bool> {
    crate::run_store::insert_run(&mut tx, id, revision, launch).await?;
    sqlx::query("INSERT INTO run_workspace(run_id,identity,restored_from) SELECT $1,p.workspace,f.source_run FROM repair_reservation p JOIN linked_failure f ON f.id=p.linked_failure_id WHERE p.launch=$2 ON CONFLICT DO NOTHING").bind(&launch.key.run_id).bind(json!(launch)).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO linked_run_input(run_id,failure_id,document) SELECT $1,f.id,f.document FROM repair_reservation p JOIN linked_failure f ON f.id=p.linked_failure_id WHERE p.launch=$2 ON CONFLICT DO NOTHING").bind(&launch.key.run_id).bind(json!(launch)).execute(&mut *tx).await?;
    sqlx::query("UPDATE repair_reservation SET status='started',repair_run_id=$2 WHERE launch=$1")
        .bind(json!(launch))
        .bind(&launch.key.run_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(true)
}
async fn rebind(
    pool: &PgPool,
    broker: &GitBroker,
    incarnation: &str,
    config: &Config,
    id: &str,
    old: Job,
) -> Result<Job> {
    let mut tx = crate::run_store::lock(pool).await?;
    if !allowed(&mut tx, id, incarnation).await? {
        return Ok(old);
    }
    let job = crate::runtime_resume::create(
        old.source,
        old.manifest,
        broker,
        incarnation,
        old.workspace.requirement,
        old.workspace.revision,
        &config.launcher()?,
    )?;
    save_rebound(tx, id, &job).await?;
    Ok(job)
}
async fn save_rebound(mut tx: Transaction<'_, Postgres>, id: &str, job: &Job) -> Result<()> {
    if !crate::storage_service::reserve_workspace(&mut tx, &job.workspace).await? {
        return Err("repair storage unavailable".into());
    }
    sqlx::query("INSERT INTO repair_intent_history(requirement_id,ordinal,launch,workspace,reason) SELECT requirement_id,ordinal,launch,workspace,'linked repair cold-start; no Run dispatched' FROM repair_reservation WHERE linked_failure_id=$1 AND status='reserved'").bind(id).execute(&mut *tx).await?;
    sqlx::query("UPDATE repair_reservation SET launch=$2,workspace=$3 WHERE linked_failure_id=$1 AND status='reserved'").bind(id).bind(json!(job.launch)).bind(json!(job.workspace)).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

pub(crate) async fn candidate_allowed(
    pool: &PgPool,
    broker: &GitBroker,
    run: &str,
    manifest: &crate::workspace::Manifest,
) -> Result<bool> {
    let row: Option<(String,String,Value)> = sqlx::query_as("SELECT f.id,f.baseline,f.paths FROM linked_failure f JOIN agent_run a ON a.requirement_id=f.requirement_id WHERE a.id=$1 AND f.state='reserved' ORDER BY f.created_at DESC LIMIT 1").bind(run).fetch_optional(pool).await?;
    let Some((id, baseline, paths)) = row else {
        return Ok(true);
    };
    let allowed: Vec<String> = serde_json::from_value(paths)?;
    let changed = broker.changed_paths(&manifest.workspace, &baseline, &manifest.head)?;
    let reason = if changed.is_empty() {
        Some("no progress: repair produced no source change")
    } else if changed.iter().any(|path| !allowed.contains(path)) {
        Some("repair changes exceed original authorized paths")
    } else {
        None
    };
    if let Some(reason) = reason {
        sqlx::query(
            "UPDATE linked_failure SET state='blocked',blocker=$2 WHERE id=$1 AND state='reserved'",
        )
        .bind(id)
        .bind(reason)
        .execute(pool)
        .await?;
        return Ok(false);
    }
    Ok(true)
}

#[cfg(test)]
#[path = "../tests/unit/linked_repair_worker.rs"]
pub(crate) mod tests;
