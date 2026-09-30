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
    hooks: serde_json::Value,
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
        hooks: serde_json::Value::Null,
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
    product_tick_at(p, "product_tick").await;
}

/// Read-only scheduling facts for a tick that found no work: the pending
/// predicate inputs, throttle, acceptance projection, failures and proofs.
async fn tick_state(p: &Product) -> serde_json::Value {
    let facts: serde_json::Value = sqlx::query_scalar("SELECT jsonb_build_object('now',extract(epoch FROM now())::bigint,'requirement',(SELECT jsonb_agg(jsonb_build_object('state',state,'paused',paused,'cancel_requested',cancel_requested,'version',version)) FROM requirement),'control',(SELECT jsonb_agg(jsonb_build_object('paused',paused,'recovery_complete',recovery_complete)) FROM execution_control),'deliveries',(SELECT jsonb_agg(jsonb_build_object('action_key',d.action_key,'validation_id',d.validation_id,'released',d.released,'acceptance_started',d.local_acceptance_started,'acceptance_quiescent',d.local_acceptance_quiescent,'acceptance',d.local_acceptance,'action_state',a.state,'attempts',a.attempts,'next_attempt_at',a.next_attempt_at,'error',a.error,'hook_invalidated',v.hook_invalidated,'superseded_by',v.superseded_by,'result',v.result) ORDER BY d.action_key) FROM delivery d LEFT JOIN delivery_action a ON a.action_key=d.action_key AND a.kind='publish' LEFT JOIN candidate_validation v ON v.id=d.validation_id),'linked_failures',(SELECT jsonb_agg(jsonb_build_object('id',id,'state',state,'local_delivery',local_delivery,'blocker',blocker) ORDER BY id) FROM linked_failure),'rechecks',(SELECT count(*) FROM local_acceptance_recheck),'local_blocked',(SELECT jsonb_agg(fact ORDER BY id) FROM delivery_observation WHERE kind='local_blocked'),'storage_blocked',(SELECT jsonb_agg(jsonb_build_object('blocked',blocked,'error',error)) FROM storage_guard))")
        .fetch_one(&p.pool)
        .await
        .unwrap();
    let pending = codexsymphony_server::local_delivery_store::pending(&p.pool)
        .await
        .map(|job| job.map(|job| job.action_key))
        .map_err(|error| error.to_string());
    serde_json::json!({"facts": facts, "pending": format!("{pending:?}")})
}

