use codexsymphony_server::{
    contract::{self, Repository},
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

static SUPERVISOR_ENV: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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
    sqlx::query("INSERT INTO plugin_scope(plugin_id,kind,repository_ids,enabled) SELECT 'hook:'||(h->>'name'),'all','{}'::bigint[],true FROM jsonb_array_elements($1::jsonb) h ON CONFLICT DO NOTHING")
        .bind(&repo["hooks"]).execute(&pool).await.unwrap();
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
if mode == 'hold':
    time.sleep(2)
if mode == 'overflow':
    print('x' * 10000, flush=True)
    sys.exit(0)
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

async fn await_hook_receipt(
    pool: &PgPool,
    run_id: &str,
) -> (PathBuf, codexsymphony_server::execution::Receipt) {
    for _ in 0..200 {
        let directory: Option<String> =
            sqlx::query_scalar("SELECT output_dir FROM project_hook_invocation WHERE run_id=$1")
                .bind(run_id)
                .fetch_optional(pool)
                .await
                .unwrap();
        if let Some(directory) = directory {
            let directory = PathBuf::from(directory);
            if let Ok(receipt) = process::read(&directory.join("identity.json")) {
                return (directory, receipt);
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("supervisor identity should be durable before start");
}

fn repository(hooks: Value) -> Value {
    json!({"model":null,"hooks":hooks,"project":"synthetic","remote":"test/project",
        "github_repository_id":123,"base_branch":"main","policy":{"allowed_checks":["cargo_test"],
        "max_timeout_seconds":60,"token_limit":1000,"turn_limit":10,"model_work_seconds":600,
        "gate_recovery_policy":"one_code_repair"},"revoked":false,"reason":"reviewed"})
}

#[test]
fn reviewed_repository_rejects_unbounded_or_ambiguous_hooks() {
    let root = temp();
    let script_path = root.join("hook.py");
    let digest = script(&script_path);
    let valid = hook(
        HookEvent::BeforeRun,
        "prepare",
        &script_path,
        &root.join("calls.txt"),
        &digest,
        "ok",
        ReplayPolicy::Never,
        30,
    );
    let check = |hooks: Vec<HookConfig>| {
        let repo: Repository = serde_json::from_value(repository(json!(hooks))).unwrap();
        contract::validate_repository(&repo)
    };
    assert!(check(vec![valid.clone()]).is_ok());

    let mut invalid = valid.clone();
    invalid.name.clear();
    assert_eq!(check(vec![invalid]), Err("invalid project hook"));

    for (path, identity, timeout, output) in [
        ("relative/hook".to_owned(), digest.clone(), 30, 8192),
        (valid.argv[0].clone(), "a".repeat(64), 30, 8192),
        (valid.argv[0].clone(), "sha256:abc".into(), 30, 8192),
        (
            valid.argv[0].clone(),
            format!("sha256:{}z", "a".repeat(63)),
            30,
            8192,
        ),
        (valid.argv[0].clone(), digest.clone(), 601, 8192),
        (valid.argv[0].clone(), digest.clone(), 30, 1_048_577),
    ] {
        let mut invalid = valid.clone();
        invalid.argv[0] = path;
        invalid.script_identity = identity;
        invalid.timeout_seconds = timeout;
        invalid.output_limit_bytes = output;
        assert_eq!(
            check(vec![invalid]),
            Err("invalid project hook limits or script identity")
        );
    }
    assert_eq!(
        check(vec![valid.clone(), valid]),
        Err("duplicate project hook")
    );
}

#[tokio::test]
async fn stopped_hook_reconciliation_preserves_each_recorded_outcome() {
    let _env = SUPERVISOR_ENV.lock().await;
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
    let reviewed = hook(
        HookEvent::BeforeRun,
        "recover",
        &script_path,
        &counter,
        &digest,
        "ok",
        ReplayPolicy::Never,
        10,
    );
    let pool = fixture(&repository(json!([reviewed]))).await;
    let (launch, workspace) = identities(&root);
    project_hooks::register(
        &pool,
        &launch,
        &workspace,
        HookRole::Coding,
        &json!({"hook_allowlist":[reviewed]}),
    )
    .await
    .unwrap();
    assert!(
        project_hooks::event(&pool, &root, &launch.key.run_id, HookEvent::BeforeRun, None)
            .await
            .unwrap()
    );
    let (id, directory): (String, String) = sqlx::query_as(
        "SELECT invocation_id,output_dir FROM project_hook_invocation WHERE run_id=$1",
    )
    .bind(&launch.key.run_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let directory = PathBuf::from(directory);
    let success = fs::read(directory.join("stdout.json")).unwrap();
    let mut failure: Value = serde_json::from_slice(&success).unwrap();
    failure["status"] = json!("failed");
    failure["error"] = json!({"code":"project_failure","message":"recorded failure"});
    failure.as_object_mut().unwrap().remove("artifacts");

    for (case, expected, complete) in [
        ("spawn", "failed", false),
        ("nonzero", "failed", false),
        ("missing_exit", "unknown", false),
        ("malformed", "unknown", false),
        ("failure", "failed", false),
        ("success", "success", true),
    ] {
        sqlx::query("UPDATE project_hook_invocation SET status='running',result=NULL,diagnostic=NULL WHERE invocation_id=$1")
            .bind(&id).execute(&pool).await.unwrap();
        let spawn_error = directory.join("spawn-error.json");
        if spawn_error.exists() {
            fs::remove_file(&spawn_error).unwrap();
        }
        if case == "spawn" {
            fs::write(&spawn_error, "\"could not start\"").unwrap();
        }
        let exit = directory.join("exit.json");
        if case == "missing_exit" {
            fs::remove_file(&exit).unwrap();
        } else {
            process::durable_write(&exit, &(if case == "nonzero" { 7 << 8 } else { 0 })).unwrap();
        }
        match case {
            "malformed" => fs::write(directory.join("stdout.json"), b"not json").unwrap(),
            "failure" => fs::write(
                directory.join("stdout.json"),
                serde_json::to_vec(&failure).unwrap(),
            )
            .unwrap(),
            _ => fs::write(directory.join("stdout.json"), &success).unwrap(),
        }
        assert_eq!(
            project_hooks::event(&pool, &root, &launch.key.run_id, HookEvent::BeforeRun, None)
                .await
                .unwrap(),
            complete,
            "{case}"
        );
        let (status, stopped): (String, bool) = sqlx::query_as(
            "SELECT status,stop_confirmed FROM project_hook_invocation WHERE invocation_id=$1",
        )
        .bind(&id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(status, expected, "{case}");
        assert!(stopped, "{case}");
    }
    sqlx::query("UPDATE project_hook_invocation SET status='unknown',stop_confirmed=false WHERE invocation_id=$1")
        .bind(&id).execute(&pool).await.unwrap();
    assert!(
        !project_hooks::event(&pool, &root, &launch.key.run_id, HookEvent::BeforeRun, None)
            .await
            .unwrap()
    );
    let stopped: bool = sqlx::query_scalar(
        "SELECT stop_confirmed FROM project_hook_invocation WHERE invocation_id=$1",
    )
    .bind(&id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(stopped);
    assert_eq!(fs::read_to_string(counter).unwrap().lines().count(), 1);
}

#[tokio::test]
async fn undeployed_hooks_and_corrupt_frozen_identity_fail_before_execution() {
    let root = temp();
    let script_path = root.join("hook.py");
    let counter = root.join("calls.txt");
    let digest = script(&script_path);
    let reviewed = hook(
        HookEvent::BeforeRun,
        "guard",
        &script_path,
        &counter,
        &digest,
        "ok",
        ReplayPolicy::Never,
        10,
    );
    let pool = fixture(&repository(json!([reviewed]))).await;
    let (launch, workspace) = identities(&root);
    let missing =
        project_hooks::register(&pool, &launch, &workspace, HookRole::Coding, &Value::Null)
            .await
            .unwrap_err();
    assert!(missing.to_string().contains("not deployed"));
    assert!(
        project_hooks::register(
            &pool,
            &launch,
            &workspace,
            HookRole::Coding,
            &json!({"hook_allowlist":"invalid"}),
        )
        .await
        .is_err()
    );
    project_hooks::register(
        &pool,
        &launch,
        &workspace,
        HookRole::Coding,
        &json!({"hook_allowlist":[reviewed]}),
    )
    .await
    .unwrap();
    sqlx::query("UPDATE project_hook_run SET frozen=jsonb_set(frozen,'{config_id}','\"changed\"') WHERE run_id=$1")
        .bind(&launch.key.run_id).execute(&pool).await.unwrap();
    let error = project_hooks::event(&pool, &root, &launch.key.run_id, HookEvent::BeforeRun, None)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("invalid frozen hook"));
    assert!(!counter.exists());
}

#[tokio::test]
async fn invalid_before_remove_resource_never_starts_reviewed_script() {
    let root = temp();
    let script_path = root.join("hook.py");
    let counter = root.join("calls.txt");
    let digest = script(&script_path);
    let reviewed = hook(
        HookEvent::BeforeRemove,
        "remove",
        &script_path,
        &counter,
        &digest,
        "ok",
        ReplayPolicy::Never,
        10,
    );
    let pool = fixture(&repository(json!([reviewed]))).await;
    let (launch, workspace) = identities(&root);
    project_hooks::register(
        &pool,
        &launch,
        &workspace,
        HookRole::Coding,
        &json!({"hook_allowlist":[reviewed]}),
    )
    .await
    .unwrap();
    let error = project_hooks::event(
        &pool,
        &root,
        &launch.key.run_id,
        HookEvent::BeforeRemove,
        Some(("", Path::new(&workspace.path))),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("invalid hook invocation"));
    assert!(!counter.exists());
}

#[tokio::test]
async fn script_identity_spawn_and_output_limits_fail_closed() {
    let _env = SUPERVISOR_ENV.lock().await;
    unsafe {
        std::env::set_var(
            "SYMPHONY_SUPERVISOR",
            env!("CARGO_BIN_EXE_codexsymphony-server"),
        );
    }
    for case in ["symlink", "digest", "noexec", "overflow"] {
        let root = temp();
        let script_path = root.join("hook.py");
        let counter = root.join("calls.txt");
        let digest = script(&script_path);
        let reviewed = hook(
            HookEvent::BeforeRun,
            case,
            &script_path,
            &counter,
            &digest,
            if case == "overflow" { "overflow" } else { "ok" },
            ReplayPolicy::Never,
            10,
        );
        let pool = fixture(&repository(json!([reviewed]))).await;
        let (launch, workspace) = identities(&root);
        project_hooks::register(
            &pool,
            &launch,
            &workspace,
            HookRole::Coding,
            &json!({"hook_allowlist":[reviewed]}),
        )
        .await
        .unwrap();
        match case {
            "symlink" => {
                fs::remove_file(&script_path).unwrap();
                std::os::unix::fs::symlink("/bin/true", &script_path).unwrap();
            }
            "digest" => {
                use std::io::Write;
                writeln!(
                    fs::OpenOptions::new()
                        .append(true)
                        .open(&script_path)
                        .unwrap(),
                    "# changed"
                )
                .unwrap();
            }
            "noexec" => {
                fs::set_permissions(&script_path, fs::Permissions::from_mode(0o644)).unwrap()
            }
            _ => {}
        }
        assert!(
            !project_hooks::event(&pool, &root, &launch.key.run_id, HookEvent::BeforeRun, None)
                .await
                .unwrap(),
            "{case}"
        );
        let (status, output): (String, String) =
            sqlx::query_as("SELECT status,output_dir FROM project_hook_invocation WHERE run_id=$1")
                .bind(&launch.key.run_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        if case == "overflow" {
            assert!(matches!(status.as_str(), "failed" | "unknown"));
        } else {
            assert_eq!(status, "failed", "{case}");
        }
        assert_eq!(counter.exists(), case == "overflow", "{case}");
        if case == "overflow" {
            assert!(Path::new(&output).join("truncated.json").exists());
        }
    }
}

#[test]
fn supervisor_cannot_claim_quiescence_when_hook_output_cannot_be_opened() {
    let root = temp();
    let directory = root.join("supervisor");
    fs::create_dir(&directory).unwrap();
    let id = process::new_identity().unwrap();
    let key = RunKey {
        run_id: id.clone(),
        request_id: id,
        incarnation: "boot".into(),
    };
    let launch = Launch {
        key: key.clone(),
        workspace: directory.to_string_lossy().into_owned(),
        workspace_identity: "hook-output".into(),
        program: "/bin/true".into(),
        args: vec![],
    };
    process::durable_write(&directory.join("hook.json"), &json!({})).unwrap();
    process::durable_write(&directory.join("input.json"), &json!({})).unwrap();
    process::durable_write(&directory.join("hook-limit.json"), &8192_u64).unwrap();
    fs::create_dir(directory.join("stdout.json")).unwrap();
    let mut child = process::spawn(
        Path::new(env!("CARGO_BIN_EXE_codexsymphony-server")),
        &directory,
        &launch,
    )
    .unwrap();
    process::durable_write(&directory.join("start.json"), &key).unwrap();
    assert!(!child.wait().unwrap().success());
    assert!(directory.join("identity.json").exists());
    assert!(!directory.join("quiescent.json").exists());
}

#[tokio::test]
async fn missing_supervisor_identity_fails_without_starting_the_hook() {
    let _env = SUPERVISOR_ENV.lock().await;
    unsafe { std::env::set_var("SYMPHONY_SUPERVISOR", "/bin/true") };
    let root = temp();
    let script_path = root.join("hook.py");
    let counter = root.join("calls.txt");
    let digest = script(&script_path);
    let reviewed = hook(
        HookEvent::BeforeRun,
        "missing-identity",
        &script_path,
        &counter,
        &digest,
        "ok",
        ReplayPolicy::Never,
        1,
    );
    let pool = fixture(&repository(json!([reviewed]))).await;
    let (launch, workspace) = identities(&root);
    project_hooks::register(
        &pool,
        &launch,
        &workspace,
        HookRole::Coding,
        &json!({"hook_allowlist":[reviewed]}),
    )
    .await
    .unwrap();
    assert!(
        !project_hooks::event(&pool, &root, &launch.key.run_id, HookEvent::BeforeRun, None)
            .await
            .unwrap()
    );
    let (status, output): (String, String) =
        sqlx::query_as("SELECT status,output_dir FROM project_hook_invocation WHERE run_id=$1")
            .bind(&launch.key.run_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(status, "failed");
    assert!(Path::new(&output).join("stop.json").exists());
    assert!(!counter.exists());
    unsafe {
        std::env::set_var(
            "SYMPHONY_SUPERVISOR",
            env!("CARGO_BIN_EXE_codexsymphony-server"),
        );
    }
}

#[tokio::test]
async fn lost_supervisor_after_identity_preserves_unknown_side_effects() {
    let _env = SUPERVISOR_ENV.lock().await;
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
    let reviewed = hook(
        HookEvent::BeforeRun,
        "lost-supervisor",
        &script_path,
        &counter,
        &digest,
        "hold",
        ReplayPolicy::Never,
        1,
    );
    let pool = fixture(&repository(json!([reviewed]))).await;
    let (launch, workspace) = identities(&root);
    project_hooks::register(
        &pool,
        &launch,
        &workspace,
        HookRole::Coding,
        &json!({"hook_allowlist":[reviewed]}),
    )
    .await
    .unwrap();
    let run_id = launch.key.run_id.clone();
    let task_pool = pool.clone();
    let task_root = root.clone();
    let task_run_id = run_id.clone();
    let task = tokio::spawn(async move {
        project_hooks::event(
            &task_pool,
            &task_root,
            &task_run_id,
            HookEvent::BeforeRun,
            None,
        )
        .await
        .unwrap()
    });
    let (output, receipt) = await_hook_receipt(&pool, &run_id).await;
    assert!(
        Command::new("/bin/kill")
            .args(["-KILL", &receipt.process.pid.to_string()])
            .status()
            .unwrap()
            .success()
    );
    assert!(!task.await.unwrap());
    let status: String =
        sqlx::query_scalar("SELECT status FROM project_hook_invocation WHERE run_id=$1")
            .bind(&run_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(status, "unknown");
    assert!(output.join("identity.json").exists());
    assert!(output.join("stop.json").exists());
    assert!(!output.join("quiescent.json").exists());
}

#[tokio::test]
async fn mismatched_stop_receipt_cannot_confirm_hook_quiescence() {
    let _env = SUPERVISOR_ENV.lock().await;
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
    let reviewed = hook(
        HookEvent::BeforeRun,
        "wrong-receipt",
        &script_path,
        &counter,
        &digest,
        "hold",
        ReplayPolicy::Never,
        10,
    );
    let pool = fixture(&repository(json!([reviewed]))).await;
    let (launch, workspace) = identities(&root);
    project_hooks::register(
        &pool,
        &launch,
        &workspace,
        HookRole::Coding,
        &json!({"hook_allowlist":[reviewed]}),
    )
    .await
    .unwrap();
    let run_id = launch.key.run_id.clone();
    let task_pool = pool.clone();
    let task_root = root.clone();
    let task_run_id = run_id.clone();
    let task = tokio::spawn(async move {
        project_hooks::event(
            &task_pool,
            &task_root,
            &task_run_id,
            HookEvent::BeforeRun,
            None,
        )
        .await
        .unwrap()
    });
    let (output, receipt) = await_hook_receipt(&pool, &run_id).await;
    for _ in 0..200 {
        if output.join("start.json").exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(output.join("start.json").exists());
    let mut forged = receipt.clone();
    forged.process.pid = 1;
    process::durable_write(&output.join("quiescent.json"), &forged).unwrap();
    assert!(!task.await.unwrap());
    let (status, stopped, diagnostic): (String, bool, Option<String>) = sqlx::query_as(
        "SELECT status,stop_confirmed,diagnostic FROM project_hook_invocation WHERE run_id=$1",
    )
    .bind(&run_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(status, "unknown");
    assert!(!stopped);
    assert!(
        diagnostic
            .unwrap()
            .contains("stop receipt identity mismatch")
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if process::read::<codexsymphony_server::execution::Receipt>(
                &output.join("quiescent.json"),
            )
            .is_ok_and(|stopped| stopped == receipt)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn reviewed_hooks_run_once_and_loss_reconciles_without_replaying() {
    let _env = SUPERVISOR_ENV.lock().await;
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
    let _env = SUPERVISOR_ENV.lock().await;
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
    let _env = SUPERVISOR_ENV.lock().await;
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
    let _env = SUPERVISOR_ENV.lock().await;
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
    let _env = SUPERVISOR_ENV.lock().await;
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
    let _env = SUPERVISOR_ENV.lock().await;
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_writer(std::io::sink)
        .try_init()
        .unwrap();
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

    let directory: String =
        sqlx::query_scalar("SELECT output_dir FROM project_hook_invocation WHERE run_id=$1")
            .bind(&id)
            .fetch_one(&pool)
            .await
            .unwrap();
    let directory = PathBuf::from(directory);
    let quiescent = fs::read(directory.join("quiescent.json")).unwrap();
    fs::remove_file(directory.join("quiescent.json")).unwrap();
    sqlx::query("UPDATE project_hook_invocation SET status='running',result=NULL WHERE run_id=$1")
        .bind(&id)
        .execute(&pool)
        .await
        .unwrap();
    let missing_stop = project_hooks::after_run(&pool, &root).await.unwrap_err();
    assert!(missing_stop.to_string().contains("stop proof missing"));
    assert_eq!(fs::read_to_string(&counter).unwrap().lines().count(), 1);

    fs::write(directory.join("quiescent.json"), quiescent).unwrap();
    let mut recorded: Value =
        serde_json::from_slice(&fs::read(directory.join("stdout.json")).unwrap()).unwrap();
    recorded["status"] = json!("success");
    recorded["artifacts"] = json!([]);
    recorded.as_object_mut().unwrap().remove("error");
    fs::write(
        directory.join("stdout.json"),
        serde_json::to_vec(&recorded).unwrap(),
    )
    .unwrap();
    sqlx::query("UPDATE project_hook_invocation SET status='running',result=NULL WHERE run_id=$1")
        .bind(&id)
        .execute(&pool)
        .await
        .unwrap();
    project_hooks::after_run(&pool, &root).await.unwrap();
    let status: String =
        sqlx::query_scalar("SELECT status FROM project_hook_invocation WHERE run_id=$1")
            .bind(&id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(status, "success");

    sqlx::query("UPDATE project_hook_invocation SET status='running',result=NULL WHERE run_id=$1")
        .bind(&id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE project_hook_run SET frozen=jsonb_set(frozen,'{config_id}','\"changed\"') WHERE run_id=$1")
        .bind(&id).execute(&pool).await.unwrap();
    assert!(
        project_hooks::event(&pool, &root, &id, HookEvent::AfterRun, None)
            .await
            .is_err()
    );
    project_hooks::after_run(&pool, &root).await.unwrap();
    assert_eq!(fs::read_to_string(&counter).unwrap().lines().count(), 1);
    broker.verify(&manifest).unwrap();
    sqlx::query("UPDATE workspace_snapshot SET manifest='{}' WHERE run_id=$1")
        .bind(&id)
        .execute(&pool)
        .await
        .unwrap();
    let error = codexsymphony_server::coordinator::recover(&pool, &root, "boot")
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("project hook preservation check")
    );
}

#[tokio::test]
async fn repository_scope_denies_real_hook_before_process_or_intent() {
    let root = temp();
    let script_path = root.join("hook.py");
    let counter = root.join("calls.txt");
    let digest = script(&script_path);
    let reviewed = hook(
        HookEvent::BeforeRun,
        "scoped",
        &script_path,
        &counter,
        &digest,
        "ok",
        ReplayPolicy::Never,
        10,
    );
    let pool = fixture(&repository(json!([reviewed]))).await;
    let (launch, workspace) = identities(&root);
    project_hooks::register(
        &pool,
        &launch,
        &workspace,
        HookRole::Coding,
        &json!({"hook_allowlist":[reviewed]}),
    )
    .await
    .unwrap();
    sqlx::raw_sql("INSERT INTO repository(id,version,document) VALUES(42,1,'{}'); UPDATE plugin_scope SET kind='repositories',repository_ids='{42}' WHERE plugin_id='hook:scoped';").execute(&pool).await.unwrap();
    let result =
        project_hooks::event(&pool, &root, &launch.key.run_id, HookEvent::BeforeRun, None).await;
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("scope unavailable")
    );
    assert!(!counter.exists());
    let intents: i64 = sqlx::query_scalar("SELECT count(*) FROM project_hook_invocation")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(intents, 0);
    pool.close().await;
    fs::remove_dir_all(root).unwrap();
}
