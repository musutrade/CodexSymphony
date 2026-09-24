use codexsymphony_server::{
    delivery::{Identity, PrFact, pr_fact},
    delivery_control as control,
    delivery_store::{self as store, Pending},
    delivery_worker::{self as worker, Remote},
    github_http::Error,
    run_store,
};
use serde_json::{Value, json};
use sqlx::PgPool;

// Separate schemas still share PostgreSQL advisory locks. Serialize independent
// scenarios; concurrency within each recovery/remote-race scenario is unchanged.
static DATABASE_TEST: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[path = "support/delivery.rs"]
mod database_fixture;
use database_fixture::database;
async fn job(pool: &PgPool) -> Pending {
    store::due(pool, i64::MAX).await.unwrap().remove(0)
}
async fn state(pool: &PgPool) -> (String, Option<i64>, bool) {
    sqlx::query_as("SELECT r.state,c.requirement_id,r.cleanup_complete FROM requirement r CROSS JOIN execution_control c WHERE r.id=1").fetch_one(pool).await.unwrap()
}
async fn tick(pool: &PgPool, remote: &mut Fake, now: i64) {
    worker::tick(pool, &std::env::temp_dir(), remote, now)
        .await
        .unwrap();
    control::settle(pool).await.unwrap();
}
fn pr(identity: &Identity) -> Value {
    json!({"number":12,"body":identity.marker(),"base":{"repo":{"id":7,"full_name":"owner/repo"},"ref":"main"},"head":{"repo":{"id":7},"ref":identity.branch,"sha":identity.head},"merged":false,"merged_at":null,"state":"open","merge_commit_sha":"test-merge"})
}
#[derive(Default)]
struct Fake {
    head: Option<String>,
    pr: Option<Value>,
    calls: Vec<&'static str>,
    lost: bool,
    conflict: bool,
    fail_close: bool,
    fail_find: bool,
    fail_head: bool,
    cancel_during_create: Option<PgPool>,
}
fn lost() -> Error {
    Error {
        code: "github_transient_or_unknown",
        status: None,

        retry_after_seconds: None,
    }
}
impl Remote for Fake {
    async fn find(&mut self, _: &Pending) -> Result<Option<Value>, Error> {
        self.calls.push("find");
        if self.fail_find {
            return Err(lost());
        }
        Ok(self.pr.clone())
    }
    async fn head(&mut self, _: &Pending) -> Result<Option<String>, Error> {
        self.calls.push("head");
        if self.fail_head {
            return Err(lost());
        }
        Ok(self.head.clone())
    }
    async fn push(&mut self, job: &Pending) -> Result<Value, Error> {
        self.calls.push("push");
        if self.conflict {
            return Err(lost());
        }
        self.head = Some(job.head_sha.clone());
        if self.lost {
            return Err(lost());
        }
        Ok(json!({"push":"accepted"}))
    }
    async fn create(&mut self, job: &Pending) -> Result<Value, Error> {
        self.calls.push("create");
        self.pr = Some(pr(&job.identity()));
        if let Some(pool) = &self.cancel_during_create {
            control::cancel(pool, 1).await.unwrap();
        }
        if self.lost {
            return Err(lost());
        }
        Ok(self.pr.clone().unwrap())
    }
    async fn close(&mut self, _: &Pending, _: u64) -> Result<Value, Error> {
        self.calls.push("close");
        if self.fail_close {
            return Err(lost());
        }
        self.pr.as_mut().unwrap()["state"] = json!("closed");
        if self.lost {
            return Err(lost());
        }
        Ok(self.pr.clone().unwrap())
    }
}
#[test]
fn exact_identity_and_merge_facts() {
    let identity = Identity {
        requirement: 1,
        revision: 1,
        repository_id: 7,
        repository: "owner/repo".into(),
        branch: "ai/req-1".into(),
        base_branch: "main".into(),
        head: "candidate".into(),
    };
    let mut value = pr(&identity);
    assert_eq!(pr_fact(&identity, &value), PrFact::Open);
    value["state"] = json!("closed");
    assert_eq!(pr_fact(&identity, &value), PrFact::Closed);
    value["merged"] = json!(true);
    assert_eq!(pr_fact(&identity, &value), PrFact::Merged);
    value["merged"] = Value::Null;
    assert_eq!(pr_fact(&identity, &value), PrFact::Unknown);
    for path in ["head", "base", "body", "number"] {
        let mut changed = pr(&identity);
        changed[path] = Value::Null;
        assert_eq!(pr_fact(&identity, &changed), PrFact::Conflict);
    }
    let mut changed = identity.clone();
    changed.revision += 1;
    assert_ne!(changed.action_key(), identity.action_key());
    changed = identity.clone();
    changed.head.push('b');
    assert_ne!(changed.action_key(), identity.action_key());
}
#[tokio::test]
async fn lost_push_and_create_responses_reconcile_without_duplicate_work() {
    let _serial = DATABASE_TEST.lock().await;
    let pool = database().await;
    let mut remote = Fake {
        lost: true,
        ..Default::default()
    };
    tick(&pool, &mut remote, 0).await;
    tick(&pool, &mut remote, 60).await;
    tick(&pool, &mut remote, 300).await;
    tick(&pool, &mut remote, 600).await;
    assert_eq!(remote.calls.iter().filter(|&&c| c == "push").count(), 1);
    assert_eq!(remote.calls.iter().filter(|&&c| c == "create").count(), 1);
    assert_eq!(state(&pool).await, ("Submitted".into(), Some(1), false));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM agent_run")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    let consumer: String = sqlx::query_scalar("SELECT consumer FROM validation_step")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(consumer, "pr:7:12:candidate");
    // Closed without merge and a test merge SHA keep the queue occupied.
    assert!(
        !sqlx::query_scalar::<_, bool>("SELECT released FROM delivery")
            .fetch_one(&pool)
            .await
            .unwrap()
    );
}
#[tokio::test]
async fn crashes_before_send_after_remote_and_during_commit() {
    let _serial = DATABASE_TEST.lock().await;
    let pool = database().await;
    let saved = job(&pool).await;
    let attempt = store::begin(&pool, &saved, "push", 0)
        .await
        .unwrap()
        .unwrap();
    let mut remote = Fake::default();
    tick(&pool, &mut remote, 60).await; // crash before the first actual send
    assert_eq!(remote.calls.iter().filter(|&&c| c == "push").count(), 1);
    store::receipt(&pool, attempt, &json!({"late":true}))
        .await
        .unwrap();
    assert!(
        sqlx::query_scalar::<_, bool>(
            "SELECT finished_at>=created_at FROM delivery_attempt WHERE id=$1"
        )
        .bind(attempt)
        .fetch_one(&pool)
        .await
        .unwrap()
    );
    // Simulate crash after PR acceptance, then a deferred DB commit failure.
    remote.pr = Some(pr(&saved.identity()));
    sqlx::raw_sql("CREATE FUNCTION fail_delivery_commit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected commit failure'; END $$; CREATE CONSTRAINT TRIGGER fail_delivery_commit AFTER INSERT ON delivery_observation DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION fail_delivery_commit();").execute(&pool).await.unwrap();
    assert!(
        worker::tick(&pool, &std::env::temp_dir(), &mut remote, 120)
            .await
            .is_err()
    );
    assert_eq!(state(&pool).await.0, "Running");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM github_pr")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    sqlx::query("DROP TRIGGER fail_delivery_commit ON delivery_observation")
        .execute(&pool)
        .await
        .unwrap();
    tick(&pool, &mut remote, 240).await;
    assert_eq!(state(&pool).await.0, "Submitted");
    assert!(!remote.calls.contains(&"create"));
}
#[tokio::test]
async fn cancel_in_flight_closes_once_and_merge_races_preserve_facts() {
    let _serial = DATABASE_TEST.lock().await;
    let pool = database().await;
    let mut remote = Fake {
        head: Some("candidate".into()),
        lost: true,
        cancel_during_create: Some(pool.clone()),
        ..Default::default()
    };
    tick(&pool, &mut remote, 0).await;
    assert_eq!(state(&pool).await, ("Cancelled".into(), Some(1), false));
    tick(&pool, &mut remote, 60).await;
    control::cancel(&pool, 1).await.unwrap();
    tick(&pool, &mut remote, 120).await;
    tick(&pool, &mut remote, 300).await;
    assert_eq!(state(&pool).await, ("Cancelled".into(), None, true));
    assert_eq!(remote.calls.iter().filter(|&&c| c == "close").count(), 1);
    assert_eq!(remote.head, Some("candidate".into()));
    let pool = database().await;
    let saved = job(&pool).await;
    let mut value = pr(&saved.identity());
    value["merged"] = json!(true);
    let mut remote = Fake {
        pr: Some(value),
        ..Default::default()
    };
    control::cancel(&pool, 1).await.unwrap();
    tick(&pool, &mut remote, 0).await;
    assert_eq!(state(&pool).await, ("Cancelled".into(), None, true));
    assert!(!remote.calls.contains(&"close"));
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT released FROM delivery")
            .fetch_one(&pool)
            .await
            .unwrap()
    );
}
#[tokio::test]
async fn pause_authorization_conflicts_and_bounded_cleanup() {
    let _serial = DATABASE_TEST.lock().await;
    let pool = database().await;
    run_store::pause(&pool, Some(1)).await.unwrap();
    let mut remote = Fake::default();
    tick(&pool, &mut remote, 0).await;
    assert!(!remote.calls.contains(&"push"));
    assert_eq!(state(&pool).await.1, Some(1));
    assert!(control::resume(&pool, Some(1)).await.unwrap());
    sqlx::query("UPDATE repository SET document=jsonb_set(document,'{revoked}','true')")
        .execute(&pool)
        .await
        .unwrap();
    tick(&pool, &mut remote, 60).await;
    assert!(!remote.calls.contains(&"push"));
    sqlx::query("UPDATE repository SET document=jsonb_set(document,'{revoked}','false')")
        .execute(&pool)
        .await
        .unwrap();
    remote.head = Some("someone-else".into());
    tick(&pool, &mut remote, 120).await;
    assert_eq!(job(&pool).await.state, "blocked");
    assert!(!remote.calls.contains(&"push"));
    let saved = job(&pool).await;
    remote.pr = Some(pr(&saved.identity()));
    tick(&pool, &mut remote, 300).await;
    control::cancel(&pool, 1).await.unwrap();
    remote.fail_close = true;
    for now in [400, 600, 800, 1000, 1200] {
        tick(&pool, &mut remote, now).await;
    }
    assert_eq!(remote.calls.iter().filter(|&&c| c == "close").count(), 3);
    assert_eq!(state(&pool).await.1, Some(1));
    assert_eq!(job(&pool).await.state, "blocked");
    remote.pr.as_mut().unwrap()["head"]["sha"] = json!("other");
    tick(&pool, &mut remote, 1400).await;
    assert_eq!(state(&pool).await.1, Some(1));
}

