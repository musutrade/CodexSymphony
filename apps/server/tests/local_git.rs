use codexsymphony_server::local_git::{self, Observation, Target};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
    process::Command,
};

fn git(path: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().into()
}
fn fixture() -> (FixtureDirectory, Target, String, String) {
    let root = FixtureDirectory::new();
    let source = root.path().join("source");
    fs::create_dir(&source).unwrap();
    git(&source, &["init", "-b", "main"]);
    fs::write(source.join("value"), "before\n").unwrap();
    git(&source, &["add", "value"]);
    git(&source, &["commit", "-m", "initial"]);
    let base = git(&source, &["rev-parse", "HEAD"]);
    git(
        root.path(),
        &["clone", "--bare", source.to_str().unwrap(), "target.git"],
    );
    fs::write(source.join("value"), "after\n").unwrap();
    git(&source, &["commit", "-am", "candidate"]);
    let candidate = git(&source, &["rev-parse", "HEAD"]);
    fs::set_permissions(
        root.path().join("target.git"),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let target = Target {
        reference: "fixture".into(),
        repository_id: 1,
        repository_version: 1,
        path: root.path().join("target.git"),
        branch: "main".into(),
    };
    (root, target, base, candidate)
}

#[test]
fn atomic_delivery_receipt_survives_lost_reply_and_later_branch_change() {
    let (root, target, base, candidate) = fixture();
    let binding =
        local_git::resolve(std::slice::from_ref(&target), "fixture", 1, 1, "main").unwrap();
    assert_eq!(local_git::head(&binding).unwrap(), base);
    assert_eq!(
        local_git::observe(&binding, "first", &base, &candidate).unwrap(),
        Observation::NotSubmitted
    );
    assert_eq!(
        local_git::submit(
            &binding,
            &root.path().join("source"),
            "first",
            &base,
            &candidate
        )
        .unwrap(),
        Observation::Delivered
    );
    assert_eq!(local_git::head(&binding).unwrap(), candidate);
    assert_eq!(
        local_git::submit(
            &binding,
            &root.path().join("source"),
            "first",
            &base,
            &candidate
        )
        .unwrap(),
        Observation::Delivered
    );
    git(
        &target.path,
        &["update-ref", "refs/heads/main", &base, &candidate],
    );
    assert_eq!(
        local_git::observe(&binding, "first", &base, &candidate).unwrap(),
        Observation::Delivered
    );
    assert_eq!(
        local_git::observe(&binding, "first", &base, &base).unwrap(),
        Observation::Conflict
    );
    git(
        &target.path,
        &["update-ref", "refs/heads/main", &candidate, &base],
    );
    assert_eq!(
        local_git::observe(&binding, "other", &base, &candidate).unwrap(),
        Observation::Conflict
    );
    assert_eq!(
        local_git::submit(
            &binding,
            &root.path().join("source"),
            "other",
            &base,
            &candidate
        )
        .unwrap(),
        Observation::Conflict
    );
}

#[test]
fn target_and_oid_substitution_is_rejected() {
    let (root, target, base, candidate) = fixture();
    for (name, id, version, branch) in [
        ("missing", 1, 1, "main"),
        ("fixture", 2, 1, "main"),
        ("fixture", 1, 2, "main"),
        ("fixture", 1, 1, "other"),
    ] {
        assert!(
            local_git::resolve(std::slice::from_ref(&target), name, id, version, branch).is_err()
        );
    }
    for (field, value) in [
        ("reference", ""),
        ("branch", "main\ncreate refs/heads/evil"),
        ("branch", "main..bad"),
    ] {
        let mut invalid = target.clone();
        if field == "reference" {
            invalid.reference = value.into()
        } else {
            invalid.branch = value.into()
        }
        assert!(local_git::bind(&invalid).is_err());
    }
    let mut invalid = target.clone();
    invalid.repository_id = 0;
    assert!(local_git::bind(&invalid).is_err());
    invalid = target.clone();
    invalid.path = root.path().join("source");
    assert!(local_git::bind(&invalid).is_err());
    invalid.path = "relative.git".into();
    assert!(local_git::bind(&invalid).is_err());
    let link = root.path().join("link.git");
    symlink(&target.path, &link).unwrap();
    invalid.path = link;
    assert!(local_git::bind(&invalid).is_err());
    let binding = local_git::bind(&target).unwrap();
    for value in ["short".to_owned(), "z".repeat(40)] {
        assert!(local_git::observe(&binding, "action", &value, &candidate).is_err());
    }
    fs::set_permissions(&target.path, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(local_git::check(&binding).is_err());
    fs::set_permissions(&target.path, fs::Permissions::from_mode(0o700)).unwrap();
    git(&target.path, &["config", "test.changed", "true"]);
    assert!(local_git::check(&binding).is_err());
    assert_eq!(git(&target.path, &["rev-parse", "main"]), base);
}

#[test]
fn non_fast_forward_and_missing_candidate_preserve_target() {
    let (root, target, base, candidate) = fixture();
    let binding = local_git::bind(&target).unwrap();
    assert!(
        local_git::submit(
            &binding,
            &root.path().join("source"),
            "missing",
            &base,
            &"0".repeat(40)
        )
        .is_err()
    );
    local_git::submit(
        &binding,
        &root.path().join("source"),
        "forward",
        &base,
        &candidate,
    )
    .unwrap();
    assert!(
        local_git::submit(
            &binding,
            &root.path().join("source"),
            "backward",
            &candidate,
            &base
        )
        .is_err()
    );
    assert_eq!(local_git::head(&binding).unwrap(), candidate);
}

#[test]
fn registry_requires_protected_file_and_unique_references() {
    let (root, target, _, _) = fixture();
    let path = root.path().join("registry.json");
    unsafe { std::env::remove_var("LOCAL_GIT_TARGETS") };
    assert!(local_git::installed().unwrap().is_empty());
    fs::write(&path, serde_json::to_vec(&vec![target.clone()]).unwrap()).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    unsafe { std::env::set_var("LOCAL_GIT_TARGETS", &path) };
    assert_eq!(local_git::installed().unwrap(), vec![target.clone()]);
    fs::write(
        &path,
        serde_json::to_vec(&vec![target.clone(), target]).unwrap(),
    )
    .unwrap();
    assert!(local_git::installed().is_err());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o666)).unwrap();
    assert!(local_git::installed().is_err());
    unsafe { std::env::remove_var("LOCAL_GIT_TARGETS") };
}

struct FixtureDirectory(std::path::PathBuf);
impl FixtureDirectory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "gh105-{}",
            codexsymphony_server::process::new_identity().unwrap()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for FixtureDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

