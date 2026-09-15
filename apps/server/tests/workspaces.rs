//! GH-15: real Git object graphs and isolated PostgreSQL failure fixtures.
use codexsymphony_server::{
    execution::RunKey,
    git_broker::GitBroker,
    process,
    workspace::{Manifest, Recovery, Workspace, recovery},
    workspace_store::{self, Operation},
};
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    process::Command,
};

// Advisory locks are database-wide even though each fixture has its own schema.
static DATABASE_TEST: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn git(path: &Path, args: &[&str]) -> Vec<u8> {
    let output = Command::new("/usr/bin/git")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@localhost")
        .env("GIT_COMMITTER_NAME", "fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@localhost")
        .arg("-C")
        .arg(path)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {:?}: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}
fn text(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).unwrap().trim().into()
}

struct Fixture {
    root: PathBuf,
    broker: GitBroker,
    baseline: String,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("gh15-{}", process::new_identity().unwrap()));
        fs::create_dir(&root).unwrap();
        let seed = root.join("seed");
        fs::create_dir(&seed).unwrap();
        git(&seed, &["init", "--template=", "-b", "main"]);
        fs::write(seed.join("source.rs"), b"baseline\n").unwrap();
        fs::write(seed.join("deleted.rs"), b"delete me\n").unwrap();
        git(&seed, &["add", "."]);
        git(&seed, &["commit", "-m", "baseline"]);
        let baseline = text(git(&seed, &["rev-parse", "HEAD"]));
        let bundle = root.join("seed.bundle");
        git(
            &seed,
            &["bundle", "create", bundle.to_str().unwrap(), "--all"],
        );
        let broker = GitBroker::initialize(&root.join("managed"), &bundle).unwrap();
        Self {
            root,
            broker,
            baseline,
        }
    }
    fn workspace(&self, id: &str) -> Workspace {
        Workspace {
            key: RunKey {
                run_id: id.into(),
                request_id: format!("request-{id}"),
                incarnation: "current".into(),
            },
            identity: format!("identity-{id}"),
            requirement: 1,
            revision: 1,
            phase: "execution".into(),
            baseline: self.baseline.clone(),
            branch: format!("ai/req-1-{id}"),
            path: self.broker.path(id).unwrap().to_str().unwrap().into(),
        }
    }
}