async fn product_tick_at(p: &Product, stage: &str) {
    let before = tick_state(p).await;
    let worked = codexsymphony_server::local_delivery::tick_with_hooks(
        &p.pool,
        p.root.path(),
        &p.broker,
        &p.hooks,
    )
    .await
    .unwrap();
    if !worked {
        panic!(
            "local delivery tick found no work at {stage}\nbefore: {before:#}\nafter: {:#}",
            tick_state(p).await
        );
    }
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
        product_tick_at(&p, "javascript: deliver").await;
        product_tick_at(&p, "javascript: accept").await;
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
            model_capabilities: None,
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
        product_tick_at(&p, "local_failure: deliver").await;
        fs::write(p.root.path().join("post-fail"), "dependency changed").unwrap();
        product_tick_at(&p, "local_failure: failed acceptance").await;
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

async fn failed_validator() -> (
    Product,
    codexsymphony_server::local_acceptance_recheck::Command,
) {
    failed_validator_with_hooks(false).await
}

async fn failed_validator_with_hooks(
    with_hooks: bool,
) -> (
    Product,
    codexsymphony_server::local_acceptance_recheck::Command,
) {
    use codexsymphony_server::{local_acceptance_recheck::Command, validation::sha256};
    let mut p = product_group(true).await;
    if with_hooks {
        configure_recheck_hooks(&mut p).await;
    }
    let script = "#!/bin/sh\nset -eu\ncat value\ntest \"$(head -n1 value)\" = after\nif [ -f \"$SYMPHONY_DIAGNOSTIC_DIR/../job.json\" ] && [ \"${1:-}\" != corrected-local-phase ]; then echo 'AssertionError: validator confused local acceptance with integration'; exit 1; fi\n";
    fs::write(&p.plan.entry, script).unwrap();
    p.plan.entry_sha256 = sha256(script);
    assert!(validate_product(&p).await);
    product_tick_at(&p, "failed_validator: deliver").await;
    product_tick_at(&p, "failed_validator: failed acceptance").await;
    let (key, result, version): (String, serde_json::Value, i64) = sqlx::query_as("SELECT d.action_key,COALESCE(d.local_acceptance,'null'::jsonb),r.version FROM delivery d JOIN requirement r ON r.id=d.requirement_id WHERE d.validation_id='validation'").fetch_one(&p.pool).await.unwrap();
    assert_eq!(
        result["passed"],
        false,
        "delivery blocked before failed-validator fixture: {:?}",
        sqlx::query_scalar::<_, Option<serde_json::Value>>(
            "SELECT error FROM delivery_action WHERE action_key=$1"
        )
        .bind(&key)
        .fetch_one(&p.pool)
        .await
        .unwrap()
    );
    sqlx::query("UPDATE execution_control SET paused=true")
        .execute(&p.pool)
        .await
        .unwrap();
    let mut plan = p.plan.clone();
    plan.steps[0].command.push("corrected-local-phase".into());
    let command = Command {
        request_id: "reviewed-validator-correction".into(),
        requirement_id: 1,
        version,
        delivery_key: key,
        previous_result_sha256: sha256(serde_json::to_vec(&result).unwrap()),
        reason:
            "Correct validator phase identification; original code and required assertion unchanged"
                .into(),
        plan,
    };
    (p, command)
}

async fn recheck_cli(p: &Product, args: &[&str], payload: &[u8]) -> std::process::Output {
    use sqlx::ConnectOptions;
    use std::io::Write;
    let options = p.pool.connect_options();
    let mut url = options.to_url_lossy();
    // SQLx 0.8's lossy URL omits PostgreSQL options, including schema isolation.
    url.query_pairs_mut()
        .append_pair("options", options.get_options().unwrap());
    let roundtrip: sqlx::postgres::PgConnectOptions = url.as_str().parse().unwrap();
    assert_eq!(roundtrip.get_options(), options.get_options());
    let mut child = Command::new(env!("CARGO_BIN_EXE_codexsymphony-server"))
        .arg("delivery")
        .args(args)
        .env("DATABASE_URL", url.as_str())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(payload).unwrap();
    child.wait_with_output().unwrap()
}

#[tokio::test]
async fn validator_correction_preserves_failed_identity_and_never_resends_or_spends() {
    use codexsymphony_server::local_acceptance_recheck::request;
    use serde_json::Value;
    let (p, command) = failed_validator_with_hooks(true).await;
    let old: Value = sqlx::query_scalar("SELECT jsonb_build_object('job',local_acceptance_job,'result',local_acceptance) FROM delivery WHERE validation_id='validation'").fetch_one(&p.pool).await.unwrap();
    let account_sql = "SELECT jsonb_build_object('calls',(SELECT count(*) FROM model_call),'attempts',(SELECT count(*) FROM delivery_attempt),'budgets',(SELECT jsonb_agg(to_jsonb(b)) FROM requirement_budget b),'groups',(SELECT jsonb_agg(to_jsonb(b)) FROM group_budget b),'validations',(SELECT jsonb_agg(to_jsonb(v)) FROM candidate_validation v))";
    let before: Value = sqlx::query_scalar(account_sql)
        .fetch_one(&p.pool)
        .await
        .unwrap();
    let result = recheck_cli(
        &p,
        &["acceptance-recheck", "--stdin-json"],
        &serde_json::to_vec(&command).unwrap(),
    )
    .await;
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&result.stdout).unwrap()["started"],
        false
    );
    assert_eq!(request(&p.pool, &command).await.unwrap()["accepted"], true);
    let retained: Value = sqlx::query_scalar("SELECT jsonb_build_object('job',previous_job,'result',previous_result) FROM local_acceptance_recheck").fetch_one(&p.pool).await.unwrap();
    assert_eq!(retained, old);
    assert_eq!(
        sqlx::query_scalar::<_, Value>(account_sql)
            .fetch_one(&p.pool)
            .await
            .unwrap(),
        before
    );
    let mut conflict = command.clone();
    conflict.reason = "Different approval cannot replay an old identity".into();
    assert!(request(&p.pool, &conflict).await.is_err());
    conflict = command.clone();
    conflict.request_id = "second-request-while-pending".into();
    assert!(request(&p.pool, &conflict).await.is_err());
    sqlx::query("UPDATE execution_control SET paused=false")
        .execute(&p.pool)
        .await
        .unwrap();
    product_tick(&p).await;
    let (state, accepted, invocation): (String,Value,String) = sqlx::query_as("SELECT r.state,d.local_acceptance,d.local_acceptance_job->>'invocation' FROM delivery d JOIN requirement r ON r.id=d.requirement_id WHERE d.validation_id='validation'").fetch_one(&p.pool).await.unwrap();
    assert_eq!(state, "Done");
    assert_eq!(accepted["passed"], true);
    assert!(invocation.starts_with("local-acceptance-recheck-"));
    let hook_calls: Vec<Value> = fs::read_to_string(p.root.path().join("recheck-hook-calls.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        hook_calls,
        vec![
            serde_json::json!([old["job"]["invocation"], "before_run"]),
            serde_json::json!([old["job"]["invocation"], "after_run"]),
            serde_json::json!([invocation, "before_run"]),
            serde_json::json!([invocation, "after_run"]),
        ]
    );
    assert_ne!(
        accepted["evidence"]["trusted"],
        old["result"]["evidence"]["trusted"]
    );
    assert_eq!(
        accepted["evidence"]["candidate"],
        old["result"]["evidence"]["candidate"]
    );
    assert_eq!(local_git::head(&p.binding).unwrap(), p.manifest.head);
    let counts:(i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM model_call),(SELECT count(*) FROM delivery_attempt),(SELECT count(*) FROM local_acceptance_recheck)").fetch_one(&p.pool).await.unwrap();
    assert_eq!(counts, (0, 1, 1));
    assert_eq!(
        sqlx::query_scalar::<_, Value>("SELECT previous_result FROM local_acceptance_recheck")
            .fetch_one(&p.pool)
            .await
            .unwrap(),
        old["result"]
    );
    assert!(
        !codexsymphony_server::local_delivery::tick(&p.pool, p.root.path(), &p.broker)
            .await
            .unwrap()
    );
    p.pool.close().await;
}