#[tokio::test]
async fn cancelled_unknown_create_and_push_keep_ownership_until_reconciled() {
    let _serial = DATABASE_TEST.lock().await;
    let pool = database().await;
    let saved = job(&pool).await;
    store::begin(&pool, &saved, "create", 0)
        .await
        .unwrap()
        .unwrap();
    control::cancel(&pool, 1).await.unwrap();
    let mut remote = Fake::default();
    tick(&pool, &mut remote, 60).await;
    assert_eq!(state(&pool).await.1, Some(1));
    assert!(!remote.calls.contains(&"create"));
    remote.pr = Some(pr(&saved.identity()));
    tick(&pool, &mut remote, 120).await;
    tick(&pool, &mut remote, 180).await;
    tick(&pool, &mut remote, 240).await;
    assert_eq!(state(&pool).await.1, None);
    let pool = database().await;
    let saved = job(&pool).await;
    store::begin(&pool, &saved, "push", 0)
        .await
        .unwrap()
        .unwrap();
    control::cancel(&pool, 1).await.unwrap();
    let mut remote = Fake::default();
    tick(&pool, &mut remote, 60).await;
    assert_eq!(state(&pool).await.1, Some(1));
    remote.head = Some(saved.head_sha);
    tick(&pool, &mut remote, 120).await;
    assert_eq!(state(&pool).await.1, None);
    assert!(!remote.calls.contains(&"push"));
}
#[tokio::test]
async fn transaction_rollback_does_not_lose_outbox_and_wrong_pr_is_never_closed() {
    let _serial = DATABASE_TEST.lock().await;
    let pool = database().await;
    sqlx::query("DELETE FROM delivery_action")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM delivery")
        .execute(&pool)
        .await
        .unwrap();
    let mut tx = pool.begin().await.unwrap();
    store::enqueue(&mut tx, "validation").await.unwrap();
    tx.rollback().await.unwrap();
    assert!(store::due(&pool, 0).await.unwrap().is_empty());
    let mut tx = pool.begin().await.unwrap();
    store::enqueue(&mut tx, "validation").await.unwrap();
    store::enqueue(&mut tx, "validation").await.unwrap();
    tx.commit().await.unwrap();
    let saved = job(&pool).await;
    let mut value = pr(&saved.identity());
    value["body"] = json!("belongs to someone else");
    let mut remote = Fake {
        pr: Some(value),
        ..Default::default()
    };
    store::begin(&pool, &saved, "create", 0).await.unwrap();
    control::cancel(&pool, 1).await.unwrap();
    tick(&pool, &mut remote, 60).await;
    assert!(!remote.calls.contains(&"close"));
    assert_eq!(state(&pool).await.1, Some(1));
    assert!(!control::resume(&pool, Some(1)).await.unwrap());
    assert!(!control::cancel(&pool, 999).await.unwrap());
}

