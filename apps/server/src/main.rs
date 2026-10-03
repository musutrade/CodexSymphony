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
    if let Some(result) = host_admin().await {
        return result;
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

async fn host_admin() -> Option<Result<(), StartupError>> {
    let arguments = std::env::args().skip(2).collect::<Vec<_>>();
    match std::env::args().nth(1).as_deref() {
        Some("environment") => Some(codexsymphony_server::environment_recovery::run(&arguments)),
        Some("delivery") => Some(codexsymphony_server::delivery_admin::run(&arguments).await),
        Some("budget") => Some(codexsymphony_server::budget_admin::run(&arguments).await),
        _ => None,
    }
}

async fn run_service(config: Config) -> Result<(), StartupError> {
    // A fixed host path deliberately cannot be changed per cwd/database/port.
    // Never unlink the lock inode on shutdown: another instance may hold it.
    let _instance =
        process::InstanceLock::acquire(std::path::Path::new("/tmp/codexsymphony-controller.lock"))?;
    listen_for_stop()?;
    let mut workers = Workers::default();
    let (served, drained) = serve_and_drain(config, &mut workers).await;
    // Only environment probes are proven drained; other workers are aborted
    // without a quiescence claim.
    workers.stop();
    finish(served, drained)
}

#[derive(Default)]
struct Workers {
    coordinator: Option<tokio::task::JoinHandle<()>>,
    github: Option<tokio::task::JoinHandle<()>>,
}

impl Workers {
    fn stop(self) {
        if let Some(coordinator) = self.coordinator {
            coordinator.abort();
        }
        if let Some(github) = self.github {
            github.abort();
        }
    }
}

/// Polls the service and the stop drain together. A service exit (including
/// an error) requests a stop and still waits for the drain. A drain that ends
/// first drops the service, so a service held at the probe gate (startup or an
/// HTTP handler) cannot keep the process up. Only a successful drain proves no
/// probe is in flight; after a drain failure or timeout the service is dropped
/// with its outcome unknown, and the exit is non-zero.
async fn serve_and_drain(
    config: Config,
    workers: &mut Workers,
) -> (Option<Result<(), StartupError>>, Result<(), StartupError>) {
    // Both stay owned here, so the race only borrows them: the drain keeps
    // running after the service finishes, and neither is cancelled early.
    let mut service = Box::pin(serve_service(config, workers));
    let mut drain = Box::pin(drain_on_stop());
    let first = Race {
        service: service.as_mut(),
        drain: drain.as_mut(),
    }
    .await;
    match first {
        First::Served(result) => {
            request_stop();
            (Some(result), drain.await)
        }
        First::Drained(result) => (None, result),
    }
}

enum First {
    Served(Result<(), StartupError>),
    Drained(Result<(), StartupError>),
}

/// Named two-way race; neither future is cancelled by the other's progress.
/// The service is polled first, so a service result produced in the same wake
/// as the drain (e.g. a startup error right after its probe) is not lost.
struct Race<'a, S, D> {
    service: std::pin::Pin<&'a mut S>,
    drain: std::pin::Pin<&'a mut D>,
}

impl<S, D> std::future::Future for Race<'_, S, D>
where
    S: std::future::Future<Output = Result<(), StartupError>>,
    D: std::future::Future<Output = Result<(), StartupError>>,
{
    type Output = First;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<First> {
        let race = self.get_mut();
        if let std::task::Poll::Ready(result) = race.service.as_mut().poll(context) {
            return std::task::Poll::Ready(First::Served(result));
        }
        if let std::task::Poll::Ready(result) = race.drain.as_mut().poll(context) {
            return std::task::Poll::Ready(First::Drained(result));
        }
        std::task::Poll::Pending
    }
}

/// The service error wins; a drain failure is still logged and fails the exit.
fn finish(
    served: Option<Result<(), StartupError>>,
    drained: Result<(), StartupError>,
) -> Result<(), StartupError> {
    match &drained {
        Ok(()) => tracing::info!("environment probes drained; other workers aborted"),
        Err(error) => tracing::error!("environment drain failed; stop outcome unknown: {}", error),
    }
    if let Some(result) = served {
        result?;
    }
    drained
}

async fn serve_service(config: Config, workers: &mut Workers) -> Result<(), StartupError> {
    let pool = prepare_database(&config.database_url).await?;
    let mode = codexsymphony_server::service_mode::Mode::from_admission(
        codexsymphony_server::environment_service::startup(&pool).await,
    );
    if !mode.executions_enabled() {
        let (listener, policy) = listen(config).await?;
        return serve_http(listener, pool, policy, None, mode).await;
    }
    codexsymphony_server::generation_store::recover(&pool)
        .await
        .map_err(generation_recovery_failed)?;
    let (listener, policy) = listen(config).await?;
    let (worker, runtime) = start_coordinator(&pool).await?;
    workers.coordinator = Some(worker);
    workers.github = codexsymphony_server::github_service::start(&pool).await?;
    serve_http(listener, pool, policy, runtime, mode).await
}

fn generation_recovery_failed<E>(_: E) -> std::io::Error {
    std::io::Error::other("generation recovery failed")
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

static STOP: tokio::sync::Notify = tokio::sync::Notify::const_new();
static STOPPING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Resolves once a stop has been requested, even if it was requested earlier.
async fn shutdown_signal() {
    let mut notified = Box::pin(STOP.notified());
    notified.as_mut().enable();
    if !STOPPING.load(std::sync::atomic::Ordering::SeqCst) {
        notified.await;
    }
}

fn request_stop() {
    // Probe admission closes first; HTTP then stops accepting connections.
    codexsymphony_server::environment_probe::begin_drain();
    STOPPING.store(true, std::sync::atomic::Ordering::SeqCst);
    STOP.notify_waiters();
}

/// Registered before any probe can start, so neither default signal action can
/// kill this process while a supervisor has not yet written its receipt.
/// Later signals are absorbed; the drain limit bounds the stop.
fn listen_for_stop() -> std::io::Result<()> {
    use tokio::signal::unix::{SignalKind, signal};
    let terminate = signal(SignalKind::terminate())?;
    let interrupt = signal(SignalKind::interrupt())?;
    tokio::spawn(stop_on(terminate));
    tokio::spawn(stop_on(interrupt));
    Ok(())
}

async fn stop_on(mut signal: tokio::signal::unix::Signal) {
    if signal.recv().await.is_some() {
        request_stop();
    }
}

async fn drain_on_stop() -> Result<(), StartupError> {
    shutdown_signal().await;
    drain_environment().await
}

async fn drain_environment() -> Result<(), StartupError> {
    // Close admission before loading anything that can fail.
    codexsymphony_server::environment_probe::begin_drain();
    if std::env::var_os("ENVIRONMENT_CONFIG").is_none() {
        // Every probe loads this registry first, so none can have launched.
        return Ok(());
    }
    let registry = codexsymphony_server::environment_host::Registry::load()?;
    codexsymphony_server::environment_probe::drain(&registry).await
}

async fn serve_http(
    listener: TcpListener,
    pool: PgPool,
    policy: RequestPolicy,
    runtime: Option<tokio::task::AbortHandle>,
    mode: codexsymphony_server::service_mode::Mode,
) -> Result<(), StartupError> {
    let address = listener.local_addr()?;
    tracing::info!("CodexSymphony API listening at http://{}", address);
    let mut app = codexsymphony_server::router_with_mode(pool, policy, mode);
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