#[tokio::test]
async fn acceptance_correction_rejects_unbound_unsafe_and_unchanged_inputs() {
    use codexsymphony_server::local_acceptance_recheck::request;
    let (p, command) = failed_validator().await;
    assert!(!recheck_cli(&p, &["unknown"], b"").await.status.success());
    assert!(
        !recheck_cli(&p, &["acceptance-recheck", "--stdin-json"], b"{}")
            .await
            .status
            .success()
    );
    assert!(
        !recheck_cli(
            &p,
            &["acceptance-recheck", "--stdin-json"],
            &vec![b' '; 32769]
        )
        .await
        .status
        .success()
    );
    let mut bad = command.clone();
    bad.request_id.clear();
    assert!(request(&p.pool, &bad).await.is_err());
    bad = command.clone();
    bad.version += 1;
    assert!(request(&p.pool, &bad).await.is_err());
    bad = command.clone();
    bad.previous_result_sha256 = "0".repeat(64);
    assert!(request(&p.pool, &bad).await.is_err());
    bad = command.clone();
    bad.plan = p.plan.clone();
    assert!(request(&p.pool, &bad).await.is_err());
    bad = command.clone();
    bad.plan.steps.clear();
    assert!(request(&p.pool, &bad).await.is_err());
    bad = command.clone();
    bad.plan.steps[0].timeout_seconds += 1;
    assert!(request(&p.pool, &bad).await.is_err());
    bad = command.clone();
    bad.plan.steps[0].id = "different-check".into();
    assert!(request(&p.pool, &bad).await.is_err());
    bad = command.clone();
    bad.plan.steps[0].code_failure = false;
    assert!(request(&p.pool, &bad).await.is_err());
    bad = command.clone();
    bad.plan.steps.push(bad.plan.steps[0].clone());
    bad.plan.steps[1].id = "extra".into();
    assert!(request(&p.pool, &bad).await.is_err());
    let original_input: serde_json::Value =
        sqlx::query_scalar("SELECT input FROM group_execution_item WHERE requirement_id=1")
            .fetch_one(&p.pool)
            .await
            .unwrap();
    sqlx::query("UPDATE group_execution_item SET input=jsonb_set(input,'{child,kind}','\"validation_only\"') WHERE requirement_id=1")
        .execute(&p.pool).await.unwrap();
    assert!(request(&p.pool, &command).await.is_err());
    sqlx::query("UPDATE group_execution_item SET input=$1 WHERE requirement_id=1")
        .bind(original_input)
        .execute(&p.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE execution_control SET paused=false")
        .execute(&p.pool)
        .await
        .unwrap();
    assert!(request(&p.pool, &command).await.is_err());
    sqlx::query("UPDATE execution_control SET paused=true")
        .execute(&p.pool)
        .await
        .unwrap();
    git(
        &p.binding.target.path,
        &[
            "update-ref",
            "refs/heads/main",
            &p.manifest.workspace.baseline,
            &p.manifest.head,
        ],
    );
    assert!(request(&p.pool, &command).await.is_err());
    git(
        &p.binding.target.path,
        &[
            "update-ref",
            "refs/heads/main",
            &p.manifest.head,
            &p.manifest.workspace.baseline,
        ],
    );
    sqlx::query("UPDATE repository SET revoked_through_version=version WHERE id=1")
        .execute(&p.pool)
        .await
        .unwrap();
    assert!(request(&p.pool, &command).await.is_err());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM local_acceptance_recheck")
            .fetch_one(&p.pool)
            .await
            .unwrap(),
        0
    );
    p.pool.close().await;
}

