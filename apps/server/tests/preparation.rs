//! GH-17 deterministic budgets, persistence admission, and retained originals.
use codexsymphony_server::{
    execution::{Launch, RunKey},
    preparation::{Evidence, Failure, NetworkEvidence, Retry, network_ready},
    preparation_store, process, run_store, storage,
};
use serde_json::json;
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{fs, path::PathBuf};
static DATABASE_TEST: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn evidence() -> Evidence {
    Evidence {
        deployment_identity: "deployment".into(),
        sandbox_identity: "uid-cwd-policy-sample".into(),
        network: NetworkEvidence {
            configuration_identity: "deployment".into(),
            allowed_domains: vec!["crates.io".into()],
            enforced: true,
            allowed_probe: true,
            denied_probe: true,
            direct_connection_rejected: true,
        },
        failures: vec![],
        sample: json!({"model_calls":0}),
    }
}

#[test]
fn fixed_retry_budget_and_pause_are_independent() {
    let failure = Failure::new(
        "preparation_dependency_missing",
        "rustc-dev missing",
        "probe.json",
    );
    let mut retry = Retry::new("preparation", 100);
    assert!(!retry.begin(100, true));
    assert!(retry.begin(100, false));
    assert!(
        !retry.begin(100, false),
        "unknown attempt cannot be replayed"
    );
    retry.fail(failure.clone(), 100);
    assert!(!retry.begin(129, false));
    assert!(retry.begin(130, false));
    retry.fail(failure.clone(), 130);
    assert!(!retry.begin(249, false));
    assert!(retry.begin(250, false));
    retry.fail(failure.clone(), 250);
    assert_eq!((retry.attempts, retry.probes, retry.todo), (3, 2, true));
    assert!(!retry.begin(500, false));
    retry.authorize_retry_group(600);
    assert_eq!(retry.attempts, 3);
    assert!(!retry.begin(600, true));
    assert!(retry.begin(600, false));
    retry.success();
    assert!(retry.last_failure.is_none());
    assert!(!retry.due(601, true));
    let mut expired = Retry::new("validation", 100);
    assert!(!expired.begin(700, false));
    assert!(expired.todo);
    assert_eq!(expired.phase, "validation");
}

#[test]
fn enforcement_requires_identity_allow_deny_and_bypass() {
    let proof = evidence();
    assert!(
        proof.failure("deployment", &[]).is_none(),
        "empty declaration still permits deployed set"
    );
    assert!(network_ready(
        "deployment",
        &["crates.io".into()],
        &proof.network
    ));
    assert!(!network_ready(
        "deployment",
        &["example.com".into()],
        &proof.network
    ));
    assert!(proof.failure("changed", &[]).is_some());
    assert_eq!(
        proof
            .failure("deployment", &["example.com".into()])
            .unwrap()
            .code,
        "network_scope_unavailable"
    );
    for field in [
        "enforced",
        "allowed_probe",
        "denied_probe",
        "direct_connection_rejected",
    ] {
        let mut value = json!(proof.network);
        value[field] = json!(false);
        assert!(!network_ready(
            "deployment",
            &[],
            &serde_json::from_value(value).unwrap()
        ));
    }
    assert!(!network_ready("", &[], &proof.network));
    let mut unknown = evidence();
    unknown.sandbox_identity.clear();
    assert!(unknown.failure("deployment", &[]).is_some());
    let mut missing = evidence();
    missing.failures.push(Failure::new(
        "preparation_path_unwritable",
        "target",
        "probe",
    ));
    assert_eq!(
        missing.failure("deployment", &[]).unwrap().code,
        "preparation_path_unwritable"
    );
}

fn root() -> PathBuf {
    let root = std::env::temp_dir().join(format!("gh17-{}", process::new_identity().unwrap()));
    fs::create_dir(&root).unwrap();
    root
}

async fn database() -> PgPool {
    let url = std::env::var("TEST_DATABASE_URL").expect("disposable PostgreSQL required");
    let schema = format!("gh17_{}", process::new_identity().unwrap().replace('-', ""));
    let admin = PgPoolOptions::new().connect(&url).await.unwrap();
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .unwrap();
    let options: PgConnectOptions = url.parse().unwrap();
    let pool = PgPoolOptions::new()
        .connect_with(options.options([("search_path", schema.as_str())]))
        .await
        .unwrap();
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
    sqlx::raw_sql("INSERT INTO repository(id,version,document) VALUES(1,1,'{\"revoked\":false,\"github_repository_id\":99}'); INSERT INTO requirement(version,state,contract,revision) VALUES(1,'Ready','{}',1); INSERT INTO requirement_revision(requirement_id,revision,document) VALUES(1,1,'{\"repository_version\":1,\"repository\":{\"model\":\"reviewed-model\"},\"contract\":{\"network_access\":[\"crates.io\"]}}'); UPDATE execution_control SET incarnation='current',recovery_complete=true; INSERT INTO github_repository(repository_id,repository_version,policy,probe_pr,capability,checked_at,stale) VALUES(99,1,'{}',1,'{\"policy\":{},\"blockers\":[]}',extract(epoch FROM now())::bigint,false)")
        .execute(&pool).await.unwrap();
    pool
}

