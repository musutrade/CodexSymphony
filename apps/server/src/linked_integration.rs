//! Rebuild all required integration checks on the new exact repository combination.
use crate::{execution::Launch, git_broker::GitBroker, integration_process::Job};
use serde_json::Value;
use sqlx::PgPool;
use std::path::Path;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
type Tx<'a> = sqlx::Transaction<'a, sqlx::Postgres>;
#[derive(sqlx::FromRow)]
struct Repair {
    failure: String,
    repository: i64,
    version: Value,
    previous: String,
    job: Value,
    launch: Value,
}
pub(crate) async fn tick(
    pool: &PgPool,
    root: &Path,
    supervisor: &Path,
    broker: &GitBroker,
    incarnation: &str,
) -> Result<()> {
    let mut tx = crate::run_store::lock(pool).await?;
    let row: Option<Repair> = sqlx::query_as("SELECT f.id AS failure,f.repository_id AS repository,f.final_version AS version,v.id AS previous,v.job,v.launch FROM linked_failure f JOIN execution_control c ON c.requirement_id=f.requirement_id JOIN LATERAL (SELECT * FROM integration_validation WHERE requirement_id=f.requirement_id ORDER BY created_at DESC,id DESC LIMIT 1) v ON true WHERE f.state='merged' AND f.revalidation_id IS NULL AND v.quiescent AND v.state='failed' AND NOT EXISTS(SELECT 1 FROM linked_failure x WHERE x.requirement_id=f.requirement_id AND x.state IN ('observed','reserved','blocked')) ORDER BY f.created_at DESC LIMIT 1").fetch_optional(&mut *tx).await?;

    let Some(repair) = row else {
        return Ok(());
    };
    rerun(tx, root, supervisor, broker, incarnation, repair).await
}
async fn rerun(
    mut tx: Tx<'_>,
    root: &Path,
    supervisor: &Path,
    broker: &GitBroker,
    incarnation: &str,
    repair: Repair,
) -> Result<()> {
    let mut job: Job = serde_json::from_value(repair.job.clone())?;
    if !crate::integration_store::allowed(&mut tx, &job).await? {
        return Ok(());
    }
    update_version(&mut job, &repair)?;
    if !checkout_all(&mut tx, broker, incarnation, &mut job).await? {
        return Ok(());
    }
    let launch = launch(root, supervisor, incarnation, &job, repair.launch.clone())?;
    persist(tx, &repair, &job, &launch).await
}
fn update_version(job: &mut Job, repair: &Repair) -> Result<()> {
    let candidate: crate::validation::Candidate =
        serde_json::from_value(repair.version["candidate"].clone())?;
    let version = job
        .binding
        .versions
        .iter_mut()
        .find(|v| v.repository_id == repair.repository)
        .ok_or("repair repository missing from original integration binding")?;
    version.candidate = candidate;
    version
        .artifacts
        .push(format!("linked-failure:{}", repair.failure));
    job.invocation = format!("integration-{}", crate::process::new_identity()?);
    Ok(())
}
async fn checkout_all(
    tx: &mut Tx<'_>,
    broker: &GitBroker,
    incarnation: &str,
    job: &mut Job,
) -> Result<bool> {
    let mut checkouts = Vec::new();
    for version in &job.binding.versions {
        let workspace = workspace(broker, incarnation, job, version)?;
        if !checkout(tx, broker, &workspace, version).await? {
            return Ok(false);
        }
        checkouts.push(workspace.path.into());
    }
    job.checkouts = checkouts;
    Ok(true)
}
fn workspace(
    broker: &GitBroker,
    incarnation: &str,
    job: &Job,
    version: &crate::integration::Version,
) -> Result<crate::workspace::Workspace> {
    let id = format!("{}-repo-{}", job.invocation, version.repository_id);
    Ok(crate::workspace::Workspace {
        key: crate::execution::RunKey {
            run_id: id.clone(),
            request_id: id.clone(),
            incarnation: incarnation.into(),
        },
        identity: id.clone(),
        requirement: job.binding.requirement,
        revision: job.binding.revision,
        phase: "integration".into(),
        baseline: version.candidate.sha.clone(),
        branch: format!("ai/req-{}-{id}", job.binding.requirement),
        path: broker.path(&id)?.to_string_lossy().into_owned(),
    })
}
async fn checkout(
    tx: &mut Tx<'_>,
    broker: &GitBroker,
    workspace: &crate::workspace::Workspace,
    version: &crate::integration::Version,
) -> Result<bool> {
    if !crate::storage_service::reserve_workspace(tx, workspace).await? {
        return Ok(false);
    }
    broker.prepare(workspace, true)?;
    crate::budget_store::require(
        crate::validation_runner::candidate(Path::new(&workspace.path))? == version.candidate,
        "updated integration checkout mismatch",
    )?;
    Ok(true)
}
fn launch(
    root: &Path,
    supervisor: &Path,
    incarnation: &str,
    job: &Job,
    value: Value,
) -> Result<Launch> {
    let mut launch: Launch = serde_json::from_value(value)?;
    let id = &job.invocation;
    launch.key.run_id = id.clone();
    launch.key.request_id = id.clone();
    launch.key.incarnation = incarnation.into();
    launch.workspace = job
        .checkouts
        .first()
        .ok_or("integration version set empty")?
        .to_string_lossy()
        .into_owned();
    launch.workspace_identity = id.clone();
    launch.program = supervisor.to_string_lossy().into_owned();
    launch.args = Vec::from([
        "--integration-validation".into(),
        root.join(id).to_string_lossy().into_owned(),
    ]);
    Ok(launch)
}
async fn persist(mut tx: Tx<'_>, repair: &Repair, job: &Job, launch: &Launch) -> Result<()> {
    crate::integration_claim::persist(&mut tx, &job.invocation, job, launch).await?;
    sqlx::query("UPDATE linked_failure SET revalidation_id=$2 WHERE id=$1")
        .bind(&repair.failure)
        .bind(&job.invocation)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE integration_validation SET blocker='updated exact-version validation scheduled: '||$2 WHERE id=$1").bind(&repair.previous).bind(&job.invocation).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
#[path = "../tests/unit/linked_integration.rs"]
mod tests;