#[tokio::test]
async fn failed_validator_rechecks_stop_and_have_a_cumulative_attempt_limit() {
    use codexsymphony_server::{local_acceptance_recheck::request, validation::sha256};
    let (p, mut command) = failed_validator().await;
    for ordinal in 1..=3 {
        command.request_id = format!("failed-validator-correction-{ordinal}");
        command.plan.steps[0].command =
            vec!["/gate-entry".into(), format!("still-wrong-{ordinal}")];
        let result: serde_json::Value = sqlx::query_scalar(
            "SELECT local_acceptance FROM delivery WHERE validation_id='validation'",
        )
        .fetch_one(&p.pool)
        .await
        .unwrap();
        command.previous_result_sha256 = sha256(serde_json::to_vec(&result).unwrap());
        request(&p.pool, &command).await.unwrap();
        sqlx::query("UPDATE execution_control SET paused=false")
            .execute(&p.pool)
            .await
            .unwrap();
        product_tick(&p).await;
        assert!(
            !codexsymphony_server::local_delivery::tick(&p.pool, p.root.path(), &p.broker)
                .await
                .unwrap()
        );
        sqlx::query("UPDATE execution_control SET paused=true")
            .execute(&p.pool)
            .await
            .unwrap();
    }
    command.request_id = "fourth-correction".into();
    command.plan.steps[0].command = vec!["/gate-entry".into(), "corrected-local-phase".into()];
    let result: serde_json::Value = sqlx::query_scalar(
        "SELECT local_acceptance FROM delivery WHERE validation_id='validation'",
    )
    .fetch_one(&p.pool)
    .await
    .unwrap();
    command.previous_result_sha256 = sha256(serde_json::to_vec(&result).unwrap());
    assert!(
        request(&p.pool, &command)
            .await
            .unwrap_err()
            .to_string()
            .contains("limit reached")
    );
    let state: String = sqlx::query_scalar("SELECT state FROM requirement WHERE id=1")
        .fetch_one(&p.pool)
        .await
        .unwrap();
    assert_eq!(state, "Submitted");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM local_acceptance_recheck")
            .fetch_one(&p.pool)
            .await
            .unwrap(),
        3
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM delivery_attempt")
            .fetch_one(&p.pool)
            .await
            .unwrap(),
        1
    );
    p.pool.close().await;
}

fn local_revalidation(
    p: &Product,
    request: &str,
    version: i64,
    delivery_key: &str,
) -> codexsymphony_server::extension_recovery::Decision {
    use codexsymphony_server::extension_recovery::{Action, Decision};
    Decision {
        request_id: request.into(),
        version,
        revision: 1,
        validation_id: "validation".into(),
        reason: "control interruption invalidated delivered proof; same candidate only".into(),
        action: Action::RevalidateLocalDelivery {
            plan_digest: p.plan.identity().unwrap().config_sha256,
            resume_condition: "fresh proof for the unchanged delivered version".into(),
            delivery_key: delivery_key.into(),
        },
    }
}

async fn requirement_version(p: &Product) -> i64 {
    sqlx::query_scalar("SELECT version FROM requirement WHERE id=1")
        .fetch_one(&p.pool)
        .await
        .unwrap()
}

