//! A10: controlled clocks, real PostgreSQL and real filesystem operations.
use codexsymphony_server::{
    execution::{Launch, RunKey},
    process,
    storage_archive::{self, Package},
    storage_cleanup,
    storage_files::Directory,
    storage_lifecycle::{
        self as domain, CATEGORIES, Category, Identity, Kind, Limit, Policy, Protection, Usage,
    },
    storage_measure, storage_service,
    storage_store::{self as store, Deployment, Root},
    workspace::Workspace,
};
use serde_json::json;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
};

struct Tree(PathBuf);
impl Tree {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("storage-test-{}", process::new_identity().unwrap()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn dir(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::create_dir_all(&path).unwrap();
        path
    }
}
impl Drop for Tree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn policy() -> Policy {
    Policy {
        version: "test-1".into(),
        reason: "controlled storage acceptance".into(),
        global_bytes: 4 << 30,
        control_bytes: 256 << 20,
        run_bytes: 32 << 20,
        requirement_bytes: 128 << 20,
        entry_bytes: 1 << 20,
        entry_count: 10000,
        categories: CATEGORIES
            .into_iter()
            .map(|category| {
                (
                    category,
                    Limit {
                        bytes: 2 << 30,
                        seconds: if category == Category::Record {
                            3600
                        } else {
                            10
                        },
                        reserve_bytes: 1 << 20,
                    },
                )
            })
            .collect(),
    }
}
fn identity(run: &str, attempt: u64) -> Identity {
    Identity {
        repository: "owner/repo".into(),
        requirement: 1,
        revision: 1,
        run: run.into(),
        attempt,
        candidate: Some("abc".into()),
        stage: "pre_merge".into(),
        policy: "policy-a".into(),
        pr: Some(1),
    }
}

#[test]
fn controlled_admission_retention_and_resolution_boundaries() {
    let mut policy = policy();
    policy.validate().unwrap();
    let usage = Usage {
        available: 8 << 30,
        ..Usage::default()
    };
    for category in CATEGORIES {
        assert!(domain::admit(&policy, &usage, category, 100));
    }
    assert!(!domain::admit(&policy, &usage, Category::Hot, 0));
    let mut exhausted = usage.clone();
    exhausted.actual = policy.global_bytes;
    assert!(!domain::admit(&policy, &exhausted, Category::Hot, 1));
    exhausted.actual = 0;
    exhausted.reserved = policy.global_bytes;
    assert!(!domain::admit(&policy, &exhausted, Category::Hot, 1));
    exhausted = usage.clone();
    exhausted.category_actual = policy.categories[&Category::Cold].bytes;
    assert!(!domain::admit(&policy, &exhausted, Category::Cold, 1));
    exhausted = usage.clone();
    exhausted.category_reserved = policy.categories[&Category::Hot].bytes;
    assert!(!domain::admit(&policy, &exhausted, Category::Hot, 1));
    exhausted = usage.clone();
    exhausted.run_allocated = policy.run_bytes;
    assert!(!domain::admit(&policy, &exhausted, Category::Workspace, 1));
    exhausted = usage.clone();
    exhausted.requirement_allocated = policy.requirement_bytes;
    assert!(!domain::admit(&policy, &exhausted, Category::Workspace, 1));
    exhausted = usage.clone();
    exhausted.available = policy.control_bytes;
    assert!(!domain::admit(&policy, &exhausted, Category::Workspace, 1));
    exhausted = usage.clone();
    exhausted.actual = u64::MAX;
    assert!(!domain::admit(&policy, &exhausted, Category::Record, 1));
    exhausted = usage.clone();
    exhausted.unknown = true;
    assert!(!domain::admit(&policy, &exhausted, Category::Record, 1));
    policy.entry_bytes = 0;
    assert!(policy.validate().is_err());
    policy = policy_fixture();
    policy.version.clear();
    assert!(policy.validate().is_err());
    policy = policy_fixture();
    policy.categories.remove(&Category::Cold);
    assert!(policy.validate().is_err());
    policy = policy_fixture();
    policy.categories.get_mut(&Category::Hot).unwrap().seconds = u64::MAX;
    assert!(!domain::admit(&policy, &usage, Category::Record, 1));

    let protection = Protection::default();
    assert!(!domain::expired(99, 100, &protection));
    assert!(domain::expired(100, 100, &protection));
    for blocked in [
        Protection {
            active: true,
            ..Default::default()
        },
        Protection {
            consumer: true,
            ..Default::default()
        },
        Protection {
            unique: true,
            ..Default::default()
        },
        Protection {
            unknown: true,
            ..Default::default()
        },
        Protection {
            unreconciled: true,
            ..Default::default()
        },
        Protection {
            current_success: true,
            ..Default::default()
        },
    ] {
        assert!(blocked.reason().is_some());
        assert!(!domain::expired(1000, 100, &blocked));
    }
    let old = identity("failure", 1);
    let mut newer = identity("success", 2);
    newer.candidate = Some("def".into());
    assert!(domain::replaces(&old, &newer, "failure"));
    assert!(!domain::replaces(&old, &newer, "different"));
    for changed in [
        Identity {
            repository: "other/repo".into(),
            ..newer.clone()
        },
        Identity {
            revision: 2,
            ..newer.clone()
        },
        Identity {
            pr: Some(2),
            ..newer.clone()
        },
        Identity {
            stage: "post_merge".into(),
            ..newer.clone()
        },
        Identity {
            policy: "new-policy".into(),
            ..newer.clone()
        },
        Identity {
            attempt: 0,
            ..newer.clone()
        },
        Identity {
            requirement: 2,
            ..newer.clone()
        },
    ] {
        assert!(!domain::replaces(&old, &changed, "failure"));
    }
}
fn policy_fixture() -> Policy {
    policy()
}

#[test]
fn archive_verification_and_descriptor_bound_interrupted_deletion() {
    assert!(
        Directory::open(Path::new("/"))
            .unwrap()
            .child(Path::new("proc"))
            .is_err()
    );
    let tree = Tree::new();
    let path = tree.dir("source");
    let cold = tree.dir("cold");
    fs::create_dir(path.join("nested")).unwrap();
    fs::write(path.join("nested/log"), b"important exact log").unwrap();
    fs::write(path.join("output"), b"rebuildable").unwrap();
    let outside = tree.dir("outside");
    fs::write(outside.join("unique"), b"only copy").unwrap();
    symlink(&outside, path.join("link")).unwrap();
    let source = Directory::open(&path).unwrap();
    let target = Directory::open(&cold).unwrap();
    assert!(Directory::open(Path::new("relative")).is_err());
    assert!(Directory::open(&path.join("link")).is_err());
    assert!(source.read(Path::new("link/unique")).is_err());
    assert!(source.inventory(1).is_err());
    let files = source.inventory(100).unwrap();
    assert!(source.usage(100).unwrap() > 0);
    let incomplete: Vec<_> = files
        .iter()
        .filter(|entry| !entry.directory)
        .cloned()
        .collect();
    assert!(source.remove(&incomplete).is_err());
    assert!(path.join("nested/log").exists());
    storage_archive::write(&source, &target, &files, 1000).unwrap();
    storage_archive::write(&source, &target, &files, 1000).unwrap();
    let package = Package {
        path: cold.clone(),
        identity: target.identity().unwrap(),
        files: files.clone(),
    };
    storage_archive::verify(&package).unwrap();
    let existing_gz = fs::read_dir(&cold).unwrap().next().unwrap().unwrap().path();
    let saved_gz = tree.0.join("saved.gz");
    fs::rename(&existing_gz, &saved_gz).unwrap();
    fs::create_dir(&existing_gz).unwrap();
    assert!(storage_archive::write(&source, &target, &files, 1000).is_err());
    fs::remove_dir(&existing_gz).unwrap();
    fs::rename(saved_gz, existing_gz).unwrap();

    assert!(storage_archive::write(&source, &target, &files, 1).is_err());
    fs::write(path.join("new"), b"new run content").unwrap();
    assert!(source.remove(&files).is_err());
    fs::remove_file(path.join("new")).unwrap();
    // Simulate a crash after one unlink. Retry checks every remaining identity.
    fs::remove_file(path.join("output")).unwrap();
    let renamed = tree.0.join("old-instance");
    fs::rename(&path, &renamed).unwrap();
    fs::create_dir(&path).unwrap();
    fs::write(path.join("new"), b"new Run").unwrap();
    assert!(
        Directory::open(&path)
            .unwrap()
            .matches(&source.identity().unwrap())
            .is_err()
    );
    source.remove(&files).unwrap();
    source.remove(&files).unwrap();
    assert_eq!(fs::read(path.join("new")).unwrap(), b"new Run");
    assert_eq!(fs::read(outside.join("unique")).unwrap(), b"only copy");
    let gz = fs::read_dir(&cold).unwrap().next().unwrap().unwrap().path();
    fs::write(gz, b"incomplete gzip retry").unwrap();
    assert!(storage_archive::verify(&package).is_err());
}

fn root(path: PathBuf) -> Root {
    Root {
        identity: Directory::open(&path).unwrap().identity().unwrap(),
        path,
    }
}

#[test]
fn live_runtime_socket_is_measured_but_never_archived() {
    use std::os::unix::{fs::MetadataExt, net::UnixListener};
    let tree = Tree::new();
    let path = tree.0.join("runtime.sock");
    let socket = UnixListener::bind(&path).unwrap();
    fs::write(tree.0.join("original"), b"saved work").unwrap();
    let directory = Directory::open(&tree.0).unwrap();
    let expected = [&tree.0, &path, &tree.0.join("original")]
        .iter()
        .map(|path| fs::symlink_metadata(path).unwrap().blocks() * 512)
        .sum::<u64>();
    assert_eq!(directory.usage(10).unwrap(), expected);
    assert!(directory.inventory(10).is_err());
    assert_eq!(fs::read(tree.0.join("original")).unwrap(), b"saved work");
    drop(socket);
    fs::remove_file(path).unwrap();
    assert_eq!(directory.inventory(10).unwrap().len(), 1);
}
fn deployment(tree: &Tree) -> Deployment {
    Deployment {
        policy: policy(),
        execution: root(tree.dir("execution")),
        cold: root(tree.dir("cold")),
        database_filesystem: root(tree.0.clone()),
        database_extras: vec![root(tree.dir("backups"))],
    }
}
async fn fixture() -> PgPool {
    let options: sqlx::postgres::PgConnectOptions =
        std::env::var("TEST_DATABASE_URL").unwrap().parse().unwrap();
    let admin = PgPoolOptions::new()
        .connect_with(options.clone())
        .await
        .unwrap();
    let schema = format!(
        "storage_{}",
        process::new_identity().unwrap().replace('-', "")
    );
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect_with(options.options([("search_path", schema)]))
        .await
        .unwrap();
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
    sqlx::raw_sql("UPDATE storage_guard SET policy_version=NULL,blocked=false,error=NULL; TRUNCATE requirement,repository,business_request,storage_policy RESTART IDENTITY CASCADE;
      INSERT INTO repository(id,version,document) VALUES(1,1,'{\"revoked\":false}');
      INSERT INTO requirement(version,state,contract,revision) VALUES(1,'Cancelled','{}',1);
      INSERT INTO requirement_revision VALUES(1,1,'{\"repository_version\":1,\"repository\":{\"remote\":\"owner/repo\"}}');
      INSERT INTO storage_guard(id) VALUES(1);
      INSERT INTO execution_control(id,incarnation,recovery_complete) VALUES(1,'storage',true);")
        .execute(&pool).await.unwrap();
    pool
}
async fn run(pool: &PgPool, id: &str) -> Workspace {
    let key = RunKey {
        run_id: id.into(),
        request_id: format!("start-{id}"),
        incarnation: "storage".into(),
    };
    let launch = Launch {
        key: key.clone(),
        workspace: "/unused".into(),
        workspace_identity: id.into(),
        program: "/bin/true".into(),
        args: vec![],
    };
    sqlx::query("INSERT INTO agent_run(id,requirement_id,revision,incarnation,request_id,workspace,workspace_identity,launch,state,quiescent) VALUES($1,1,1,'storage',$2,'/unused',$1,$3,'Failed',true)")
        .bind(id).bind(&key.request_id).bind(json!(launch)).execute(pool).await.unwrap();
    Workspace {
        key,
        identity: id.into(),
        requirement: 1,
        revision: 1,
        phase: "execution".into(),
        baseline: "base".into(),
        branch: id.into(),
        path: "/unused".into(),
    }
}
async fn material(
    pool: &PgPool,
    config: &Deployment,
    run: &str,
    id: &str,
    path: &Path,
    kind: Kind,
) {
    let mut tx = lock(pool).await.unwrap();
    let identity = identity(run, 1);
    store::register(
        &mut tx,
        &identity,
        &json!({"reason":"retained exact failure"}),
    )
    .await
    .unwrap();
    store::material(
        &mut tx,
        store::Material {
            id,
            run,
            path,
            kind,
            category: Category::Hot,
            now: 100,
        },
        &config.policy,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
}

#[tokio::test]
async fn active_runtime_socket_keeps_originals_and_accounts_actual_bytes() {
    use std::os::unix::net::UnixListener;
    let pool = fixture().await;
    let tree = Tree::new();
    let config = deployment(&tree);
    store::install(&pool, &config).await.unwrap();
    run(&pool, "socket-run").await;
    sqlx::query("UPDATE agent_run SET state='Running',quiescent=false WHERE id='socket-run'")
        .execute(&pool)
        .await
        .unwrap();
    let path = config.execution.path.join("socket-run");
    fs::create_dir(&path).unwrap();
    fs::write(path.join("original"), b"saved runtime evidence").unwrap();
    let _socket = UnixListener::bind(path.join("runtime.sock")).unwrap();
    material(
        &pool,
        &config,
        "socket-run",
        "socket-run-runtime",
        &path,
        Kind::Retrospective,
    )
    .await;
    storage_cleanup::scan(&pool, 1000).await.unwrap();
    let (bytes, status, protection): (i64, String, Option<String>) = sqlx::query_as(
        "SELECT actual_bytes,status,protection FROM storage_material WHERE id='socket-run-runtime'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        bytes as u64,
        Directory::open(&path).unwrap().usage(100).unwrap()
    );
    assert_eq!(status, "available");
    assert!(protection.is_some());
    assert_eq!(
        fs::read(path.join("original")).unwrap(),
        b"saved runtime evidence"
    );
    assert!(storage_service::capacity(&pool).await.unwrap());
    let blocked: bool = sqlx::query_scalar("SELECT blocked FROM storage_guard")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(!blocked);
}

#[tokio::test]
async fn measurement_rejects_missing_roots_and_inventory_overflow() {
    let pool = fixture().await;
    let tree = Tree::new();
    let config = deployment(&tree);
    let evidence = config.execution.path.join("retained-evidence");
    fs::write(&evidence, b"preserve on measurement failure").unwrap();
    let mut tx = pool.begin().await.unwrap();

    let mut missing = config.clone();
    missing.execution.path = tree.0.join("missing-root");
    let error = storage_measure::measure(&mut tx, &missing)
        .await
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::NotFound
    );

    let mut bounded = config.clone();
    bounded.policy.entry_count = 0;
    let error = storage_measure::measure(&mut tx, &bounded)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("material inventory limit"));
    assert_eq!(
        fs::read(&evidence).unwrap(),
        b"preserve on measurement failure"
    );

    // Failed measurements must neither poison the transaction nor erase data.
    let measured = storage_measure::measure(&mut tx, &config).await.unwrap();
    assert!(measured.categories[&Category::Workspace] > 0);
    tx.rollback().await.unwrap();
    pool.close().await;
}

#[tokio::test]
async fn persisted_retention_archive_retries_and_cumulative_admission() {
    let pool = fixture().await;
    let tree = Tree::new();
    let config = deployment(&tree);
    let workspace = run(&pool, "unconfigured-run").await;
    assert!(storage_service::capacity(&pool).await.unwrap());
    assert!(
        storage_service::admit_workspace(&pool, &workspace)
            .await
            .unwrap()
    );
    storage_service::preparation(&pool, &workspace, &tree.0)
        .await
        .unwrap();
    store::install(&pool, &config).await.unwrap();
    assert_eq!(
        storage_service::entry_limit(&pool).await.unwrap(),
        config.policy.entry_bytes
    );
    let mut ingress = config.clone();
    ingress.policy.version = "ingress-limit".into();
    ingress.policy.entry_bytes = 4096;
    ingress.policy.entry_count = 2;
    store::install(&pool, &ingress).await.unwrap();
    let evidence_run = run(&pool, "ingress-limit").await;
    codexsymphony_server::runtime_store::evidence(
        &pool,
        &evidence_run.key,
        "stdout",
        &vec![b'x'; config.policy.entry_bytes as usize],
    )
    .await
    .unwrap();
    let kept: i64 =
        sqlx::query_scalar("SELECT kept_bytes FROM runtime_evidence WHERE run_id='ingress-limit'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(kept as u64, ingress.policy.entry_bytes - 512);
    sqlx::query("UPDATE runtime_evidence SET records=$1 WHERE run_id='ingress-limit'")
        .bind(ingress.policy.entry_count as i32)
        .execute(&pool)
        .await
        .unwrap();
    codexsymphony_server::runtime_store::evidence(
        &pool,
        &evidence_run.key,
        "stdout",
        b"over count",
    )
    .await
    .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT kept_bytes FROM runtime_evidence WHERE run_id='ingress-limit'"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        kept
    );
    // Isolate this ingress scenario from later retrospective row-count assertions.
    sqlx::query("DELETE FROM runtime_evidence_chunk WHERE run_id='ingress-limit'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM runtime_evidence WHERE run_id='ingress-limit'")
        .execute(&pool)
        .await
        .unwrap();
    store::install(&pool, &config).await.unwrap();
    let oversized=sqlx::query("INSERT INTO business_request VALUES('oversized-entry',jsonb_build_object('raw',repeat('x',1100000)),'{}')").execute(&pool).await.unwrap_err();
    assert_eq!(
        oversized.as_database_error().unwrap().code().as_deref(),
        Some("54000")
    );

    store::install(&pool, &config).await.unwrap();
    let mut conflict = config.clone();
    conflict.policy.reason = "changed under same version".into();
    assert!(store::install(&pool, &conflict).await.is_err());
    run(&pool, "failed-run").await;
    let source = config.execution.path.join("failed-logs");
    fs::create_dir(&source).unwrap();
    fs::write(
        source.join("stderr.log"),
        "failure at exact candidate SHA; important diagnostic",
    )
    .unwrap();
    material(
        &pool,
        &config,
        "failed-run",
        "failed-logs",
        &source,
        Kind::Retrospective,
    )
    .await;
    storage_cleanup::scan(&pool, 109).await.unwrap();
    assert!(source.join("stderr.log").exists());
    storage_cleanup::scan(&pool, 110).await.unwrap();
    assert!(!source.join("stderr.log").exists());
    let (status, archive, manifest): (String, serde_json::Value, serde_json::Value) =
        sqlx::query_as(
            "SELECT status,archive,manifest FROM storage_material WHERE id='failed-logs'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "deleted");
    assert!(manifest.as_array().unwrap().len() == 1);
    storage_archive::verify(&serde_json::from_value(archive).unwrap()).unwrap();
    storage_cleanup::scan(&pool, 111).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT state FROM agent_run WHERE id='failed-run'")
            .fetch_one(&pool)
            .await
            .unwrap(),
        "Failed"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM model_call")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );

    // Active consumers and unknown/partial work survive well beyond expiry.
    let protected = config.execution.path.join("protected");
    fs::create_dir(&protected).unwrap();
    fs::write(protected.join("unique"), "unique original").unwrap();
    material(
        &pool,
        &config,
        "failed-run",
        "protected",
        &protected,
        Kind::Recovery,
    )
    .await;
    sqlx::query("UPDATE agent_run SET quiescent=false WHERE id='failed-run'")
        .execute(&pool)
        .await
        .unwrap();
    storage_cleanup::scan(&pool, 120).await.unwrap();
    assert!(protected.join("unique").exists());
    sqlx::query("UPDATE agent_run SET quiescent=true WHERE id='failed-run'")
        .execute(&pool)
        .await
        .unwrap();
    storage_cleanup::scan(&pool, 121).await.unwrap();
    assert!(protected.join("unique").exists());

    let failed = config.execution.path.join("permission");
    fs::create_dir(&failed).unwrap();
    fs::write(failed.join("log"), "retain after permission failure").unwrap();
    material(
        &pool,
        &config,
        "failed-run",
        "permission",
        &failed,
        Kind::Retrospective,
    )
    .await;
    let mut interrupted = codexsymphony_server::preparation::Retry::new("cleanup", 100);
    assert!(interrupted.begin(100, false));
    sqlx::query("UPDATE storage_material SET retry=$1 WHERE id='permission'")
        .bind(json!(interrupted))
        .execute(&pool)
        .await
        .unwrap();
    fs::set_permissions(&config.cold.path, fs::Permissions::from_mode(0o500)).unwrap();
    for now in [130, 159, 160, 280, 500] {
        storage_cleanup::scan(&pool, now).await.unwrap();
    }
    fs::set_permissions(&config.cold.path, fs::Permissions::from_mode(0o700)).unwrap();
    let retry: serde_json::Value =
        sqlx::query_scalar("SELECT retry FROM storage_material WHERE id='permission'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(retry["attempts"], 3);
    assert_eq!(retry["todo"], true);
    assert!(failed.join("log").exists());

    // Every category participates; deleted bytes do not refund allocations.
    let mut tx = lock(&pool).await.unwrap();
    let measured = storage_measure::measure(&mut tx, &config).await.unwrap();
    assert!(measured.categories[&Category::Database] > 0);
    assert!(measured.categories[&Category::Record] > 0);
    assert!(
        store::reserve(
            &mut tx,
            "one",
            "failed-run",
            Category::Workspace,
            1000,
            measured.usage(Category::Workspace),
            &config.policy
        )
        .await
        .unwrap()
    );
    assert!(
        store::reserve(
            &mut tx,
            "one",
            "failed-run",
            Category::Workspace,
            1000,
            measured.usage(Category::Workspace),
            &config.policy
        )
        .await
        .unwrap()
    );
    assert!(
        !store::reserve(
            &mut tx,
            "one",
            "failed-run",
            Category::Workspace,
            1001,
            measured.usage(Category::Workspace),
            &config.policy
        )
        .await
        .unwrap()
    );
    assert!(store::reconcile(&mut tx, "one", 500, false).await.unwrap());
    assert!(store::reconcile(&mut tx, "one", 500, true).await.unwrap());
    assert!(!store::reconcile(&mut tx, "one", 0, true).await.unwrap());
    tx.commit().await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT allocated FROM storage_allocation WHERE request_id='one'"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        1000
    );
    let workspace = run(&pool, "admission-run").await;
    assert!(
        storage_service::admit_workspace(&pool, &workspace)
            .await
            .unwrap()
    );
    assert!(
        storage_service::validation(&pool, "admission-run")
            .await
            .unwrap()
    );
    assert!(storage_service::capacity(&pool).await.unwrap());
    database_failure_preserves_verified_package(&pool, &config).await;
    raw_retrospective_and_preparation(&pool, &config).await;
    explicit_retry_chain(&pool, &config).await;
    scan_failure_budget(&pool, &config).await;
    actual_overrun(&pool, &config).await;
    policy_and_control(&pool, &config).await;
    repeated_requirements_stop_before_cumulative_exhaustion(&pool).await;
    sqlx::raw_sql("UPDATE storage_guard SET policy_version=NULL,blocked=false,error=NULL; TRUNCATE storage_allocation,storage_material,storage_attempt; DELETE FROM storage_policy;")
        .execute(&pool).await.unwrap();
}

async fn lock(pool: &PgPool) -> Result<sqlx::Transaction<'_, sqlx::Postgres>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(13002)")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SELECT id FROM execution_control WHERE id=1 FOR UPDATE")
        .execute(&mut *tx)
        .await?;
    Ok(tx)
}

async fn raw_retrospective_and_preparation(pool: &PgPool, config: &Deployment) {
    use codexsymphony_server::{storage_db, storage_inventory, storage_view};
    let next = run(pool, "resolved-run").await;
    let mut tx = lock(pool).await.unwrap();
    storage_inventory::register_workspace(&mut tx, &next)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    sqlx::raw_sql("UPDATE agent_run SET created_at=to_timestamp(100) WHERE id='failed-run'; UPDATE storage_attempt SET resolved_by='resolved-run' WHERE run_id='failed-run'; INSERT INTO runtime_evidence(run_id,channel,kept_bytes,records) VALUES('failed-run','stderr',12,1); INSERT INTO runtime_evidence_chunk VALUES('failed-run','stderr',1,decode('6661696c757265206c6f670a','hex'));")
        .execute(pool).await.unwrap();
    sqlx::query("UPDATE agent_run SET quiescent=false WHERE id='resolved-run'")
        .execute(pool)
        .await
        .unwrap();
    storage_db::collect(pool, config, 200).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM runtime_evidence_chunk")
            .fetch_one(pool)
            .await
            .unwrap(),
        1
    );
    sqlx::query("UPDATE agent_run SET quiescent=true WHERE id='resolved-run'")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO runtime_evidence_chunk SELECT 'failed-run','stderr',n,decode('6162','hex') FROM generate_series(2,6) n").execute(pool).await.unwrap();
    fs::set_permissions(&config.cold.path, fs::Permissions::from_mode(0o500)).unwrap();
    storage_db::collect(pool, config, 190).await.unwrap();
    // Simulate a crash before the export outcome was persisted.
    let mut interrupted = codexsymphony_server::preparation::Retry::new("cleanup", 100);
    assert!(interrupted.begin(100, false));
    sqlx::query("UPDATE runtime_evidence SET cleanup_retry=$1 WHERE run_id='failed-run'")
        .bind(json!(interrupted))
        .execute(pool)
        .await
        .unwrap();
    fs::set_permissions(&config.cold.path, fs::Permissions::from_mode(0o500)).unwrap();
    for at in [200, 229, 230, 350, 500] {
        storage_db::collect(pool, config, at).await.unwrap();
    }
    fs::set_permissions(&config.cold.path, fs::Permissions::from_mode(0o700)).unwrap();
    let failed: serde_json::Value = sqlx::query_scalar(
        "SELECT cleanup_retry FROM runtime_evidence WHERE run_id='failed-run' AND channel='stderr'",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(failed["todo"], true);
    assert_eq!(failed["attempts"], 3);
    let mut retry = lock(pool).await.unwrap();
    codexsymphony_server::storage_db::authorize(&mut retry, 1, 501)
        .await
        .unwrap();
    retry.commit().await.unwrap();
    storage_db::collect(pool, config, 501).await.unwrap();

    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM runtime_evidence_chunk")
            .fetch_one(pool)
            .await
            .unwrap(),
        0
    );
    let (replacement, record): (String, String) = sqlx::query_as(
        "SELECT replacement,retrospective FROM runtime_evidence WHERE run_id='failed-run'",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    let doc: serde_json::Value = serde_json::from_str(&record).unwrap();
    assert_eq!(doc["complete_replay"], false);
    assert!(record.contains("failure log"));
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT expired_at FROM runtime_evidence WHERE run_id='failed-run'"
        )
        .fetch_one(pool)
        .await
        .unwrap(),
        501
    );
    let preview = codexsymphony_server::operator_view::evidence(pool, 1, "failed-run", "stderr")
        .await
        .unwrap();
    assert_eq!(preview["preview_only"], true);
    assert!(
        preview["text"]
            .as_str()
            .unwrap()
            .contains("complete_replay")
    );

    let target = config
        .cold
        .open()
        .unwrap()
        .child(Path::new(&replacement))
        .unwrap();
    storage_archive::verify_record(&target, record.as_bytes()).unwrap();
    sqlx::query(
        "UPDATE runtime_evidence SET records=$1 WHERE run_id='failed-run' AND channel='stderr'",
    )
    .bind(codexsymphony_server::runtime::MAX_RECORDS as i32)
    .execute(pool)
    .await
    .unwrap();
    codexsymphony_server::runtime_store::evidence(
        pool,
        &RunKey {
            run_id: "failed-run".into(),
            request_id: "start-failed-run".into(),
            incarnation: "storage".into(),
        },
        "stderr",
        b"late bytes",
    )
    .await
    .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM runtime_evidence_chunk WHERE run_id='failed-run'"
        )
        .fetch_one(pool)
        .await
        .unwrap(),
        0
    );
    storage_db::collect(pool, config, 1000).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT replacement FROM runtime_evidence WHERE run_id='failed-run'"
        )
        .fetch_one(pool)
        .await
        .unwrap(),
        replacement
    );
    let preparing = config
        .execution
        .path
        .join(".preparation-resolved-run-abandoned");
    assert!(
        storage_service::preparation(pool, &next, &preparing)
            .await
            .is_err()
    );
    fs::create_dir(&preparing).unwrap();
    storage_service::preparation(pool, &next, &preparing)
        .await
        .unwrap();
    fs::write(preparing.join("raw.log"), b"preparation interrupted").unwrap();
    fs::write(preparing.join("quiescent.json"), br#"{"quiescent":false}"#).unwrap();
    // A marker is a retained producer fact, never guessed from absence of a PID.
    storage_cleanup::scan(pool, codexsymphony_server::runtime_client::now() + 20)
        .await
        .unwrap();
    assert!(preparing.join("raw.log").exists());
    let mut tx = lock(pool).await.unwrap();
    let view = storage_view::detail(&mut tx, 1).await.unwrap();
    assert_eq!(view["configured"], true);
    assert!(view["protected_bytes"].as_i64().unwrap() > 0);
    storage_cleanup::authorize(&mut tx, 1, codexsymphony_server::runtime_client::now() + 21)
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

async fn scan_failure_budget(pool: &PgPool, config: &Deployment) {
    let displaced = config.cold.path.with_extension("displaced");
    fs::rename(&config.cold.path, &displaced).unwrap();
    fs::create_dir(&config.cold.path).unwrap();
    let start = codexsymphony_server::runtime_client::now() + 100;
    for (offset, delay) in [(0, 30), (30, 120), (150, 300)] {
        assert!(storage_cleanup::scan(pool, start + offset).await.is_err());
        assert_eq!(
            storage_service::next_scan_delay(pool, start + offset)
                .await
                .unwrap()
                .as_secs(),
            delay
        );
    }
    storage_cleanup::scan(pool, start + 1000).await.unwrap();
    let retry: serde_json::Value = sqlx::query_scalar("SELECT scan_retry FROM storage_guard")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(retry["todo"], true);
    assert_eq!(retry["group_attempts"], 3);
    assert_eq!(
        retry["last_failure"]["evidence"],
        "storage_guard.scan_retry"
    );
    assert!(
        retry["last_failure"]["detail"]
            .as_str()
            .unwrap()
            .contains("identity changed")
    );
    assert_eq!(fs::read_dir(&config.cold.path).unwrap().count(), 0);
    fs::remove_dir(&config.cold.path).unwrap();
    fs::rename(displaced, &config.cold.path).unwrap();
    let mut tx = lock(pool).await.unwrap();
    storage_cleanup::authorize(&mut tx, 1, start + 1001)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    storage_cleanup::scan(pool, start + 1001).await.unwrap();
    assert_eq!(
        storage_service::next_scan_delay(pool, start + 1001)
            .await
            .unwrap()
            .as_secs(),
        300
    );
}

async fn explicit_retry_chain(pool: &PgPool, config: &Deployment) {
    use codexsymphony_server::{storage_consumers, storage_inventory};
    for id in ["chain-a", "chain-b", "chain-c", "chain-later-failure"] {
        let workspace = run(pool, id).await;
        let mut tx = lock(pool).await.unwrap();
        storage_inventory::register_workspace(&mut tx, &workspace)
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }
    for (old, new) in [("chain-a", "chain-b"), ("chain-b", "chain-c")] {
        sqlx::query("INSERT INTO runtime_resume(source_run,job,status) VALUES($1,$2,'dispatched')")
            .bind(old)
            .bind(
                json!({"launch":{"key":{"run_id":new}},"workspace":{"requirement":1,"revision":1}}),
            )
            .execute(pool)
            .await
            .unwrap();
    }
    sqlx::raw_sql("INSERT INTO candidate_validation(id,requirement_id,revision,source_run_id,candidate_sha,candidate_tree,trusted,required_steps,source_before,source_after,entry_before,entry_after,stage,result) VALUES('chain-v',1,1,'chain-c','chain-sha','tree','{}','[]','before','after','entry','entry','done','succeeded'); INSERT INTO delivery(action_key,validation_id,requirement_id,revision,repository_id,repository,branch,base_branch,head_sha,manifest,policy,pr_number,released) VALUES('chain-delivery','chain-v',1,1,1,'owner/repo','chain-branch','main','chain-sha','{}','{}',17,true); INSERT INTO delivery_action(action_key,kind,state) VALUES('chain-delivery','publish','pending');")
        .execute(pool).await.unwrap();
    let mut tx = lock(pool).await.unwrap();
    storage_inventory::discover(&mut tx, config, 2000)
        .await
        .unwrap();
    storage_consumers::resolve(&mut tx).await.unwrap();
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM storage_attempt WHERE run_id LIKE 'chain-%' AND resolved_by IS NOT NULL").fetch_one(&mut *tx).await.unwrap(),0);
    tx.commit().await.unwrap();
    sqlx::query("UPDATE delivery_action SET state='confirmed' WHERE action_key='chain-delivery'")
        .execute(pool)
        .await
        .unwrap();
    let mut tx = lock(pool).await.unwrap();
    storage_consumers::resolve(&mut tx).await.unwrap();
    let resolved: Vec<String> = sqlx::query_scalar(
        "SELECT run_id FROM storage_attempt WHERE resolved_by='chain-c' ORDER BY run_id",
    )
    .fetch_all(&mut *tx)
    .await
    .unwrap();
    assert_eq!(resolved, ["chain-a", "chain-b"]);
    let protection = storage_consumers::protection(&mut tx, config, "chain-c", "retrospective")
        .await
        .unwrap();
    assert!(protection.current_success);
    sqlx::query("UPDATE storage_attempt SET resolved_by=NULL,identity=jsonb_set(identity,'{pr}','18') WHERE run_id='chain-a'").execute(&mut *tx).await.unwrap();
    storage_consumers::resolve(&mut tx).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT resolved_by FROM storage_attempt WHERE run_id='chain-a'"
        )
        .fetch_one(&mut *tx)
        .await
        .unwrap(),
        None
    );
    tx.commit().await.unwrap();
}

