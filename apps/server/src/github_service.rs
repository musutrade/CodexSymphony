//! Optional control-plane configuration and bounded polling, never a model call.
use crate::{github::Policy, github_http::AppClient, github_observe, github_store};
use serde::Deserialize;
use sqlx::PgPool;
use std::{error::Error, path::Path};
type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub app_id: u64,
    pub api_url: Option<String>,
    pub private_key_path: String,
    pub policy: Policy,
    pub probe_pr: u64,
}
pub fn load(path: &Path) -> Result<(Config, AppClient)> {
    let config: Config = serde_json::from_slice(&std::fs::read(path)?)?;
    let pem = std::fs::read(&config.private_key_path)?;
    let client = AppClient::new(
        config
            .api_url
            .as_deref()
            .unwrap_or("https://api.github.com/"),
        config.app_id,
        &pem,
    )?;
    Ok((config, client))
}
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
pub async fn inspect(path: &Path) -> Result<()> {
    let (config, mut client) = load(path)?;
    let capability =
        github_observe::preflight(&mut client, &config.policy, config.probe_pr, now()).await?;
    std::io::Write::write_all(
        &mut std::io::stdout(),
        format!("{}\n", serde_json::to_string_pretty(&capability)?).as_bytes(),
    )?;
    if !capability.blockers.is_empty() {
        return Err("repository capability mismatch".into());
    }
    Ok(())
}
pub async fn start(pool: &PgPool) -> Result<Option<tokio::task::JoinHandle<()>>> {
    let Some(path) = std::env::var_os("GITHUB_APP_CONFIG") else {
        return Ok(None);
    };
    start_path(pool, Path::new(&path)).await.map(Some)
}
pub async fn start_path(pool: &PgPool, path: &Path) -> Result<tokio::task::JoinHandle<()>> {
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
    if value.get("app").is_none() {
        let (config, client) = load(path)?;
        return start_configured(pool, &config, client).await;
    }
    start_multiple(pool, serde_json::from_value(value)?).await
}
async fn start_multiple(
    pool: &PgPool,
    deployment: Deployment,
) -> Result<tokio::task::JoinHandle<()>> {
    let pem = std::fs::read(&deployment.app.private_key_path)?;
    let client = AppClient::new(
        deployment
            .app
            .api_url
            .as_deref()
            .unwrap_or("https://api.github.com/"),
        deployment.app.app_id,
        &pem,
    )?;
    github_store::configure(pool, &deployment.app.policy, deployment.app.probe_pr).await?;
    for repository in deployment.repositories {
        github_store::configure(pool, &repository.policy, repository.probe_pr).await?;
    }
    Ok(tokio::spawn(poll(pool.clone(), client)))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Deployment {
    app: Config,
    repositories: Vec<Repository>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Repository {
    policy: Policy,
    probe_pr: u64,
}

pub async fn start_configured(
    pool: &PgPool,
    config: &Config,
    client: AppClient,
) -> Result<tokio::task::JoinHandle<()>> {
    github_store::configure(pool, &config.policy, config.probe_pr).await?;
    Ok(tokio::spawn(poll(pool.clone(), client)))
}
async fn poll(pool: PgPool, mut client: AppClient) {
    loop {
        if let Err(error) = tick(&pool, &mut client, now()).await {
            tracing::error!("GitHub sync unavailable: {}", error);
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}
pub async fn tick(pool: &PgPool, client: &mut AppClient, now: i64) -> Result<()> {
    let started = tokio::time::Instant::now();
    for (policy, number, failures) in github_store::due_repositories(pool, now).await? {
        let policy: Policy = serde_json::from_value(policy)?;
        // Date evidence at this request's start, never at completion or the
        // start of an earlier repository's slow request. The TTL remains 60s.
        let observed_at = now + started.elapsed().as_secs() as i64;
        match github_observe::preflight(client, &policy, number as u64, observed_at).await {
            Ok(capability) => github_store::save_capability(pool, &capability).await?,
            Err(error) => {
                github_store::failed(
                    pool,
                    policy.repository_id,
                    None,
                    failures,
                    observed_at,
                    &error,
                )
                .await?
            }
        }
    }
    sync_prs(pool, client, now + started.elapsed().as_secs() as i64).await?;
    crate::recovery_remote::tick(pool, client, now + started.elapsed().as_secs() as i64).await?;
    delivery_tick(pool, client, now + started.elapsed().as_secs() as i64).await
}
async fn delivery_tick(pool: &PgPool, client: &mut AppClient, now: i64) -> Result<()> {
    let root = std::path::PathBuf::from(
        std::env::var("EXECUTION_DIRECTORY").unwrap_or(".local-data/execution".into()),
    );
    deliver(pool, client, &root, now).await
}
pub async fn deliver(pool: &PgPool, client: &mut AppClient, root: &Path, now: i64) -> Result<()> {
    crate::delivery_control::settle(pool).await?;
    let jobs = crate::delivery_store::due(pool, now).await?;
    let Some(job) = jobs.first() else {
        return Ok(());
    };
    let broker = crate::git_broker::GitBroker::open(&root.join("workspaces"))?;
    let policy: Option<serde_json::Value> =
        sqlx::query_scalar("SELECT policy FROM github_repository WHERE repository_id=$1")
            .bind(job.repository_id)
            .fetch_optional(pool)
            .await?;
    let Some(policy) = policy else {
        return Ok(());
    };
    let mut remote = crate::delivery_remote::Github {
        client,
        policy: serde_json::from_value(policy)?,
        broker: &broker,
        now,
    };
    crate::delivery_worker::tick(pool, root, &mut remote, now).await
}
async fn sync_prs(pool: &PgPool, client: &mut AppClient, now: i64) -> Result<()> {
    let started = tokio::time::Instant::now();
    for (policy, number, failures) in github_store::due_prs(pool, now).await? {
        let policy: Policy = serde_json::from_value(policy)?;
        let observed_at = now + started.elapsed().as_secs() as i64;
        match github_observe::observe(client, &policy, number as u64, observed_at).await {
            Ok(observation) => {
                github_store::save_observation(pool, &observation).await?;
                crate::recovery_observe::observe(pool, client, &observation).await?;
            }
            Err(error) => {
                github_store::failed(
                    pool,
                    policy.repository_id,
                    Some(number as u64),
                    failures,
                    observed_at,
                    &error,
                )
                .await?
            }
        }
    }
    Ok(())
}