fn dirty(workspace: &Workspace) -> String {
    let path = Path::new(&workspace.path);
    fs::write(path.join("source.rs"), b"intermediate\n").unwrap();
    git(path, &["add", "source.rs"]);
    git(path, &["commit", "-m", "reachable intermediate"]);
    let intermediate = text(git(path, &["rev-parse", "HEAD"]));
    fs::write(path.join("source.rs"), b"staged\n").unwrap();
    fs::write(path.join("binary.dat"), [0, 255, 128, 0, 12]).unwrap();
    fs::write(path.join("run.sh"), b"#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(path.join("run.sh"), fs::Permissions::from_mode(0o755)).unwrap();
    git(path, &["add", "."]);
    fs::write(path.join("source.rs"), b"unstaged\n").unwrap();
    fs::write(path.join("binary.dat"), [255, 0, 128, 1, 99]).unwrap();
    // Avoid a same-size, same-timestamp fixture write being hidden by Git's
    // stat cache after write-tree refreshes the index timestamp.
    fs::File::open(path.join("binary.dat"))
        .unwrap()
        .set_modified(std::time::SystemTime::UNIX_EPOCH)
        .unwrap();
    fs::remove_file(path.join("deleted.rs")).unwrap();
    fs::create_dir(path.join("tests")).unwrap();
    fs::write(path.join("tests/untracked.rs"), b"paid test work\n").unwrap();
    fs::write(path.join("progress.md"), b"resume here\n").unwrap();
    fs::create_dir(path.join("target")).unwrap();
    fs::write(path.join("target/cache"), b"rebuildable").unwrap();
    intermediate
}

#[test]
fn git_round_trip_and_security() {
    let f = Fixture::new();
    let original = f.workspace("original");
    f.broker.prepare(&original, true).unwrap();
    assert!(f.broker.prepare(&original, true).is_err());
    let intermediate = dirty(&original);
    let commit = f
        .broker
        .commit(&original, "--help; $(touch /tmp/never-executed)")
        .unwrap();
    assert_ne!(commit, intermediate);
    // Reintroduce staged/unstaged divergence after the Broker commit.
    let path = Path::new(&original.path);
    fs::write(path.join("after-commit.md"), b"uncommitted progress\n").unwrap();
    fs::write(path.join("tests/new-untracked.rs"), b"new paid test\n").unwrap();
    fs::write(path.join("binary.dat"), [21, 0, 64, 255, 0]).unwrap();
    fs::File::open(path.join("binary.dat"))
        .unwrap()
        .set_modified(std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(123))
        .unwrap();
    fs::remove_file(path.join("tests/untracked.rs")).unwrap();
    fs::write(path.join("source.rs"), b"second staged\n").unwrap();
    git(path, &["add", "source.rs"]);
    fs::write(path.join("source.rs"), b"second unstaged\n").unwrap();
    let before = [
        git(path, &["diff", "--binary"]),
        git(path, &["diff", "--cached", "--binary"]),
        git(
            path,
            &["status", "--porcelain", "--", ".", ":(exclude)target"],
        ),
        git(path, &["log", "--format=%H"]),
    ];
    let manifest = f.broker.preserve(&original).unwrap();
    assert_eq!(manifest.excluded, ["target"]);
    assert!(
        manifest
            .files
            .iter()
            .any(|entry| entry.path == "run.sh" && entry.executable)
    );
    assert!(f.broker.preserve(&original).is_err());
    let mut restored = f.workspace("restored");
    restored.baseline = manifest.head.clone();
    f.broker.restore(&restored, &manifest).unwrap();
    assert!(f.broker.restore(&restored, &manifest).is_err());
    let destination = Path::new(&restored.path);
    let after = [
        git(destination, &["diff", "--binary"]),
        git(destination, &["diff", "--cached", "--binary"]),
        git(destination, &["status", "--porcelain"]),
        git(destination, &["log", "--format=%H"]),
    ];
    assert_eq!(before, after);
    assert_eq!(
        git(path, &["show", &format!("{intermediate}:source.rs")]),
        git(destination, &["show", &format!("{intermediate}:source.rs")])
    );
    assert_eq!(
        fs::read(path.join("binary.dat")).unwrap(),
        fs::read(destination.join("binary.dat")).unwrap()
    );
    assert!(!destination.join("target").exists());
    assert!(path.join("progress.md").exists());
    let mut candidate = f.workspace("candidate");
    candidate.baseline = manifest.head.clone();
    f.broker.restore_candidate(&candidate, &manifest).unwrap();
    assert!(git(Path::new(&candidate.path), &["status", "--porcelain"]).is_empty());
    assert!(!Path::new(&candidate.path).join("after-commit.md").exists());
    // A replacement canonical clone can recover using the archive alone.
    let replacement = GitBroker::initialize(
        &f.root.join("replacement"),
        &f.broker
            .archive_path("original")
            .unwrap()
            .join("history.bundle"),
    )
    .unwrap();
    assert!(
        Command::new("/bin/cp")
            .arg("-a")
            .arg(f.broker.archive_path("original").unwrap())
            .arg(replacement.archive_path("original").unwrap())
            .status()
            .unwrap()
            .success()
    );
    let mut replacement_run = restored.clone();
    replacement_run.path = replacement
        .path("restored")
        .unwrap()
        .to_str()
        .unwrap()
        .into();
    replacement.restore(&replacement_run, &manifest).unwrap();
    assert_eq!(
        git(Path::new(&replacement_run.path), &["log", "--format=%H"]),
        before[3]
    );
    // Hooks are outside bundle data and cannot run, even in platform metadata.
    let hooks = f.root.join("managed/canonical.git/hooks");
    fs::create_dir(&hooks).unwrap();
    let marker = f.root.join("hook-executed");
    for name in [
        "pre-commit",
        "post-commit",
        "post-checkout",
        "reference-transaction",
    ] {
        let hook = hooks.join(name);
        fs::write(&hook, format!("#!/bin/sh\ntouch {}\n", marker.display())).unwrap();
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    }
    f.broker.commit(&restored, "safe").unwrap();
    f.broker.prepare(&f.workspace("hooks"), true).unwrap();
    assert!(!marker.exists());
    let config = f.root.join("managed/canonical.git/config");
    let trusted = fs::read(&config).unwrap();
    fs::write(
        &config,
        [
            trusted.as_slice(),
            b"[credential]\n helper = !touch /tmp/never-helper\n",
        ]
        .concat(),
    )
    .unwrap();
    assert!(GitBroker::open(&f.root.join("managed")).is_err());
    assert!(f.broker.commit(&restored, "blocked").is_err());
    fs::write(&config, trusted).unwrap();
    GitBroker::open(&f.root.join("managed")).unwrap();
    assert!(git(&f.root.join("managed/canonical.git"), &["remote"]).is_empty());
    let mut wrong = restored.clone();
    wrong.path = f.root.to_str().unwrap().into();
    assert!(f.broker.head(&wrong).is_err());
    wrong = restored.clone();
    wrong.branch = "main".into();
    assert!(f.broker.head(&wrong).is_err());
    assert!(f.broker.path("../escape").is_err());
    assert!(f.broker.path("").is_err());
    assert!(f.broker.commit(&restored, "").is_err());
    let mut bad = manifest.clone();
    bad.bundle_digest = "bad".into();
    assert!(f.broker.verify(&bad).is_err());
    let archive = f.broker.archive_path("original").unwrap();
    fs::write(archive.join("files/progress.md"), b"tampered").unwrap();
    assert!(f.broker.verify(&manifest).is_err());
    fs::write(archive.join("files/progress.md"), b"resume here\n").unwrap();
    fs::write(archive.join("history.bundle"), b"not a bundle").unwrap();
    let checksum = Command::new("/usr/bin/sha256sum")
        .arg(archive.join("history.bundle"))
        .output()
        .unwrap();
    let mut malformed = manifest.clone();
    malformed.bundle_digest = String::from_utf8(checksum.stdout).unwrap()[..64].into();
    fs::write(
        archive.join("manifest.json"),
        serde_json::to_vec(&malformed).unwrap(),
    )
    .unwrap();
    assert!(f.broker.verify(&malformed).is_err());
}

#[test]
fn preservation_failures_leave_sources_and_partial_files() {
    let f = Fixture::new();
    assert!(GitBroker::initialize(&f.root.join("managed"), &f.root.join("seed.bundle")).is_err());
    let mut wrong = f.workspace("wrong-branch");
    wrong.branch = "main".into();
    assert!(f.broker.prepare(&wrong, true).is_err());
    let regular = f.root.join("exclusive");
    codexsymphony_server::workspace_files::write(&regular, b"original", false).unwrap();
    assert!(codexsymphony_server::workspace_files::write(&regular, b"overwrite", false).is_err());
    assert_eq!(fs::read(&regular).unwrap(), b"original");
    assert!(
        codexsymphony_server::workspace_files::write(Path::new("/"), b"invalid", false).is_err()
    );
    let fifo = f.root.join("fifo");
    assert!(
        Command::new("/usr/bin/mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    assert!(codexsymphony_server::workspace_files::read(&fifo).is_err());
    let history = f.workspace("secret-history");
    f.broker.prepare(&history, true).unwrap();
    let directory = Path::new(&history.path);
    fs::write(directory.join(".env"), b"synthetic private fixture").unwrap();
    git(directory, &["add", ".env"]);
    git(directory, &["commit", "-m", "historical credential path"]);
    git(directory, &["rm", ".env"]);
    git(directory, &["commit", "-m", "removed from current tree"]);
    assert!(f.broker.preserve(&history).is_err());
    assert!(
        !f.broker
            .archive_path("secret-history")
            .unwrap()
            .join("history.bundle")
            .exists()
    );
    for (id, name) in [("link", "outside"), ("secret", ".env"), ("cache", "target")] {
        let workspace = f.workspace(id);
        f.broker.prepare(&workspace, true).unwrap();
        let path = Path::new(&workspace.path).join(name);
        if id == "link" {
            symlink(&f.root, &path).unwrap();
        } else {
            fs::write(&path, b"must not disappear").unwrap();
        }
        if id == "cache" {
            git(Path::new(&workspace.path), &["add", "target"]);
        }
        assert!(f.broker.preserve(&workspace).is_err());
        let archive = f.broker.archive_path(id).unwrap();
        assert!(archive.join("pending").exists());
        assert!(!archive.join("manifest.json").exists());
        assert!(path.exists());
    }
    let workspace = f.workspace("bad-index");
    f.broker.prepare(&workspace, true).unwrap();
    let index = f
        .root
        .join("managed/canonical.git/worktrees/bad-index/index");
    fs::write(&index, b"corrupt").unwrap();
    assert!(f.broker.preserve(&workspace).is_err());
    assert!(index.exists());
    let workspace = f.workspace("wrong-pointer");
    f.broker.prepare(&workspace, true).unwrap();
    fs::write(
        Path::new(&workspace.path).join(".git"),
        b"gitdir: /tmp/elsewhere\n",
    )
    .unwrap();
    assert!(f.broker.preserve(&workspace).is_err());
    assert!(f.broker.prepare(&f.workspace("bad-sha"), false).is_ok());
    let mut workspace = f.workspace("invalid");
    workspace.baseline = "--all".into();
    assert!(f.broker.prepare(&workspace, true).is_err());
    for (phase, candidate, validation, work, expected) in [
        ("handoff", true, true, true, Recovery::Handoff),
        ("handoff", false, true, true, Recovery::Blocked),
        ("validation", true, false, true, Recovery::Validate),
        (
            "declaration",
            false,
            false,
            false,
            Recovery::PreserveDeclaration,
        ),
        ("execution", false, false, true, Recovery::Work),
        ("execution", false, false, false, Recovery::Blocked),
    ] {
        assert_eq!(recovery(phase, candidate, validation, work), expected);
    }
}

async fn database() -> PgPool {
    let url = std::env::var("TEST_DATABASE_URL").expect("disposable database required");
    let options: PgConnectOptions = url.parse().unwrap();
    let admin = PgPoolOptions::new()
        .connect_with(options.clone())
        .await
        .unwrap();
    let schema = format!(
        "workspace_{}",
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
    sqlx::raw_sql("INSERT INTO repository(id,version,document) VALUES (1,1,'{\"revoked\":false}'); INSERT INTO requirement(version,state,contract,revision) VALUES (1,'Running','{}',1); INSERT INTO requirement_revision(requirement_id,revision,document) VALUES (1,1,'{\"repository_version\":1}'); UPDATE execution_control SET requirement_id=1,incarnation='current',recovery_complete=true;").execute(&pool).await.unwrap();
    pool
}
async fn insert(pool: &PgPool, workspace: &Workspace) {
    sqlx::query("INSERT INTO agent_run(id,requirement_id,revision,incarnation,request_id,workspace,workspace_identity,launch,state,phase) VALUES ($1,1,1,'current',$2,$3,$4,'{}','Created',$5)")
        .bind(&workspace.key.run_id).bind(&workspace.key.request_id).bind(&workspace.path).bind(&workspace.identity).bind(&workspace.phase).execute(pool).await.unwrap();
}
async fn state(pool: &PgPool, id: &str, state: &str, quiet: bool) {
    sqlx::query("UPDATE agent_run SET state=$2,quiescent=$3 WHERE id=$1")
        .bind(id)
        .bind(state)
        .bind(quiet)
        .execute(pool)
        .await
        .unwrap();
}
async fn execute(
    pool: &PgPool,
    f: &Fixture,
    w: &Workspace,
    request: &str,
    op: Operation,
) -> serde_json::Value {
    workspace_store::execute(pool, &f.broker, &w.key, request, op)
        .await
        .unwrap()
}

#[tokio::test]
async fn authorization_idempotency_database_failure_and_recovery() {
    let _serial = DATABASE_TEST.lock().await;
    let pool = database().await;
    let f = Fixture::new();
    let original = f.workspace("run-one");
    insert(&pool, &original).await;
    let prepare = Operation::Prepare {
        baseline: f.baseline.clone(),
    };
    let first = execute(&pool, &f, &original, "prepare", prepare.clone()).await;
    assert_eq!(
        first,
        execute(&pool, &f, &original, "prepare", prepare).await
    );
    assert!(
        workspace_store::execute(
            &pool,
            &f.broker,
            &original.key,
            "prepare",
            Operation::Preserve
        )
        .await
        .is_err()
    );
    assert!(
        workspace_store::execute(
            &pool,
            &f.broker,
            &original.key,
            "commit",
            Operation::Commit {
                message: "too early".into()
            }
        )
        .await
        .is_err()
    );
    state(&pool, "run-one", "Running", false).await;
    dirty(&original);
    let op = Operation::Commit {
        message: "candidate".into(),
    };
    let committed = execute(&pool, &f, &original, "commit", op.clone()).await;
    assert_eq!(committed, execute(&pool, &f, &original, "commit", op).await);
    sqlx::query("UPDATE requirement SET paused=true")
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        workspace_store::execute(
            &pool,
            &f.broker,
            &original.key,
            "paused",
            Operation::Commit {
                message: "denied".into()
            }
        )
        .await
        .is_err()
    );
    state(&pool, "run-one", "Interrupted", true).await;
    let saved = execute(&pool, &f, &original, "preserve", Operation::Preserve).await;
    assert_eq!(
        saved,
        execute(&pool, &f, &original, "preserve", Operation::Preserve).await
    );
    let manifest: Manifest = serde_json::from_value(saved).unwrap();
    let restored = f.workspace("run-two");
    sqlx::query("UPDATE agent_run SET created_at='infinity' WHERE id='run-one'")
        .execute(&pool)
        .await
        .unwrap();
    insert(&pool, &restored).await;
    assert!(
        workspace_store::execute(
            &pool,
            &f.broker,
            &original.key,
            "stale",
            Operation::Preserve
        )
        .await
        .is_err()
    );
    assert!(
        workspace_store::execute(
            &pool,
            &f.broker,
            &restored.key,
            "restore",
            Operation::Restore {
                source: "run-one".into()
            }
        )
        .await
        .is_err()
    );
    sqlx::query("UPDATE requirement SET paused=false")
        .execute(&pool)
        .await
        .unwrap();
    let restored_value = execute(
        &pool,
        &f,
        &restored,
        "restore",
        Operation::Restore {
            source: "run-one".into(),
        },
    )
    .await;
    let restored: Workspace = serde_json::from_value(restored_value).unwrap();
    assert_eq!(restored.baseline, manifest.head);
    assert_eq!(
        git(Path::new(&original.path), &["diff", "--binary"]),
        git(Path::new(&restored.path), &["diff", "--binary"])
    );
    state(&pool, "run-two", "Interrupted", true).await;
    // A real SQL failure AFTER files have been durably written must roll back
    // the manifest reference and leave a partial intent and both source/files.
    sqlx::raw_sql("CREATE FUNCTION reject_snapshot() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected persistence failure'; END $$; CREATE TRIGGER fail_snapshot BEFORE INSERT ON workspace_snapshot FOR EACH ROW EXECUTE FUNCTION reject_snapshot();").execute(&pool).await.unwrap();
    assert!(
        workspace_store::execute(
            &pool,
            &f.broker,
            &restored.key,
            "save-fails",
            Operation::Preserve
        )
        .await
        .is_err()
    );
    let status: String = sqlx::query_scalar(
        "SELECT status FROM workspace_operation WHERE run_id='run-two' AND request_id='save-fails'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(status, "partial");
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM workspace_snapshot WHERE run_id='run-two'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 0);
    assert!(
        f.broker
            .archive_path("run-two")
            .unwrap()
            .join("manifest.json")
            .exists()
    );
    assert!(Path::new(&restored.path).join("progress.md").exists());
    assert!(
        !codexsymphony_server::coordinator::recover(&pool, &f.root, "current")
            .await
            .unwrap()
    );
    assert!(
        !workspace_store::recover_stopped(&pool, &f.root.join("managed"))
            .await
            .unwrap()
    );
    assert!(
        !codexsymphony_server::run_store::finish_recovery(&pool, "current")
            .await
            .unwrap()
    );
    sqlx::query("DROP TRIGGER fail_snapshot ON workspace_snapshot")
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        workspace_store::execute(
            &pool,
            &f.broker,
            &restored.key,
            "save-fails",
            Operation::Preserve
        )
        .await
        .is_err()
    );
    // Recovered process crash with a durable intent must not start a new copy.
    let pending = f.workspace("pending");
    insert(&pool, &pending).await;
    sqlx::query(
        "INSERT INTO workspace_operation VALUES ('pending','crashed',$1,'pending',NULL,NULL)",
    )
    .bind(
        serde_json::to_value(Operation::Prepare {
            baseline: f.baseline.clone(),
        })
        .unwrap(),
    )
    .execute(&pool)
    .await
    .unwrap();
    assert!(
        workspace_store::execute(
            &pool,
            &f.broker,
            &pending.key,
            "crashed",
            Operation::Prepare {
                baseline: f.baseline.clone()
            }
        )
        .await
        .is_err()
    );
    assert!(!Path::new(&pending.path).exists());
    pool.close().await;
}

#[tokio::test]
async fn stage_selection_and_cold_start_preservation() {
    let _serial = DATABASE_TEST.lock().await;
    let pool = database().await;
    let f = Fixture::new();
    let original = f.workspace("validation-source");
    insert(&pool, &original).await;
    execute(
        &pool,
        &f,
        &original,
        "prepare",
        Operation::Prepare {
            baseline: f.baseline.clone(),
        },
    )
    .await;
    state(&pool, &original.key.run_id, "Running", false).await;
    dirty(&original);
    execute(
        &pool,
        &f,
        &original,
        "commit",
        Operation::Commit {
            message: "candidate".into(),
        },
    )
    .await;
    fs::write(
        Path::new(&original.path).join("after-commit.md"),
        b"new paid progress\n",
    )
    .unwrap();
    sqlx::query("UPDATE agent_run SET phase='validation'")
        .execute(&pool)
        .await
        .unwrap();
    state(&pool, &original.key.run_id, "Interrupted", true).await;
    // Cold start has a new incarnation and closed startup gate; preservation
    // still runs once the old execution group has proven quiescent.
    codexsymphony_server::run_store::begin_incarnation(&pool, "new-incarnation")
        .await
        .unwrap();
    assert!(
        codexsymphony_server::coordinator::recover(&pool, &f.root, "new-incarnation")
            .await
            .is_err()
    );
    assert!(
        !codexsymphony_server::run_store::finish_recovery(&pool, "new-incarnation")
            .await
            .unwrap()
    );
    assert!(
        workspace_store::recover_stopped(&pool, &f.root.join("managed"))
            .await
            .unwrap()
    );
    assert!(
        workspace_store::recover_stopped(&pool, &f.root.join("managed"))
            .await
            .unwrap()
    );
    assert!(
        codexsymphony_server::run_store::finish_recovery(&pool, "new-incarnation")
            .await
            .unwrap()
    );
    sqlx::query("UPDATE execution_control SET incarnation='current'")
        .execute(&pool)
        .await
        .unwrap();
    let mut target = f.workspace("validation-target");
    target.phase = "validation".into();
    insert(&pool, &target).await;
    let value = execute(
        &pool,
        &f,
        &target,
        "restore",
        Operation::Restore {
            source: original.key.run_id.clone(),
        },
    )
    .await;
    let restored: Workspace = serde_json::from_value(value).unwrap();
    assert!(git(Path::new(&restored.path), &["status", "--porcelain"]).is_empty());
    assert!(!Path::new(&restored.path).join("after-commit.md").exists());
    // The full paid work is still in the source archive, separate from the
    // immutable candidate selected for validation.
    assert!(
        f.broker
            .archive_path(&original.key.run_id)
            .unwrap()
            .join("files/after-commit.md")
            .exists()
    );
    state(&pool, &target.key.run_id, "Interrupted", true).await;
    let mut stale = f.workspace("historical-target");
    stale.phase = "validation".into();
    insert(&pool, &stale).await;
    assert!(
        workspace_store::execute(
            &pool,
            &f.broker,
            &stale.key,
            "restore",
            Operation::Restore {
                source: original.key.run_id
            }
        )
        .await
        .is_err()
    );
    assert!(!Path::new(&stale.path).exists());
    pool.close().await;
}

#[tokio::test]
async fn invalidated_candidate_and_wrong_ownership_cannot_restore() {
    let _serial = DATABASE_TEST.lock().await;
    let pool = database().await;
    let f = Fixture::new();
    let original = f.workspace("original");
    insert(&pool, &original).await;
    execute(
        &pool,
        &f,
        &original,
        "prepare",
        Operation::Prepare {
            baseline: f.baseline.clone(),
        },
    )
    .await;
    state(&pool, &original.key.run_id, "Running", false).await;
    sqlx::query("UPDATE agent_run SET workspace_identity='wrong'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        workspace_store::execute(
            &pool,
            &f.broker,
            &original.key,
            "wrong-owner",
            Operation::Commit {
                message: "no".into()
            }
        )
        .await
        .is_err()
    );
    assert_eq!(f.broker.head(&original).unwrap(), f.baseline);
    // Only the fixture operator repairs the failed intent/ownership.
    sqlx::query("DELETE FROM workspace_operation WHERE request_id='wrong-owner'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE agent_run SET workspace_identity=$1,stop_requested=false,blocker=NULL")
        .bind(&original.identity)
        .execute(&pool)
        .await
        .unwrap();
    execute(
        &pool,
        &f,
        &original,
        "commit",
        Operation::Commit {
            message: "candidate".into(),
        },
    )
    .await;
    state(&pool, &original.key.run_id, "Interrupted", true).await;
    sqlx::query("UPDATE agent_run SET phase='validation'")
        .execute(&pool)
        .await
        .unwrap();
    execute(&pool, &f, &original, "preserve", Operation::Preserve).await;
    sqlx::query("UPDATE run_workspace SET candidate_sha=NULL")
        .execute(&pool)
        .await
        .unwrap();
    let mut target = f.workspace("target");
    target.phase = "validation".into();
    insert(&pool, &target).await;
    assert!(
        workspace_store::execute(
            &pool,
            &f.broker,
            &target.key,
            "restore",
            Operation::Restore {
                source: original.key.run_id.clone()
            }
        )
        .await
        .is_err()
    );
    assert!(!Path::new(&target.path).exists());
    pool.close().await;
}

#[test]
fn disk_write_failure_child() {
    let Ok(root) = std::env::var("GH15_FAILURE_ROOT") else {
        return;
    };
    let workspace: Workspace =
        serde_json::from_str(&std::env::var("GH15_FAILURE_WORKSPACE").unwrap()).unwrap();
    let broker = GitBroker::open(Path::new(&root)).unwrap();
    let failed = broker.preserve(&workspace).is_err();
    let (target, manifest): (Workspace, Manifest) =
        serde_json::from_str(&std::env::var("GH15_FAILURE_RESTORE").unwrap()).unwrap();
    let restore_failed = broker.restore(&target, &manifest).is_err();
    // Restore the fixture limit before LLVM flushes its real counters at exit.
    // Otherwise the injected disk error would also truncate coverage evidence.
    assert!(
        Command::new("/usr/bin/prlimit")
            .args([
                "--pid",
                &std::process::id().to_string(),
                "--fsize=unlimited:unlimited"
            ])
            .status()
            .unwrap()
            .success()
    );
    assert!(failed);
    assert!(restore_failed);
    assert!(Path::new(&target.path).join("large.bin").exists());
    assert!(
        broker
            .archive_path(&workspace.key.run_id)
            .unwrap()
            .join("pending")
            .exists()
    );
    assert!(
        !broker
            .archive_path(&workspace.key.run_id)
            .unwrap()
            .join("manifest.json")
            .exists()
    );
    assert_eq!(
        fs::metadata(Path::new(&workspace.path).join("large.bin"))
            .unwrap()
            .len(),
        32768
    );
}

#[test]
fn operating_system_write_failure_preserves_original() {
    let f = Fixture::new();
    let workspace = f.workspace("write-failure");
    f.broker.prepare(&workspace, true).unwrap();
    fs::write(
        Path::new(&workspace.path).join("large.bin"),
        vec![123; 32768],
    )
    .unwrap();
    let source = f.workspace("restore-source");
    f.broker.prepare(&source, true).unwrap();
    fs::write(Path::new(&source.path).join("large.bin"), vec![123; 32768]).unwrap();
    let manifest = f.broker.preserve(&source).unwrap();
    let mut target = f.workspace("restore-failure");
    target.baseline = manifest.head.clone();
    let output = Command::new("/usr/bin/python3")
        .args(["-c", "import os,resource,signal,sys; signal.signal(signal.SIGXFSZ,signal.SIG_IGN); resource.setrlimit(resource.RLIMIT_FSIZE,(8192,resource.getrlimit(resource.RLIMIT_FSIZE)[1])); os.execv(sys.argv[1],sys.argv[1:])"])
        .arg(std::env::current_exe().unwrap()).args(["--exact", "disk_write_failure_child", "--nocapture"])
        .env("GH15_FAILURE_ROOT", f.root.join("managed"))
        .env("GH15_FAILURE_RESTORE", serde_json::to_string(&(target, manifest)).unwrap())
        .env("GH15_FAILURE_WORKSPACE", serde_json::to_string(&workspace).unwrap()).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let archive = f.broker.archive_path(&workspace.key.run_id).unwrap();
    assert!(archive.join("files/large.bin").exists());
    assert_eq!(
        fs::metadata(archive.join("files/large.bin")).unwrap().len(),
        8192
    );
    assert!(!archive.join("manifest.json").exists());
}

#[test]
fn crash_after_files_child() {
    let Ok(root) = std::env::var("GH15_CRASH_ROOT") else {
        return;
    };
    let workspace: Workspace =
        serde_json::from_str(&std::env::var("GH15_CRASH_WORKSPACE").unwrap()).unwrap();
    GitBroker::open(Path::new(&root))
        .unwrap()
        .preserve(&workspace)
        .unwrap();
    std::process::exit(73);
}

#[tokio::test]
async fn crash_after_files_leaves_pending_without_reference() {
    let _serial = DATABASE_TEST.lock().await;
    let pool = database().await;
    let f = Fixture::new();
    let workspace = f.workspace("crashed");
    insert(&pool, &workspace).await;
    execute(
        &pool,
        &f,
        &workspace,
        "prepare",
        Operation::Prepare {
            baseline: f.baseline.clone(),
        },
    )
    .await;
    state(&pool, &workspace.key.run_id, "Interrupted", true).await;
    sqlx::query("INSERT INTO workspace_operation VALUES ('crashed','save',$1,'pending',NULL,NULL)")
        .bind(serde_json::to_value(Operation::Preserve).unwrap())
        .execute(&pool)
        .await
        .unwrap();
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "crash_after_files_child"])
        .env("GH15_CRASH_ROOT", f.root.join("managed"))
        .env(
            "GH15_CRASH_WORKSPACE",
            serde_json::to_string(&workspace).unwrap(),
        )
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(73));
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM workspace_snapshot")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    assert!(
        f.broker
            .archive_path("crashed")
            .unwrap()
            .join("manifest.json")
            .exists()
    );
    assert!(Path::new(&workspace.path).join("source.rs").exists());
    assert!(
        !workspace_store::recover_stopped(&pool, &f.root.join("managed"))
            .await
            .unwrap()
    );
    assert!(
        workspace_store::execute(
            &pool,
            &f.broker,
            &workspace.key,
            "save",
            Operation::Preserve
        )
        .await
        .is_err()
    );
    pool.close().await;
}
