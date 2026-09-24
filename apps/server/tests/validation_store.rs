use codexsymphony_server::{validation::*, validation_store as store};
use serde_json::{Value, json};
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};

async fn database() -> PgPool {
    let options: PgConnectOptions = std::env::var("TEST_DATABASE_URL").unwrap().parse().unwrap();
    let admin = PgPoolOptions::new()
        .connect_with(options.clone())
        .await
        .unwrap();
    let schema = format!(
        "validation_{}",
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
    sqlx::raw_sql("INSERT INTO requirement(version,state,contract,revision) VALUES(1,'Running','{}',1),(1,'Running','{}',1); INSERT INTO execution_control(id) VALUES(1) ON CONFLICT DO NOTHING;").execute(&pool).await.unwrap();
    let contract = json!({"title":"test","description":"test","acceptance_criteria":[{"description":"passes","verification_ref":"test"}],"validation_plan":[{"id":"test","check":"cargo_test","selector":"validation","expected_result":"pass","timeout_seconds":30}],"network_access":[]});
    for id in [1i64, 2] {
        sqlx::query("INSERT INTO requirement_revision VALUES($1,1,$2)")
            .bind(id)
            .bind(json!({"contract":contract,"repository":{"github_repository_id":7,"remote":"owner/repo","base_branch":"main"}}))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO agent_run(id,requirement_id,revision,incarnation,request_id,workspace,workspace_identity,launch,state,quiescent) VALUES($1,$2,1,'boot','req','/tmp','owned','{}','Succeeded',true)").bind(format!("run{id}")).bind(id).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO workspace_snapshot(run_id,manifest,candidate) VALUES($1,$2,true)")
            .bind(format!("run{id}"))
            .bind(json!({"head":"a","workspace":{"branch":format!("ai/req-{id}")}}))
            .execute(&pool)
            .await
            .unwrap();
    }
    pool
}
fn facts() -> (Candidate, TrustedIdentity, StepEvidence) {
    (
        Candidate {
            sha: "a".into(),
            tree: "tree".into(),
            immutable: true,
        },
        TrustedIdentity {
            command_sha256: "command".into(),
            config_sha256: "config".into(),
            protected_entry: "entry".into(),
            protected_entry_sha256: "digest".into(),
            tool: "tool".into(),
            tool_version: "1".into(),
        },
        StepEvidence {
            id: "test".into(),
            command: vec!["test".into()],
            exit_code: Some(0),
            output: "PASS".into(),
            output_sha256: sha256("PASS"),
            log_ref: "log".into(),
            consumer: "handoff".into(),
            code_failure: false,
        },
    )
}
async fn create(pool: &PgPool, id: &str, req: i64) -> bool {
    let (c, t, _) = facts();
    store::create(
        pool,
        id,
        req,
        1,
        &format!("run{req}"),
        &c,
        &t,
        "source",
        "entry",
    )
    .await
    .unwrap()
}
#[tokio::test]
async fn durable_exact_evidence_and_read_only_api() {
    let pool = database().await;
    let (c, t, s) = facts();
    let missing_launch = codexsymphony_server::execution::Launch {
        key: codexsymphony_server::execution::RunKey {
            run_id: "missing".into(),
            request_id: "missing".into(),
            incarnation: "boot".into(),
        },
        workspace: "/tmp".into(),
        workspace_identity: "owned".into(),
        program: "/bin/true".into(),
        args: vec![],
    };
    assert!(
        codexsymphony_server::preparation_store::begin(
            &pool,
            &missing_launch,
            999,
            1,
            "preparation",
            0
        )
        .await
        .is_err()
    );

    assert!(store::status(&pool, "missing").await.unwrap().is_none());
    let mut mutable = c.clone();
    mutable.immutable = false;
    assert!(
        !store::create(
            &pool, "mutable", 1, 1, "run1", &mutable, &t, "source", "entry"
        )
        .await
        .unwrap()
    );
    assert!(
        !store::create(&pool, "missing", 1, 1, "absent", &c, &t, "source", "entry")
            .await
            .unwrap()
    );
    assert!(create(&pool, "v1", 1).await);
    assert!(!create(&pool, "v1", 1).await);
    assert!(
        !store::record_step(&pool, "v1", &s, "succeeded")
            .await
            .unwrap()
    );
    assert!(store::begin(&pool, "v1").await.unwrap());
    assert!(sqlx::query_scalar::<_,bool>("SELECT started_at IS NOT NULL AND finished_at IS NULL FROM candidate_validation WHERE id='v1'").fetch_one(&pool).await.unwrap());
    assert!(!store::begin(&pool, "v1").await.unwrap());
    assert!(
        !store::record_step(&pool, "v1", &s, "invalid")
            .await
            .unwrap()
    );
    assert!(
        store::record_step(&pool, "v1", &s, "succeeded")
            .await
            .unwrap()
    );
    let mut extra = s.clone();
    extra.id = "z-extra".into();
    assert!(
        store::record_step(&pool, "v1", &extra, "succeeded")
            .await
            .unwrap()
    );
    let records = vec![extra, s.clone()];
    let mut altered = s.clone();
    altered.output = "replacement".into();
    assert!(
        !store::record_step(&pool, "v1", &altered, "succeeded")
            .await
            .unwrap()
    );
    assert!(
        store::finish(
            &pool,
            "v1",
            &c,
            &t,
            "source",
            "entry",
            &records,
            &["test".into()]
        )
        .await
        .unwrap()
    );
    assert!(
        sqlx::query_scalar::<_, bool>(
            "SELECT finished_at>=started_at FROM candidate_validation WHERE id='v1'"
        )
        .fetch_one(&pool)
        .await
        .unwrap()
    );
    assert!(
        !store::finish(
            &pool,
            "v1",
            &c,
            &t,
            "source",
            "entry",
            std::slice::from_ref(&s),
            &["test".into()]
        )
        .await
        .unwrap()
    );
    assert!(!store::record_step(&pool, "v1", &s, "failed").await.unwrap());
    let status = store::status(&pool, "v1").await.unwrap().unwrap();
    assert_eq!(status["stage"], "handoff");
    assert_eq!(status["result"], "succeeded");
    assert_eq!(status["steps"][0]["output_sha256"], sha256("PASS"));
    assert!(create(&pool, "v2", 2).await);
    store::begin(&pool, "v2").await.unwrap();
    assert!(
        !store::finish(
            &pool,
            "v2",
            &c,
            &t,
            "source",
            "entry",
            &[s],
            &["test".into()]
        )
        .await
        .unwrap()
    );
    assert_eq!(
        store::status(&pool, "v2").await.unwrap().unwrap()["result"],
        "blocked"
    );
    let run: String = sqlx::query_scalar("SELECT state FROM agent_run WHERE id='run2'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(run, "Succeeded");
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    let app = auth_client::router(
        pool.clone(),
        codexsymphony_server::security::RequestPolicy::new(
            "127.0.0.1:3081".parse().unwrap(),
            "https://localhost:4200".into(),
        )
        .unwrap(),
    );
    for (id, expected) in [("v1", 200), ("absent", 404)] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .header("host", "127.0.0.1:3081")
                    .header("origin", "https://localhost:4200")
                    .header("x-codexsymphony-csrf", "1")
                    .uri(format!("/api/validations/{id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
    pool.close().await;
    let response = app
        .oneshot(
            Request::builder()
                .header("host", "127.0.0.1:3081")
                .header("origin", "https://localhost:4200")
                .header("x-codexsymphony-csrf", "1")
                .uri("/api/validations/v1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 503);
}
#[tokio::test]
async fn repairs_are_atomic_and_requirement_owned() {
    let pool = database().await;
    let (c, t, mut s) = facts();
    s.exit_code = Some(1);
    s.code_failure = true;
    for req in [1, 2] {
        let id = format!("v{req}");
        assert!(create(&pool, &id, req).await);
        store::begin(&pool, &id).await.unwrap();
        store::record_step(&pool, &id, &s, "failed").await.unwrap();
        assert!(
            !store::finish(
                &pool,
                &id,
                &c,
                &t,
                "source",
                "entry",
                std::slice::from_ref(&s),
                &["test".into()]
            )
            .await
            .unwrap()
        );
    }
    assert!(
        !store::reserve_repair(&pool, 1, 1, "v1", &json!({}), FailureKind::Infrastructure)
            .await
            .unwrap()
    );
    assert!(
        !store::reserve_repair(&pool, 1, 2, "v1", &json!({}), FailureKind::Code)
            .await
            .unwrap()
    );
    assert!(
        !store::reserve_repair(&pool, 2, 1, "v1", &json!({}), FailureKind::Code)
            .await
            .unwrap()
    );
    let failure = json!({"raw_output":"PASS","step":"test"});
    let (a, b) = tokio::join!(
        store::reserve_repair(&pool, 1, 1, "v1", &failure, FailureKind::Code),
        store::reserve_repair(&pool, 1, 1, "v1", &failure, FailureKind::Code)
    );
    assert_ne!(a.unwrap(), b.unwrap());
    assert!(
        store::reserve_repair(&pool, 2, 1, "v2", &failure, FailureKind::Code)
            .await
            .unwrap()
    );
    assert_eq!(
        store::status(&pool, "v1").await.unwrap().unwrap()["stage"],
        "repair_reservation"
    );
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM repair_reservation")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 2);
    let _unused: Value = json!(n);
    repair_launch(&pool).await;
    pool.close().await;
}

async fn repair_launch(pool: &PgPool) {
    use codexsymphony_server::{
        execution::{Launch, RunKey},
        validation_repair as repair,
        workspace::Workspace,
    };
    let launch = Launch {
        key: RunKey {
            run_id: "repair1".into(),
            request_id: "repair1".into(),
            incarnation: "boot".into(),
        },
        workspace: "/tmp/repair".into(),
        workspace_identity: "repair".into(),
        program: "/bin/false".into(),
        args: vec![],
    };
    let workspace = Workspace {
        key: launch.key.clone(),
        identity: launch.workspace_identity.clone(),
        requirement: 1,
        revision: 1,
        phase: "execution".into(),
        baseline: "a".into(),
        branch: "repair".into(),
        path: launch.workspace.clone(),
    };
    assert!(!repair::bind(pool, &launch).await.unwrap());
    assert!(!repair::plan(pool, 1, &launch, &workspace).await.unwrap());
    sqlx::raw_sql("INSERT INTO repository(id,version,document) VALUES(1,1,'{\"revoked\":false}'); UPDATE requirement_revision SET document=document || '{\"repository_version\":1}'; UPDATE execution_control SET requirement_id=1,incarnation='boot',recovery_complete=true; INSERT INTO requirement_budget(requirement_id,limits) VALUES(1,'{\"tokens\":1000,\"turns\":10,\"model_seconds\":100}');").execute(pool).await.unwrap();
    assert!(!repair::plan(pool, 2, &launch, &workspace).await.unwrap());
    assert!(repair::plan(pool, 1, &launch, &workspace).await.unwrap());
    assert!(!repair::plan(pool, 1, &launch, &workspace).await.unwrap());
    assert!(!repair::bind(pool, &launch).await.unwrap());
    sqlx::query("INSERT INTO preparation_record(run_id,requirement_id,revision,launch,retry,ready,checked_at) VALUES('repair1',1,1,$1,'{}',true,extract(epoch FROM now())::bigint)").bind(json!(launch)).execute(pool).await.unwrap();
    sqlx::query("UPDATE requirement SET paused=true WHERE id=1")
        .execute(pool)
        .await
        .unwrap();
    assert!(!repair::bind(pool, &launch).await.unwrap());
    sqlx::query("UPDATE requirement SET paused=false WHERE id=1")
        .execute(pool)
        .await
        .unwrap();
    assert!(repair::bind(pool, &launch).await.unwrap());
    assert!(!repair::bind(pool, &launch).await.unwrap());
    let input = codexsymphony_server::runtime_store::input(pool, &launch.key)
        .await
        .unwrap();
    assert!(input.contains("raw_output"));
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM agent_run WHERE id='repair1'")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[path = "support/validation_runner.rs"]
mod runner_fixture;
#[tokio::test]
async fn real_validation_service_success_failure_and_recovery() {
    use codexsymphony_server::{
        validation_runner as runner,
        validation_service::{self as service, Request},
    };
    let pool = database().await;
    let (root, repo, plan) = runner_fixture::fixture();
    let c = runner::candidate(&repo).unwrap();
    let trusted = plan.identity().unwrap();
    sqlx::query(
        "UPDATE workspace_snapshot SET manifest=jsonb_set(manifest,'{head}',to_jsonb($1::text))",
    )
    .bind(&c.sha)
    .execute(&pool)
    .await
    .unwrap();
    let directory = root.join("service");
    let request = || Request {
        id: "service1",
        source_run: "run1",
        requirement: 1,
        revision: 1,
        checkout: &repo,
        directory: &directory,
        candidate: &c,
        plan: &plan,
    };
    assert!(service::validate(&pool, request()).await.unwrap());
    assert!(service::validate(&pool, request()).await.unwrap());
    let mut failure = plan.clone();
    failure.steps[0].command.push("fail".into());
    let dir = root.join("failure");
    let failed = Request {
        id: "service2",
        source_run: "run2",
        requirement: 2,
        revision: 1,
        checkout: &repo,
        directory: &dir,
        candidate: &c,
        plan: &failure,
    };
    assert!(!service::validate(&pool, failed).await.unwrap());
    assert_eq!(
        store::status(&pool, "service2").await.unwrap().unwrap()["stage"],
        "repair_reservation"
    );
    // A crash before launching has no process/output proof and must not replay.
    // Use a new candidate identity for a separately persisted recovery fixture.
    let mut next = c.clone();
    next.sha = "next".into();
    sqlx::query("UPDATE workspace_snapshot SET manifest='{\"head\":\"next\"}' WHERE run_id='run1'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        store::create(
            &pool,
            "unknown",
            1,
            1,
            "run1",
            &next,
            &trusted,
            &next.tree,
            &trusted.protected_entry_sha256
        )
        .await
        .unwrap()
    );
    assert!(store::begin(&pool, "unknown").await.unwrap());
    let dir = root.join("unknown");
    let r = Request {
        id: "unknown",
        source_run: "run1",
        requirement: 1,
        revision: 1,
        checkout: &repo,
        directory: &dir,
        candidate: &next,
        plan: &plan,
    };
    assert!(service::validate(&pool, r).await.is_err());
    assert!(!dir.exists());
    pool.close().await;
}

#[tokio::test]
async fn reconciles_durable_process_result_and_rejects_corrupt_ledger() {
    use codexsymphony_server::{
        validation_runner as runner,
        validation_service::{self as service, Request},
    };
    let pool = database().await;
    let (root, repo, plan) = runner_fixture::fixture();
    let c = runner::candidate(&repo).unwrap();
    let t = plan.identity().unwrap();
    let directory = root.join("reconcile");
    sqlx::query(
        "UPDATE workspace_snapshot SET manifest=jsonb_set(manifest,'{head}',to_jsonb($1::text))",
    )
    .bind(&c.sha)
    .execute(&pool)
    .await
    .unwrap();
    assert!(
        store::create(
            &pool,
            "crash",
            1,
            1,
            "run1",
            &c,
            &t,
            &c.tree,
            &t.protected_entry_sha256
        )
        .await
        .unwrap()
    );
    assert!(store::begin(&pool, "crash").await.unwrap());
    let evidence = runner::execute(&repo, &directory, &c, &plan).unwrap();
    store::record_step(&pool, "crash", &evidence[0], "succeeded")
        .await
        .unwrap();
    let request = || Request {
        id: "crash",
        source_run: "run1",
        requirement: 1,
        revision: 1,
        checkout: &repo,
        directory: &directory,
        candidate: &c,
        plan: &plan,
    };
    assert!(service::validate(&pool, request()).await.unwrap());
    std::fs::write(directory.join("step-0.log"), "tampered").unwrap();
    assert!(service::validate(&pool, request()).await.is_err());
    let mut changed = plan.clone();
    changed.steps[0].timeout_seconds = 2;
    let request = Request {
        id: "crash",
        source_run: "run1",
        requirement: 1,
        revision: 1,
        checkout: &repo,
        directory: &directory,
        candidate: &c,
        plan: &changed,
    };
    assert!(service::validate(&pool, request).await.is_err());
    sqlx::query(
        "UPDATE requirement_revision SET document='{\"contract\":{}}' WHERE requirement_id=2",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert!(
        store::create(
            &pool,
            "invalid-contract",
            2,
            1,
            "run2",
            &c,
            &t,
            &c.tree,
            &t.protected_entry_sha256
        )
        .await
        .is_err()
    );
    let contract = json!({"title":"x","description":"x","acceptance_criteria":[],"validation_plan":[],"network_access":[]});
    sqlx::query("UPDATE requirement_revision SET document=$1 WHERE requirement_id=2")
        .bind(json!({"contract":contract,"repository":{"github_repository_id":7,"remote":"owner/repo","base_branch":"main"}}))
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        store::create(
            &pool,
            "empty-contract",
            2,
            1,
            "run2",
            &c,
            &t,
            &c.tree,
            &t.protected_entry_sha256
        )
        .await
        .is_err()
    );
    pool.close().await;
}

#[tokio::test]
async fn worker_drives_fixed_candidate_and_one_repair_to_new_validation() {
    use codexsymphony_server::{
        execution::RunKey, git_broker::GitBroker, runtime_service, validation_repair_worker,
        validation_worker, workspace::Workspace,
    };
    use std::{fs, path::Path, process::Command};
    for repaired in [true, false] {
        let pool = database().await;
        let (root, repo, mut plan) = runner_fixture::fixture();
        fs::write(
            &plan.entry,
            "#!/bin/sh\ncat source\ntest \"$(cat source)\" = fixed\n",
        )
        .unwrap();
        plan.entry_sha256 = sha256(fs::read(&plan.entry).unwrap());
        let bundle = root.join("seed.bundle");
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(["bundle", "create"])
                .arg(&bundle)
                .arg("--all")
                .output()
                .unwrap()
                .status
                .success()
        );
        let broker = GitBroker::initialize(&root.join("broker"), &bundle).unwrap();
        let original = codexsymphony_server::validation_runner::candidate(&repo).unwrap();
        let workspace = Workspace {
            key: RunKey {
                run_id: "run1".into(),
                request_id: "req".into(),
                incarnation: "boot".into(),
            },
            identity: "owned".into(),
            requirement: 1,
            revision: 1,
            phase: "execution".into(),
            baseline: original.sha.clone(),
            branch: "ai/req-1-run1".into(),
            path: broker.path("run1").unwrap().to_string_lossy().into_owned(),
        };
        broker.prepare(&workspace, true).unwrap();
        let manifest = broker.preserve(&workspace).unwrap();
        sqlx::query("UPDATE workspace_snapshot SET manifest=$1 WHERE run_id='run1'")
            .bind(json!(manifest))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::raw_sql("UPDATE agent_run SET phase='validation' WHERE id='run1'; INSERT INTO repository(id,version,document) VALUES(1,1,'{\"revoked\":false}'); UPDATE requirement_revision SET document=document || '{\"repository_version\":1}'; UPDATE execution_control SET requirement_id=1,incarnation='boot',recovery_complete=true; INSERT INTO requirement_budget(requirement_id,limits) VALUES(1,'{\"tokens\":1000,\"turns\":10,\"model_seconds\":100}');").execute(&pool).await.unwrap();
        let adapter = root.join("preflight.py");
        fs::write(&adapter,r#"import json,sys
p=json.load(sys.stdin)
print(json.dumps({'deployment_identity':'fixture','execution_identity':'sandbox','network':{'configuration_identity':'fixture','reachable':True},'failures':[],'sample':{'cwd':p['workspace']}}))
"#).unwrap();
        let config = runtime_service::Config {
            validation: Some(plan.clone()),
            settings: codexsymphony_server::runtime_client::Settings {
                startup_seconds: 5,
                response_seconds: 5,
                stall_seconds: 5,
                reservation: codexsymphony_server::budget::Amount {
                    tokens: 100,
                    turns: 1,
                    model_seconds: 10,
                },
                codex_config: String::new(),
            },
            preparation_adapter: adapter,
            preparation: json!({"launcher":["/bin/true"],"deployment_identity":"fixture"}),
        };
        sqlx::query("UPDATE repository SET document='{\"revoked\":true}' WHERE id=1")
            .execute(&pool)
            .await
            .unwrap();
        assert!(
            !validation_worker::tick(&pool, &root, &broker, &plan)
                .await
                .unwrap()
        );
        sqlx::query("UPDATE repository SET document='{\"revoked\":false}' WHERE id=1")
            .execute(&pool)
            .await
            .unwrap();
        runtime_service::tick(
            &pool,
            &root,
            Path::new(env!("CARGO_BIN_EXE_codexsymphony-server")),
            &broker,
            "boot",
            &config,
        )
        .await
        .unwrap();
        assert!(
            !validation_worker::tick(&pool, &root, &broker, &plan)
                .await
                .unwrap()
        );
        // Pause between reservation and preparation leaves the same intent pending.
        sqlx::query("UPDATE requirement SET paused=true WHERE id=1")
            .execute(&pool)
            .await
            .unwrap();
        validation_repair_worker::tick(&pool, &root, &broker, "boot", &config)
            .await
            .unwrap();
        sqlx::query("UPDATE requirement SET paused=false WHERE id=1")
            .execute(&pool)
            .await
            .unwrap();
        validation_repair_worker::tick(&pool, &root, &broker, "boot", &config)
            .await
            .unwrap();
        let (run, identity): (String, Value) = sqlx::query_as(
            "SELECT repair_run_id,workspace FROM repair_reservation WHERE requirement_id=1",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let repair: Workspace = serde_json::from_value(identity).unwrap();
        assert_eq!(run, repair.key.run_id);
        sqlx::query("UPDATE repair_reservation SET status='reserved' WHERE requirement_id=1")
            .execute(&pool)
            .await
            .unwrap();
        assert!(
            validation_repair_worker::tick(&pool, &root, &broker, "other-boot", &config)
                .await
                .is_err()
        );
        validation_repair_worker::tick(&pool, &root, &broker, "boot", &config)
            .await
            .unwrap();
        sqlx::query("UPDATE repair_reservation SET status='started' WHERE requirement_id=1")
            .execute(&pool)
            .await
            .unwrap();
        validation_repair_worker::tick(&pool, &root, &broker, "boot", &config)
            .await
            .unwrap();
        // Stand in for the Agent only at the coding boundary: real Broker commit,
        // preservation and a fresh validation run follow, without an external model.
        fs::write(
            Path::new(&repair.path).join("source"),
            if repaired { "fixed" } else { "still failing" },
        )
        .unwrap();
        broker.commit(&repair, "repair candidate").unwrap();
        let snapshot = broker.preserve(&repair).unwrap();
        sqlx::query("INSERT INTO workspace_snapshot VALUES($1,$2,true)")
            .bind(&run)
            .bind(json!(snapshot))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "UPDATE agent_run SET state='Succeeded',quiescent=true,phase='validation' WHERE id=$1",
        )
        .bind(&run)
        .execute(&pool)
        .await
        .unwrap();
        assert!(
            validation_worker::tick(&pool, &root, &broker, &plan)
                .await
                .unwrap()
        );
        let state: String =
            sqlx::query_scalar("SELECT status FROM repair_reservation WHERE requirement_id=1")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(state, if repaired { "succeeded" } else { "failed" });
        validation_repair_worker::tick(&pool, &root, &broker, "boot", &config)
            .await
            .unwrap();
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM agent_run WHERE requirement_id=1")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(count, 2);
        pool.close().await;
    }
}

#[tokio::test]
async fn infrastructure_retry_preserves_failed_candidate_and_never_reserves_code() {
    use codexsymphony_server::{
        execution::RunKey, git_broker::GitBroker, validation_worker, workspace::Workspace,
    };
    for (restored, prior_repair) in [(true, false), (false, false), (true, true)] {
        let pool = database().await;
        let (root, repo, mut plan) = runner_fixture::fixture();
        let ready = root.join("service-ready");
        std::fs::write(&plan.entry,"#!/bin/sh\nif test -f \"$1\"; then echo service-restored; exit 0; fi\necho 'connection refused: disposable database'; exit 1\n").unwrap();
        plan.entry_sha256 = sha256(std::fs::read(&plan.entry).unwrap());
        plan.steps[0].command.push(ready.to_string_lossy().into());
        let bundle = root.join("source.bundle");
        assert!(
            std::process::Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(["bundle", "create"])
                .arg(&bundle)
                .arg("--all")
                .output()
                .unwrap()
                .status
                .success()
        );
        let broker = GitBroker::initialize(&root.join("broker"), &bundle).unwrap();
        let candidate = codexsymphony_server::validation_runner::candidate(&repo).unwrap();
        let workspace = Workspace {
            key: RunKey {
                run_id: "run1".into(),
                request_id: "req".into(),
                incarnation: "boot".into(),
            },
            identity: "owned".into(),
            requirement: 1,
            revision: 1,
            phase: "execution".into(),
            baseline: candidate.sha,
            branch: "ai/req-1-run1".into(),
            path: broker.path("run1").unwrap().to_string_lossy().into(),
        };
        broker.prepare(&workspace, true).unwrap();
        let manifest = broker.preserve(&workspace).unwrap();
        sqlx::query("UPDATE workspace_snapshot SET manifest=$1 WHERE run_id='run1'")
            .bind(json!(manifest))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::raw_sql("INSERT INTO repository(id,version,document) VALUES(1,1,'{\"revoked\":false}'); UPDATE requirement_revision SET document=document||'{\"repository_version\":1}'; UPDATE execution_control SET requirement_id=1,incarnation='boot',recovery_complete=true; UPDATE agent_run SET phase='validation' WHERE id='run1'; INSERT INTO repair_authorization VALUES(1,3,'bounded_v1');").execute(&pool).await.unwrap();
        if prior_repair {
            preceding_repair(&pool).await;
        }
        assert!(
            validation_worker::tick(&pool, &root, &broker, &plan)
                .await
                .unwrap()
        );
        assert_eq!(
            store::status(&pool, "validation-run1")
                .await
                .unwrap()
                .unwrap()["result"],
            "gate_failed"
        );
        if restored {
            std::fs::write(&ready, "ready").unwrap();
        }
        sqlx::query("UPDATE recovery_retry SET next_attempt_at=0")
            .execute(&pool)
            .await
            .unwrap();
        assert!(
            validation_worker::tick(&pool, &root, &broker, &plan)
                .await
                .unwrap()
        );
        if restored {
            // Lost completion after validation committed: reconciliation must
            // reuse the persisted attempt and binding, never rerun the command.
            sqlx::query("UPDATE recovery_retry SET state='unknown',next_attempt_at=0")
                .execute(&pool)
                .await
                .unwrap();
            assert!(
                validation_worker::tick(&pool, &root, &broker, &plan)
                    .await
                    .unwrap()
            );
            let attempts: i32 = sqlx::query_scalar("SELECT attempts FROM recovery_retry")
                .fetch_one(&pool)
                .await
                .unwrap();
            assert_eq!(attempts, 1);
        } else {
            let previous: Value = sqlx::query_scalar("SELECT remote FROM recovery_retry")
                .fetch_one(&pool)
                .await
                .unwrap();
            sqlx::query("UPDATE recovery_retry SET state='unknown',next_attempt_at=0,remote='{\"validation\":\"missing-retry-receipt\"}'").execute(&pool).await.unwrap();
            assert!(
                codexsymphony_server::recovery_retry::local(&pool, &root, &broker, &plan)
                    .await
                    .unwrap()
            );
            let pending: (String, i32) =
                sqlx::query_as("SELECT state,attempts FROM recovery_retry")
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            assert_eq!(pending, ("unknown".into(), 1));
            sqlx::query("UPDATE recovery_retry SET state='pending',remote=$1")
                .bind(previous)
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query("UPDATE recovery_retry SET next_attempt_at=0")
                .execute(&pool)
                .await
                .unwrap();
            assert!(
                validation_worker::tick(&pool, &root, &broker, &plan)
                    .await
                    .unwrap()
            );
        }
        let state: String = sqlx::query_scalar("SELECT state FROM recovery_retry")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(state, if restored { "complete" } else { "blocked" });
        assert_eq!(
            store::status(&pool, "validation-run1")
                .await
                .unwrap()
                .unwrap()["result"],
            "gate_failed"
        );
        let repairs: i64 = sqlx::query_scalar("SELECT count(*) FROM repair_reservation")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(repairs, i64::from(prior_repair));
        if prior_repair {
            let status: String = sqlx::query_scalar("SELECT status FROM repair_reservation")
                .fetch_one(&pool)
                .await
                .unwrap();
            let decision: String = sqlx::query_scalar(
                "SELECT decision FROM recovery_failure WHERE event_key='preceding-code'",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(status, "succeeded");
            assert_eq!(decision, "repaired");
        }
        let original: String = sqlx::query_scalar(
            "SELECT output FROM validation_step WHERE validation_id='validation-run1'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(original.contains("connection refused"));
        pool.close().await;
    }
}

#[path = "support/auth.rs"]
mod auth_client;

async fn preceding_repair(pool: &PgPool) {
    // Seed the already authorized coding boundary; the successor validation and
    // infrastructure fault/reconciliation execute through the real Git runner.
    sqlx::raw_sql("INSERT INTO agent_run(id,requirement_id,revision,incarnation,request_id,workspace,workspace_identity,launch,state,quiescent) VALUES('preceding',1,1,'boot','preceding','/tmp','preceding','{}','Succeeded',true); INSERT INTO candidate_validation(id,requirement_id,revision,source_run_id,candidate_sha,candidate_tree,trusted,required_steps,source_before,source_after,entry_before,entry_after,stage,result) VALUES('preceding-v',1,1,'preceding','preceding-sha','tree','{}','[]','tree','tree','entry','entry','validation','gate_failed'); INSERT INTO recovery_failure(event_key,requirement_id,source_validation_id,phase,facts,fingerprint,decision,reason) VALUES('preceding-code',1,'preceding-v','local','{\"candidate_sha\":\"preceding-sha\",\"raw\":\"error[E0308]: original compiler diagnostic\",\"log_ref\":\"preceding.log\"}','preceding-fingerprint','reserved','authorized original code failure'); INSERT INTO repair_reservation(requirement_id,ordinal,source_validation_id,failure,status,repair_run_id,event_key) VALUES(1,1,'preceding-v','{}','started','run1','preceding-code');").execute(pool).await.unwrap();
}