async fn database_failure_preserves_verified_package(pool: &PgPool, config: &Deployment) {
    let path = config.execution.path.join("db-failure");
    fs::create_dir(&path).unwrap();
    fs::write(path.join("critical.log"), b"only original diagnostic").unwrap();
    material(
        pool,
        config,
        "failed-run",
        "db-failure",
        &path,
        Kind::Retrospective,
    )
    .await;
    let now = codexsymphony_server::runtime_client::now() + 1;
    sqlx::raw_sql("CREATE FUNCTION interrupt_storage_manifest() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.id='db-failure' AND NEW.status='deleting' THEN NEW.manifest=NULL; END IF; RETURN NEW; END $$; CREATE TRIGGER interrupt_storage_manifest BEFORE UPDATE ON storage_material FOR EACH ROW EXECUTE FUNCTION interrupt_storage_manifest();").execute(pool).await.unwrap();
    storage_cleanup::scan(pool, now).await.unwrap();
    assert!(path.join("critical.log").exists());
    let saved_package: serde_json::Value =
        sqlx::query_scalar("SELECT archive FROM storage_material WHERE id='db-failure'")
            .fetch_one(pool)
            .await
            .unwrap();
    storage_archive::verify(&serde_json::from_value(saved_package.clone()).unwrap()).unwrap();
    sqlx::raw_sql("DROP TRIGGER interrupt_storage_manifest ON storage_material; DROP FUNCTION interrupt_storage_manifest();").execute(pool).await.unwrap();
    sqlx::query("UPDATE storage_material SET manifest=$1 WHERE id='db-failure'")
        .bind(&saved_package["files"])
        .execute(pool)
        .await
        .unwrap();
    sqlx::raw_sql("CREATE FUNCTION reject_storage_delete() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.id='db-failure' AND NEW.status='deleted' THEN RAISE EXCEPTION 'injected database failure'; END IF; RETURN NEW; END $$; CREATE TRIGGER reject_storage_delete BEFORE UPDATE ON storage_material FOR EACH ROW EXECUTE FUNCTION reject_storage_delete();")
        .execute(pool).await.unwrap();
    storage_cleanup::scan(pool, now + 30).await.unwrap();
    let (status, archive): (String, serde_json::Value) =
        sqlx::query_as("SELECT status,archive FROM storage_material WHERE id='db-failure'")
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(status, "deleting");
    let package: Package = serde_json::from_value(archive.clone()).unwrap();
    storage_archive::verify(&package).unwrap();
    assert!(
        package
            .files
            .iter()
            .any(|file| file.path == Path::new("critical.log"))
    );
    sqlx::raw_sql("DROP TRIGGER reject_storage_delete ON storage_material; DROP FUNCTION reject_storage_delete();").execute(pool).await.unwrap();
    storage_cleanup::scan(pool, now + 150).await.unwrap();
    let (status, saved): (String, serde_json::Value) =
        sqlx::query_as("SELECT status,archive FROM storage_material WHERE id='db-failure'")
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(status, "deleted");
    assert_eq!(archive, saved);
}