/// Mirrors R2: a failed local acceptance of an already delivered version whose
/// proof a later global pause invalidated. Recovery must keep the original
/// action, target and attempt ledger, and must never write the target again.
async fn recover_delivered_local_proof(recheck_first: bool) {
    use codexsymphony_server::{
        extension_recovery::{self as recovery, Action},
        local_acceptance_recheck::request,
        local_delivery_store as store,
    };
    use serde_json::{Value, json};
    let (p, mut command) = failed_validator().await;
    let key = command.delivery_key.clone();
    // Persisted outcome of the control interruption; never cleared below.
    sqlx::query("UPDATE candidate_validation SET hook_invalidated=true WHERE id='validation'")
        .execute(&p.pool)
        .await
        .unwrap();
    let account_sql = "SELECT jsonb_build_object('calls',(SELECT count(*) FROM model_call),'budgets',(SELECT jsonb_agg(to_jsonb(b)) FROM requirement_budget b),'groups',(SELECT jsonb_agg(to_jsonb(b)) FROM group_budget b))";
    let account: Value = sqlx::query_scalar(account_sql)
        .fetch_one(&p.pool)
        .await
        .unwrap();
    let action_sql = "SELECT to_jsonb(a) FROM delivery_action a WHERE action_key=$1";
    let original_action: Value = sqlx::query_scalar(action_sql)
        .bind(&key)
        .fetch_one(&p.pool)
        .await
        .unwrap();
    // An unstartable correction is rejected before registration.
    let rejected = request(&p.pool, &command).await.unwrap_err();
    assert!(
        rejected.to_string().contains("approve a same-candidate"),
        "{rejected}"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM local_acceptance_recheck")
            .fetch_one(&p.pool)
            .await
            .unwrap(),
        0
    );
    // Recovery authority is unavailable while paused.
    let decision = local_revalidation(
        &p,
        "local-proof-same-candidate",
        requirement_version(&p).await,
        &key,
    );
    assert!(recovery::decide(&p.pool, 1, &decision).await.is_err());
    sqlx::query("UPDATE execution_control SET paused=false")
        .execute(&p.pool)
        .await
        .unwrap();
    // Without a same-candidate generation the delivery waits instead of looping.
    assert!(store::pending(&p.pool).await.unwrap().is_none());
    let mut ordinary = decision.clone();
    ordinary.action = Action::Revalidate {
        plan_digest: p.plan.identity().unwrap().config_sha256,
        resume_condition: "ordinary revalidation cannot bind a delivered version".into(),
    };
    assert!(recovery::decide(&p.pool, 1, &ordinary).await.is_err());
    let mut wrong = decision.clone();
    wrong.action = local_revalidation(&p, "unused", 0, "local-other").action;
    assert!(recovery::decide(&p.pool, 1, &wrong).await.is_err());
    for (change, restore) in [
        (
            "UPDATE delivery_action SET attempts=2",
            "UPDATE delivery_action SET attempts=1",
        ),
        (
            "INSERT INTO delivery_attempt(action_key,kind,ordinal,operation) SELECT action_key,'publish',2,'local_update' FROM delivery",
            "DELETE FROM delivery_attempt WHERE ordinal=2",
        ),
        (
            "UPDATE delivery SET released=true",
            "UPDATE delivery SET released=false",
        ),
    ] {
        sqlx::query(change).execute(&p.pool).await.unwrap();
        assert!(
            recovery::decide(&p.pool, 1, &decision).await.is_err(),
            "{change}"
        );
        sqlx::query(restore).execute(&p.pool).await.unwrap();
    }
    git(
        &p.binding.target.path,
        &[
            "update-ref",
            "refs/heads/main",
            &p.manifest.workspace.baseline,
            &p.manifest.head,
        ],
    );
    assert!(recovery::decide(&p.pool, 1, &decision).await.is_err());
    git(
        &p.binding.target.path,
        &[
            "update-ref",
            "refs/heads/main",
            &p.manifest.head,
            &p.manifest.workspace.baseline,
        ],
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM business_request WHERE input ? 'extension_recovery'"
        )
        .fetch_one(&p.pool)
        .await
        .unwrap(),
        0
    );
    let accepted = recovery::decide(&p.pool, 1, &decision).await.unwrap();
    assert_eq!(accepted["started"], false);
    assert_eq!(
        accepted,
        recovery::decide(&p.pool, 1, &decision).await.unwrap()
    );
    command.version = requirement_version(&p).await;
    let mut receipt = Value::Null;
    if recheck_first {
        // Admitted because exactly this proof has a pending approved generation.
        receipt = request(&p.pool, &command).await.unwrap();
        assert_eq!(receipt["accepted"], true);
        assert_eq!(receipt["started"], false);
        assert!(receipt.get("proof").is_none());
        assert_eq!(request(&p.pool, &command).await.unwrap(), receipt);
        // Registered, but held until the fresh proof is bound.
        assert!(store::pending(&p.pool).await.unwrap().is_none());
        assert!(
            !codexsymphony_server::local_delivery::tick(&p.pool, p.root.path(), &p.broker)
                .await
                .unwrap()
        );
    }
    let before: Value = sqlx::query_scalar("SELECT to_jsonb(d) FROM delivery d")
        .fetch_one(&p.pool)
        .await
        .unwrap();
    assert!(
        codexsymphony_server::extension_revalidation::tick(
            &p.pool,
            p.root.path(),
            &p.broker,
            &p.plan,
            &p.hooks
        )
        .await
        .unwrap()
    );
    let (successor, state): (String, String) = sqlx::query_as(
        "SELECT successor_validation,resolution_state FROM recovery_failure WHERE event_key=$1",
    )
    .bind(accepted["event_key"].as_str().unwrap())
    .fetch_one(&p.pool)
    .await
    .unwrap();
    assert_eq!(state, "complete");
    assert!(successor.starts_with("revalidate-"));
    let current: Value = sqlx::query_scalar("SELECT to_jsonb(d) FROM delivery d")
        .fetch_one(&p.pool)
        .await
        .unwrap();
    let mut expected = before;
    expected["validation_id"] = json!(successor);
    assert_eq!(current, expected, "only the proof binding may change");
    assert_eq!(
        sqlx::query_scalar::<_, Value>(action_sql)
            .bind(&key)
            .fetch_one(&p.pool)
            .await
            .unwrap(),
        original_action
    );
    let preserved: (String, bool, Option<String>) = sqlx::query_as(
        "SELECT result,hook_invalidated,superseded_by FROM candidate_validation WHERE id='validation'",
    )
    .fetch_one(&p.pool)
    .await
    .unwrap();
    assert_eq!(
        preserved,
        ("succeeded".into(), true, Some(successor.clone()))
    );
    let fact: Value =
        sqlx::query_scalar("SELECT fact FROM delivery_observation WHERE kind='validation_rebound'")
            .fetch_one(&p.pool)
            .await
            .unwrap();
    assert_eq!(fact["previous_validation"], "validation");
    assert_eq!(fact["validation"], successor);
    assert_eq!(fact["recovery_event"], accepted["event_key"]);
    assert_eq!(fact["external_attempts"], 0);
    assert_eq!(
        sqlx::query_scalar::<_, Vec<String>>(
            "SELECT array_agg(DISTINCT consumer) FROM validation_step WHERE validation_id=$1"
        )
        .bind(&successor)
        .fetch_one(&p.pool)
        .await
        .unwrap(),
        vec![format!("outbox:{key}")]
    );
    // Replaying the outbox hand-off never derives a second local action.
    let mut tx = p.pool.begin().await.unwrap();
    codexsymphony_server::delivery_store::enqueue(&mut tx, &successor)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let keys: Vec<String> = sqlx::query_scalar("SELECT action_key FROM delivery")
        .fetch_all(&p.pool)
        .await
        .unwrap();
    assert_eq!(keys, vec![key.clone()]);
    let facts: (i64, i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM delivery_action),(SELECT count(*) FROM delivery_attempt),(SELECT count(*) FROM delivery_observation WHERE kind='validation_rebound')")
        .fetch_one(&p.pool).await.unwrap();
    assert_eq!(facts, (1, 1, 1));
    assert_eq!(local_git::head(&p.binding).unwrap(), p.manifest.head);
    if !recheck_first {
        // The retained failed acceptance keeps the delivery idle until corrected.
        assert!(store::pending(&p.pool).await.unwrap().is_none());
        receipt = request(&p.pool, &command).await.unwrap();
        assert_eq!(receipt["accepted"], true);
        assert!(receipt.get("proof").is_none());
    }
    // Replay returns the saved receipt, never a reclassification of current state.
    assert_eq!(request(&p.pool, &command).await.unwrap(), receipt);
    // Read-only: the correction reset the throttle and nothing blocked since,
    // so the rebound delivery is due now without advancing any clock.
    let due: bool = sqlx::query_scalar(
        "SELECT next_attempt_at<=extract(epoch FROM now())::bigint FROM delivery_action WHERE action_key=$1",
    )
    .bind(&key)
    .fetch_one(&p.pool)
    .await
    .unwrap();
    assert!(due, "{:#}", tick_state(&p).await);
    product_tick_at(
        &p,
        if recheck_first {
            "recover: recheck registered before rebind"
        } else {
            "recover: recheck registered after rebind"
        },
    )
    .await;
    let (state, accepted, invocation): (String, Value, String) = sqlx::query_as("SELECT r.state,d.local_acceptance,d.local_acceptance_job->>'invocation' FROM delivery d JOIN requirement r ON r.id=d.requirement_id")
        .fetch_one(&p.pool).await.unwrap();
    assert_eq!(
        state,
        "Done",
        "{:?}",
        sqlx::query_scalar::<_, Option<Value>>("SELECT error FROM delivery_action")
            .fetch_one(&p.pool)
            .await
            .unwrap()
    );
    assert_eq!(accepted["passed"], true);
    assert!(invocation.starts_with("local-acceptance-recheck-"));
    assert_eq!(local_git::head(&p.binding).unwrap(), p.manifest.head);
    // Even after Done (admission would now fail) the same request replays equal.
    assert_eq!(request(&p.pool, &command).await.unwrap(), receipt);
    let counts: (i64, i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM delivery),(SELECT count(*) FROM delivery_attempt),(SELECT count(*) FROM local_acceptance_recheck)")
        .fetch_one(&p.pool).await.unwrap();
    assert_eq!(counts, (1, 1, 1));
    assert_eq!(
        sqlx::query_scalar::<_, Value>(account_sql)
            .fetch_one(&p.pool)
            .await
            .unwrap(),
        account
    );
    p.pool.close().await;
    unsafe { std::env::remove_var("LOCAL_GIT_TARGETS") };
}