async fn database() -> sqlx::PgPool {
    use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
    let options: PgConnectOptions = std::env::var("TEST_DATABASE_URL").unwrap().parse().unwrap();
    let admin = PgPoolOptions::new()
        .connect_with(options.clone())
        .await
        .unwrap();
    let schema = format!(
        "local_{}",
        codexsymphony_server::process::new_identity()
            .unwrap()
            .replace('-', "")
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
    pool
}

struct Product {
    root: FixtureDirectory,
    pool: sqlx::PgPool,
    broker: codexsymphony_server::git_broker::GitBroker,
    manifest: codexsymphony_server::workspace::Manifest,
    plan: codexsymphony_server::validation_runner::Plan,
    binding: local_git::Binding,
}
async fn product() -> Product {
    product_group(false).await
}
async fn product_group(grouped: bool) -> Product {
    product_checked(grouped, None).await
}
async fn product_checked(grouped: bool, javascript_check: Option<&str>) -> Product {
    use codexsymphony_server::{
        execution::RunKey,
        validation::sha256,
        validation_runner::{Plan, Step},
        workspace::Workspace,
    };
    use serde_json::json;
    let (root, target, base, _) = fixture();
    let source = root.path().join("source");
    let bundle = root.path().join("seed.bundle");
    git(
        &source,
        &["bundle", "create", bundle.to_str().unwrap(), "--all"],
    );
    let broker = codexsymphony_server::git_broker::GitBroker::initialize(
        &root.path().join("workspaces"),
        &bundle,
    )
    .unwrap();
    let workspace = Workspace {
        key: RunKey {
            run_id: "source".into(),
            request_id: "source".into(),
            incarnation: "boot".into(),
        },
        identity: "source".into(),
        requirement: 1,
        revision: 1,
        phase: "validation".into(),
        baseline: base,
        branch: "ai/req-1-source".into(),
        path: broker
            .path("source")
            .unwrap()
            .to_string_lossy()
            .into_owned(),
    };
    broker.prepare(&workspace, true).unwrap();
    fs::write(Path::new(&workspace.path).join("value"), "after\n").unwrap();
    if javascript_check.is_some() {
        fs::write(
            Path::new(&workspace.path).join("value.mjs"),
            "export function value() { return 'after'; }\n",
        )
        .unwrap();
        fs::write(
            Path::new(&workspace.path).join("check.mjs"),
            "import assert from 'node:assert/strict';\nimport { value } from './value.mjs';\nassert.equal(value(), 'after');\nconsole.log('behavior passed');\n",
        )
        .unwrap();
    }
    broker
        .commit(&workspace, "Implement local fixture")
        .unwrap();
    let manifest = broker.preserve(&workspace).unwrap();
    let binding = local_git::bind(&target).unwrap();
    let registry = root.path().join("targets.json");
    fs::write(&registry, json!([target]).to_string()).unwrap();
    fs::set_permissions(&registry, fs::Permissions::from_mode(0o600)).unwrap();
    unsafe { std::env::set_var("LOCAL_GIT_TARGETS", &registry) };
    let entry = root.path().join("validate");
    let script = "#!/bin/sh\ncat value\nif [ -f \"$(dirname \"$0\")/post-fail\" ] && ! grep -q repaired value; then echo 'AssertionError: local delivered check'; exit 1; fi\ntest \"$(head -n1 value)\" = after\n";
    let script = javascript_check.unwrap_or(script);
    fs::write(&entry, script).unwrap();
    fs::set_permissions(&entry, fs::Permissions::from_mode(0o755)).unwrap();
    let mut plan = Plan {
        entry,
        entry_sha256: sha256(script),
        steps: vec![Step {
            id: "test".into(),
            command: vec!["/gate-entry".into()],
            timeout_seconds: 2,
            code_failure: true,
        }],
    };
    unsafe {
        std::env::set_var(
            "SYMPHONY_SUPERVISOR",
            env!("CARGO_BIN_EXE_codexsymphony-server"),
        )
    };
    let pool = database().await;
    let mut repository = groups::repository();
    repository["delivery"] = json!("local_git");
    repository["remote"] = json!("fixture");
    if javascript_check.is_some() {
        repository["project"] = json!("JavaScript project without dependencies or services");
        repository["policy"]["allowed_checks"] = json!(["npm_test"]);
        plan.steps[0].command.push("behavior".into());
        plan.steps.push(Step {
            id: "syntax".into(),
            command: vec!["/gate-entry".into(), "syntax".into()],
            timeout_seconds: 2,
            code_failure: true,
        });
    }
    if grouped {
        repository["policy"]["gate_recovery_policy"] = json!("bounded_v1");
    }
    repository
        .as_object_mut()
        .unwrap()
        .remove("github_repository_id");
    let mut contract = json!({"title":"local fixture","description":"update value","acceptance_criteria":[{"description":"value is after","verification_ref":"test"}],"validation_plan":[{"id":"test","check":"cargo_test","selector":"fixture","expected_result":"pass","timeout_seconds":30}],"network_access":[]});
    if javascript_check.is_some() {
        contract["validation_plan"][0]["check"] = json!("npm_test");
        contract["validation_plan"].as_array_mut().unwrap().push(json!({"id":"syntax","check":"npm_test","selector":"syntax","expected_result":"pass","timeout_seconds":30}));
    }
    let document = json!({"repository_id":1,"repository_version":1,"repository":repository,"contract":contract});
    sqlx::query("INSERT INTO repository(id,version,document) VALUES(1,1,$1)")
        .bind(&repository)
        .execute(&pool)
        .await
        .unwrap();
    if grouped {
        configure_group(&pool, &plan, &manifest.workspace.baseline).await;
        sqlx::query("UPDATE requirement SET state='Running' WHERE id=1")
            .execute(&pool)
            .await
            .unwrap();
    } else {
        sqlx::query(
            "INSERT INTO requirement(version,state,contract,revision) VALUES(1,'Running',$1,1)",
        )
        .bind(&contract)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO requirement_revision VALUES(1,1,$1)")
            .bind(&document)
            .execute(&pool)
            .await
            .unwrap();
    }
    sqlx::query(
        "UPDATE execution_control SET requirement_id=1,incarnation='boot',recovery_complete=true",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE plugin_scope SET enabled=true,version=version+1,kind='repositories',repository_ids=ARRAY[1] WHERE plugin_id='delivery:local_git'").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO requirement_budget(requirement_id,limits) VALUES(1,'{\"tokens\":100,\"turns\":10,\"model_seconds\":100}') ON CONFLICT DO NOTHING").execute(&pool).await.unwrap();
    let launch = codexsymphony_server::execution::Launch {
        key: workspace.key.clone(),
        workspace: workspace.path.clone(),
        workspace_identity: workspace.identity.clone(),
        program: "/bin/true".into(),
        args: vec![],
    };
    sqlx::query("INSERT INTO initial_run(requirement_id,revision,launch,workspace,local_binding) VALUES(1,1,$3,$1,$2)").bind(json!(workspace)).bind(json!(binding)).bind(json!(launch)).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO agent_run(id,requirement_id,revision,incarnation,request_id,workspace,workspace_identity,launch,state,quiescent,phase) VALUES('source',1,1,'boot','source',$1,'source','{}','Succeeded',true,'validation')").bind(&workspace.path).execute(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO workspace_snapshot(run_id,manifest,candidate) VALUES('source',$1,true)",
    )
    .bind(json!(manifest))
    .execute(&pool)
    .await
    .unwrap();
    Product {
        root,
        pool,
        broker,
        manifest,
        plan,
        binding,
    }
}
async fn validate_product(p: &Product) -> bool {
    let checkout = Path::new(&p.manifest.workspace.path);
    codexsymphony_server::validation_service::validate(
        &p.pool,
        codexsymphony_server::validation_service::Request {
            id: "validation",
            source_run: "source",
            requirement: 1,
            revision: 1,
            checkout,
            directory: &p.root.path().join("validation"),
            candidate: &codexsymphony_server::validation_runner::candidate(checkout).unwrap(),
            plan: &p.plan,
        },
    )
    .await
    .unwrap()
}
async fn product_tick(p: &Product) {
    assert!(
        codexsymphony_server::local_delivery::tick(&p.pool, p.root.path(), &p.broker)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn product_validation_delivers_and_accepts_without_any_github_records() {
    use codexsymphony_server::local_delivery_store as store;
    let p = product().await;
    let mut tx = p.pool.begin().await.unwrap();
    assert!(store::claim_ready(&mut tx, 1, 1).await.unwrap());
    tx.commit().await.unwrap();
    assert!(validate_product(&p).await);
    assert!(
        codexsymphony_server::delivery_store::due(&p.pool, i64::MAX)
            .await
            .unwrap()
            .is_empty()
    );
    product_tick(&p).await;
    assert_eq!(local_git::head(&p.binding).unwrap(), p.manifest.head);
    let (state,owner):(String,Option<i64>)=sqlx::query_as("SELECT state,requirement_id FROM requirement CROSS JOIN execution_control WHERE requirement.id=1").fetch_one(&p.pool).await.unwrap();
    assert_eq!(state, "Submitted");
    assert_eq!(owner, Some(1));
    product_tick(&p).await;
    let (state,owner):(String,Option<i64>)=sqlx::query_as("SELECT state,requirement_id FROM requirement CROSS JOIN execution_control WHERE requirement.id=1").fetch_one(&p.pool).await.unwrap();
    assert_eq!(
        state,
        "Done",
        "{:?}",
        sqlx::query_scalar::<_, Option<serde_json::Value>>("SELECT error FROM delivery_action")
            .fetch_one(&p.pool)
            .await
            .unwrap()
    );
    assert_eq!(owner, None);
    assert!(
        !codexsymphony_server::local_delivery::tick(&p.pool, p.root.path(), &p.broker)
            .await
            .unwrap()
    );
    let facts: serde_json::Value = sqlx::query_scalar("SELECT local_acceptance FROM delivery")
        .fetch_one(&p.pool)
        .await
        .unwrap();
    assert_eq!(facts["passed"], true);
    assert_eq!(facts["evidence"]["candidate"]["sha"], p.manifest.head);
    let count:i64=sqlx::query_scalar("SELECT (SELECT count(*) FROM github_repository)+(SELECT count(*) FROM github_pr)+(SELECT count(*) FROM merge_operation)").fetch_one(&p.pool).await.unwrap();
    assert_eq!(count, 0);
    let attempts: i64 = sqlx::query_scalar("SELECT count(*) FROM delivery_attempt")
        .fetch_one(&p.pool)
        .await
        .unwrap();
    assert_eq!(attempts, 1);
    p.pool.close().await;
    unsafe { std::env::remove_var("LOCAL_GIT_TARGETS") };
}

#[tokio::test]
async fn javascript_local_delivery_accepts_two_reviewed_check_implementations() {
    use codexsymphony_server::validation::sha256;
    let implementations = [
        "#!/bin/sh\nset -eu\ncase \"$1\" in behavior) node check.mjs;; syntax) node --check value.mjs; echo 'syntax passed';; *) exit 2;; esac\n",
        "#!/bin/sh\nset -eu\ncase \"$1\" in behavior) node --input-type=module -e \"import {value} from './value.mjs'; if (value() !== 'after') process.exit(1); console.log('behavior passed')\";; syntax) node --check value.mjs; echo 'syntax passed';; *) exit 2;; esac\n",
    ];
    let mut identities = Vec::new();
    for script in implementations {
        let p = product_checked(false, Some(script)).await;
        let checkout = Path::new(&p.manifest.workspace.path);
        for absent in ["Cargo.toml", "package.json", ".github", ".harness-gate"] {
            assert!(!checkout.join(absent).exists());
        }
        identities.push(p.plan.identity().unwrap().protected_entry_sha256);
        assert!(validate_product(&p).await);
        product_tick(&p).await;
        product_tick(&p).await;
        let state: String = sqlx::query_scalar("SELECT state FROM requirement WHERE id=1")
            .fetch_one(&p.pool)
            .await
            .unwrap();
        assert_eq!(state, "Done");
        let facts: serde_json::Value = sqlx::query_scalar("SELECT local_acceptance FROM delivery")
            .fetch_one(&p.pool)
            .await
            .unwrap();
        assert_eq!(facts["passed"], true);
        assert_eq!(facts["evidence"]["candidate"]["sha"], p.manifest.head);
        assert_eq!(
            facts["evidence"]["trusted"]["protected_entry_sha256"],
            sha256(script)
        );
        let steps = facts["evidence"]["steps"].as_array().unwrap();
        assert_eq!(steps.len(), 2);
        for (step, id, output) in [
            (&steps[0], "test", "behavior passed"),
            (&steps[1], "syntax", "syntax passed"),
        ] {
            assert_eq!(step["id"], id);
            assert_eq!(step["exit_code"], 0);
            assert!(step["output"].as_str().unwrap().contains(output));
        }
        let counts: (i64, i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM github_repository),(SELECT count(*) FROM github_pr),(SELECT count(*) FROM delivery_attempt)")
            .fetch_one(&p.pool).await.unwrap();
        assert_eq!(counts, (0, 0, 1));
        // Reopen the target and reconcile the original receipt, without another delivery.
        assert_eq!(local_git::head(&p.binding).unwrap(), p.manifest.head);
        assert!(
            !codexsymphony_server::local_delivery::tick(&p.pool, p.root.path(), &p.broker)
                .await
                .unwrap()
        );
        p.pool.close().await;
        unsafe { std::env::remove_var("LOCAL_GIT_TARGETS") };
    }
    assert_ne!(identities[0], identities[1]);
}

#[tokio::test]
async fn lost_reply_reconciles_receipt_and_never_repeats_local_update() {
    use codexsymphony_server::local_delivery_store as store;
    let p = product().await;
    assert!(validate_product(&p).await);
    let job = store::pending(&p.pool).await.unwrap().unwrap();
    assert!(store::begin(&p.pool, &job).await.unwrap().is_some());
    // Apply the native operation then lose both its response and DB observation.
    local_git::submit(
        &p.binding,
        &p.root.path().join("workspaces/canonical.git"),
        &job.action_key,
        job.baseline().unwrap(),
        &job.head_sha,
    )
    .unwrap();
    product_tick(&p).await;
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT state FROM requirement")
            .fetch_one(&p.pool)
            .await
            .unwrap(),
        "Done"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM delivery_attempt")
            .fetch_one(&p.pool)
            .await
            .unwrap(),
        1
    );
    p.pool.close().await;
    unsafe { std::env::remove_var("LOCAL_GIT_TARGETS") };
}

#[tokio::test]
async fn pause_scope_revocation_changed_candidate_and_unknown_send_block_delivery() {
    use codexsymphony_server::local_delivery_store as store;
    for change in [
        "UPDATE requirement SET paused=true",
        "UPDATE plugin_scope SET enabled=false,version=version+1 WHERE plugin_id='delivery:local_git'",
        "UPDATE repository SET revoked_through_version=1",
        "UPDATE candidate_validation SET result='blocked'",
    ] {
        let p = product().await;
        assert!(validate_product(&p).await);
        sqlx::query(change).execute(&p.pool).await.unwrap();
        product_tick(&p).await;
        assert_eq!(
            local_git::head(&p.binding).unwrap(),
            p.manifest.workspace.baseline
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM delivery_attempt")
                .fetch_one(&p.pool)
                .await
                .unwrap(),
            0
        );
        p.pool.close().await;
    }
    let p = product().await;
    assert!(validate_product(&p).await);
    let job = store::pending(&p.pool).await.unwrap().unwrap();
    assert!(store::begin(&p.pool, &job).await.unwrap().is_some());
    product_tick(&p).await;
    assert_eq!(
        local_git::head(&p.binding).unwrap(),
        p.manifest.workspace.baseline
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM delivery_attempt")
            .fetch_one(&p.pool)
            .await
            .unwrap(),
        1
    );
    p.pool.close().await;
    unsafe { std::env::remove_var("LOCAL_GIT_TARGETS") };
}

#[tokio::test]
async fn cancellation_before_send_withdraws_and_failed_validation_creates_no_delivery() {
    let p = product().await;
    assert!(validate_product(&p).await);
    codexsymphony_server::delivery_control::cancel(&p.pool, 1)
        .await
        .unwrap();
    product_tick(&p).await;
    assert_eq!(
        local_git::head(&p.binding).unwrap(),
        p.manifest.workspace.baseline
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT state FROM delivery_action")
            .fetch_one(&p.pool)
            .await
            .unwrap(),
        "withdrawn"
    );
    p.pool.close().await;
    let mut p = product().await;
    p.plan.steps[0].command.push("fail".into());
    let script = "#!/bin/sh\necho quality-failed\nexit 1\n";
    fs::write(&p.plan.entry, script).unwrap();
    p.plan.entry_sha256 = codexsymphony_server::validation::sha256(script);
    assert!(!validate_product(&p).await);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM delivery")
            .fetch_one(&p.pool)
            .await
            .unwrap(),
        0
    );
    p.pool.close().await;
    unsafe { std::env::remove_var("LOCAL_GIT_TARGETS") };
}