async fn policy_and_control(pool: &PgPool, config: &Deployment) {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    use codexsymphony_server::operator_control::{self, Action, Command};
    sqlx::query("UPDATE storage_attempt SET expires_at=extract(epoch FROM now())::bigint+10000")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("UPDATE requirement SET paused=true,cancel_requested=true")
        .execute(pool)
        .await
        .unwrap();
    storage_service::block(pool, "synthetic storage outage")
        .await
        .unwrap();
    let request = Command {
        version: 1,
        request_id: "storage-recovery".into(),
        action: Action::StorageRecheck,
    };
    let displaced = config.execution.path.with_extension("recovery-missing");
    fs::rename(&config.execution.path, &displaced).unwrap();
    assert!(
        codexsymphony_server::storage::recover(pool, &config.execution.path)
            .await
            .is_err()
    );
    assert!(storage_service::capacity(pool).await.is_err());
    let workspace = run(pool, "missing-mount-admission").await;
    assert!(
        storage_service::admit_workspace(pool, &workspace)
            .await
            .is_err()
    );
    assert!(
        storage_service::validation(pool, "failed-run")
            .await
            .is_err()
    );
    assert!(operator_control::execute(pool, 1, &request).await.is_err());
    fs::rename(&displaced, &config.execution.path).unwrap();
    assert!(
        codexsymphony_server::storage::recover(pool, &config.execution.path)
            .await
            .unwrap()
    );
    let recovered = operator_control::execute(pool, 1, &request).await.unwrap();
    assert_eq!(
        recovered,
        operator_control::execute(pool, 1, &request).await.unwrap()
    );
    let facts: (bool, bool, String) =
        sqlx::query_as("SELECT paused,cancel_requested,state FROM requirement WHERE id=1")
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(facts, (true, true, "Cancelled".into()));
    assert!(
        !sqlx::query_scalar::<_, bool>("SELECT blocked FROM storage_guard")
            .fetch_one(pool)
            .await
            .unwrap()
    );
    // Execution writes still work, but accounting cannot verify the cold root.
    // Exercise the actual admission error path and its retained diagnostic.
    assert!(
        codexsymphony_server::storage::recover(pool, &config.execution.path)
            .await
            .unwrap()
    );
    assert!(codexsymphony_server::storage::permit(pool, &config.execution.path).await);
    control_lock_contention_denies_without_relatching(pool, config).await;
    let displaced_cold = config.cold.path.with_extension("admission-missing");
    fs::rename(&config.cold.path, &displaced_cold).unwrap();
    assert!(!codexsymphony_server::storage::permit(pool, &config.execution.path).await);
    let blocked: bool = sqlx::query_scalar("SELECT blocked FROM storage_guard")
        .fetch_one(pool)
        .await
        .unwrap();
    assert!(blocked);
    assert!(displaced_cold.is_dir());
    fs::rename(&displaced_cold, &config.cold.path).unwrap();
    assert!(
        codexsymphony_server::storage::recover(pool, &config.execution.path)
            .await
            .unwrap()
    );
    sqlx::query("UPDATE storage_attempt SET expires_at=1")
        .execute(pool)
        .await
        .unwrap();
    assert!(!storage_service::capacity(pool).await.unwrap());
    let version: i64 = sqlx::query_scalar("SELECT version FROM requirement WHERE id=1")
        .fetch_one(pool)
        .await
        .unwrap();
    let rejected = Command {
        version,
        request_id: "storage-still-full".into(),
        action: Action::StorageRecheck,
    };
    assert!(operator_control::execute(pool, 1, &rejected).await.is_err());
    let after: i64 = sqlx::query_scalar("SELECT version FROM requirement WHERE id=1")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(version, after);
    let mut lower = config.clone();
    lower.policy.version = "lower-new-admission".into();
    lower.policy.run_bytes = 1;
    store::install(pool, &lower).await.unwrap();
    let workspace = run(pool, "over-budget-run").await;
    let mut tx = lock(pool).await.unwrap();
    codexsymphony_server::storage_inventory::register_workspace(&mut tx, &workspace)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert!(
        !storage_service::validation(pool, "over-budget-run")
            .await
            .unwrap()
    );
    assert!(
        !storage_service::admit_workspace(pool, &workspace)
            .await
            .unwrap()
    );
    let total: i64 = sqlx::query_scalar("SELECT SUM(allocated)::bigint FROM storage_allocation")
        .fetch_one(pool)
        .await
        .unwrap();
    assert!(total > 0);
    lower.policy.run_bytes = 128 << 20;
    assert!(store::install(pool, &lower).await.is_err());
    lower.policy.version = "extended-record-policy".into();
    lower
        .policy
        .categories
        .get_mut(&Category::Record)
        .unwrap()
        .seconds = 86400;
    store::install(pool, &lower).await.unwrap();
    assert_eq!(
        total,
        sqlx::query_scalar::<_, i64>("SELECT SUM(allocated)::bigint FROM storage_allocation")
            .fetch_one(pool)
            .await
            .unwrap()
    );
}