#[tokio::test]
async fn interrupted_delivered_proof_waits_for_same_candidate_generation_before_correction() {
    recover_delivered_local_proof(true).await;
}

#[tokio::test]
async fn rebound_delivered_proof_admits_correction_as_current_without_new_write() {
    recover_delivered_local_proof(false).await;
}

/// Environment-bound proof: the real pause trigger invalidates it, an expired
/// uninterrupted proof is refused, and a successor of the delivered version
/// never falls back to a new local action when its rebinding cannot be proven.
#[tokio::test]
async fn environment_bound_delivered_proof_fails_closed_on_drift_and_unapproved_successor() {
    use codexsymphony_server::{
        extension_recovery as recovery, local_acceptance_recheck::request,
        local_delivery_store as store,
    };
    use serde_json::Value;
    let (p, command) = failed_validator().await;
    let key = command.delivery_key.clone();
    // The fixture has no host environment registry, so emulate only the
    // retained invocation deadline that `proof_status` classifies.
    sqlx::query("UPDATE candidate_validation SET hook_required=true,hook_context=jsonb_build_object('call',jsonb_build_object('deadline_unix_ms',1)) WHERE id='validation'")
        .execute(&p.pool)
        .await
        .unwrap();
    let expired = request(&p.pool, &command).await.unwrap_err();
    assert!(
        expired.to_string().contains("expired without interruption"),
        "{expired}"
    );
    // A real control interruption, through the migration trigger.
    for paused in [false, true] {
        sqlx::query("UPDATE execution_control SET paused=$1")
            .bind(paused)
            .execute(&p.pool)
            .await
            .unwrap();
    }
    assert!(
        sqlx::query_scalar::<_, bool>(
            "SELECT hook_invalidated FROM candidate_validation WHERE id='validation'"
        )
        .fetch_one(&p.pool)
        .await
        .unwrap()
    );
    let interrupted = request(&p.pool, &command).await.unwrap_err();
    assert!(interrupted.to_string().contains("approve a same-candidate"));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM local_acceptance_recheck")
            .fetch_one(&p.pool)
            .await
            .unwrap(),
        0
    );
    sqlx::query("UPDATE execution_control SET paused=false")
        .execute(&p.pool)
        .await
        .unwrap();
    assert!(store::pending(&p.pool).await.unwrap().is_none());
    // Drop the emulated context: the original supervisor stop check decodes it.
    sqlx::query("UPDATE candidate_validation SET hook_context=NULL WHERE id='validation'")
        .execute(&p.pool)
        .await
        .unwrap();
    let decision = local_revalidation(
        &p,
        "environment-bound-same-candidate",
        requirement_version(&p).await,
        &key,
    );
    recovery::decide(&p.pool, 1, &decision).await.unwrap();
    let original: Value = sqlx::query_scalar("SELECT jsonb_build_object('delivery',(SELECT to_jsonb(d) FROM delivery d),'action',(SELECT to_jsonb(a) FROM delivery_action a))")
        .fetch_one(&p.pool)
        .await
        .unwrap();
    // The target moves after authorization; rebinding rechecks it under lock.
    git(
        &p.binding.target.path,
        &[
            "update-ref",
            "refs/heads/main",
            &p.manifest.workspace.baseline,
            &p.manifest.head,
        ],
    );
    assert!(
        codexsymphony_server::extension_revalidation::tick(
            &p.pool,
            p.root.path(),
            &p.broker,
            &p.plan,
            &p.hooks
        )
        .await
        .unwrap()
    );
    let (successor, state): (String, String) = sqlx::query_as(
        "SELECT successor_validation,resolution_state FROM recovery_failure WHERE resolution#>>'{command,request_id}'=$1",
    )
    .bind(&decision.request_id)
    .fetch_one(&p.pool)
    .await
    .unwrap();
    assert_eq!(state, "blocked");
    let reason: String =
        sqlx::query_scalar("SELECT reason FROM recovery_failure WHERE successor_validation=$1")
            .bind(&successor)
            .fetch_one(&p.pool)
            .await
            .unwrap();
    assert!(reason.contains("current confirmed target"), "{reason}");
    let unchanged = || async {
        let current: Value = sqlx::query_scalar("SELECT jsonb_build_object('delivery',(SELECT to_jsonb(d) FROM delivery d),'action',(SELECT to_jsonb(a) FROM delivery_action a))")
            .fetch_one(&p.pool)
            .await
            .unwrap();
        assert_eq!(current, original);
        let counts: (i64, i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM delivery),(SELECT count(*) FROM delivery_attempt),(SELECT count(*) FROM delivery_observation WHERE kind='validation_rebound')")
            .fetch_one(&p.pool)
            .await
            .unwrap();
        assert_eq!(counts, (1, 1, 0));
    };
    unchanged().await;
    // Even a successful successor whose approval is no longer active cannot
    // take the ordinary outbox path and derive a second local action.
    sqlx::query(
        "UPDATE candidate_validation SET result='succeeded',hook_invalidated=false WHERE id=$1",
    )
    .bind(&successor)
    .execute(&p.pool)
    .await
    .unwrap();
    let mut tx = p.pool.begin().await.unwrap();
    let error = codexsymphony_server::delivery_store::enqueue(&mut tx, &successor)
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("lacks its approved rebinding"),
        "{error}"
    );
    tx.rollback().await.unwrap();
    unchanged().await;
    assert_eq!(
        local_git::head(&p.binding).unwrap(),
        p.manifest.workspace.baseline
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM model_call")
            .fetch_one(&p.pool)
            .await
            .unwrap(),
        0
    );
    p.pool.close().await;
    unsafe { std::env::remove_var("LOCAL_GIT_TARGETS") };
}

