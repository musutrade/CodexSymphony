//! Short tick: at most one asynchronous database/recovery operation in flight.
use crate::{
    execution::{CODING_BLOCKER, Receipt},
    process,
    run_store::{self, Run},
};
use sqlx::PgPool;
use std::path::{Path, PathBuf};
use tokio::task::JoinHandle;

pub struct Coordinator {
    pool: PgPool,
    root: PathBuf,
    incarnation: String,
    pending: Option<JoinHandle<()>>,
}

impl Coordinator {
    pub fn new(pool: PgPool, root: PathBuf, incarnation: String) -> Self {
        Self {
            pool,
            root,
            incarnation,
            pending: None,
        }
    }

    pub fn tick(&mut self) {
        if let Some(task) = &self.pending
            && !task.is_finished()
        {
            return;
        }
        let pool = self.pool.clone();
        let root = self.root.clone();
        let incarnation = self.incarnation.clone();
        self.pending = Some(tokio::spawn(async move {
            if let Err(error) = recover(&pool, &root, &incarnation).await {
                tracing::error!("execution recovery remains blocked: {}", error);
            }
        }));
    }

    pub fn coding_blocker(&self) -> &'static str {
        CODING_BLOCKER
    }
}

/// Called only after acquiring the process-lifetime instance lock. The HTTP
/// service remains responsive while old execution facts are reconciled.
pub async fn recover(pool: &PgPool, root: &Path, incarnation: &str) -> Result<bool, sqlx::Error> {
    for run in run_store::unresolved(pool).await? {
        if run.incarnation != incarnation || run.stop_requested {
            recover_run(pool, root, run).await?;
        } else {
            observe(pool, root, run).await?;
        }
    }
    match crate::workspace_store::recover_stopped(pool, &root.join("workspaces")).await {
        Ok(true) => {}
        Ok(false) => return Ok(false),
        Err(_) => {
            return Err(sqlx::Error::Protocol(
                "workspace preservation requires reconciliation".into(),
            ));
        }
    }
    run_store::finish_recovery(pool, incarnation).await
}

async fn observe(pool: &PgPool, root: &Path, run: Run) -> Result<(), sqlx::Error> {
    let Ok(directory) = process::run_directory(root, &run.id) else {
        return Ok(());
    };
    if let Ok(receipt) = process::read(&directory.join("quiescent.json")) {
        run_store::confirm_quiescent(pool, &run, &receipt).await?;
    } else if let Some(expected) = run.process()
        && process::identity(expected.pid).ok().as_ref() != Some(&expected)
    {
        run_store::block(
            pool,
            &run.id,
            "supervisor lost; descendant identity unknown",
        )
        .await?;
    }
    Ok(())
}

impl Drop for Coordinator {
    fn drop(&mut self) {
        if let Some(task) = &self.pending {
            task.abort();
        }
    }
}

/// A caller must reserve a fully prepared Run first. This performs a parked
/// supervisor handshake: no writer starts before its identity is in PostgreSQL.
pub async fn start_reserved(
    pool: &PgPool,
    root: &Path,
    supervisor: &Path,
    launch: &crate::execution::Launch,
) -> Result<std::process::Child, Box<dyn std::error::Error + Send + Sync>> {
    if !run_store::reserved_launch(pool, launch).await? {
        return Err("Run is not authorized to start".into());
    }
    let directory = process::run_directory(root, &launch.key.run_id)?;
    let worker_directory = directory.clone();
    let supervisor = supervisor.to_owned();
    let copy = launch.clone();
    let child =
        tokio::task::spawn_blocking(move || process::spawn(&supervisor, &worker_directory, &copy))
            .await??;
    finish_launch(pool, &directory, launch, child).await
}

async fn finish_launch(
    pool: &PgPool,
    directory: &Path,
    launch: &crate::execution::Launch,
    mut child: std::process::Child,
) -> Result<std::process::Child, Box<dyn std::error::Error + Send + Sync>> {
    match handshake(pool, directory, launch).await {
        Ok(()) => Ok(child),
        Err(error) => {
            let _ = run_store::block(
                pool,
                &launch.key.run_id,
                "launch handshake failed; stop proof required",
            )
            .await;
            let _ = process::durable_write(&directory.join("stop.json"), &launch.key);
            // Reap the helper even when the caller receives a failed handshake.
            // Only its durable receipt, never this wait result, opens recovery.
            tokio::task::spawn_blocking(move || child.wait());
            Err(error)
        }
    }
}

async fn handshake(
    pool: &PgPool,
    directory: &Path,
    launch: &crate::execution::Launch,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let receipt = wait_identity(directory).await?;
    permit(pool, directory, launch, &receipt).await
}

async fn permit(
    pool: &PgPool,
    directory: &Path,
    launch: &crate::execution::Launch,
    receipt: &Receipt,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if receipt.key != launch.key {
        return Err("supervisor identity mismatch".into());
    }
    if !run_store::attach_process(pool, receipt).await? {
        return Err("process identity was already recorded".into());
    }
    grant_start(pool, directory, &launch.key).await
}

async fn grant_start(
    pool: &PgPool,
    directory: &Path,
    key: &crate::execution::RunKey,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Keep pause/revocation/rollover serialized until the filesystem permission
    // is durable. A pause committed first cannot be followed by a new grant.
    let tx = run_store::lock(pool).await?;
    if run_store::actions_allowed(pool, key).await? {
        process::durable_write(&directory.join("start.json"), key)?;
    } else {
        process::durable_write(&directory.join("stop.json"), key)?;
    }
    tx.commit().await?;
    Ok(())
}

async fn wait_identity(directory: &Path) -> std::io::Result<Receipt> {
    // The helper fsyncs its one-use claim before publishing identity. Real disk
    // contention can take several seconds; keep a bounded 15-second window.
    // No start permission is issued until identity is persisted and authorized.
    for _ in 0..750 {
        if let Ok(receipt) = process::read(&directory.join("identity.json")) {
            return Ok(receipt);
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    Err(std::io::Error::other(
        "supervisor identity unavailable; retain launch intent",
    ))
}

async fn recover_run(pool: &PgPool, root: &Path, mut run: Run) -> Result<(), sqlx::Error> {
    run_store::block(pool, &run.id, "awaiting complete descendant stop proof").await?;
    let root = root.to_owned();
    let copy = run.clone();
    let evidence = tokio::task::spawn_blocking(move || stop_evidence(&root, &copy)).await;
    let Ok(Ok((identity, stopped))) = evidence else {
        return run_store::block(
            pool,
            &run.id,
            "process identity or stop evidence unavailable",
        )
        .await;
    };
    if run.process_identity.is_none() {
        run_store::attach_process(pool, &identity).await?;
        run.process_identity = serde_json::to_value(&identity.process).ok();
    }
    if let Some(receipt) = stopped {
        run_store::confirm_quiescent(pool, &run, &receipt).await?;
    }
    Ok(())
}

fn stop_evidence(root: &Path, run: &Run) -> std::io::Result<(Receipt, Option<Receipt>)> {
    let directory = process::run_directory(root, &run.id)?;
    // A start-record gap is blocked too. Stop is durable before inspecting PID.
    std::fs::create_dir_all(&directory)?;
    process::durable_write(&directory.join("stop.json"), &run.key())?;
    let identity: Receipt = process::read(&directory.join("identity.json"))?;
    if identity.key != run.key() {
        return Err(std::io::Error::other("Run identity mismatch"));
    }
    let stopped = process::read(&directory.join("quiescent.json")).ok();
    Ok((identity, stopped))
}