async fn control_lock_contention_denies_without_relatching(pool: &PgPool, config: &Deployment) {
    let options = pool
        .connect_options()
        .as_ref()
        .clone()
        .options([("lock_timeout", "50ms")]);
    let contender = PgPoolOptions::new()
        .max_connections(2)
        .connect_with(options)
        .await
        .unwrap();
    for query in [
        "SELECT id FROM storage_guard FOR UPDATE",
        "SELECT pg_advisory_xact_lock(13002)",
    ] {
        let mut held = pool.begin().await.unwrap();
        sqlx::query(query).execute(&mut *held).await.unwrap();
        assert!(!codexsymphony_server::storage::permit(&contender, &config.execution.path).await);
        held.rollback().await.unwrap();
        let blocked: bool = sqlx::query_scalar("SELECT blocked FROM storage_guard")
            .fetch_one(pool)
            .await
            .unwrap();
        assert!(
            !blocked,
            "known lock contention must not undo an explicit recovery"
        );
        assert!(codexsymphony_server::storage::permit(&contender, &config.execution.path).await);
    }
    contender.close().await;
}

async fn repeated_requirements_stop_before_cumulative_exhaustion(pool: &PgPool) {
    let mut rejected = false;
    for index in 0..32 {
        let workspace = run(pool, &format!("batch-{index}")).await;
        let mut tx = lock(pool).await.unwrap();
        let config = store::deployment(&mut tx).await.unwrap().unwrap();
        codexsymphony_server::storage_inventory::register_workspace(&mut tx, &workspace)
            .await
            .unwrap();
        let measured = storage_measure::measure(&mut tx, &config).await.unwrap();
        let accepted = store::reserve(
            &mut tx,
            &format!("batch-allocation-{index}"),
            &workspace.key.run_id,
            Category::Workspace,
            8 << 20,
            measured.usage(Category::Workspace),
            &config.policy,
        )
        .await
        .unwrap();
        if accepted {
            store::reconcile(&mut tx, &format!("batch-allocation-{index}"), 0, true)
                .await
                .unwrap();
        }
        tx.commit().await.unwrap();
        if !accepted {
            rejected = true;
            break;
        }
    }
    assert!(
        rejected,
        "reclaimed retries must not reset cumulative requirement allocation"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM model_call")
            .fetch_one(pool)
            .await
            .unwrap(),
        0
    );
}

