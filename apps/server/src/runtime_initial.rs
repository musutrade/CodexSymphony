//! Minimal Ready-to-Run bridge. Deployment supplies a fixed local baseline;
//! preparation, repository authorization and queue admission stay authoritative.
use crate::{
    execution::{Launch, RunKey},
    git_broker::GitBroker,
    process, run_store,
    runtime_service::Config,
    workspace::Workspace,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::path::Path;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub async fn tick(
    pool: &PgPool,
    root: &Path,
    broker: &GitBroker,
    incarnation: &str,
    config: &Config,
) -> Result<()> {
    let Some(baseline) = config.preparation["baseline"].as_str() else {
        return Ok(());
    };
    let Some((launch, workspace)) =
        plan(pool, broker, incarnation, &config.launcher()?, baseline).await?
    else {
        return Ok(());
    };
    if !crate::storage_service::admit_workspace(pool, &workspace).await? {
        return Ok(());
    }
    prepare_worktree(broker, &workspace)?;
    prepare_run(pool, root, broker, config, &launch, &workspace).await
}
fn prepare_worktree(broker: &GitBroker, workspace: &Workspace) -> Result<()> {
    if !Path::new(&workspace.path).exists() {
        broker.prepare(workspace, true)?;
    }
    if broker.head(workspace)? != workspace.baseline {
        return Err("initial worktree changed; preserve and reconcile".into());
    }
    Ok(())
}
async fn prepare_run(
    pool: &PgPool,
    root: &Path,
    broker: &GitBroker,
    config: &Config,
    launch: &Launch,
    workspace: &Workspace,
) -> Result<()> {
    if run_store::reserve_prepared(pool, launch).await? {
        return Ok(());
    }
    if crate::preparation_service::prepare(
        pool,
        crate::preparation_service::Request {
            launch,
            requirement: workspace.requirement,
            revision: workspace.revision,
            phase: "preparation",
            now: crate::runtime_client::now(),
            adapter: &config.preparation_adapter,
            control_directory: root,
            broker,
            workspace,
            config: config.preparation.clone(),
        },
    )
    .await?
    {
        run_store::reserve_prepared(pool, launch).await?;
    }
    Ok(())
}

pub async fn plan(
    pool: &PgPool,
    broker: &GitBroker,
    incarnation: &str,
    launcher: &[String],
    baseline: &str,
) -> Result<Option<(Launch, Workspace)>> {
    let mut tx = run_store::lock(pool).await?;
    let Some((requirement, revision)) = eligible(&mut tx, incarnation).await? else {
        return Ok(None);
    };
    if let Some(job) = saved(&mut tx, requirement, revision, incarnation).await? {
        return Ok(Some(job));
    }
    let (launch, workspace) = allocate(
        broker,
        incarnation,
        launcher,
        baseline,
        requirement,
        revision,
    )?;
    sqlx::query("INSERT INTO initial_run VALUES($1,$2,$3,$4)")
        .bind(requirement)
        .bind(revision)
        .bind(json!(launch))
        .bind(json!(workspace))
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Some((launch, workspace)))
}
fn allocate(
    broker: &GitBroker,
    incarnation: &str,
    launcher: &[String],
    baseline: &str,
    requirement: i64,
    revision: i64,
) -> Result<(Launch, Workspace)> {
    let (program, args) = launcher.split_first().ok_or("Runtime launcher missing")?;
    let id = process::new_identity()?;
    let key = RunKey {
        run_id: id.clone(),
        request_id: format!("initial-{id}"),
        incarnation: incarnation.into(),
    };
    let workspace = Workspace {
        key: key.clone(),
        identity: process::new_identity()?,
        requirement,
        revision,
        phase: "execution".into(),
        baseline: baseline.into(),
        branch: format!("ai/req-{requirement}-{id}"),
        path: broker.path(&id)?.to_string_lossy().into_owned(),
    };
    let mut args = args.to_vec();
    args.push("app-server".into());
    let launch = Launch {
        key,
        workspace: workspace.path.clone(),
        workspace_identity: workspace.identity.clone(),
        program: program.clone(),
        args,
    };
    Ok((launch, workspace))
}

async fn eligible(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    incarnation: &str,
) -> Result<Option<(i64, i64)>> {
    if !run_store::claim_allowed(tx, incarnation).await? {
        return Ok(None);
    }
    Ok(run_store::queued(tx).await?)
}
async fn saved(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    requirement: i64,
    revision: i64,
    incarnation: &str,
) -> Result<Option<(Launch, Workspace)>> {
    let saved: Option<(Value, Value)> = sqlx::query_as(
        "SELECT launch,workspace FROM initial_run WHERE requirement_id=$1 AND revision=$2",
    )
    .bind(requirement)
    .bind(revision)
    .fetch_optional(&mut **tx)
    .await?;
    if let Some((launch, workspace)) = saved {
        let launch: Launch = serde_json::from_value(launch)?;
        if launch.key.incarnation != incarnation {
            return Err(
                "initial preparation belongs to prior incarnation; reconcile saved worktree".into(),
            );
        }
        return Ok(Some((launch, serde_json::from_value(workspace)?)));
    }
    Ok(None)
}
