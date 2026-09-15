//! GH-14 acceptance with PostgreSQL and real Linux descendant processes.
use codexsymphony_server::{
    coordinator::{self, Coordinator},
    execution::{CODING_BLOCKER, Launch, Receipt, RunKey},
    process, run_store,
};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command},
    time::{Duration, Instant},
};

struct Worker {
    child: Child,
    directory: PathBuf,
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = process::durable_write(&self.directory.join("stop.json"), &true);
        for _ in 0..200 {
            if self.child.try_wait().ok().flatten().is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn root() -> PathBuf {
    let path = std::env::temp_dir()
        .join("codexsymphony-fixture")
        .join(process::new_identity().unwrap());
    fs::create_dir_all(&path).unwrap();
    path.canonicalize().unwrap()
}
fn launch(root: &Path, incarnation: &str, script: &str) -> Launch {
    let cwd = root.join(process::new_identity().unwrap());
    fs::create_dir(&cwd).unwrap();
    Launch {
        key: RunKey {
            run_id: process::new_identity().unwrap(),
            request_id: process::new_identity().unwrap(),
            incarnation: incarnation.into(),
        },
        workspace: cwd.to_str().unwrap().into(),
        workspace_identity: process::new_identity().unwrap(),
        program: "/bin/sh".into(),
        args: vec!["-c".into(), script.into()],
    }
}
fn supervisor() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_codexsymphony-server"))
}
async fn wait_file(path: &Path) {
    let start = Instant::now();
    while !path.exists() {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "missing {}",
            path.display()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
async fn pool() -> PgPool {
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(
            &std::env::var("TEST_DATABASE_URL").expect("disposable TEST_DATABASE_URL required"),
        )
        .await
        .unwrap();
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
    pool
}
async fn reset(pool: &PgPool, incarnation: &str) {
    sqlx::raw_sql("TRUNCATE business_request,business_event,requirement_revision,requirement,repository,run_event RESTART IDENTITY CASCADE;
        INSERT INTO repository(id,version,document) VALUES (1,1,'{\"revoked\":false,\"github_repository_id\":99}');
        INSERT INTO requirement(version,state,contract,revision) VALUES (1,'Ready','{}',1),(1,'Ready','{}',1);
        INSERT INTO requirement_revision(requirement_id,revision,document) SELECT id,1,'{\"repository_version\":1}' FROM requirement;")
        .execute(pool).await.unwrap();
    sqlx::raw_sql("INSERT INTO github_repository(repository_id,repository_version,policy,probe_pr,capability,checked_at,stale) VALUES (99,1,'{}',1,'{\"policy\":{},\"blockers\":[]}',extract(epoch FROM now())::bigint,false) ON CONFLICT(repository_id) DO UPDATE SET checked_at=extract(epoch FROM now())::bigint,stale=false;")
        .execute(pool).await.unwrap();
    run_store::begin_incarnation(pool, incarnation)
        .await
        .unwrap();
}
async fn owner(pool: &PgPool) -> Option<i64> {
    sqlx::query_scalar("SELECT requirement_id FROM execution_control WHERE id=1")
        .fetch_one(pool)
        .await
        .unwrap()
}
async fn recovered(pool: &PgPool, root: &Path, incarnation: &str) {
    let start = Instant::now();
    while !coordinator::recover(pool, root, incarnation).await.unwrap() {
        assert!(start.elapsed() < Duration::from_secs(5));
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
}

#[tokio::test]
async fn execution_acceptance() {
    let pool = pool().await;
    let root = root();
    claim_and_pause(&pool, &root).await;
    delayed_identity(&pool, &root).await;
    live_descendants(&pool, &root).await;
    start_record_window(&pool, &root).await;
    failed_and_paused_handshake(&pool, &root).await;
    permission_lock_timeout(&pool, &root).await;
    restart_live_and_lost_supervisor(&pool, &root).await;
    unknown_identity(&pool, &root).await;
    slow_query(&pool, &root).await;
    pause_api(&pool).await;
    sqlx::raw_sql("TRUNCATE business_request,business_event,requirement_revision,requirement,repository,run_event RESTART IDENTITY CASCADE; INSERT INTO execution_control(id) VALUES(1)").execute(&pool).await.unwrap();
}

async fn permission_lock_timeout(pool: &PgPool, root: &Path) {
    reset(pool, "permission-contention").await;
    recovered(pool, root, "permission-contention").await;
    let launch = launch(root, "permission-contention", "touch forbidden");
    assert!(run_store::reserve_prepared(pool, &launch).await.unwrap());
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(13002)")
        .execute(&mut *tx)
        .await
        .unwrap();
    assert!(
        coordinator::start_reserved(pool, root, supervisor(), &launch)
            .await
            .is_err()
    );
    tx.rollback().await.unwrap();
    let directory = process::run_directory(root, &launch.key.run_id).unwrap();
    wait_file(&directory.join("quiescent.json")).await;
    assert!(!directory.join("start.json").exists());
    assert!(!Path::new(&launch.workspace).join("forbidden").exists());
    recovered(pool, root, "permission-contention").await;
    assert_eq!(owner(pool).await, Some(1));
}

fn helper(root: &Path, script: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = root.join(process::new_identity().unwrap());
    fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    path
}

async fn delayed_identity(pool: &PgPool, root: &Path) {
    reset(pool, "delayed-identity").await;
    recovered(pool, root, "delayed-identity").await;
    let launch = launch(root, "delayed-identity", "touch writer");
    assert!(run_store::reserve_prepared(pool, &launch).await.unwrap());
    let delayed = helper(
        root,
        &format!(
            "while [ ! -f \"$2/release\" ]; do sleep 0.02; done\nexec '{}' --supervise \"$2\"",
            supervisor().display()
        ),
    );
    let db = pool.clone();
    let base = root.to_owned();
    let copy = launch.clone();
    let task =
        tokio::spawn(async move { coordinator::start_reserved(&db, &base, &delayed, &copy).await });
    let directory = process::run_directory(root, &launch.key.run_id).unwrap();
    wait_file(&directory.join("launch.json")).await;
    // Exceed the old handshake window while the helper is demonstrably alive.
    tokio::time::sleep(Duration::from_secs(3)).await;
    let waiting = !task.is_finished();
    assert!(!directory.join("identity.json").exists());
    assert!(!directory.join("start.json").exists());
    assert!(!Path::new(&launch.workspace).join("writer").exists());
    fs::write(directory.join("release"), b"release").unwrap();
    let result = task.await.unwrap();
    assert!(waiting, "live helper must survive delayed durable identity");
    let _worker = Worker {
        child: result.unwrap(),
        directory: directory.clone(),
    };
    wait_file(&directory.join("quiescent.json")).await;
    assert!(Path::new(&launch.workspace).join("writer").exists());
    assert!(
        run_store::unresolved(pool).await.unwrap()[0]
            .process_identity
            .is_some()
    );
}

async fn failed_and_paused_handshake(pool: &PgPool, root: &Path) {
    reset(pool, "timeout").await;
    recovered(pool, root, "timeout").await;
    let timed = launch(root, "timeout", "touch writer");
    assert!(run_store::reserve_prepared(pool, &timed).await.unwrap());
    assert!(
        coordinator::start_reserved(pool, root, Path::new("/bin/true"), &timed)
            .await
            .is_err()
    );
    assert!(
        run_store::unresolved(pool).await.unwrap()[0]
            .process_identity
            .is_none()
    );
    assert_eq!(owner(pool).await, Some(1));

    reset(pool, "mismatch").await;
    recovered(pool, root, "mismatch").await;
    let mismatched = launch(root, "mismatch", "touch writer");
    assert!(
        run_store::reserve_prepared(pool, &mismatched)
            .await
            .unwrap()
    );
    let mut receipt = Receipt {
        key: mismatched.key.clone(),
        process: process::identity(std::process::id()).unwrap(),
    };
    receipt.key.request_id = "wrong".into();
    let source = root.join("wrong-identity.json");
    process::durable_write(&source, &receipt).unwrap();
    let script = helper(
        root,
        &format!("cp '{}' \"$2/identity.json\"", source.display()),
    );
    assert!(
        coordinator::start_reserved(pool, root, &script, &mismatched)
            .await
            .is_err()
    );
    assert!(
        run_store::unresolved(pool).await.unwrap()[0]
            .process_identity
            .is_none()
    );

    reset(pool, "paused-handshake").await;
    recovered(pool, root, "paused-handshake").await;
    let parked = launch(root, "paused-handshake", "touch writer");
    assert!(run_store::reserve_prepared(pool, &parked).await.unwrap());
    let delayed = helper(
        root,
        &format!(
            "sleep 3\nexec '{}' --supervise \"$2\"",
            supervisor().display()
        ),
    );
    let db = pool.clone();
    let base = root.to_owned();
    let copy = parked.clone();
    let task =
        tokio::spawn(async move { coordinator::start_reserved(&db, &base, &delayed, &copy).await });
    let directory = process::run_directory(root, &parked.key.run_id).unwrap();
    wait_file(&directory.join("launch.json")).await;
    run_store::pause(pool, Some(1)).await.unwrap();
    let _worker = Worker {
        child: task.await.unwrap().unwrap(),
        directory: directory.clone(),
    };
    recovered(pool, root, "paused-handshake").await;
    assert!(!Path::new(&parked.workspace).join("writer").exists());
    assert!(!directory.join("start.json").exists());
    assert_eq!(owner(pool).await, Some(1));
}

async fn restart_live_and_lost_supervisor(pool: &PgPool, root: &Path) {
    reset(pool, "crash-live").await;
    recovered(pool, root, "crash-live").await;
    let launch = launch(root, "crash-live", "echo $$ > parent.pid; sleep 60");
    assert!(run_store::reserve_prepared(pool, &launch).await.unwrap());
    let child = coordinator::start_reserved(pool, root, supervisor(), &launch)
        .await
        .unwrap();
    let directory = process::run_directory(root, &launch.key.run_id).unwrap();
    let _worker = Worker {
        child,
        directory: directory.clone(),
    };
    wait_file(&Path::new(&launch.workspace).join("parent.pid")).await;
    run_store::begin_incarnation(pool, "restart-live")
        .await
        .unwrap();
    assert!(!run_store::actions_allowed(pool, &launch.key).await.unwrap());
    recovered(pool, root, "restart-live").await;
    assert_eq!(owner(pool).await, Some(1));
    assert!(directory.join("quiescent.json").exists());

    reset(pool, "lost").await;
    recovered(pool, root, "lost").await;
    let mut parked = launch.clone();
    parked.key.run_id = process::new_identity().unwrap();
    parked.key.incarnation = "lost".into();
    assert!(run_store::reserve_prepared(pool, &parked).await.unwrap());
    let directory = process::run_directory(root, &parked.key.run_id).unwrap();
    let mut child = process::spawn(supervisor(), &directory, &parked).unwrap();
    wait_file(&directory.join("identity.json")).await;
    let identity: Receipt = process::read(&directory.join("identity.json")).unwrap();
    run_store::attach_process(pool, &identity).await.unwrap();
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(!coordinator::recover(pool, root, "lost").await.unwrap());
    assert!(
        run_store::unresolved(pool).await.unwrap()[0]
            .blocker
            .as_deref()
            .unwrap()
            .contains("supervisor lost")
    );
    run_store::begin_incarnation(pool, "lost-restart")
        .await
        .unwrap();
    assert!(
        !coordinator::recover(pool, root, "lost-restart")
            .await
            .unwrap()
    );
    assert_eq!(owner(pool).await, Some(1));
    assert!(!directory.join("quiescent.json").exists());
}

async fn api(
    pool: &PgPool,
    method: &str,
    path: &str,
    body: &str,
    csrf: bool,
) -> axum::http::StatusCode {
    use tower::ServiceExt;
    let policy = codexsymphony_server::security::RequestPolicy::new(
        "127.0.0.1:3081".parse().unwrap(),
        "http://localhost:4200".into(),
    )
    .unwrap();
    let app = codexsymphony_server::router(pool.clone(), policy);
    let mut request = axum::http::Request::builder()
        .method(method)
        .uri(path)
        .header("host", "127.0.0.1:3081")
        .header("origin", "http://localhost:4200")
        .header("content-type", "application/json");
    if csrf {
        request = request.header("x-codexsymphony-csrf", "1");
    }
    app.oneshot(
        request
            .body(axum::body::Body::from(body.to_owned()))
            .unwrap(),
    )
    .await
    .unwrap()
    .status()
}

async fn pause_api(pool: &PgPool) {
    reset(pool, "api").await;
    assert_eq!(
        api(
            pool,
            "POST",
            "/api/execution/pause",
            r#"{"pause":false}"#,
            true
        )
        .await,
        422
    );
    assert_eq!(
        api(
            pool,
            "POST",
            "/api/requirements/1/pause",
            r#"{"pause":false}"#,
            true
        )
        .await,
        422
    );
    assert_eq!(api(pool, "GET", "/api/execution", "", false).await, 200);
    assert_eq!(
        api(
            pool,
            "POST",
            "/api/execution/pause",
            r#"{"pause":true}"#,
            false
        )
        .await,
        403
    );
    let paused: bool = sqlx::query_scalar("SELECT paused FROM execution_control")
        .fetch_one(pool)
        .await
        .unwrap();
    assert!(!paused);
    assert_eq!(
        api(
            pool,
            "POST",
            "/api/execution/pause",
            "{\"unknown\":true}",
            true
        )
        .await,
        422
    );
    assert_eq!(
        api(
            pool,
            "POST",
            "/api/execution/pause",
            r#"{"pause":true}"#,
            true
        )
        .await,
        200
    );
    assert_eq!(
        api(
            pool,
            "POST",
            "/api/execution/pause",
            r#"{"pause":true}"#,
            true
        )
        .await,
        200
    );
    assert_eq!(
        api(
            pool,
            "POST",
            "/api/requirements/1/pause",
            r#"{"pause":true}"#,
            true
        )
        .await,
        200
    );
    assert_eq!(
        api(
            pool,
            "POST",
            "/api/requirements/1/pause",
            r#"{"pause":true}"#,
            true
        )
        .await,
        200
    );
    assert_eq!(
        api(
            pool,
            "POST",
            "/api/requirements/1/pause",
            "{\"unknown\":true}",
            true
        )
        .await,
        422
    );
    assert_eq!(
        api(
            pool,
            "POST",
            "/api/requirements/999/pause",
            r#"{"pause":true}"#,
            true
        )
        .await,
        404
    );
    let paused: (bool, bool) = sqlx::query_as(
        "SELECT c.paused,r.paused FROM execution_control c CROSS JOIN requirement r WHERE r.id=1",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(paused, (true, true));
    let closed = PgPoolOptions::new()
        .connect_lazy(&std::env::var("TEST_DATABASE_URL").unwrap())
        .unwrap();
    closed.close().await;
    assert_eq!(api(&closed, "GET", "/api/execution", "", false).await, 503);
    assert_eq!(
        api(
            &closed,
            "POST",
            "/api/execution/pause",
            r#"{"pause":true}"#,
            true
        )
        .await,
        503
    );
    assert_eq!(
        api(
            &closed,
            "POST",
            "/api/requirements/1/pause",
            r#"{"pause":true}"#,
            true
        )
        .await,
        503
    );
}

async fn claim_and_pause(pool: &PgPool, root: &Path) {
    reset(pool, "one").await;
    let first = launch(root, "one", "exit 0");
    assert!(
        !run_store::reserve_prepared(pool, &first).await.unwrap(),
        "cold gate closed"
    );
    recovered(pool, root, "one").await;
    run_store::pause(pool, Some(1)).await.unwrap();
    assert!(
        !run_store::reserve_prepared(pool, &first).await.unwrap(),
        "paused head cannot be skipped"
    );
    sqlx::query("UPDATE requirement SET paused=false")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("UPDATE repository SET revoked_through_version=1")
        .execute(pool)
        .await
        .unwrap();
    assert!(!run_store::reserve_prepared(pool, &first).await.unwrap());
    sqlx::query("UPDATE repository SET revoked_through_version=0")
        .execute(pool)
        .await
        .unwrap();
    let second = launch(root, "one", "exit 0");
    let (a, b) = tokio::join!(
        run_store::reserve_prepared(pool, &first),
        run_store::reserve_prepared(pool, &second)
    );
    assert_ne!(a.as_ref().unwrap(), b.as_ref().unwrap());
    let selected = if a.unwrap() { first } else { second };
    assert_eq!(owner(pool).await, Some(1));
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM agent_run")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    let mut tampered = selected.clone();
    tampered.args = vec!["-c".into(), "touch unauthorized".into()];
    assert!(
        coordinator::start_reserved(pool, root, supervisor(), &tampered)
            .await
            .is_err()
    );
    let child = coordinator::start_reserved(pool, root, supervisor(), &selected)
        .await
        .unwrap();
    let directory = process::run_directory(root, &selected.key.run_id).unwrap();
    let _worker = Worker {
        child,
        directory: directory.clone(),
    };
    wait_file(&directory.join("quiescent.json")).await;
    recovered(pool, root, "one").await;
    for phase in [
        "validation",
        "handoff_retry",
        "ci_wait",
        "question_wait",
        "paused",
    ] {
        sqlx::query("UPDATE agent_run SET phase=$1")
            .bind(phase)
            .execute(pool)
            .await
            .unwrap();
        assert!(
            !run_store::reserve_prepared(pool, &launch(root, "one", "exit 0"))
                .await
                .unwrap()
        );
        assert_eq!(owner(pool).await, Some(1));
    }
    run_store::pause(pool, None).await.unwrap();
    run_store::pause(pool, None).await.unwrap();
    run_store::begin_incarnation(pool, "two").await.unwrap();
    recovered(pool, root, "two").await;
    assert_eq!(owner(pool).await, Some(1));
    assert!(
        !run_store::archive_event(pool, &selected.key, serde_json::json!({"completion":true}))
            .await
            .unwrap()
    );
    assert!(
        !run_store::reserve_prepared(pool, &launch(root, "two", "exit 0"))
            .await
            .unwrap()
    );
}

async fn live_descendants(pool: &PgPool, root: &Path) {
    reset(pool, "live").await;
    recovered(pool, root, "live").await;
    let launch = launch(
        root,
        "live",
        "setsid sh -c 'echo $$ > escaped.pid; while :; do echo write >> writes; sleep 0.02; done' >/dev/null 2>&1 & exit 0",
    );
    assert!(run_store::reserve_prepared(pool, &launch).await.unwrap());
    let child = coordinator::start_reserved(pool, root, supervisor(), &launch)
        .await
        .unwrap();
    let directory = process::run_directory(root, &launch.key.run_id).unwrap();
    let mut worker = Worker {
        child,
        directory: directory.clone(),
    };
    let cwd = Path::new(&launch.workspace);
    wait_file(&cwd.join("writes")).await;
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert!(
        worker.child.try_wait().unwrap().is_none(),
        "parent exit must not stop supervision"
    );
    assert!(!directory.join("quiescent.json").exists());
    assert!(
        run_store::archive_event(pool, &launch.key, serde_json::json!({"log":"current"}))
            .await
            .unwrap()
    );
    let mut stale = launch.key.clone();
    stale.request_id = "late-request".into();
    assert!(
        !run_store::archive_event(pool, &stale, serde_json::json!({}))
            .await
            .unwrap()
    );
    run_store::pause(pool, Some(1)).await.unwrap();
    run_store::pause(pool, Some(1)).await.unwrap();
    assert!(!run_store::actions_allowed(pool, &launch.key).await.unwrap());
    recovered(pool, root, "live").await;
    wait_file(&directory.join("quiescent.json")).await;
    let escaped: u32 = fs::read_to_string(cwd.join("escaped.pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(process::identity(escaped).is_err());
    let before = fs::read(cwd.join("writes")).unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert_eq!(before, fs::read(cwd.join("writes")).unwrap());
    assert_eq!(owner(pool).await, Some(1));
    assert!(
        !run_store::archive_event(pool, &launch.key, serde_json::json!({"completion":true}))
            .await
            .unwrap()
    );
    worker.child.wait().unwrap();
    assert!(
        !Command::new(supervisor())
            .arg("--supervise")
            .arg(&directory)
            .status()
            .unwrap()
            .success(),
        "one-use launch journal cannot replay"
    );
}

async fn start_record_window(pool: &PgPool, root: &Path) {
    reset(pool, "before-crash").await;
    recovered(pool, root, "before-crash").await;
    let launch = launch(root, "before-crash", "echo started > started; sleep 60");
    assert!(run_store::reserve_prepared(pool, &launch).await.unwrap());
    let directory = process::run_directory(root, &launch.key.run_id).unwrap();
    let child = process::spawn(supervisor(), &directory, &launch).unwrap();
    let _worker = Worker {
        child,
        directory: directory.clone(),
    };
    wait_file(&directory.join("identity.json")).await;
    assert!(
        !Path::new(&launch.workspace).join("started").exists(),
        "writer is parked until DB identity commit"
    );
    assert!(
        run_store::unresolved(pool).await.unwrap()[0]
            .process_identity
            .is_none()
    );
    run_store::begin_incarnation(pool, "after-crash")
        .await
        .unwrap();
    recovered(pool, root, "after-crash").await;
    assert!(!Path::new(&launch.workspace).join("started").exists());
    assert_eq!(owner(pool).await, Some(1));
    let row: (bool, bool, String) =
        sqlx::query_as("SELECT quiescent,process_identity IS NOT NULL,phase FROM agent_run")
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(row, (true, true, "execution".into()));
    assert!(
        !run_store::archive_event(pool, &launch.key, serde_json::json!({"late":true}))
            .await
            .unwrap()
    );
}

async fn unknown_identity(pool: &PgPool, root: &Path) {
    reset(pool, "unknown").await;
    recovered(pool, root, "unknown").await;
    let launch = launch(root, "unknown", "sleep 60");
    assert!(run_store::reserve_prepared(pool, &launch).await.unwrap());
    run_store::begin_incarnation(pool, "new").await.unwrap();
    sqlx::query("UPDATE agent_run SET state='Failed'")
        .execute(pool)
        .await
        .unwrap();
    assert!(!coordinator::recover(pool, root, "new").await.unwrap());
    let mut run = run_store::unresolved(pool).await.unwrap().remove(0);
    assert!(run.blocker.unwrap().contains("unavailable"));
    assert_eq!(owner(pool).await, Some(1));
    let receipt = Receipt {
        key: launch.key.clone(),
        process: process::identity(std::process::id()).unwrap(),
    };
    assert!(
        !run_store::confirm_quiescent(
            pool,
            &run_store::unresolved(pool).await.unwrap()[0],
            &receipt
        )
        .await
        .unwrap()
    );
    run_store::attach_process(pool, &receipt).await.unwrap();
    let directory = process::run_directory(root, &launch.key.run_id).unwrap();
    process::durable_write(&directory.join("identity.json"), &receipt).unwrap();
    let mut stale = receipt.clone();
    stale.process.start_ticks += 1;
    process::durable_write(&directory.join("quiescent.json"), &stale).unwrap();
    assert!(
        !coordinator::recover(pool, root, "new").await.unwrap(),
        "reused PID/start identity does not prove quiescence"
    );
    stale = receipt.clone();
    stale.key.incarnation = "different".into();
    process::durable_write(&directory.join("identity.json"), &stale).unwrap();
    assert!(!coordinator::recover(pool, root, "new").await.unwrap());
    run = run_store::unresolved(pool).await.unwrap().remove(0);
    assert_eq!(run.process().unwrap(), receipt.process);
    assert!(run.stop_requested);
    assert_eq!(owner(pool).await, Some(1));
    sqlx::query("DELETE FROM execution_control")
        .execute(pool)
        .await
        .unwrap();
    assert!(
        run_store::begin_incarnation(pool, "missing-owner")
            .await
            .is_err(),
        "missing owner must not be reconstructed as free while Runs exist"
    );
}

async fn slow_query(pool: &PgPool, root: &Path) {
    reset(pool, "slow").await;
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("LOCK TABLE agent_run IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *tx)
        .await
        .unwrap();
    let mut coordinator = Coordinator::new(pool.clone(), root.into(), "slow".into());
    assert_eq!(coordinator.coding_blocker(), CODING_BLOCKER);
    coordinator.tick();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let started = Instant::now();
    for _ in 0..100 {
        coordinator.tick();
    }
    assert!(started.elapsed() < Duration::from_millis(100));
    use tower::ServiceExt;
    let policy = codexsymphony_server::security::RequestPolicy::new(
        "127.0.0.1:3081".parse().unwrap(),
        "http://localhost:4200".into(),
    )
    .unwrap();
    let app = codexsymphony_server::router(pool.clone(), policy);
    let response = tokio::time::timeout(
        Duration::from_secs(1),
        app.oneshot(
            axum::http::Request::builder()
                .uri("/api/health")
                .header("host", "127.0.0.1:3081")
                .body(axum::body::Body::empty())
                .unwrap(),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(response.status(), 200);
    tx.rollback().await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    coordinator.tick();
    drop(coordinator);
}

#[test]
fn os_identity_and_lock_boundaries() {
    let root = root();
    let path = root.join("instance.lock");
    let lock = process::InstanceLock::acquire(&path).unwrap();
    assert!(process::InstanceLock::acquire(&path).is_err());
    drop(lock);
    assert!(process::InstanceLock::acquire(&path).is_ok());
    assert!(process::InstanceLock::acquire(&root.join("absent/lock")).is_err());
    assert!(process::run_directory(&root, "../escape").is_err());
    assert!(process::run_directory(&root, "").is_err());
    assert!(process::identity(u32::MAX).is_err());
    assert!(process::parse_identity(1, "broken", "boot".into()).is_err());
    assert!(process::parse_identity(1, "1 (bad) x", "boot".into()).is_err());
    let current = process::identity(std::process::id()).unwrap();
    assert!(process::durable_write(Path::new("/"), &true).is_err());
    assert!(process::durable_write(&root.join("absent/value"), &true).is_err());
    assert!(current.start_ticks > 0);
    assert!(
        !Command::new(supervisor())
            .arg("--supervise")
            .status()
            .unwrap()
            .success()
    );
}