async fn actual_overrun(pool: &PgPool, config: &Deployment) {
    let workspace = run(pool, "overflow-run").await;
    let mut tx = lock(pool).await.unwrap();
    codexsymphony_server::storage_inventory::register_workspace(&mut tx, &workspace)
        .await
        .unwrap();
    let measured = storage_measure::measure(&mut tx, config).await.unwrap();
    assert!(
        store::reserve(
            &mut tx,
            "overflow-run-hot",
            "overflow-run",
            Category::Hot,
            1,
            measured.usage(Category::Hot),
            &config.policy
        )
        .await
        .unwrap()
    );
    tx.commit().await.unwrap();
    let path = config.execution.path.join("overflow-data");
    fs::create_dir(&path).unwrap();
    fs::write(path.join("unique"), b"original bytes exceed reservation").unwrap();
    let mut tx = lock(pool).await.unwrap();
    store::material(
        &mut tx,
        store::Material {
            id: "overflow-material",
            run: "overflow-run",
            path: &path,
            kind: Kind::Recovery,
            category: Category::Hot,
            now: 100,
        },
        &config.policy,
    )
    .await
    .unwrap();
    storage_cleanup::reconcile(&mut tx, config).await.unwrap();
    tx.commit().await.unwrap();
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT blocked FROM storage_guard")
            .fetch_one(pool)
            .await
            .unwrap()
    );
    assert!(path.join("unique").exists());
    assert!(
        sqlx::query_scalar::<_, i64>(
            "SELECT allocated FROM storage_allocation WHERE request_id='overflow-run-hot'"
        )
        .fetch_one(pool)
        .await
        .unwrap()
            > 1
    );
}