#[path = "support/validation_runner.rs"]
mod source_fixture;
#[tokio::test]
async fn initial_ready_plan_is_durable_and_admission_binds_the_same_worktree() {
    let _serial = DATABASE_TEST.lock().await;
    use codexsymphony_server::{git_broker::GitBroker, runtime_initial};
    let pool = database().await;
    sqlx::raw_sql(
        "UPDATE requirement SET state='Ready'; UPDATE execution_control SET requirement_id=NULL;",
    )
    .execute(&pool)
    .await
    .unwrap();
    let (root, repo, _) = source_fixture::fixture();
    let baseline = codexsymphony_server::validation_runner::candidate(&repo)
        .unwrap()
        .sha;
    let bundle = root.join("seed.bundle");
    assert!(
        std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["bundle", "create"])
            .arg(&bundle)
            .arg("--all")
            .status()
            .unwrap()
            .success()
    );
    let broker = GitBroker::initialize(&root.join("workspaces"), &bundle).unwrap();
    let launcher = vec!["/usr/bin/codex".into()];
    // The worker may have selected a repository before the user withdrew its
    // queue head. A different head must not inherit the selected configuration.
    assert!(
        runtime_initial::plan_selected(
            &pool,
            &broker,
            "boot",
            &launcher,
            &baseline,
            Some((999, 1))
        )
        .await
        .unwrap()
        .is_none()
    );
    assert!(
        runtime_initial::plan_selected(
            &pool,
            &broker,
            "boot",
            &launcher,
            &baseline,
            Some((1, 999))
        )
        .await
        .unwrap()
        .is_none()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM initial_run")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    let (launch, workspace) = runtime_initial::plan(&pool, &broker, "boot", &launcher, &baseline)
        .await
        .unwrap()
        .unwrap();
    let saved = runtime_initial::plan(&pool, &broker, "boot", &launcher, &baseline)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(launch.key, saved.0.key);
    let mut config: codexsymphony_server::runtime_service::Config=serde_json::from_value(json!({"validation":null,"settings":{"startup_seconds":10,"response_seconds":10,"stall_seconds":10,"reservation":{"tokens":100,"turns":1,"model_seconds":10},"codex_config":""},"preparation_adapter":"/missing-adapter","preparation":{"launcher":launcher,"baseline":baseline}})).unwrap();
    let route_file = root.join("routes.json");
    std::fs::write(&route_file,json!({"validation":null,"settings":{"startup_seconds":10,"response_seconds":10,"stall_seconds":10,"reservation":{"tokens":100,"turns":1,"model_seconds":10},"codex_config":""},"preparation_adapter":"/missing-adapter","preparation":{"launcher":launcher,"baseline":baseline}}).to_string()).unwrap();
    let routes = codexsymphony_server::runtime_routes::Deployment::load(&route_file).unwrap();
    assert!(
        codexsymphony_server::runtime_service::tick_routes(
            &pool,
            &root,
            std::path::Path::new(env!("CARGO_BIN_EXE_codexsymphony-server")),
            &broker,
            "boot",
            &routes
        )
        .await
        .is_err()
    );
    // Invalid deployment config creates no probe and cannot bypass admission.
    assert!(
        runtime_initial::tick(&pool, &root, &broker, "boot", &config)
            .await
            .is_err()
    );
    assert!(!run_store::reserve_prepared(&pool, &launch).await.unwrap());
    config.preparation["deployment_identity"] = json!("deployment");
    runtime_initial::tick(&pool, &root, &broker, "boot", &config)
        .await
        .unwrap();
    assert!(!run_store::reserve_prepared(&pool, &launch).await.unwrap());
    sqlx::query("INSERT INTO preparation_record(run_id,requirement_id,revision,launch,retry,ready,checked_at) VALUES($1,1,1,$2,'{}',true,extract(epoch FROM now())::bigint) ON CONFLICT(run_id) DO UPDATE SET ready=true,retry='{}',checked_at=EXCLUDED.checked_at").bind(&launch.key.run_id).bind(json!(launch)).execute(&pool).await.unwrap();
    run_store::pause(&pool, Some(1)).await.unwrap();
    assert!(
        runtime_initial::plan(&pool, &broker, "boot", &launcher, &baseline)
            .await
            .unwrap()
            .is_none()
    );
    assert!(control::resume(&pool, Some(1)).await.unwrap());
    runtime_initial::tick(&pool, &root, &broker, "boot", &config)
        .await
        .unwrap();
    assert!(!run_store::reserve_prepared(&pool, &launch).await.unwrap());
    let bound: Value = sqlx::query_scalar("SELECT identity FROM run_workspace WHERE run_id=$1")
        .bind(&launch.key.run_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(bound, json!(workspace));
    assert!(
        runtime_initial::plan(&pool, &broker, "boot", &launcher, &baseline)
            .await
            .unwrap()
            .is_none()
    );
    // Resume an explicit coding pause from preserved work, without a question.
    run_store::pause(&pool, Some(1)).await.unwrap();
    sqlx::query("UPDATE agent_run SET quiescent=true,state='Interrupted' WHERE id=$1")
        .bind(&launch.key.run_id)
        .execute(&pool)
        .await
        .unwrap();
    let manifest = broker.preserve(&workspace).unwrap();
    sqlx::query("INSERT INTO workspace_snapshot(run_id,manifest,candidate) VALUES($1,$2,false)")
        .bind(&launch.key.run_id)
        .bind(json!(manifest))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO requirement_budget(requirement_id,limits) VALUES(1,'{\"tokens\":1000,\"turns\":10,\"model_seconds\":1000}')").execute(&pool).await.unwrap();
    assert!(
        codexsymphony_server::runtime_resume::next(&pool, &broker, "boot", &launcher)
            .await
            .unwrap()
            .is_none()
    );
    assert!(control::resume(&pool, Some(1)).await.unwrap());
    let restored = codexsymphony_server::runtime_resume::next(&pool, &broker, "boot", &launcher)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(restored.launch.key.run_id, launch.key.run_id);
    assert_eq!(broker.head(&restored.workspace).unwrap(), baseline);
    assert_eq!(state(&pool).await.1, Some(1));
    config.preparation = json!({});
    runtime_initial::tick(&pool, &root, &broker, "boot", &config)
        .await
        .unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn fresh_exact_merge_releases_without_done_and_api_exposes_delivery() {
    let _serial = DATABASE_TEST.lock().await;
    use tower::ServiceExt;
    let pool = database().await;
    let saved = job(&pool).await;
    let mut value = pr(&saved.identity());
    value["state"] = json!("closed");
    store::confirmed(&pool, &saved, &value).await.unwrap();
    control::settle(&pool).await.unwrap();
    assert_eq!(state(&pool).await.1, Some(1));
    let observation =
        json!({"merge":"Merged","head":"candidate","head_ref":"ai/req-1","base_ref":"main"});
    sqlx::query("UPDATE github_pr SET observation=$1,stale=true,last_synced_at=extract(epoch FROM now())::bigint").bind(&observation).execute(&pool).await.unwrap();
    control::settle(&pool).await.unwrap();
    assert_eq!(state(&pool).await.1, Some(1));
    run_store::pause(&pool, None).await.unwrap();
    sqlx::query("UPDATE github_pr SET stale=false")
        .execute(&pool)
        .await
        .unwrap();
    control::settle(&pool).await.unwrap();
    assert_eq!(state(&pool).await.1, Some(1));
    assert!(control::resume(&pool, None).await.unwrap());
    control::settle(&pool).await.unwrap();
    assert_eq!(state(&pool).await, ("Submitted".into(), None, false));
    let app = auth_client::router(
        pool.clone(),
        codexsymphony_server::security::RequestPolicy::new(
            "127.0.0.1:3081".parse().unwrap(),
            "https://localhost:4200".into(),
        )
        .unwrap(),
    );
    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .header("host", "127.0.0.1:3081")
                .header("origin", "https://localhost:4200")
                .header("x-codexsymphony-csrf", "1")
                .uri("/api/requirements/1/delivery")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["deliveries"][0]["validation_id"], "validation");
    assert_eq!(body["deliveries"][0]["pr_number"], 12);
    assert_eq!(body["deliveries"][0]["merged"], true);
    let response = app
        .oneshot(
            axum::http::Request::builder()
                .header("host", "127.0.0.1:3081")
                .header("origin", "https://localhost:4200")
                .header("x-codexsymphony-csrf", "1")
                .method("POST")
                .uri("/api/requirements/1/cancel")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let before: i64 = sqlx::query_scalar("SELECT version FROM requirement WHERE id=1")
        .fetch_one(&pool)
        .await
        .unwrap();
    control::cancel(&pool, 1).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT version FROM requirement WHERE id=1")
            .fetch_one(&pool)
            .await
            .unwrap(),
        before
    );
    control::settle(&pool).await.unwrap();
    assert_eq!(state(&pool).await, ("Cancelled".into(), None, true));
}

#[tokio::test]
async fn remote_read_failures_and_disappeared_pr_never_trigger_writes() {
    let _serial = DATABASE_TEST.lock().await;
    for failure in ["find", "head", "missing", "close"] {
        let pool = database().await;
        let mut remote = Fake {
            fail_find: failure == "find",
            fail_head: failure == "head",
            ..Default::default()
        };
        if failure == "missing" {
            sqlx::query("UPDATE delivery SET pr_number=12")
                .execute(&pool)
                .await
                .unwrap();
        }
        if failure == "close" {
            sqlx::query("UPDATE delivery_action SET kind='close'")
                .execute(&pool)
                .await
                .unwrap();
        }
        tick(&pool, &mut remote, 0).await;
        assert!(
            remote
                .calls
                .iter()
                .all(|call| matches!(*call, "find" | "head"))
        );
        let pending = job(&pool).await;
        assert_eq!(pending.attempts, 0);
        assert_eq!(state(&pool).await.1, Some(1));
        pool.close().await;
    }
}

#[tokio::test]
async fn missing_delivery_identity_rolls_back_and_cancel_reports_absent_requirement() {
    let _serial = DATABASE_TEST.lock().await;
    use tower::ServiceExt;
    let pool = database().await;
    sqlx::query(
        "UPDATE requirement_revision SET document=jsonb_set(document,'{repository,remote}','null')",
    )
    .execute(&pool)
    .await
    .unwrap();
    let mut tx = pool.begin().await.unwrap();
    assert!(
        store::enqueue(&mut tx, "validation")
            .await
            .unwrap_err()
            .to_string()
            .contains("delivery identity missing")
    );
    tx.rollback().await.unwrap();
    assert_eq!(store::due(&pool, i64::MAX).await.unwrap().len(), 1);
    let app = auth_client::router(
        pool.clone(),
        codexsymphony_server::security::RequestPolicy::new(
            "127.0.0.1:3081".parse().unwrap(),
            "https://localhost:4200".into(),
        )
        .unwrap(),
    );
    let response = app
        .oneshot(
            axum::http::Request::builder()
                .header("host", "127.0.0.1:3081")
                .header("origin", "https://localhost:4200")
                .header("x-codexsymphony-csrf", "1")
                .method("POST")
                .uri("/api/requirements/999/cancel")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn operator_retry_preserves_attempts_and_resumes_after_push() {
    use codexsymphony_server::operator_control::{self, Action, Command};
    let _serial = DATABASE_TEST.lock().await;
    let pool = database().await;
    let mut remote = Fake {
        conflict: true,
        ..Default::default()
    };
    for now in [0, 200] {
        tick(&pool, &mut remote, now).await;
    }
    remote.conflict = false;
    tick(&pool, &mut remote, 400).await;
    tick(&pool, &mut remote, 600).await;
    assert_eq!(job(&pool).await.state, "blocked");
    assert!(!remote.calls.contains(&"create"));
    let command = Command {
        version: 1,
        request_id: "retry-delivery".into(),
        action: Action::DeliveryRecheck,
    };
    sqlx::query("UPDATE delivery_action SET error='{}'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(operator_control::execute(&pool, 1, &command).await.is_err());
    sqlx::query("UPDATE delivery_action SET error='{\"code\":\"delivery_retry_exhausted\"}'")
        .execute(&pool)
        .await
        .unwrap();
    let receipt = operator_control::execute(&pool, 1, &command).await.unwrap();
    assert_eq!(
        receipt,
        operator_control::execute(&pool, 1, &command).await.unwrap()
    );
    let ledger: (i32,i32,i64) = sqlx::query_as("SELECT attempts,attempt_limit,(SELECT count(*) FROM delivery_attempt) FROM delivery_action").fetch_one(&pool).await.unwrap();
    assert_eq!(ledger, (3, 6, 3));
    let stale = Command {
        version: 1,
        request_id: "stale-retry".into(),
        action: Action::DeliveryRecheck,
    };
    assert!(operator_control::execute(&pool, 1, &stale).await.is_err());
    tick(&pool, &mut remote, 800).await;
    tick(&pool, &mut remote, 1000).await;
    assert_eq!(
        remote.calls.iter().filter(|&&call| call == "push").count(),
        3
    );
    assert_eq!(
        remote
            .calls
            .iter()
            .filter(|&&call| call == "create")
            .count(),
        1
    );
    assert_eq!(state(&pool).await.0, "Submitted");
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM operator_intervention WHERE reason='delivery_recheck'"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        1
    );
}

#[path = "support/auth.rs"]
mod auth_client;

async fn repair_delivery(pool: &PgPool) {
    sqlx::raw_sql("INSERT INTO agent_run(id,requirement_id,revision,incarnation,request_id,workspace,workspace_identity,launch,state,quiescent) VALUES('repair',1,1,'boot','repair','/tmp','repair','{}','Succeeded',true); INSERT INTO workspace_snapshot(run_id,manifest,candidate) VALUES('repair','{\"head\":\"repaired\",\"workspace\":{\"branch\":\"repair-worktree\",\"baseline\":\"candidate\"}}',true); INSERT INTO repair_reservation(requirement_id,ordinal,source_validation_id,failure,status,repair_run_id,event_key) VALUES(1,1,'validation','{}','succeeded','repair','ci-failure'); INSERT INTO candidate_validation(id,requirement_id,revision,source_run_id,candidate_sha,candidate_tree,trusted,required_steps,source_before,source_after,entry_before,entry_after,stage,result) VALUES('repaired-validation',1,1,'repair','repaired','tree','{}','[]','tree','tree','entry','entry','handoff','succeeded'); UPDATE requirement SET state='Running';")
        .execute(pool).await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    store::enqueue(&mut tx, "repaired-validation")
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

#[tokio::test]
async fn repair_updates_original_pr_and_reconciles_lost_push_without_overwriting_external_head() {
    let _guard = DATABASE_TEST.lock().await;
    for external in [false, true] {
        let pool = database().await;
        let original = job(&pool).await;
        let mut remote = Fake {
            pr: Some(pr(&original.identity())),
            ..Default::default()
        };
        tick(&pool, &mut remote, 10).await;
        repair_delivery(&pool).await;
        let repair = job(&pool).await;
        assert_eq!(repair.branch, original.branch);
        assert_eq!(
            repair.original_action_key,
            Some(original.action_key.clone())
        );
        assert_eq!(repair.expected_head.as_deref(), Some("candidate"));
        if external {
            remote.pr.as_mut().unwrap()["head"]["sha"] = json!("external");
        }
        remote.lost = true;
        tick(&pool, &mut remote, 20).await;
        assert_eq!(
            remote.calls.iter().filter(|call| **call == "push").count(),
            usize::from(!external)
        );
        if !external {
            // Remote applied the update but the response was lost; the next
            // read proves the candidate without issuing another write.
            remote.pr.as_mut().unwrap()["head"]["sha"] = json!("repaired");
            tick(&pool, &mut remote, 60).await;
            assert_eq!(
                remote.calls.iter().filter(|call| **call == "push").count(),
                1
            );
            let superseded: Option<String> =
                sqlx::query_scalar("SELECT superseded_by FROM delivery WHERE action_key=$1")
                    .bind(&original.action_key)
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            assert_eq!(superseded, Some(repair.action_key));
            assert_eq!(state(&pool).await.0, "Submitted");
        }
        assert_eq!(
            remote
                .calls
                .iter()
                .filter(|call| **call == "create")
                .count(),
            0
        );
        pool.close().await;
    }
}

#[tokio::test]
async fn cancellation_with_unsent_repair_closes_only_the_published_candidate() {
    let _guard = DATABASE_TEST.lock().await;
    let pool = database().await;
    let original = job(&pool).await;
    let mut remote = Fake {
        pr: Some(pr(&original.identity())),
        ..Default::default()
    };
    tick(&pool, &mut remote, 10).await;
    repair_delivery(&pool).await;
    control::cancel(&pool, 1).await.unwrap();
    control::settle(&pool).await.unwrap();
    tick(&pool, &mut remote, 20).await;
    tick(&pool, &mut remote, 60).await;
    assert!(state(&pool).await.2);
    assert_eq!(state(&pool).await.1, None);
    assert_eq!(
        remote.calls.iter().filter(|call| **call == "push").count(),
        0
    );
    assert_eq!(
        remote.calls.iter().filter(|call| **call == "close").count(),
        1
    );
    pool.close().await;
}

#[tokio::test]
async fn delivery_backoff_preserves_server_retry_after() {
    let _guard = DATABASE_TEST.lock().await;
    let pool = database().await;
    let job = job(&pool).await;
    let mut error = lost();
    error.status = Some(429);
    error.retry_after_seconds = Some(900);
    store::failed(&pool, &job, 100, &error).await.unwrap();
    assert!(store::due(&pool, 999).await.unwrap().is_empty());
    assert_eq!(store::due(&pool, 1000).await.unwrap().len(), 1);
    error.retry_after_seconds = Some(u64::MAX);
    store::failed(&pool, &job, 100, &error).await.unwrap();
    let next: i64 = sqlx::query_scalar(
        "SELECT next_attempt_at FROM delivery_action WHERE action_key=$1 AND kind='publish'",
    )
    .bind(&job.action_key)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(next, i64::MAX);
    pool.close().await;
}
