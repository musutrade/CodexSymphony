use codexsymphony_server::{
    config::Config, coordinator::Coordinator, process, run_store, security::RequestPolicy,
};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{error::Error, time::Duration};
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

type StartupError = Box<dyn Error + Send + Sync>;

fn main() -> Result<(), StartupError> {
    // Enter the single-threaded helper before constructing any Tokio runtime.
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--supervise")) {
        let directory = std::env::args_os().nth(2).ok_or("Run directory required")?;
        return Ok(process::supervise(std::path::Path::new(&directory))?);
    }
    serve()
}

#[tokio::main]
async fn serve() -> Result<(), StartupError> {
    initialize_logging();
    let config = Config::from_env()?;
    // A fixed host path deliberately cannot be changed per cwd/database/port.
    // Never unlink the lock inode on shutdown: another instance may hold it.
    let _instance =
        process::InstanceLock::acquire(std::path::Path::new("/tmp/codexsymphony-controller.lock"))?;
    let pool = connect(&config.database_url).await?;
    sqlx::migrate!("../../migrations").run(&pool).await?;
    let (listener, policy) = listen(config).await?;
    let worker = start_coordinator(&pool).await?;
    tracing::info!(
        "CodexSymphony API listening at http://{}",
        listener.local_addr()?
    );
    axum::serve(listener, codexsymphony_server::router(pool, policy))
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    worker.abort();
    Ok(())
}

async fn listen(config: Config) -> Result<(TcpListener, RequestPolicy), StartupError> {
    let listener = TcpListener::bind(config.bind_address).await?;
    let policy = RequestPolicy::new(listener.local_addr()?, config.web_origin)?;
    Ok((listener, policy))
}

async fn start_coordinator(pool: &PgPool) -> Result<tokio::task::JoinHandle<()>, StartupError> {
    let root = std::path::PathBuf::from(
        std::env::var("EXECUTION_DIRECTORY").unwrap_or(".local-data/execution".into()),
    );
    let incarnation = process::new_identity()?;
    run_store::begin_incarnation(pool, &incarnation).await?;
    Ok(tokio::spawn(coordinate(Coordinator::new(
        pool.clone(),
        root,
        incarnation,
    ))))
}

async fn coordinate(mut coordinator: Coordinator) {
    let blocker = coordinator.coding_blocker();
    tracing::info!("coding remains disabled: {}", blocker);
    loop {
        coordinator.tick();
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
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