#[test]
fn invalid_storage_category_fails_closed() {
    assert!(store::name(&json!({"not":"category"})).is_err());
}

#[test]
fn reader_panic_is_reported_to_capture_owner() {
    struct BrokenReader;
    impl std::io::Read for BrokenReader {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            panic!("reader failure")
        }
    }
    let tree = Tree::new();
    let mut capture = codexsymphony_server::storage_output::Capture::default();
    capture.stream(
        BrokenReader,
        fs::File::create(tree.0.join("output")).unwrap(),
        100,
    );
    assert_eq!(
        capture.finish().unwrap_err().to_string(),
        "output reader failed"
    );
}

#[test]
fn accounting_restarts_the_whole_pass_only_for_bounded_disappearance() {
    use codexsymphony_server::storage_files::retry_listing;
    use std::io;
    let mut attempts = 0;
    let entries = retry_listing(|| {
        attempts += 1;
        if attempts < 3 {
            Err(io::Error::from(io::ErrorKind::NotFound))
        } else {
            Ok(Vec::new())
        }
    })
    .unwrap();
    assert!(entries.is_empty());
    assert_eq!(attempts, 3);
    attempts = 0;
    assert!(
        retry_listing(|| {
            attempts += 1;
            Err(io::Error::from(io::ErrorKind::NotFound))
        })
        .is_err()
    );
    assert_eq!(attempts, 3);
    attempts = 0;
    assert!(
        retry_listing(|| {
            attempts += 1;
            Err(io::Error::from(io::ErrorKind::PermissionDenied))
        })
        .is_err()
    );
    assert_eq!(attempts, 1);
}