#[tokio::test]
async fn local_block_reason_survives_later_confirmed_observation() {
    use codexsymphony_server::local_delivery_store as store;
    let p = product().await;
    assert!(validate_product(&p).await);
    let job = store::pending(&p.pool).await.unwrap().unwrap();
    for reason in [
        "transient admission fault",
        "transient admission fault",
        "second cause",
    ] {
        store::blocked(&p.pool, &job, reason).await.unwrap();
    }
    sqlx::query("UPDATE delivery_action SET next_attempt_at=0")
        .execute(&p.pool)
        .await
        .unwrap();
    product_tick(&p).await;
    let error: Option<serde_json::Value> = sqlx::query_scalar("SELECT error FROM delivery_action")
        .fetch_one(&p.pool)
        .await
        .unwrap();
    assert_eq!(error, None);
    let reasons: Vec<String> = sqlx::query_scalar(
        "SELECT fact->>'reason' FROM delivery_observation WHERE kind='local_blocked' ORDER BY id",
    )
    .fetch_all(&p.pool)
    .await
    .unwrap();
    assert_eq!(reasons, vec!["transient admission fault", "second cause"]);
    let candidate: String = sqlx::query_scalar(
        "SELECT DISTINCT fact->>'candidate' FROM delivery_observation WHERE kind='local_blocked'",
    )
    .fetch_one(&p.pool)
    .await
    .unwrap();
    assert_eq!(candidate, p.manifest.head);
    p.pool.close().await;
    unsafe { std::env::remove_var("LOCAL_GIT_TARGETS") };
}

