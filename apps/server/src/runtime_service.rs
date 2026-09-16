//! Optional host-configured Runtime worker. HTTP never supplies executable paths.
use crate::{execution::Launch, git_broker::GitBroker, runtime_client, runtime_resume};
use serde::Deserialize;
use serde_json::Value;
use sqlx::PgPool;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub validation: Option<crate::validation_runner::Plan>,
    pub settings: runtime_client::Settings,
    pub preparation_adapter: PathBuf,
    pub preparation: Value,
}
impl Config {
    pub fn launcher(&self) -> Result<Vec<String>> {
        self.settings.validate()?;
        let launcher: Vec<String> = serde_json::from_value(self.preparation["launcher"].clone())?;
        if launcher.is_empty()
            || !Path::new(&launcher[0]).is_absolute()
            || !self.preparation_adapter.is_absolute()
        {
            return Err(
                "absolute operator-owned Runtime launcher and preparation adapter required".into(),
            );
        }
        Ok(launcher)
    }
}
pub fn start(
    pool: PgPool,
    root: PathBuf,
    incarnation: String,
) -> Result<Option<tokio::task::JoinHandle<()>>> {
    let Some(path) = std::env::var_os("RUNTIME_CONFIG") else {
        return Ok(None);
    };
    let config: Config = serde_json::from_slice(&std::fs::read(path)?)?;
    config.launcher()?;
    let supervisor = std::env::current_exe()?;
    let broker = GitBroker::open(&root.join("workspaces"))?;
    Ok(Some(tokio::spawn(async move {
        loop {
            if tick(&pool, &root, &supervisor, &broker, &incarnation, &config)
                .await
                .is_err()
            {
                // Detailed failures remain in their bounded, Run-owned records.
                tracing::error!("Runtime execution or answer recovery requires reconciliation");
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    })))
}
pub async fn tick(
    pool: &PgPool,
    root: &Path,
    supervisor: &Path,
    broker: &GitBroker,
    incarnation: &str,
    config: &Config,
) -> Result<()> {
    if let Some(launch) = reserved(pool, incarnation).await? {
        return runtime_client::execute(pool, root, supervisor, broker, &launch, &config.settings)
            .await;
    }
    if let Some(plan) = &config.validation
        && crate::validation_worker::tick(pool, root, broker, plan).await?
    {
        return Ok(());
    }
    if config.validation.is_some() {
        crate::validation_repair_worker::tick(pool, root, broker, incarnation, config).await?;
    }
    resume(pool, root, supervisor, broker, incarnation, config).await
}
async fn resume(
    pool: &PgPool,
    root: &Path,
    supervisor: &Path,
    broker: &GitBroker,
    incarnation: &str,
    config: &Config,
) -> Result<()> {
    let Some(job) = runtime_resume::next(pool, broker, incarnation, &config.launcher()?).await?
    else {
        return Ok(());
    };
    if !runtime_resume::prepare(
        pool,
        root,
        broker,
        &job,
        &config.preparation_adapter,
        config.preparation.clone(),
    )
    .await?
    {
        return Ok(());
    }
    if runtime_resume::bind(pool, &job).await? {
        runtime_client::execute(
            pool,
            root,
            supervisor,
            broker,
            &job.launch,
            &config.settings,
        )
        .await?;
    }
    Ok(())
}
async fn reserved(pool: &PgPool, incarnation: &str) -> Result<Option<Launch>> {
    // Initial reservations still require the upstream prepared-Run boundary.
    // A lost worker cannot replay a Run whose Runtime session was already opened.
    let value: Option<Value> = sqlx::query_scalar("SELECT a.launch FROM agent_run a JOIN run_workspace w ON w.run_id=a.id WHERE a.incarnation=$1 AND a.state='Created' AND NOT a.stop_requested AND NOT a.quiescent AND NOT EXISTS(SELECT 1 FROM runtime_session s WHERE s.run_id=a.id) ORDER BY a.run_sequence LIMIT 1")
        .bind(incarnation).fetch_optional(pool).await?;
    value
        .map(serde_json::from_value)
        .transpose()
        .map_err(Into::into)
}