#[tokio::test]
async fn phase_notifications_require_real_changes() {
    let pool = fixture().await;
    run(&pool, "notification-run").await;
    let mut listener = sqlx::postgres::PgListener::connect_with(&pool)
        .await
        .unwrap();
    listener.listen("storage_phase_ended").await.unwrap();
    let mut connection = pool.acquire().await.unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *connection)
        .await
        .unwrap();
    sqlx::raw_sql("UPDATE agent_run SET state=state,phase=phase,quiescent=quiescent; UPDATE candidate_validation SET stage=stage,result=result WHERE false; UPDATE delivery_action SET state=state WHERE false; INSERT INTO preparation_history OVERRIDING SYSTEM VALUE SELECT * FROM preparation_history WHERE false;")
        .execute(&mut *connection).await.unwrap();
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(150),
            notification_from(&mut listener, pid)
        )
        .await
        .is_err(),
        "no-op controller ticks must not wake storage scans"
    );
    sqlx::query("UPDATE agent_run SET state='Interrupted' WHERE id='notification-run'")
        .execute(&mut *connection)
        .await
        .unwrap();
    let notification = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        notification_from(&mut listener, pid),
    )
    .await
    .unwrap();
    assert_eq!(notification.channel(), "storage_phase_ended");
    drop(listener);
    drop(connection);
    pool.close().await;
}