#[tokio::test]
async fn local_queue_projection_does_not_require_github_capabilities() {
    use serde_json::json;
    let p = product_group(true).await;
    sqlx::query("UPDATE requirement SET state='Ready' WHERE id=1")
        .execute(&p.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE execution_control SET requirement_id=NULL")
        .execute(&p.pool)
        .await
        .unwrap();
    let draft: String =
        sqlx::query_scalar("SELECT draft_id FROM group_execution_item WHERE requirement_id=1")
            .fetch_one(&p.pool)
            .await
            .unwrap();
    let app = groups::app(&p.pool);
    let view = groups::request(
        &app,
        "GET",
        &format!("/api/drafts/{draft}/review"),
        json!({}),
        200,
    )
    .await;
    assert_eq!(
        view["execution"]["items"][0]["waiting_reason"],
        "waiting_repository_baseline_or_preparation"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM github_repository")
            .fetch_one(&p.pool)
            .await
            .unwrap(),
        0
    );
    fs::write(p.root.path().join("targets.json"), "[]").unwrap();
    groups::request(
        &app,
        "GET",
        &format!("/api/drafts/{draft}/review"),
        json!({}),
        503,
    )
    .await;
    p.pool.close().await;
}

async fn configure_recheck_hooks(p: &mut Product) {
    use serde_json::json;
    let script = p.root.path().join("recheck-hook.py");
    let counter = p.root.path().join("recheck-hook-calls.jsonl");
    let text = r#"#!/usr/bin/python3
import json, pathlib, sys
request = json.load(sys.stdin)
with pathlib.Path(sys.argv[1]).open('a') as stream:
    stream.write(json.dumps([request['run_id'], request['event']]) + '\n')
identity = {key: request[key] for key in ('protocol_version', 'requirement_id', 'revision', 'run_id', 'resource_id', 'invocation_id', 'attempt', 'config_id')}
print(json.dumps(dict(identity, status='success', artifacts=[])), flush=True)
"#;
    fs::write(&script, text).unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    let digest = format!("sha256:{}", codexsymphony_server::validation::sha256(text));
    let hooks: Vec<_> = ["before_run", "after_run"]
        .iter()
        .map(|event| {
            json!({
                "name": format!("recheck-{event}"), "event":event, "roles":["validation"],
                "argv":[script,counter], "script_identity":digest, "timeout_seconds":10,
                "output_limit_bytes":8192, "replay":"never"
            })
        })
        .collect();
    sqlx::query("UPDATE repository SET document=jsonb_set(document,'{hooks}',$1) WHERE id=1")
        .bind(json!(hooks))
        .execute(&p.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE requirement_revision SET document=jsonb_set(document,'{repository,hooks}',$1) WHERE requirement_id=1")
        .bind(json!(hooks)).execute(&p.pool).await.unwrap();
    sqlx::query("INSERT INTO plugin_scope(plugin_id,kind,repository_ids,enabled) SELECT 'hook:'||(h->>'name'),'repositories',ARRAY[1]::bigint[],true FROM jsonb_array_elements($1::jsonb) h")
        .bind(json!(hooks)).execute(&p.pool).await.unwrap();
    p.hooks = json!({"hook_allowlist":hooks});
}
