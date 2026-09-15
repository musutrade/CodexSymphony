use codexsymphony_server::{config::Config, security::RequestPolicy};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{error::Error, time::Duration};
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

type StartupError = Box<dyn Error + Send + Sync>;

#[tokio::main]
async fn main() -> Result<(), StartupError> {
    initialize_logging();
    let config = Config::from_env()?;
    let pool = connect(&config.database_url).await?;
    sqlx::migrate!("../../migrations").run(&pool).await?;
    let listener = TcpListener::bind(config.bind_address).await?;
    let policy = RequestPolicy::new(listener.local_addr()?, config.web_origin)?;
    tracing::info!(
        "CodexSymphony API listening at http://{}",
        listener.local_addr()?
    );
    axum::serve(listener, codexsymphony_server::router(pool, policy))
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

fn initialize_logging() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(default_logging))
        .init();
}

fn default_logging(_: tracing_subscriber::filter::FromEnvError) -> EnvFilter {
    EnvFilter::new("info")
}

async fn connect(database_url: &str) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(2))
        .connect(database_url)
        .await
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