fn launch(root: &std::path::Path) -> Launch {
    Launch {
        key: RunKey {
            run_id: process::new_identity().unwrap(),
            request_id: "request".into(),
            incarnation: "current".into(),
        },
        workspace: root.to_str().unwrap().into(),
        workspace_identity: "worktree".into(),
        program: "/should-never-start-a-model".into(),
        args: vec!["app-server".into()],
    }
}

async fn retry(pool: &PgPool, launch: &Launch) -> Retry {
    let value: serde_json::Value =
        sqlx::query_scalar("SELECT retry FROM preparation_record WHERE run_id=$1")
            .bind(&launch.key.run_id)
            .fetch_one(pool)
            .await
            .unwrap();
    serde_json::from_value(value).unwrap()
}

#[tokio::test]
async fn persisted_attempts_resume_phase_and_preserve_pause() {
    let _serial = DATABASE_TEST.lock().await;
    let pool = database().await;
    let directory = root();
    let launch = launch(&directory);
    assert!(!run_store::reserve_prepared(&pool, &launch).await.unwrap());
    let mut failure = evidence();
    failure.failures.push(Failure::new(
        "preparation_dependency_missing",
        "rustc-dev missing",
        "sandbox-probe.json",
    ));
    for now in [100, 130, 250] {
        assert!(
            preparation_store::begin(&pool, &launch, 1, 1, "preparation", now)
                .await
                .unwrap()
        );
        assert!(
            !preparation_store::finish(&pool, &launch, "deployment", &failure, now)
                .await
                .unwrap()
        );
    }
    let saved = retry(&pool, &launch).await;
    assert_eq!((saved.attempts, saved.probes, saved.todo), (3, 2, true));
    assert!(
        !preparation_store::begin(&pool, &launch, 1, 1, "execution", 1000)
            .await
            .unwrap()
    );
    assert_eq!(retry(&pool, &launch).await.phase, "preparation");
    assert!(
        preparation_store::authorize_retry(&pool, &launch.key.run_id, 1000, "")
            .await
            .is_err()
    );
    preparation_store::authorize_retry(
        &pool,
        &launch.key.run_id,
        1000,
        "administrator restored dependency",
    )
    .await
    .unwrap();
    run_store::pause(&pool, None).await.unwrap();
    assert!(
        !preparation_store::begin(&pool, &launch, 1, 1, "preparation", 1000)
            .await
            .unwrap()
    );
    sqlx::query("UPDATE execution_control SET paused=false")
        .execute(&pool)
        .await
        .unwrap();
    let now = codexsymphony_server::github_service::now();
    preparation_store::authorize_retry(&pool, &launch.key.run_id, now, "resume actual preparation")
        .await
        .unwrap();
    assert!(
        preparation_store::begin(&pool, &launch, 1, 1, "execution", now)
            .await
            .unwrap()
    );
    run_store::pause(&pool, Some(1)).await.unwrap();
    assert!(
        preparation_store::finish(&pool, &launch, "deployment", &evidence(), now)
            .await
            .unwrap()
    );
    assert!(!run_store::reserve_prepared(&pool, &launch).await.unwrap());
    assert_eq!(retry(&pool, &launch).await.attempts, 4);
    assert_eq!(retry(&pool, &launch).await.phase, "preparation");
    sqlx::query("UPDATE requirement SET paused=false")
        .execute(&pool)
        .await
        .unwrap();
    let mut changed = launch.clone();
    changed.workspace_identity = "different".into();
    assert!(!run_store::reserve_prepared(&pool, &changed).await.unwrap());
    assert!(run_store::reserve_prepared(&pool, &launch).await.unwrap());
    let model: String = sqlx::query_scalar("SELECT model FROM agent_run WHERE id=$1")
        .bind(&launch.key.run_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(model, "reviewed-model");
    assert!(PathBuf::from(&launch.workspace).exists());
}

#[tokio::test]
async fn storage_failure_latches_without_deleting_originals_or_unpausing() {
    let _serial = DATABASE_TEST.lock().await;
    let pool = database().await;
    let directory = root();
    fs::write(directory.join("original"), "retain me").unwrap();
    assert!(storage::permit(&pool, &directory).await);
    // Linux /dev/full gives deterministic ENOSPC without filling a shared disk.
    std::os::unix::fs::symlink("/dev/full", directory.join("delivery.tmp")).unwrap();
    let error = storage::write(&pool, &directory.join("delivery.json"), &"intent")
        .await
        .unwrap_err();
    assert_eq!(error.raw_os_error(), Some(28));
    let mut github_writes = 0;
    if storage::permit(&pool, &directory).await {
        github_writes += 1;
    }
    assert_eq!(github_writes, 0);
    fs::remove_file(directory.join("delivery.tmp")).unwrap();
    assert!(storage::recover(&pool, &directory).await.unwrap());
    assert!(!storage::permit(&pool, &directory.join("missing-volume")).await);
    assert!(
        !storage::permit(&pool, &directory).await,
        "recovery must be explicit"
    );
    run_store::pause(&pool, None).await.unwrap();
    assert!(storage::recover(&pool, &directory).await.unwrap());
    let paused: bool = sqlx::query_scalar("SELECT paused FROM execution_control")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(paused);
    assert_eq!(
        fs::read_to_string(directory.join("original")).unwrap(),
        "retain me"
    );
    assert!(storage::parse_available("invalid").is_err());
    assert!(storage::parse_available("18446744073709551615 2").is_err());
    assert_eq!(storage::parse_available("3 4096").unwrap(), 12288);
    // procfs reports no usable blocks. Exercise the actual capacity admission
    // branch without consuming shared disk or changing deployment mounts.
    assert_eq!(
        storage::check(std::path::Path::new("/proc"))
            .unwrap_err()
            .raw_os_error(),
        Some(28)
    );
    let mut locked = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM storage_guard FOR UPDATE")
        .execute(&mut *locked)
        .await
        .unwrap();
    assert!(
        !storage::permit(&pool, &directory).await,
        "stalled persistence refuses new actions within a fixed timeout"
    );
    locked.rollback().await.unwrap();
    assert!(storage::recover(&pool, &directory).await.unwrap());
    pool.close().await;
    assert!(
        !storage::permit(&pool, &directory).await,
        "DB failure denies external writes"
    );
    assert_eq!(
        fs::read_to_string(directory.join("original")).unwrap(),
        "retain me"
    );
    let recovery_pool = database().await;
    assert!(storage::recover(&recovery_pool, &directory).await.unwrap());
}

#[tokio::test]
async fn stale_success_requires_new_authorization_and_cannot_be_overwritten() {
    let _serial = DATABASE_TEST.lock().await;
    let pool = database().await;
    let launch = launch(&root());
    let now = codexsymphony_server::github_service::now();
    assert!(
        preparation_store::begin(&pool, &launch, 1, 1, "preparation", now - 61)
            .await
            .unwrap()
    );
    assert!(
        preparation_store::finish(&pool, &launch, "deployment", &evidence(), now - 61)
            .await
            .unwrap()
    );
    assert!(!run_store::reserve_prepared(&pool, &launch).await.unwrap());
    assert!(
        !preparation_store::begin(&pool, &launch, 1, 1, "preparation", now)
            .await
            .unwrap()
    );
    let mut altered = launch.clone();
    altered.workspace_identity = "not the same worktree".into();
    assert!(
        !preparation_store::begin(&pool, &altered, 1, 1, "preparation", now)
            .await
            .unwrap()
    );
    assert!(
        !preparation_store::finish(&pool, &launch, "changed", &evidence(), now)
            .await
            .unwrap()
    );
    preparation_store::failed_probe(
        &pool,
        &launch,
        Failure::new("unknown", "late duplicate", "fixture"),
        now,
    )
    .await
    .unwrap();
    assert!(retry(&pool, &launch).await.last_failure.is_none());
    preparation_store::authorize_retry(
        &pool,
        &launch.key.run_id,
        now,
        "revalidate stale deployment evidence",
    )
    .await
    .unwrap();
    assert!(
        preparation_store::begin(&pool, &launch, 1, 1, "preparation", now)
            .await
            .unwrap()
    );
    assert!(
        preparation_store::finish(&pool, &launch, "deployment", &evidence(), now)
            .await
            .unwrap()
    );
    assert!(run_store::reserve_prepared(&pool, &launch).await.unwrap());
    assert!(
        preparation_store::authorize_retry(&pool, &launch.key.run_id, now, "already claimed")
            .await
            .is_err()
    );
}

fn broker_fixture(
    directory: &std::path::Path,
) -> (
    codexsymphony_server::git_broker::GitBroker,
    codexsymphony_server::workspace::Workspace,
    Launch,
) {
    use codexsymphony_server::{git_broker::GitBroker, workspace::Workspace};
    use std::process::Command;
    let seed = directory.join("seed");
    fs::create_dir(&seed).unwrap();
    let git = |args: &[&str]| {
        let output = Command::new("/usr/bin/git")
            .arg("-C")
            .arg(&seed)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_AUTHOR_NAME", "fixture")
            .env("GIT_AUTHOR_EMAIL", "fixture@localhost")
            .env("GIT_COMMITTER_NAME", "fixture")
            .env("GIT_COMMITTER_EMAIL", "fixture@localhost")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    };
    git(&["init", "--template=", "-b", "main"]);
    git(&["commit", "--allow-empty", "-m", "fixture"]);
    let baseline = git(&["rev-parse", "HEAD"]);
    let bundle = directory.join("seed.bundle");
    git(&["bundle", "create", bundle.to_str().unwrap(), "--all"]);
    let broker = GitBroker::initialize(&directory.join("broker"), &bundle).unwrap();
    let mut launch = launch(directory);
    launch.workspace = broker
        .path(&launch.key.run_id)
        .unwrap()
        .to_str()
        .unwrap()
        .into();
    let workspace = Workspace {
        key: launch.key.clone(),
        identity: launch.workspace_identity.clone(),
        requirement: 1,
        revision: 1,
        phase: "preparation".into(),
        baseline,
        branch: format!("ai/req-1-{}", launch.key.run_id),
        path: launch.workspace.clone(),
    };
    broker.prepare(&workspace, true).unwrap();
    (broker, workspace, launch)
}

#[tokio::test]
async fn preparation_service_persists_real_adapter_errors_and_admits_only_success() {
    use codexsymphony_server::preparation_service::{self, Request};
    let _serial = DATABASE_TEST.lock().await;
    let pool = database().await;
    let directory = root();
    let (broker, workspace, launch) = broker_fixture(&directory);
    let adapter = directory.join("fixture.py");
    let config = json!({"deployment_identity":"deployment", "launcher":[launch.program]});
    let request = |now| Request {
        launch: &launch,
        requirement: 1,
        revision: 1,
        phase: "preparation",
        now,
        adapter: &adapter,
        config: config.clone(),
        control_directory: &directory,
        broker: &broker,
        workspace: &workspace,
    };
    let now = codexsymphony_server::github_service::now();
    fs::write(
        &adapter,
        "import sys,time\ntime.sleep(1)\nprint('missing capability',file=sys.stderr)\nsys.exit(7)\n",
    )
    .unwrap();
    assert!(
        !preparation_service::prepare(&pool, request(now))
            .await
            .unwrap()
    );
    let failed = retry(&pool, &launch).await;
    let next_attempt = failed.next_attempt_at.unwrap();
    assert!(
        next_attempt >= now + 31,
        "backoff starts after the slow probe completes"
    );
    let failure = failed.last_failure.unwrap();
    assert!(failure.detail.contains('7'));
    assert!(
        fs::read_to_string(PathBuf::from(failure.evidence).join("stderr.log"))
            .unwrap()
            .contains("missing capability")
    );
    assert!(
        !preparation_service::prepare(&pool, request(next_attempt - 1))
            .await
            .unwrap()
    );
    fs::write(&adapter, "print('malformed')\n").unwrap();
    assert!(
        !preparation_service::prepare(&pool, request(next_attempt))
            .await
            .unwrap()
    );
    let next_attempt = retry(&pool, &launch).await.next_attempt_at.unwrap();
    fs::write(&adapter, format!("import json,sys\nconfig=json.load(sys.stdin)\nassert config['workspace']=={0:?}\nprint({1:?})\n", launch.workspace, json!(evidence()).to_string())).unwrap();
    assert!(
        preparation_service::prepare(&pool, request(next_attempt))
            .await
            .unwrap()
    );
    assert!(run_store::reserve_prepared(&pool, &launch).await.unwrap());
    assert_eq!(retry(&pool, &launch).await.attempts, 3);
    assert!(
        !PathBuf::from(&launch.workspace)
            .join(".preparation")
            .exists()
    );
}

#[test]
fn deadline_completion_and_identity_mismatch_require_reconciliation() {
    let mut retry = Retry::new("preparation", 0);
    assert!(retry.begin(599, false));
    retry.complete(None, 600);
    assert!(retry.todo);
    assert_eq!(retry.last_failure.unwrap().code, "budget_exhausted");
    let mut retry = Retry::new("preparation", 0);
    assert!(retry.begin(0, false));
    retry.fail(
        Failure::new("policy_identity_mismatch", "deployment changed", "probe"),
        0,
    );
    assert!(retry.todo);
    assert!(!retry.active());
}

#[tokio::test]
async fn service_rejects_wrong_broker_and_bounds_output_and_latches_storage_failure() {
    use codexsymphony_server::preparation_service::{self, Request};
    let _serial = DATABASE_TEST.lock().await;
    let pool = database().await;
    let directory = root();
    let (broker, workspace, launch) = broker_fixture(&directory);
    let adapter = directory.join("fixture.py");
    let now = codexsymphony_server::github_service::now();
    let config = json!({"deployment_identity":"deployment", "launcher":[launch.program]});
    let prepare = |workspace, config| Request {
        launch: &launch,
        requirement: 1,
        revision: 1,
        phase: "preparation",
        now,
        adapter: &adapter,
        config,
        control_directory: &directory,
        broker: &broker,
        workspace,
    };
    assert!(
        preparation_service::prepare(&pool, prepare(&workspace, json!({})))
            .await
            .is_err()
    );
    assert!(
        preparation_service::prepare(
            &pool,
            prepare(
                &workspace,
                json!({"deployment_identity":"deployment","launcher":["different"]})
            )
        )
        .await
        .is_err()
    );
    let mut wrong_revision = workspace.clone();
    wrong_revision.revision = 2;
    assert!(
        !preparation_service::prepare(&pool, prepare(&wrong_revision, config.clone()))
            .await
            .unwrap()
    );
    preparation_store::authorize_retry(&pool, &launch.key.run_id, now, "reconciled wrong revision")
        .await
        .unwrap();
    let mut wrong = workspace.clone();
    wrong.identity = "wrong".into();
    assert!(
        !preparation_service::prepare(&pool, prepare(&wrong, config.clone()))
            .await
            .unwrap()
    );
    preparation_store::authorize_retry(
        &pool,
        &launch.key.run_id,
        now,
        "reconciled wrong workspace",
    )
    .await
    .unwrap();
    fs::write(&adapter, "print('a' * 1048577)").unwrap();
    assert!(
        !preparation_service::prepare(&pool, prepare(&workspace, config.clone()))
            .await
            .unwrap()
    );
    assert!(
        retry(&pool, &launch)
            .await
            .last_failure
            .unwrap()
            .detail
            .contains("1 MiB")
    );
    preparation_store::authorize_retry(
        &pool,
        &launch.key.run_id,
        now,
        "reconciled oversized output",
    )
    .await
    .unwrap();
    let mut proof = evidence();
    proof.failures.push(Failure::new(
        "storage_unavailable",
        "target ENOSPC",
        "sandbox",
    ));
    fs::write(&adapter, format!("print({:?})", json!(proof).to_string())).unwrap();
    assert!(
        !preparation_service::prepare(&pool, prepare(&workspace, config.clone()))
            .await
            .unwrap()
    );
    assert!(!storage::permit(&pool, &directory).await);
    assert!(
        !preparation_service::prepare(&pool, prepare(&workspace, config))
            .await
            .unwrap()
    );
    assert!(storage::recover(&pool, &directory).await.unwrap());
    // Corrupt stored retry data cannot silently start a fresh group.
    sqlx::query("UPDATE preparation_record SET retry='null' WHERE run_id=$1")
        .bind(&launch.key.run_id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        preparation_store::begin(&pool, &launch, 1, 1, "preparation", now)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn malformed_reviewed_network_declaration_cannot_admit_a_run() {
    let _serial = DATABASE_TEST.lock().await;
    let pool = database().await;
    let launch = launch(&root());
    assert!(
        preparation_store::begin(&pool, &launch, 1, 1, "preparation", 100)
            .await
            .unwrap()
    );
    sqlx::query("UPDATE requirement_revision SET document=jsonb_set(document,'{contract,network_access}','true')")
        .execute(&pool).await.unwrap();
    assert!(
        preparation_store::finish(&pool, &launch, "deployment", &evidence(), 100)
            .await
            .is_err()
    );
    assert!(!run_store::reserve_prepared(&pool, &launch).await.unwrap());
}