#[path = "support/groups.rs"]
#[allow(dead_code)]
mod groups;

#[tokio::test]
async fn registered_local_project_runs_real_codex_and_reaches_done() {
    use codexsymphony_server::{
        runtime_client, runtime_initial, runtime_service, validation::sha256,
    };
    use serde_json::json;
    let (root, target, base, _) = fixture();
    let pool = database().await;
    let app = groups::app(&pool);
    let registry = root.path().join("targets.json");
    fs::write(&registry, json!([target]).to_string()).unwrap();
    fs::set_permissions(&registry, fs::Permissions::from_mode(0o600)).unwrap();
    unsafe {
        std::env::set_var("LOCAL_GIT_TARGETS", &registry);
        std::env::set_var(
            "SYMPHONY_SUPERVISOR",
            env!("CARGO_BIN_EXE_codexsymphony-server"),
        );
    }
    let mut repository = groups::repository();
    repository["delivery"] = json!("local_git");
    repository["remote"] = json!("fixture");
    repository["model"] = json!("gpt-6-astra");
    repository
        .as_object_mut()
        .unwrap()
        .remove("github_repository_id");
    groups::request(
        &app,
        "PUT",
        "/api/repository",
        json!({"request_id":"local-repository","version":0,"repository":repository}),
        200,
    )
    .await;
    sqlx::query("UPDATE plugin_scope SET enabled=true,version=version+1,kind='repositories',repository_ids=ARRAY[1] WHERE plugin_id='delivery:local_git'").execute(&pool).await.unwrap();
    let view = groups::request(&app, "GET", "/api/multi/repository", json!(null), 200).await;
    assert_eq!(view["repositories"][0]["delivery_ready"], true);
    assert!(
        view["repositories"][0]["repository"]
            .get("github_repository_id")
            .is_none()
    );
    let contract = json!({"title":"Change local value","description":"Set value to after and commit it","acceptance_criteria":[{"description":"value is after","verification_ref":"test"}],"validation_plan":[{"id":"test","check":"cargo_test","selector":"local_fixture","expected_result":"pass","timeout_seconds":30}],"network_access":[]});
    let created = groups::request(
        &app,
        "POST",
        "/api/requirements",
        json!({"request_id":"local-task","version":0,"contract":contract}),
        201,
    )
    .await;
    groups::request(
        &app,
        "POST",
        "/api/requirements/1/ready",
        json!({"request_id":"local-ready","version":created["version"],"repository_version":1}),
        200,
    )
    .await;
    codexsymphony_server::run_store::begin_incarnation(&pool, "boot")
        .await
        .unwrap();
    sqlx::query("UPDATE execution_control SET recovery_complete=true")
        .execute(&pool)
        .await
        .unwrap();
    let bundle = root.path().join("seed.bundle");
    git(
        &target.path,
        &["bundle", "create", bundle.to_str().unwrap(), "--all"],
    );
    let broker = codexsymphony_server::git_broker::GitBroker::initialize(
        &root.path().join("workspaces"),
        &bundle,
    )
    .unwrap();
    let codex = String::from_utf8(Command::new("which").arg("codex").output().unwrap().stdout)
        .unwrap()
        .trim()
        .to_owned();
    let registry_file = root.path().join("targets.json");
    let retained_registry = root.path().join("targets.retained.json");
    fs::rename(&registry_file, &retained_registry).unwrap();
    assert!(
        runtime_initial::plan(
            &pool,
            &broker,
            "boot",
            std::slice::from_ref(&codex),
            &"0".repeat(40)
        )
        .await
        .is_err()
    );
    let unavailable = groups::request(&app, "GET", "/api/multi/repository", json!({}), 200).await;
    assert_eq!(unavailable["repositories"][0]["delivery_ready"], false);
    fs::rename(&retained_registry, &registry_file).unwrap();
    let (launch, workspace) = runtime_initial::plan(
        &pool,
        &broker,
        "boot",
        std::slice::from_ref(&codex),
        &"0".repeat(40),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(workspace.baseline, base);
    broker.prepare(&workspace, true).unwrap();
    // A separately approved preparation fixture isolates runtime/delivery here.
    sqlx::query("INSERT INTO preparation_record(run_id,requirement_id,revision,launch,retry,ready,checked_at) VALUES($1,1,1,$2,'{}',true,extract(epoch FROM now())::bigint)").bind(&launch.key.run_id).bind(json!(launch)).execute(&pool).await.unwrap();
    assert!(
        codexsymphony_server::run_store::reserve_prepared(&pool, &launch)
            .await
            .unwrap()
    );
    let counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let calls = counter.clone();
    let path = workspace.path.clone();
    let model=axum::Router::new().route("/responses",axum::routing::post(move || {
        let counter=counter.clone();let path=path.clone();
        async move {
            let index=counter.fetch_add(1,std::sync::atomic::Ordering::SeqCst);
            let (name,args)=match index {
                0=>("exec_command",json!({"cmd":"printf 'after\\n' > value","yield_time_ms":1000,"max_output_tokens":1000})),
                1=>("create_local_commit",json!({"message":"Implement local value"})),
                _=>("report_completion",json!({"candidate_sha":git(Path::new(&path),&["rev-parse","HEAD"]),"summary":"Local value implemented"}))
            };
            let events=[json!({"type":"response.created","response":{"id":format!("local-{index}")}}),json!({"type":"response.output_item.done","item":{"type":"function_call","call_id":format!("call-{index}"),"name":name,"arguments":args.to_string()}}),json!({"type":"response.completed","response":{"id":format!("local-{index}"),"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}})];
            ([("content-type","text/event-stream")],events.iter().map(|v|format!("data: {v}\n\n")).collect::<String>())
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move { axum::serve(listener, model).await.unwrap() });
    let script = "#!/bin/sh\ncat value\ntest \"$(cat value)\" = after\n";
    let entry = root.path().join("validate");
    fs::write(&entry, script).unwrap();
    fs::set_permissions(&entry, fs::Permissions::from_mode(0o755)).unwrap();
    let config = runtime_service::Config {
        preparation_adapter: "/bin/true".into(),
        preparation: json!({"launcher":[codex],"baseline":base}),
        validation: Some(codexsymphony_server::validation_runner::Plan {
            entry,
            entry_sha256: sha256(script),
            steps: vec![codexsymphony_server::validation_runner::Step {
                id: "test".into(),
                command: vec!["/gate-entry".into()],
                timeout_seconds: 2,
                code_failure: true,
            }],
        }),
        settings: runtime_client::Settings {
            startup_seconds: 30,
            response_seconds: 30,
            stall_seconds: 30,
            reservation: codexsymphony_server::budget::Amount {
                tokens: 20,
                turns: 1,
                model_seconds: 30,
            },
            codex_config: format!(
                r#"model = "gpt-6-astra"
model_provider = "local_fixture"
approval_policy = "never"
sandbox_mode = "danger-full-access"
[features]
apps = false
plugins = false
remote_plugin = false
goals = false
[model_providers.local_fixture]
name = "Local delivery fixture"
base_url = "http://127.0.0.1:{port}"
wire_api = "responses"
requires_openai_auth = false
supports_websockets = false
request_max_retries = 0
stream_max_retries = 0
"#
            ),
        },
    };
    let route_file = root.path().join("routes.json");
    fs::write(&route_file,json!({"repositories":{"1":{"remote":"fixture","base_branch":"main","version":1,"runtime":{"validation":config.validation,"settings":config.settings,"preparation_adapter":config.preparation_adapter,"preparation":config.preparation}}}}).to_string()).unwrap();
    let routes = codexsymphony_server::runtime_routes::Deployment::load(&route_file).unwrap();
    assert!(routes.selected(&pool).await.unwrap().is_some());
    tokio::time::timeout(std::time::Duration::from_secs(90), async {
        for _ in 0..100 {
            runtime_service::tick(
                &pool,
                root.path(),
                Path::new(env!("CARGO_BIN_EXE_codexsymphony-server")),
                &broker,
                "boot",
                &config,
            )
            .await
            .unwrap();
            codexsymphony_server::coordinator::recover(&pool, root.path(), "boot")
                .await
                .unwrap();
            if sqlx::query_scalar::<_, String>("SELECT state FROM requirement")
                .fetch_one(&pool)
                .await
                .unwrap()
                == "Done"
            {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!("local project did not complete");
    })
    .await
    .unwrap();
    server.abort();
    assert!(calls.load(std::sync::atomic::Ordering::SeqCst) >= 3);
    let candidate = git(&target.path, &["rev-parse", "refs/heads/main"]);
    assert_ne!(candidate, base);
    assert_eq!(git(&target.path, &["show", "main:value"]), "after");
    let detail = codexsymphony_server::operator_view::detail(&pool, 1)
        .await
        .unwrap();
    assert!(detail.to_string().contains("local_git"));
    let counts:(i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM github_repository),(SELECT count(*) FROM github_pr),(SELECT count(*) FROM delivery_attempt)").fetch_one(&pool).await.unwrap();
    assert_eq!(counts, (0, 0, 1));
    pool.close().await;
    unsafe {
        std::env::remove_var("LOCAL_GIT_TARGETS");
    }
}

async fn configure_group(
    pool: &sqlx::PgPool,
    plan: &codexsymphony_server::validation_runner::Plan,
    github_version: &str,
) {
    use serde_json::json;
    let mut other = groups::repository();
    other["remote"] = json!("test/second");
    other["github_repository_id"] = json!(456);
    sqlx::query("INSERT INTO repository(id,version,document) VALUES(2,1,$1)")
        .bind(other)
        .execute(pool)
        .await
        .unwrap();
    let mut document = groups::sample();
    let mut last = document["children"][3].clone();
    last["depends_on"] = json!(["C1"]);
    last["order"] = json!(2);
    document["children"] = json!([document["children"][0], last]);
    let mut review = groups::review();
    review["items"][0]["repair_scope"] =
        json!(json!({"schema":"linked-repair/v1","checks":{"test":["value"]}}).to_string());
    let mut item = review["items"][3].clone();
    item["integration"] = json!({"configuration_sha256":plan.identity().unwrap().config_sha256,"repositories":[{"repository_id":1,"repository_version":1,"repair_scope":review["items"][0]["repair_scope"],"selection":{"kind":"completed_dependencies"}},{"repository_id":2,"repository_version":1,"repair_scope":"none","selection":{"kind":"fixed","sha":github_version}}]});
    review["items"] = json!([review["items"][0], item]);
    let app = groups::app(pool);
    let draft = groups::request(&app, "POST", "/api/drafts", groups::body(document, 0), 200).await;
    let id = draft["id"].as_str().unwrap();
    groups::request(
        &app,
        "PUT",
        &format!("/api/drafts/{id}/review"),
        json!({"version":0,"draft_revision":1,"review":review}),
        200,
    )
    .await;
    groups::request(
        &app,
        "POST",
        &format!("/api/drafts/{id}/authorize"),
        json!({"version":1,"draft_revision":1,"request_id":"local-group"}),
        200,
    )
    .await;
    codexsymphony_server::group_queue_store::materialize(pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn local_completion_feeds_mixed_versions_and_pure_validation_without_empty_delivery() {
    use codexsymphony_server::{delivered_version, integration_worker};
    let p = product_group(true).await;
    assert!(validate_product(&p).await);
    product_tick(&p).await;
    product_tick(&p).await;
    let fact: serde_json::Value =
        sqlx::query_scalar("SELECT fact FROM group_completion WHERE requirement_id=1")
            .fetch_one(&p.pool)
            .await
            .unwrap();
    assert!(fact.get("pr_number").is_none());
    assert!(fact.get("merged_sha").is_none());
    assert_eq!(fact["delivery_version"], p.manifest.head);
    let mut tx = p.pool.begin().await.unwrap();
    let completion = delivered_version::decode(&mut tx, fact.clone())
        .await
        .unwrap();
    assert_eq!(completion.repository_id(), 1);
    assert_eq!(completion.github_id(), 0);
    assert_eq!(completion.commit(), p.manifest.head);
    assert!(completion.artifact().starts_with("local-delivery:"));
    let mut wrong = fact.clone();
    wrong["delivery_version"] = serde_json::json!("0".repeat(40));
    assert!(delivered_version::decode(&mut tx, wrong).await.is_err());
    tx.rollback().await.unwrap();
    for _ in 0..200 {
        integration_worker::tick(
            &p.pool,
            p.root.path(),
            Path::new(env!("CARGO_BIN_EXE_codexsymphony-server")),
            &p.broker,
            "boot",
            &p.plan,
        )
        .await
        .unwrap();
        let state: Option<String> =
            sqlx::query_scalar("SELECT state FROM integration_validation LIMIT 1")
                .fetch_optional(&p.pool)
                .await
                .unwrap();
        if state.as_deref() == Some("passed") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    let (state, binding): (String, serde_json::Value) =
        sqlx::query_as("SELECT state,binding FROM integration_validation")
            .fetch_one(&p.pool)
            .await
            .unwrap();
    assert_eq!(state, "passed");
    assert!(binding["versions"][0].get("github_repository_id").is_none());
    assert_eq!(binding["versions"][0]["candidate"]["sha"], p.manifest.head);
    assert_eq!(binding["versions"][1]["github_repository_id"], 456);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM delivery")
            .fetch_one(&p.pool)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM group_acceptance")
            .fetch_one(&p.pool)
            .await
            .unwrap(),
        1
    );
    p.pool.close().await;
    unsafe {
        std::env::remove_var("LOCAL_GIT_TARGETS");
    }
}

fn worker_config(p: &Product) -> codexsymphony_server::runtime_service::Config {
    serde_json::from_value(serde_json::json!({"validation":p.plan,"settings":{"startup_seconds":5,"response_seconds":5,"stall_seconds":5,"reservation":{"tokens":20,"turns":1,"model_seconds":10},"codex_config":""},"preparation_adapter":"/missing-reviewed-preparation","preparation":{"launcher":["/bin/true"],"baseline":p.manifest.head}})).unwrap()
}

#[tokio::test]
async fn local_failed_acceptance_uses_original_item_quota_paths_and_new_delivered_version() {
    use codexsymphony_server::{
        execution::Launch,
        runtime_service,
        workspace::{Manifest, Workspace},
    };
    use serde_json::{Value, json};
    let p = product_group(true).await;
    assert!(validate_product(&p).await);
    product_tick(&p).await;
    fs::write(p.root.path().join("post-fail"), "dependency changed").unwrap();
    product_tick(&p).await;
    let original: Value = sqlx::query_scalar(
        "SELECT local_acceptance FROM delivery WHERE validation_id='validation'",
    )
    .fetch_one(&p.pool)
    .await
    .unwrap();
    assert_eq!(original["passed"], false);
    let app = codexsymphony_server::execution_api::routes().with_state(p.pool.clone());
    let view = groups::request(&app, "GET", "/api/requirements/1/delivery", json!({}), 200).await;
    let local = &view["deliveries"][0];
    assert_eq!(local["mode"], "local_git");
    assert_eq!(local["repository_id"], 1);
    assert_eq!(local["delivery_version"], p.manifest.head);
    assert_eq!(local["acceptance"], original);
    assert!(local.get("merged").is_none());
    assert!(local.get("pr_number").is_none());
    let detail = codexsymphony_server::operator_view::detail(&p.pool, 1)
        .await
        .unwrap();
    let observation: Value =
        serde_json::from_str(detail["external"][0]["observation"].as_str().unwrap()).unwrap();
    assert_eq!(observation["delivery_version"], p.manifest.head);
    let config = worker_config(&p);
    let supervisor = Path::new(env!("CARGO_BIN_EXE_codexsymphony-server"));
    // The worker freezes the current local baseline and reserves the existing
    // reviewed quota before a separately approved preparation fixture binds it.
    let _ = runtime_service::tick(
        &p.pool,
        p.root.path(),
        supervisor,
        &p.broker,
        "boot",
        &config,
    )
    .await;
    let failure: Value = sqlx::query_scalar("SELECT to_jsonb(f) FROM linked_failure f")
        .fetch_one(&p.pool)
        .await
        .unwrap();
    assert_eq!(failure["state"], "reserved", "{failure}");
    assert_eq!(failure["baseline"], p.manifest.head);
    assert_eq!(failure["paths"], json!(["value"]));
    assert_eq!(failure["evidence"], original["evidence"]);
    let (launch, workspace): (Value, Value) =
        sqlx::query_as("SELECT launch,workspace FROM repair_reservation")
            .fetch_one(&p.pool)
            .await
            .unwrap();
    let launch: Launch = serde_json::from_value(launch).unwrap();
    let workspace: Workspace = serde_json::from_value(workspace).unwrap();
    sqlx::query("INSERT INTO preparation_record(run_id,requirement_id,revision,launch,retry,ready,checked_at) VALUES($1,1,1,$2,'{}',true,extract(epoch FROM now())::bigint) ON CONFLICT(run_id) DO UPDATE SET ready=true,retry='{}',checked_at=EXCLUDED.checked_at").bind(&launch.key.run_id).bind(json!(launch)).execute(&p.pool).await.unwrap();
    runtime_service::tick(
        &p.pool,
        p.root.path(),
        supervisor,
        &p.broker,
        "boot",
        &config,
    )
    .await
    .unwrap();
    // Controlled executor: the real Runtime path is covered independently above.
    fs::write(
        Path::new(&workspace.path).join("value"),
        "after\nrepaired\n",
    )
    .unwrap();
    p.broker
        .commit(&workspace, "Repair only the reviewed value")
        .unwrap();
    let manifest: Manifest = p.broker.preserve(&workspace).unwrap();
    sqlx::query(
        "UPDATE agent_run SET state='Succeeded',quiescent=true,phase='validation' WHERE id=$1",
    )
    .bind(&launch.key.run_id)
    .execute(&p.pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO workspace_snapshot(run_id,manifest,candidate) VALUES($1,$2,true)")
        .bind(&launch.key.run_id)
        .bind(json!(manifest))
        .execute(&p.pool)
        .await
        .unwrap();
    assert!(
        codexsymphony_server::validation_worker::tick(&p.pool, p.root.path(), &p.broker, &p.plan)
            .await
            .unwrap()
    );
    product_tick(&p).await;
    product_tick(&p).await;
    assert_eq!(local_git::head(&p.binding).unwrap(), manifest.head);
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT state FROM requirement WHERE id=1")
            .fetch_one(&p.pool)
            .await
            .unwrap(),
        "Done"
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT state FROM linked_failure")
            .fetch_one(&p.pool)
            .await
            .unwrap(),
        "complete"
    );
    let saved: Value = sqlx::query_scalar(
        "SELECT local_acceptance FROM delivery WHERE validation_id='validation'",
    )
    .fetch_one(&p.pool)
    .await
    .unwrap();
    assert_eq!(saved, original);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM repair_reservation")
            .fetch_one(&p.pool)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM github_pr")
            .fetch_one(&p.pool)
            .await
            .unwrap(),
        0
    );
    p.pool.close().await;
    unsafe {
        std::env::remove_var("LOCAL_GIT_TARGETS");
    }
}

#[tokio::test]
async fn cancellation_stops_acceptance_and_reconciles_before_releasing_owner() {
    use codexsymphony_server::{delivery_control, local_delivery};
    let mut p = product().await;
    let script =
        "#!/bin/sh\ncat value\nif [ -f \"$(dirname \"$0\")/wait-post\" ]; then sleep 60; fi\n";
    fs::write(&p.plan.entry, script).unwrap();
    p.plan.entry_sha256 = codexsymphony_server::validation::sha256(script);
    assert!(validate_product(&p).await);
    product_tick(&p).await;
    fs::write(p.root.path().join("wait-post"), "wait").unwrap();
    let waiting = async {
        for _ in 0..300 {
            let started: bool = sqlx::query_scalar("SELECT local_acceptance_started FROM delivery")
                .fetch_one(&p.pool)
                .await
                .unwrap();
            if started {
                delivery_control::cancel(&p.pool, 1).await.unwrap();
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("acceptance did not launch");
    };
    let (result, _) = tokio::join!(
        local_delivery::tick(&p.pool, p.root.path(), &p.broker),
        waiting
    );
    assert!(result.unwrap());
    sqlx::query("UPDATE delivery_action SET next_attempt_at=0")
        .execute(&p.pool)
        .await
        .unwrap();
    product_tick(&p).await;
    let (state,quiet,released,owner):(String,bool,bool,Option<i64>)=sqlx::query_as("SELECT r.state,d.local_acceptance_quiescent,d.released,c.requirement_id FROM requirement r CROSS JOIN delivery d CROSS JOIN execution_control c").fetch_one(&p.pool).await.unwrap();
    assert_eq!(state, "Cancelled");
    assert!(quiet && released);
    assert_eq!(owner, None);
    assert_eq!(local_git::head(&p.binding).unwrap(), p.manifest.head);
    // A cancelled delivered version stays delivered. Its failed invocation is
    // retained and never replaced by a later passing business completion.
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM delivery_attempt")
            .fetch_one(&p.pool)
            .await
            .unwrap(),
        1
    );
    p.pool.close().await;
    unsafe {
        std::env::remove_var("LOCAL_GIT_TARGETS");
    }
}

#[tokio::test]
async fn unknown_acceptance_launch_missing_target_and_budget_never_authorize_new_work() {
    use codexsymphony_server::local_delivery_store as store;
    for change in [
        "UPDATE requirement_budget SET exhausted=true",
        "UPDATE repository SET version=2",
    ] {
        let p = product().await;
        assert!(validate_product(&p).await);
        sqlx::query(change).execute(&p.pool).await.unwrap();
        product_tick(&p).await;
        assert_eq!(
            local_git::head(&p.binding).unwrap(),
            p.manifest.workspace.baseline
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM delivery_attempt")
                .fetch_one(&p.pool)
                .await
                .unwrap(),
            0
        );
        p.pool.close().await;
    }
    let p = product().await;
    assert!(validate_product(&p).await);
    product_tick(&p).await;
    product_tick(&p).await;
    let task: serde_json::Value = sqlx::query_scalar("SELECT local_acceptance_job FROM delivery")
        .fetch_one(&p.pool)
        .await
        .unwrap();
    let directory = p.root.path().join(task["invocation"].as_str().unwrap());
    fs::rename(
        directory.join("identity.json"),
        directory.join("retained-identity.json"),
    )
    .unwrap();
    sqlx::query("UPDATE delivery SET released=false,local_acceptance_quiescent=false")
        .execute(&p.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE execution_control SET requirement_id=1")
        .execute(&p.pool)
        .await
        .unwrap();
    product_tick(&p).await;
    assert!(directory.join("stop.json").exists());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM delivery_attempt")
            .fetch_one(&p.pool)
            .await
            .unwrap(),
        1
    );
    let job = store::pending(&p.pool).await.unwrap();
    assert!(job.is_none()); // backoff preserves the same intent
    let owner: Option<i64> = sqlx::query_scalar("SELECT requirement_id FROM execution_control")
        .fetch_one(&p.pool)
        .await
        .unwrap();
    assert_eq!(owner, Some(1));
    p.pool.close().await;
    unsafe {
        std::env::remove_var("LOCAL_GIT_TARGETS");
    }
}

#[test]
fn malformed_review_and_deleted_target_branch_cannot_forge_delivery() {
    use serde_json::json;
    let (root, target, base, candidate) = fixture();
    let binding = local_git::bind(&target).unwrap();
    for value in [
        json!({}),
        json!({"repository":{"remote":"fixture"}}),
        json!({"repository":{"remote":"fixture"},"repository_id":1}),
        json!({"repository":{"remote":"fixture"},"repository_id":1,"repository_version":1}),
    ] {
        assert!(codexsymphony_server::local_delivery_store::resolve_document(&value).is_err());
    }
    local_git::submit(
        &binding,
        &root.path().join("source"),
        "known",
        &base,
        &candidate,
    )
    .unwrap();
    git(
        &target.path,
        &["update-ref", "-d", "refs/heads/main", &candidate],
    );
    assert_eq!(
        local_git::observe(&binding, "known", &base, &candidate).unwrap(),
        Observation::Delivered
    );
    assert!(local_git::observe(&binding, "other", &base, &candidate).is_err());
}

#[tokio::test]
async fn acceptance_workspace_and_evidence_use_existing_storage_reservations() {
    let p = product().await;
    assert!(validate_product(&p).await);
    let _cold = install_storage(&p).await;
    product_tick(&p).await;
    product_tick(&p).await;
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT state FROM requirement")
            .fetch_one(&p.pool)
            .await
            .unwrap(),
        "Done"
    );
    let attempts: Vec<String> =
        sqlx::query_scalar("SELECT run_id FROM storage_attempt ORDER BY run_id")
            .fetch_all(&p.pool)
            .await
            .unwrap();
    assert!(attempts.iter().any(|id| id.starts_with("local-checkout-")));
    assert!(
        attempts
            .iter()
            .any(|id| id.starts_with("local-acceptance-"))
    );
    p.pool.close().await;
    unsafe {
        std::env::remove_var("LOCAL_GIT_TARGETS");
    }
}

#[tokio::test]
async fn mixed_integration_repairs_local_version_then_revalidates_complete_combination() {
    use codexsymphony_server::{
        execution::Launch, integration_worker, runtime_service, workspace::Workspace,
    };
    use serde_json::{Value, json};
    let p = product_group(true).await;
    assert!(validate_product(&p).await);
    product_tick(&p).await;
    product_tick(&p).await;
    fs::write(
        p.root.path().join("post-fail"),
        "integration dependency changed",
    )
    .unwrap();
    let supervisor = Path::new(env!("CARGO_BIN_EXE_codexsymphony-server"));
    for _ in 0..200 {
        integration_worker::tick(
            &p.pool,
            p.root.path(),
            supervisor,
            &p.broker,
            "boot",
            &p.plan,
        )
        .await
        .unwrap();
        if sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM integration_validation WHERE state='failed')",
        )
        .fetch_one(&p.pool)
        .await
        .unwrap()
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    let original: Value =
        sqlx::query_scalar("SELECT result FROM integration_validation WHERE state='failed'")
            .fetch_one(&p.pool)
            .await
            .unwrap();
    let config = worker_config(&p);
    let _ = runtime_service::tick(
        &p.pool,
        p.root.path(),
        supervisor,
        &p.broker,
        "boot",
        &config,
    )
    .await;
    let failure: Value = sqlx::query_scalar("SELECT to_jsonb(f) FROM linked_failure f")
        .fetch_one(&p.pool)
        .await
        .unwrap();
    assert_eq!(failure["state"], "reserved", "{failure}");
    assert_eq!(failure["repository_id"], 1);
    assert_eq!(failure["baseline"], p.manifest.head);
    assert_eq!(failure["paths"], json!(["value"]));
    let (launch, workspace): (Value, Value) =
        sqlx::query_as("SELECT launch,workspace FROM repair_reservation")
            .fetch_one(&p.pool)
            .await
            .unwrap();
    let launch: Launch = serde_json::from_value(launch).unwrap();
    let workspace: Workspace = serde_json::from_value(workspace).unwrap();
    assert_eq!(workspace.requirement, 2);
    sqlx::query("INSERT INTO preparation_record(run_id,requirement_id,revision,launch,retry,ready,checked_at) VALUES($1,2,1,$2,'{}',true,extract(epoch FROM now())::bigint) ON CONFLICT(run_id) DO UPDATE SET ready=true,retry='{}',checked_at=EXCLUDED.checked_at").bind(&launch.key.run_id).bind(json!(launch)).execute(&p.pool).await.unwrap();
    runtime_service::tick(
        &p.pool,
        p.root.path(),
        supervisor,
        &p.broker,
        "boot",
        &config,
    )
    .await
    .unwrap();
    fs::write(
        Path::new(&workspace.path).join("value"),
        "after\nrepaired\n",
    )
    .unwrap();
    p.broker
        .commit(&workspace, "Repair reviewed local integration input")
        .unwrap();
    let manifest = p.broker.preserve(&workspace).unwrap();
    sqlx::query(
        "UPDATE agent_run SET state='Succeeded',quiescent=true,phase='validation' WHERE id=$1",
    )
    .bind(&launch.key.run_id)
    .execute(&p.pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO workspace_snapshot(run_id,manifest,candidate) VALUES($1,$2,true)")
        .bind(&launch.key.run_id)
        .bind(json!(manifest))
        .execute(&p.pool)
        .await
        .unwrap();
    assert!(
        codexsymphony_server::validation_worker::tick(&p.pool, p.root.path(), &p.broker, &p.plan)
            .await
            .unwrap()
    );
    product_tick(&p).await;
    product_tick(&p).await;
    assert_eq!(local_git::head(&p.binding).unwrap(), manifest.head);
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT state FROM requirement WHERE id=2")
            .fetch_one(&p.pool)
            .await
            .unwrap(),
        "Running"
    );
    for _ in 0..200 {
        runtime_service::tick(
            &p.pool,
            p.root.path(),
            supervisor,
            &p.broker,
            "boot",
            &config,
        )
        .await
        .unwrap();
        if sqlx::query_scalar::<_, String>("SELECT state FROM requirement WHERE id=2")
            .fetch_one(&p.pool)
            .await
            .unwrap()
            == "Done"
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    let (state, binding): (String, Value) = sqlx::query_as(
        "SELECT state,binding FROM integration_validation ORDER BY created_at DESC,id DESC LIMIT 1",
    )
    .fetch_one(&p.pool)
    .await
    .unwrap();
    assert_eq!(state, "passed");
    assert_eq!(binding["versions"][0]["candidate"]["sha"], manifest.head);
    assert_eq!(
        binding["versions"][1]["candidate"]["sha"],
        p.manifest.workspace.baseline
    );
    assert!(binding["versions"][0].get("github_repository_id").is_none());
    let saved: Value =
        sqlx::query_scalar("SELECT result FROM integration_validation WHERE state='failed'")
            .fetch_one(&p.pool)
            .await
            .unwrap();
    assert_eq!(saved, original);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM delivery")
            .fetch_one(&p.pool)
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM github_pr")
            .fetch_one(&p.pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM repair_reservation WHERE requirement_id=2"
        )
        .fetch_one(&p.pool)
        .await
        .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT state FROM requirement WHERE id=2")
            .fetch_one(&p.pool)
            .await
            .unwrap(),
        "Done"
    );
    p.pool.close().await;
    unsafe {
        std::env::remove_var("LOCAL_GIT_TARGETS");
    }
}

async fn install_storage(p: &Product) -> FixtureDirectory {
    use codexsymphony_server::{
        storage_files::Directory,
        storage_lifecycle::{CATEGORIES, Limit, Policy},
        storage_store::{Deployment, Root},
    };
    let cold_directory = FixtureDirectory::new();
    let cold = cold_directory.path().to_owned();
    let root = |path: std::path::PathBuf| Root {
        identity: Directory::open(&path).unwrap().identity().unwrap(),
        path,
    };
    let config = Deployment {
        policy: Policy {
            version: "local-delivery-storage-v1".into(),
            reason: "Local validation reservation fixture".into(),
            global_bytes: 4 << 30,
            control_bytes: 256 << 20,
            run_bytes: 64 << 20,
            requirement_bytes: 128 << 20,
            entry_bytes: 1 << 20,
            entry_count: 10000,
            categories: CATEGORIES
                .into_iter()
                .map(|c| {
                    (
                        c,
                        Limit {
                            bytes: 2 << 30,
                            seconds: 3600,
                            reserve_bytes: 1 << 20,
                        },
                    )
                })
                .collect(),
        },
        execution: root(p.root.path().to_owned()),
        cold: root(cold),
        database_filesystem: root(p.root.path().to_owned()),
        database_extras: vec![],
    };
    codexsymphony_server::storage_store::install(&p.pool, &config)
        .await
        .unwrap();
    assert!(
        codexsymphony_server::storage_service::capacity(&p.pool)
            .await
            .unwrap()
    );
    cold_directory
}

#[tokio::test]
async fn local_acceptance_storage_excess_stops_process_and_retains_originals() {
    use codexsymphony_server::{local_delivery, storage_cleanup};
    let mut p = product().await;
    p.plan.steps[0].timeout_seconds = 30;
    let script =
        "#!/bin/sh\ncat value\nif [ -f \"$(dirname \"$0\")/wait-post\" ]; then sleep 60; fi\n";
    fs::write(&p.plan.entry, script).unwrap();
    p.plan.entry_sha256 = codexsymphony_server::validation::sha256(script);
    assert!(validate_product(&p).await);
    let _cold = install_storage(&p).await;
    product_tick(&p).await;
    fs::write(p.root.path().join("wait-post"), "wait").unwrap();
    let excess = async {
        for _ in 0..500 {
            let invocation: Option<String> =
                sqlx::query_scalar("SELECT local_acceptance_job->>'invocation' FROM delivery")
                    .fetch_one(&p.pool)
                    .await
                    .unwrap();
            if let Some(id) = invocation {
                let directory = p.root.path().join(id);
                if directory.join("identity.json").is_file() {
                    fs::write(directory.join("excess-output"), vec![b'x'; 2 << 20]).unwrap();
                    storage_cleanup::scan(&p.pool, codexsymphony_server::runtime_client::now())
                        .await
                        .unwrap();
                    return;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("local acceptance did not start");
    };
    let (result, _) = tokio::join!(
        local_delivery::tick(&p.pool, p.root.path(), &p.broker),
        excess
    );
    assert!(result.unwrap());
    let (blocked,quiet,released,state,owner,invocation):(bool,bool,bool,String,Option<i64>,String) = sqlx::query_as("SELECT d.local_storage_blocked,d.local_acceptance_quiescent,d.released,r.state,c.requirement_id,d.local_acceptance_job->>'invocation' FROM delivery d CROSS JOIN requirement r CROSS JOIN execution_control c").fetch_one(&p.pool).await.unwrap();
    assert!(blocked && quiet && !released);
    assert_eq!(state, "Submitted");
    assert_eq!(owner, Some(1));
    assert!(p.root.path().join(&invocation).join("stop.json").is_file());
    assert!(
        p.root
            .path()
            .join(&invocation)
            .join("excess-output")
            .is_file()
    );
    assert_eq!(local_git::head(&p.binding).unwrap(), p.manifest.head);
    p.pool.close().await;
    unsafe {
        std::env::remove_var("LOCAL_GIT_TARGETS");
    }
}

#[tokio::test]
async fn completed_acceptance_reconciles_original_invocation_after_restart() {
    use serde_json::Value;
    let p = product().await;
    assert!(validate_product(&p).await);
    product_tick(&p).await;
    product_tick(&p).await;
    let (job, evidence): (Value, Value) =
        sqlx::query_as("SELECT local_acceptance_job,local_acceptance FROM delivery")
            .fetch_one(&p.pool)
            .await
            .unwrap();
    let directory = p.root.path().join(job["invocation"].as_str().unwrap());
    let identity = fs::read(directory.join("identity.json")).unwrap();
    let outcome = fs::read(directory.join("outcome.json")).unwrap();
    // Retain the completed process, as if the final business transaction had
    // rolled back before a controller restart. Recovery must consume its files.
    sqlx::query(
        "UPDATE delivery SET released=false,local_acceptance=NULL,local_acceptance_quiescent=false",
    )
    .execute(&p.pool)
    .await
    .unwrap();
    sqlx::query("UPDATE requirement SET state='Submitted'")
        .execute(&p.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE execution_control SET requirement_id=1")
        .execute(&p.pool)
        .await
        .unwrap();
    product_tick(&p).await;
    let (state, saved): (String, Value) = sqlx::query_as(
        "SELECT r.state,d.local_acceptance FROM requirement r CROSS JOIN delivery d",
    )
    .fetch_one(&p.pool)
    .await
    .unwrap();
    assert_eq!(state, "Done");
    assert_eq!(saved, evidence);
    assert_eq!(fs::read(directory.join("identity.json")).unwrap(), identity);
    assert_eq!(fs::read(directory.join("outcome.json")).unwrap(), outcome);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM delivery_attempt")
            .fetch_one(&p.pool)
            .await
            .unwrap(),
        1
    );
    p.pool.close().await;
    unsafe {
        std::env::remove_var("LOCAL_GIT_TARGETS");
    }
}

#[tokio::test]
async fn frozen_target_drift_rejects_enqueue_without_rebinding_existing_intent() {
    use codexsymphony_server::{delivery_store, local_delivery_store};
    let p = product().await;
    assert!(validate_product(&p).await);
    let original = local_delivery_store::pending(&p.pool)
        .await
        .unwrap()
        .unwrap();
    sqlx::query("UPDATE initial_run SET local_binding=jsonb_set(local_binding,'{config_sha256}','\"changed\"')").execute(&p.pool).await.unwrap();
    let mut tx = p.pool.begin().await.unwrap();
    let error = delivery_store::enqueue(&mut tx, "validation")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("local target changed"));
    tx.rollback().await.unwrap();
    assert_eq!(
        local_delivery_store::pending(&p.pool)
            .await
            .unwrap()
            .unwrap(),
        original
    );
    assert_eq!(
        local_git::head(&p.binding).unwrap(),
        p.manifest.workspace.baseline
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM delivery_attempt")
            .fetch_one(&p.pool)
            .await
            .unwrap(),
        0
    );
    p.pool.close().await;
    unsafe {
        std::env::remove_var("LOCAL_GIT_TARGETS");
    }
}

#[tokio::test]
async fn local_failure_cannot_repair_a_revoked_target_or_unreviewed_path() {
    for revoke in [true, false] {
        let p = product_group(true).await;
        assert!(validate_product(&p).await);
        product_tick(&p).await;
        fs::write(p.root.path().join("post-fail"), "dependency changed").unwrap();
        product_tick(&p).await;
        if revoke {
            sqlx::query(
                "UPDATE plugin_scope SET enabled=false WHERE plugin_id='delivery:local_git'",
            )
            .execute(&p.pool)
            .await
            .unwrap();
        } else {
            sqlx::query("UPDATE group_execution_item SET input=jsonb_set(input,'{review,repair_scope}','\"none\"') WHERE requirement_id=1").execute(&p.pool).await.unwrap();
        }
        let _ = codexsymphony_server::runtime_service::tick(
            &p.pool,
            p.root.path(),
            Path::new(env!("CARGO_BIN_EXE_codexsymphony-server")),
            &p.broker,
            "boot",
            &worker_config(&p),
        )
        .await;
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT state FROM linked_failure")
                .fetch_one(&p.pool)
                .await
                .unwrap(),
            "blocked"
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM repair_reservation")
                .fetch_one(&p.pool)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM delivery_attempt")
                .fetch_one(&p.pool)
                .await
                .unwrap(),
            1
        );
        assert_eq!(local_git::head(&p.binding).unwrap(), p.manifest.head);
        p.pool.close().await;
        unsafe {
            std::env::remove_var("LOCAL_GIT_TARGETS");
        }
    }
}

#[test]
fn symbolic_targets_and_receipts_cannot_redirect_or_forge_delivery() {
    let (root, target, base, candidate) = fixture();
    let binding = local_git::bind(&target).unwrap();
    git(&target.path, &["update-ref", "refs/heads/other", &base]);
    git(
        &target.path,
        &["symbolic-ref", "refs/heads/main", "refs/heads/other"],
    );
    assert!(local_git::head(&binding).is_err());
    assert!(
        local_git::submit(
            &binding,
            &root.path().join("source"),
            "symbolic",
            &base,
            &candidate
        )
        .is_err()
    );
    assert_eq!(git(&target.path, &["rev-parse", "refs/heads/other"]), base);
    git(
        &target.path,
        &["update-ref", "--no-deref", "refs/heads/main", &base],
    );
    git(
        &target.path,
        &[
            "fetch",
            root.path().join("source").to_str().unwrap(),
            &candidate,
        ],
    );
    git(
        &target.path,
        &["update-ref", "refs/heads/other", &candidate],
    );
    let receipt = format!(
        "refs/symphony-deliveries/{}",
        codexsymphony_server::validation::sha256("forged")
    );
    git(
        &target.path,
        &["symbolic-ref", &receipt, "refs/heads/other"],
    );
    assert_eq!(
        local_git::observe(&binding, "forged", &base, &candidate).unwrap(),
        Observation::Conflict
    );
    assert_eq!(
        local_git::submit(
            &binding,
            &root.path().join("source"),
            "forged",
            &base,
            &candidate
        )
        .unwrap(),
        Observation::Conflict
    );
    assert_eq!(local_git::head(&binding).unwrap(), base);
}