async fn notification_from(
    listener: &mut sqlx::postgres::PgListener,
    pid: i32,
) -> sqlx::postgres::PgNotification {
    loop {
        let event = listener.recv().await.unwrap();
        if event.process_id() == pid as u32 {
            return event;
        }
    }
}

#[tokio::test]
async fn preparation_identity_mismatch_is_protected_and_deleted_material_stays_deleted() {
    let pool = fixture().await;
    let tree = Tree::new();
    let config = deployment(&tree);
    store::install(&pool, &config).await.unwrap();
    run(&pool, "preparation-owner").await;
    let name = ".preparation-preparation-owner-retained";
    let directory = config.execution.path.join(name);
    fs::create_dir(&directory).unwrap();
    fs::write(
        directory.join("storage-owner.json"),
        br#"{"run":"different-owner"}"#,
    )
    .unwrap();
    let mut tx = pool.begin().await.unwrap();
    codexsymphony_server::storage_inventory::discover(&mut tx, &config, 100)
        .await
        .unwrap();
    let protection: String =
        sqlx::query_scalar("SELECT protection FROM storage_material WHERE id=$1")
            .bind(name)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    assert_eq!(protection, "unknown preparation identity");
    sqlx::query("UPDATE storage_material SET status='deleted' WHERE id=$1")
        .bind(name)
        .execute(&mut *tx)
        .await
        .unwrap();
    codexsymphony_server::storage_inventory::discover(&mut tx, &config, 101)
        .await
        .unwrap();
    let status: String = sqlx::query_scalar("SELECT status FROM storage_material WHERE id=$1")
        .bind(name)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(status, "deleted");
    tx.commit().await.unwrap();
}

#[path = "support/delivery.rs"]
mod merge_storage_database;

#[tokio::test]
async fn merged_and_test_merge_material_is_inventoried_without_losing_originals() {
    let pool = merge_storage_database::database().await;
    let tree = Tree::new();
    let config = deployment(&tree);
    store::install(&pool, &config).await.unwrap();
    sqlx::query("INSERT INTO merge_operation(action_key,delivery_key,requirement_id,intent,state,created_at,next_attempt_at,merged_sha) SELECT 'storage-merge',action_key,requirement_id,'{\"checkout_sha\":\"pre-sha\"}'::jsonb,'merged',1,1,'post-sha' FROM delivery")
        .execute(&pool).await.unwrap();
    for path in [
        "validations/pre-merge-storage-merge-pre-sha",
        "validations/post-merge-storage-merge",
        "workspaces/runs/merge-storage-merge-pre-sha",
        "workspaces/runs/merge-storage-merge-post-sha",
    ] {
        let path = config.execution.path.join(path);
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("original"), b"retained independent validation").unwrap();
    }
    let mut tx = pool.begin().await.unwrap();
    codexsymphony_server::storage_inventory::discover(&mut tx, &config, 100)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT id,path FROM storage_material WHERE id LIKE 'merge-storage-merge-%' ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 4);
    for (_, path) in rows {
        assert_eq!(
            fs::read(PathBuf::from(path).join("original")).unwrap(),
            b"retained independent validation"
        );
    }
    pool.close().await;
}
