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
    if std::env::args().nth(1).as_deref() == Some("--delivery-hook") {
        let directory = std::env::args()
            .nth(2)
            .ok_or("delivery hook directory missing")?;
        return codexsymphony_server::delivery_hook_process::run(std::path::Path::new(&directory));
    }
    if std::env::args().nth(1).as_deref() == Some("--validation-hook") {
        let directory = std::env::args_os()
            .nth(2)
            .ok_or("validation directory required")?;
        return codexsymphony_server::validation_supervisor::run(std::path::Path::new(&directory));
    }
    if std::env::args().nth(1).as_deref() == Some("--integration-validation") {
        let directory = std::env::args_os()
            .nth(2)
            .ok_or("validation directory required")?;
        return codexsymphony_server::integration_process::run(std::path::Path::new(&directory));
    }
    serve()
}

#[tokio::main]
async fn serve() -> Result<(), StartupError> {
    if std::env::args().nth(1).as_deref() == Some("budget") {
        return codexsymphony_server::budget_admin::run(
            &std::env::args().skip(2).collect::<Vec<_>>(),
        )
        .await;
    }
    if std::env::args().nth(1).as_deref() == Some("--environment-check") {
        return codexsymphony_server::environment_cli::run(
            &std::env::args().skip(2).collect::<Vec<_>>(),
        )
        .await;
    }
    if std::env::args().nth(1).as_deref() == Some("auth") {
        return codexsymphony_server::auth_admin::run(
            &std::env::args().skip(2).collect::<Vec<_>>(),
        )
        .await;
    }
    if std::env::args().nth(1).as_deref() == Some("--recovery-drill") {
        let config = Config::from_env()?;
        return codexsymphony_server::recovery::serve(&config.database_url, config.bind_address)
            .await;
    }
    initialize_logging();
    let inspect_path = match std::env::args().nth(1).as_deref() {
        Some("--github-inspect") => std::env::args_os().nth(2),
        _ => None,
    };
    if let Some(path) = inspect_path {
        return codexsymphony_server::github_service::inspect(std::path::Path::new(&path)).await;
    }
    run_service(Config::from_env()?).await
}

async fn run_service(config: Config) -> Result<(), StartupError> {
    // A fixed host path deliberately cannot be changed per cwd/database/port.
    // Never unlink the lock inode on shutdown: another instance may hold it.
    let _instance =
        process::InstanceLock::acquire(std::path::Path::new("/tmp/codexsymphony-controller.lock"))?;
    let pool = prepare_database(&config.database_url).await?;
    codexsymphony_server::environment_service::startup(&pool).await?;
    codexsymphony_server::generation_store::recover(&pool)
        .await
        .map_err(|_| std::io::Error::other("generation recovery failed"))?;
    let (listener, policy) = listen(config).await?;
    let (worker, runtime) = start_coordinator(&pool).await?;
    let github = codexsymphony_server::github_service::start(&pool).await?;
    serve_http(listener, pool, policy, runtime).await?;
    stop_workers(worker, github);
    Ok(())
}

async fn listen(config: Config) -> Result<(TcpListener, RequestPolicy), StartupError> {
    let listener = TcpListener::bind(config.bind_address).await?;
    let policy = RequestPolicy::configured(listener.local_addr()?, config.web_origin)?;
    Ok((listener, policy))
}

async fn start_coordinator(
    pool: &PgPool,
) -> Result<
    (
        tokio::task::JoinHandle<()>,
        Option<tokio::task::AbortHandle>,
    ),
    StartupError,
> {
    let root = std::path::PathBuf::from(
        std::env::var("EXECUTION_DIRECTORY").unwrap_or(".local-data/execution".into()),
    );
    let incarnation = process::new_identity()?;
    if std::env::var_os("STORAGE_CONFIG").is_none() {
        std::fs::create_dir_all(&root)?;
    }
    run_store::begin_incarnation(pool, &incarnation).await?;
    let storage = codexsymphony_server::storage_service::start(pool, &root).await?;
    let runtime = codexsymphony_server::runtime_service::start(
        pool.clone(),
        root.clone(),
        incarnation.clone(),
    )?;
    let status = runtime.as_ref().map(tokio::task::JoinHandle::abort_handle);
    Ok((
        tokio::spawn(coordinate(
            Coordinator::new(pool.clone(), root, incarnation),
            runtime,
            storage,
        )),
        status,
    ))
}

async fn coordinate(
    mut coordinator: Coordinator,
    runtime: Option<tokio::task::JoinHandle<()>>,
    storage: Option<tokio::task::JoinHandle<()>>,
) {
    let _runtime = RuntimeWorker(runtime);
    let _storage = RuntimeWorker(storage);
    let blocker = coordinator.coding_blocker();
    tracing::info!("unprepared coding remains disabled: {}", blocker);
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

async fn serve_http(
    listener: TcpListener,
    pool: PgPool,
    policy: RequestPolicy,
    runtime: Option<tokio::task::AbortHandle>,
) -> Result<(), StartupError> {
    let address = listener.local_addr()?;
    tracing::info!("CodexSymphony API listening at http://{}", address);
    let mut app = codexsymphony_server::router(pool, policy);
    if let Some(runtime) = runtime {
        app = app.layer(axum::Extension(runtime));
    }
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;
    Ok(())
}

fn stop_workers(worker: tokio::task::JoinHandle<()>, github: Option<tokio::task::JoinHandle<()>>) {
    worker.abort();
    if let Some(github) = github {
        github.abort();
    }
}

async fn prepare_database(url: &str) -> Result<PgPool, StartupError> {
    let pool = connect(url).await?;
    codexsymphony_server::recovery::refuse_normal_start(&pool).await?;
    sqlx::migrate!("../../migrations").run(&pool).await?;
    Ok(pool)
}

struct RuntimeWorker(Option<tokio::task::JoinHandle<()>>);
impl Drop for RuntimeWorker {
    fn drop(&mut self) {
        if let Some(task) = &self.0 {
            task.abort();
        }
    }
}
