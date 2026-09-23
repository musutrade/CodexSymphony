use codexsymphony_server::{
    execution::{Launch, RunKey},
    extension_contract::{HookConfig, HookEvent, HookRole, ReplayPolicy},
    git_broker::GitBroker,
    process, project_hooks,
    workspace::Workspace,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

fn temp() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "project-hooks-{}",
        process::new_identity().unwrap()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

async fn fixture(repo: &Value) -> PgPool {
    let options: PgConnectOptions = std::env::var("TEST_DATABASE_URL").unwrap().parse().unwrap();
    let admin = PgPoolOptions::new()
        .connect_with(options.clone())
        .await
        .unwrap();
    let schema = format!(
        "hooks_{}",
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
    sqlx::query("INSERT INTO repository(id,version,document) VALUES(1,1,$1)")
        .bind(repo)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO requirement(id,version,state,contract,revision) OVERRIDING SYSTEM VALUE VALUES(1,1,'Running','{}',1)")
        .execute(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO requirement_revision(requirement_id,revision,document) VALUES(1,1,$1)",
    )
    .bind(json!({"repository":repo,"repository_id":1,"repository_version":1}))
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE execution_control SET requirement_id=1,incarnation='boot',recovery_complete=true WHERE id=1")
        .execute(&pool).await.unwrap();
    pool
}

fn script(path: &Path) -> String {
    fs::write(path, r##"#!/usr/bin/python3
import json, os, pathlib, subprocess, sys, time
r = json.load(sys.stdin)
counter = pathlib.Path(sys.argv[1])
with counter.open('a') as out: out.write(r['event'] + '\n')
mode = sys.argv[2]
if mode == 'sleep':
    subprocess.Popen(['/usr/bin/python3', '-c', 'import os,time;os.setsid();time.sleep(120)'])
    time.sleep(120)
if mode == 'fail':
    status = 'failed'
    extra = {'error': {'code':'project_failure','message':'controlled failure'}}
else:
    status = 'success'
    extra = {'artifacts': []}
if mode == 'artifact':
    (pathlib.Path(r['output_dir']) / 'summary.txt').write_text('auxiliary evidence')
    extra = {'artifacts': [{'path':'summary.txt','kind':'diagnostic'}]}
base = {key:r[key] for key in ('protocol_version','requirement_id','revision','run_id','resource_id','invocation_id','attempt','config_id')}
print(json.dumps(dict(base,status=status,**extra)), flush=True)
"##).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    format!("sha256:{:x}", Sha256::digest(fs::read(path).unwrap()))
}

#[allow(clippy::too_many_arguments)]
fn hook(
    event: HookEvent,
    name: &str,
    script: &Path,
    counter: &Path,
    identity: &str,
    mode: &str,
    replay: ReplayPolicy,
    timeout: u32,
) -> HookConfig {
    HookConfig {
        name: name.into(),
        event,
        roles: vec![HookRole::Coding],
        argv: vec![
            script.to_string_lossy().into_owned(),
            counter.to_string_lossy().into_owned(),
            mode.into(),
        ],
        script_identity: identity.into(),
        timeout_seconds: timeout,
        output_limit_bytes: 8192,
        replay,
    }
}

fn identities(root: &Path) -> (Launch, Workspace) {
    let id = format!("hook-{}", process::new_identity().unwrap());
    let workspace_path = root.join("workspace");
    fs::create_dir_all(&workspace_path).unwrap();
    let key = RunKey {
        run_id: id.clone(),
        request_id: id.clone(),
        incarnation: "boot".into(),
    };
    let workspace = Workspace {
        key: key.clone(),
        identity: "workspace-identity".into(),
        requirement: 1,
        revision: 1,
        phase: "execution".into(),
        baseline: "baseline".into(),
        branch: "branch".into(),
        path: workspace_path.to_string_lossy().into_owned(),
    };
    let launch = Launch {
        key,
        workspace: workspace.path.clone(),
        workspace_identity: workspace.identity.clone(),
        program: "/bin/true".into(),
        args: vec![],
    };
    (launch, workspace)
}

fn repository(hooks: Value) -> Value {
    json!({"model":null,"hooks":hooks,"project":"synthetic","remote":"test/project",
        "github_repository_id":123,"base_branch":"main","policy":{"allowed_checks":["cargo_test"],
        "max_timeout_seconds":60,"token_limit":1000,"turn_limit":10,"model_work_seconds":600,
        "gate_recovery_policy":"one_code_repair"},"revoked":false,"reason":"reviewed"})
}

#[tokio::test]
async fn reviewed_hooks_run_once_and_loss_reconciles_without_replaying() {
    unsafe {
        std::env::set_var(
            "SYMPHONY_SUPERVISOR",
            env!("CARGO_BIN_EXE_codexsymphony-server"),
        );
    }
    let root = temp();
    let script_path = root.join("hook.py");
    let counter = root.join("calls.txt");
    let digest = script(&script_path);
    let hooks = vec![
        hook(
            HookEvent::AfterCreate,
            "created",
            &script_path,
            &counter,
            &digest,
            "ok",
            ReplayPolicy::Never,
            10,
        ),
        hook(
            HookEvent::BeforeRun,
            "prepared",
            &script_path,
            &counter,
            &digest,
            "ok",
            ReplayPolicy::Never,
            10,
        ),
        hook(
            HookEvent::AfterRun,
            "collected",
            &script_path,
            &counter,
            &digest,
            "artifact",
            ReplayPolicy::Never,
            10,
        ),
        hook(
            HookEvent::BeforeRemove,
            "removed",
            &script_path,
            &counter,
            &digest,
            "ok",
            ReplayPolicy::Never,
            10,
        ),
    ];
    let repo = repository(json!(hooks));
    let pool = fixture(&repo).await;
    let (launch, workspace) = identities(&root);
    let allow = json!({"hook_allowlist": hooks});
    assert!(
        project_hooks::register(&pool, &launch, &workspace, HookRole::Coding, &allow)
            .await
            .unwrap()
    );
    for event in [
        HookEvent::AfterCreate,
        HookEvent::BeforeRun,
        HookEvent::AfterRun,
    ] {
        assert!(
            project_hooks::event(&pool, &root, &launch.key.run_id, event.clone(), None)
                .await
                .unwrap()
        );
        assert!(
            project_hooks::event(&pool, &root, &launch.key.run_id, event, None)
                .await
                .unwrap()
        );
    }
    assert!(
        project_hooks::before_remove(
            &pool,
            &root,
            &launch.key.run_id,
            "material-identity",
            Path::new(&workspace.path)
        )
        .await
        .unwrap()
    );
    assert_eq!(fs::read_to_string(&counter).unwrap().lines().count(), 4);
    let before: (String,String) = sqlx::query_as("SELECT invocation_id,output_dir FROM project_hook_invocation WHERE run_id=$1 AND event='before_run'")
        .bind(&launch.key.run_id).fetch_one(&pool).await.unwrap();
    sqlx::query(
        "UPDATE project_hook_invocation SET status='running',result=NULL WHERE invocation_id=$1",
    )
    .bind(&before.0)
    .execute(&pool)
    .await
    .unwrap();
    assert!(
        project_hooks::event(&pool, &root, &launch.key.run_id, HookEvent::BeforeRun, None)
            .await
            .unwrap()
    );
    assert_eq!(fs::read_to_string(&counter).unwrap().lines().count(), 4);
    fs::remove_file(Path::new(&before.1).join("quiescent.json")).unwrap();
    sqlx::query(
        "UPDATE project_hook_invocation SET status='running',result=NULL WHERE invocation_id=$1",
    )
    .bind(&before.0)
    .execute(&pool)
    .await
    .unwrap();
    assert!(
        !project_hooks::event(&pool, &root, &launch.key.run_id, HookEvent::BeforeRun, None)
            .await
            .unwrap()
    );
    assert_eq!(fs::read_to_string(&counter).unwrap().lines().count(), 4);
}

#[tokio::test]
async fn timeout_stops_detached_descendants_and_preserves_unknown_boundary() {
    unsafe {
        std::env::set_var(
            "SYMPHONY_SUPERVISOR",
            env!("CARGO_BIN_EXE_codexsymphony-server"),
        );
    }
    let root = temp();
    let script_path = root.join("hook.py");
    let counter = root.join("calls.txt");
    let digest = script(&script_path);
    let hook = hook(
        HookEvent::BeforeRun,
        "slow",
        &script_path,
        &counter,
        &digest,
        "sleep",
        ReplayPolicy::Never,
        1,
    );
    let repo = repository(json!([hook]));
    let pool = fixture(&repo).await;
    let (launch, workspace) = identities(&root);
    project_hooks::register(
        &pool,
        &launch,
        &workspace,
        HookRole::Coding,
        &json!({"hook_allowlist":[hook]}),
    )
    .await
    .unwrap();
    assert!(
        !project_hooks::event(&pool, &root, &launch.key.run_id, HookEvent::BeforeRun, None)
            .await
            .unwrap()
    );
    let (status, directory): (String, String) =
        sqlx::query_as("SELECT status,output_dir FROM project_hook_invocation WHERE run_id=$1")
            .bind(&launch.key.run_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(status, "timeout");
    assert!(Path::new(&directory).join("quiescent.json").exists());
    assert!(
        !project_hooks::event(&pool, &root, &launch.key.run_id, HookEvent::BeforeRun, None)
            .await
            .unwrap()
    );
    assert_eq!(fs::read_to_string(&counter).unwrap().lines().count(), 1);
    tokio::time::sleep(Duration::from_millis(20)).await;
}

#[tokio::test]
async fn absent_hooks_keep_default_path_and_validation_role_is_explicit() {
    unsafe {
        std::env::set_var(
            "SYMPHONY_SUPERVISOR",
            env!("CARGO_BIN_EXE_codexsymphony-server"),
        );
    }
    let root = temp();
    let pool = fixture(&repository(json!([]))).await;
    let (launch, workspace) = identities(&root);
    assert!(
        !project_hooks::register(&pool, &launch, &workspace, HookRole::Coding, &Value::Null)
            .await
            .unwrap()
    );
    for event in [
        HookEvent::AfterCreate,
        HookEvent::BeforeRun,
        HookEvent::AfterRun,
        HookEvent::BeforeRemove,
    ] {
        assert!(
            project_hooks::event(&pool, &root, &launch.key.run_id, event, None)
                .await
                .unwrap()
        );
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM project_hook_invocation")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);

    let other = temp();
    let script_path = other.join("hook.py");
    let counter = other.join("calls.txt");
    let digest = script(&script_path);
    let mut validation = hook(
        HookEvent::BeforeRun,
        "validation",
        &script_path,
        &counter,
        &digest,
        "ok",
        ReplayPolicy::Never,
        10,
    );
    validation.roles = vec![HookRole::Validation];
    let pool = fixture(&repository(json!([validation]))).await;
    let (launch, workspace) = identities(&other);
    project_hooks::register(
        &pool,
        &launch,
        &workspace,
        HookRole::Validation,
        &json!({"hook_allowlist":[validation]}),
    )
    .await
    .unwrap();
    assert!(
        project_hooks::event(
            &pool,
            &other,
            &launch.key.run_id,
            HookEvent::BeforeRun,
            None
        )
        .await
        .unwrap()
    );
    assert_eq!(fs::read_to_string(&counter).unwrap().lines().count(), 1);
}

#[tokio::test]
async fn idempotent_failure_replays_once_with_same_invocation_and_new_attempt() {
    unsafe {
        std::env::set_var(
            "SYMPHONY_SUPERVISOR",
            env!("CARGO_BIN_EXE_codexsymphony-server"),
        );
    }
    let root = temp();
    let script_path = root.join("hook.py");
    let counter = root.join("calls.txt");
    let digest = script(&script_path);
    let hook = hook(
        HookEvent::BeforeRun,
        "retry",
        &script_path,
        &counter,
        &digest,
        "fail",
        ReplayPolicy::Idempotent,
        10,
    );
    let pool = fixture(&repository(json!([hook]))).await;
    let (launch, workspace) = identities(&root);
    project_hooks::register(
        &pool,
        &launch,
        &workspace,
        HookRole::Coding,
        &json!({"hook_allowlist":[hook]}),
    )
    .await
    .unwrap();
    for _ in 0..3 {
        assert!(
            !project_hooks::event(&pool, &root, &launch.key.run_id, HookEvent::BeforeRun, None)
                .await
                .unwrap()
        );
    }
    let (attempt, status): (i32, String) =
        sqlx::query_as("SELECT attempt,status FROM project_hook_invocation WHERE run_id=$1")
            .bind(&launch.key.run_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((attempt, status.as_str()), (2, "failed"));
    assert_eq!(fs::read_to_string(&counter).unwrap().lines().count(), 2);
}

#[tokio::test]
async fn cancellation_waits_for_supervisor_stop_receipt() {
    unsafe {
        std::env::set_var(
            "SYMPHONY_SUPERVISOR",
            env!("CARGO_BIN_EXE_codexsymphony-server"),
        );
    }
    let root = temp();
    let script_path = root.join("hook.py");
    let counter = root.join("calls.txt");
    let digest = script(&script_path);
    let hook = hook(
        HookEvent::BeforeRun,
        "cancel",
        &script_path,
        &counter,
        &digest,
        "sleep",
        ReplayPolicy::Never,
        10,
    );
    let pool = fixture(&repository(json!([hook]))).await;
    let (launch, workspace) = identities(&root);
    project_hooks::register(
        &pool,
        &launch,
        &workspace,
        HookRole::Coding,
        &json!({"hook_allowlist":[hook]}),
    )
    .await
    .unwrap();
    let copy = pool.clone();
    let run = launch.key.run_id.clone();
    let output = root.clone();
    let work = tokio::spawn(async move {
        project_hooks::event(&copy, &output, &run, HookEvent::BeforeRun, None)
            .await
            .unwrap()
    });
    for _ in 0..100 {
        if counter.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(counter.exists());
    sqlx::query("UPDATE requirement SET cancel_requested=true WHERE id=1")
        .execute(&pool)
        .await
        .unwrap();
    assert!(!work.await.unwrap());
    let (status, directory): (String, String) =
        sqlx::query_as("SELECT status,output_dir FROM project_hook_invocation WHERE run_id=$1")
            .bind(&launch.key.run_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(status, "cancelled");
    assert!(Path::new(&directory).join("quiescent.json").exists());
}

fn git(path: &Path, args: &[&str]) -> String {
    let output = Command::new("/usr/bin/git")
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
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().into()
}

#[tokio::test]
async fn after_run_requires_quiescence_and_preserved_snapshot_but_auxiliary_failure_keeps_fact() {
    unsafe {
        std::env::set_var(
            "SYMPHONY_SUPERVISOR",
            env!("CARGO_BIN_EXE_codexsymphony-server"),
        );
    }
    let root = temp();
    let script_path = root.join("hook.py");
    let counter = root.join("calls.txt");
    let digest = script(&script_path);
    let hook = hook(
        HookEvent::AfterRun,
        "auxiliary",
        &script_path,
        &counter,
        &digest,
        "fail",
        ReplayPolicy::Never,
        10,
    );
    let pool = fixture(&repository(json!([hook]))).await;
    let seed = root.join("seed");
    fs::create_dir(&seed).unwrap();
    git(&seed, &["init", "--template=", "-b", "main"]);
    fs::write(seed.join("source.txt"), "baseline\n").unwrap();
    git(&seed, &["add", "source.txt"]);
    git(&seed, &["commit", "-m", "baseline"]);
    let baseline = git(&seed, &["rev-parse", "HEAD"]);
    let bundle = root.join("source.bundle");
    git(
        &seed,
        &["bundle", "create", bundle.to_str().unwrap(), "--all"],
    );
    let broker = GitBroker::initialize(&root.join("workspaces"), &bundle).unwrap();
    let id = format!("run-{}", process::new_identity().unwrap());
    let key = RunKey {
        run_id: id.clone(),
        request_id: id.clone(),
        incarnation: "boot".into(),
    };
    let workspace = Workspace {
        key: key.clone(),
        identity: "reviewed-workspace".into(),
        requirement: 1,
        revision: 1,
        phase: "execution".into(),
        baseline,
        branch: format!("ai/req-1-{id}"),
        path: broker.path(&id).unwrap().to_string_lossy().into_owned(),
    };
    broker.prepare(&workspace, true).unwrap();
    let manifest = broker.preserve(&workspace).unwrap();
    let launch = Launch {
        key: key.clone(),
        workspace: workspace.path.clone(),
        workspace_identity: workspace.identity.clone(),
        program: "/bin/true".into(),
        args: vec![],
    };
    project_hooks::register(
        &pool,
        &launch,
        &workspace,
        HookRole::Coding,
        &json!({"hook_allowlist":[hook]}),
    )
    .await
    .unwrap();
    sqlx::query("INSERT INTO agent_run(id,requirement_id,revision,incarnation,request_id,workspace,workspace_identity,launch,state,quiescent) VALUES($1,1,1,'boot',$1,$2,$3,$4,'Failed',false)")
        .bind(&id).bind(&workspace.path).bind(&workspace.identity).bind(json!(launch)).execute(&pool).await.unwrap();
    project_hooks::after_run(&pool, &root).await.unwrap();
    assert!(!counter.exists());
    sqlx::query("UPDATE agent_run SET quiescent=true WHERE id=$1")
        .bind(&id)
        .execute(&pool)
        .await
        .unwrap();
    project_hooks::after_run(&pool, &root).await.unwrap();
    assert!(!counter.exists());
    sqlx::query("INSERT INTO workspace_snapshot(run_id,manifest,candidate) VALUES($1,$2,false)")
        .bind(&id)
        .bind(json!(manifest))
        .execute(&pool)
        .await
        .unwrap();
    project_hooks::after_run(&pool, &root).await.unwrap();
    assert_eq!(fs::read_to_string(&counter).unwrap().lines().count(), 1);
    let status: String =
        sqlx::query_scalar("SELECT status FROM project_hook_invocation WHERE run_id=$1")
            .bind(&id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(status, "failed");
    let state: String = sqlx::query_scalar("SELECT state FROM agent_run WHERE id=$1")
        .bind(&id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(state, "Failed");
    broker.verify(&manifest).unwrap();
}
