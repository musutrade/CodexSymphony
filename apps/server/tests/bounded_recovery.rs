use codexsymphony_server::{
    bounded_recovery::{self as domain, Class, Failure},
    budget::{Amount, Purpose, Usage},
    budget_store::{self, Admission, CallIntent},
    execution::{Launch, RunKey},
    recovery_store,
    runtime_resume::Job,
    workspace::{Manifest, Workspace},
};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
#[path = "support/groups.rs"]
mod groups;
#[path = "support/validation_runner.rs"]
mod source_fixture;

fn failure(sha: &str) -> Failure {
    Failure {
        phase: "local".into(),
        step: "test".into(),
        candidate_sha: sha.into(),
        pr_head: None,
        input_identity: "1".into(),
        environment_identity: "fixture".into(),
        command: vec!["cargo".into(), "test".into()],
        log_ref: "retained/raw.log".into(),
        raw: "assertion failed: answer == 42".into(),
        exit_code: Some(101),
        native_code: "check_exit".into(),
        authorized_code_check: true,
        retry_after_seconds: None,
    }
}

#[test]
fn facts_classify_service_failure_before_code_and_bound_backoff() {
    let mut f = failure("a");
    assert_eq!(f.classify(), Class::Code);
    for (raw, class) in [
        (
            "assertion failed: result; connection refused",
            Class::Infrastructure,
        ),
        ("error[E0308]: mismatched types", Class::Code),
        ("permission denied", Class::Configuration),
        ("model suggests code failure", Class::Unknown),
    ] {
        f.raw = raw.into();
        f.native_code = domain::native_failure(raw).into();
        assert_eq!(f.classify(), class);
    }
    assert_eq!(domain::next_retry(100, 700, 0, Some(60)), Some(160));
    assert_eq!(domain::next_retry(100, 700, 1, Some(1)), Some(220));
    assert_eq!(domain::next_retry(100, 700, 2, None), None);
    assert_eq!(domain::next_retry(100, 700, 0, Some(601)), None);
    assert_eq!(domain::next_retry(i64::MAX, i64::MAX, 0, None), None);
    assert_eq!(domain::repair_limit("bounded_v1"), 3);
    assert_eq!(domain::repair_limit("one_code_repair"), 1);
}

async fn setup() -> (PgPool, String, i64, String) {
    let (pool, url, _) = groups::fixture().await;
    let mut repository = groups::repository();
    repository["policy"]["gate_recovery_policy"] = json!("bounded_v1");
    sqlx::query("UPDATE repository SET document=$1")
        .bind(repository)
        .execute(&pool)
        .await
        .unwrap();
    let app = groups::app(&pool);
    let draft = groups::request(
        &app,
        "POST",
        "/api/drafts",
        groups::body(groups::sample(), 0),
        200,
    )
    .await;
    let draft = draft["id"].as_str().unwrap().to_owned();
    let mut review = groups::review();
    for item in review["items"].as_array_mut().unwrap() {
        item["budget"]["turns"] = json!(5);
    }
    groups::request(
        &app,
        "PUT",
        &format!("/api/drafts/{draft}/review"),
        json!({"version":0,"draft_revision":1,"review":review}),
        200,
    )
    .await;
    groups::request(
        &app,
        "POST",
        &format!("/api/drafts/{draft}/authorize"),
        json!({"version":1,"draft_revision":1,"request_id":"bounded"}),
        200,
    )
    .await;
    codexsymphony_server::group_queue_store::materialize(&pool)
        .await
        .unwrap();
    let id: i64 = sqlx::query_scalar(
        "SELECT requirement_id FROM group_execution_item WHERE draft_id=$1 AND child_id='C1'",
    )
    .bind(&draft)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE requirement SET state='Running' WHERE id=$1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE execution_control SET requirement_id=$1,incarnation='boot',recovery_complete=true",
    )
    .bind(id)
    .execute(&pool)
    .await
    .unwrap();
    (pool, url, id, draft)
}

fn job(id: i64, n: i64) -> Job {
    let key = RunKey {
        run_id: format!("repair-{n}"),
        request_id: format!("repair-{n}"),
        incarnation: "boot".into(),
    };
    let workspace = Workspace {
        key: key.clone(),
        identity: format!("workspace-{n}"),
        requirement: id,
        revision: 1,
        phase: "execution".into(),
        baseline: format!("sha-{n}"),
        branch: format!("repair-{n}"),
        path: format!("/tmp/repair-{n}"),
    };
    Job {
        source: format!("source-{n}"),
        launch: Launch {
            key,
            workspace: workspace.path.clone(),
            workspace_identity: workspace.identity.clone(),
            program: "/bin/false".into(),
            args: vec![],
        },
        manifest: Manifest {
            workspace: workspace.clone(),
            head: workspace.baseline.clone(),
            index_tree: "tree".into(),
            files: vec![],
            excluded: vec![],
            bundle_digest: "fixture".into(),
            index_digest: "fixture".into(),
        },
        workspace,
    }
}

async fn seed(pool: &PgPool, id: i64, n: i64) -> Job {
    let mut job = job(id, n);
    let previous:Option<String>=sqlx::query_scalar("SELECT p.repair_run_id FROM repair_reservation p JOIN agent_run a ON a.id=p.repair_run_id WHERE p.requirement_id=$1 AND p.ordinal=$2 AND a.state='Succeeded' AND a.quiescent").bind(id).bind(n-1).fetch_optional(pool).await.unwrap();
    if let Some(previous) = previous {
        job.source = previous;
    } else {
        sqlx::query("INSERT INTO agent_run(id,requirement_id,revision,incarnation,request_id,workspace,workspace_identity,launch,state,quiescent) VALUES($1,$2,1,'boot',$1,'/tmp','source','{}','Succeeded',true)").bind(&job.source).bind(id).execute(pool).await.unwrap();
    }
    sqlx::query("INSERT INTO workspace_snapshot(run_id,manifest,candidate) VALUES($1,$2,true)")
        .bind(&job.source)
        .bind(json!(job.manifest))
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO candidate_validation(id,requirement_id,revision,source_run_id,candidate_sha,candidate_tree,trusted,required_steps,source_before,source_after,entry_before,entry_after,stage,result) VALUES($1,$2,1,$3,$4,'tree','{}','[]','tree','tree','entry','entry','validation','gate_failed')")
        .bind(format!("validation-{n}")).bind(id).bind(&job.source).bind(&job.workspace.baseline).execute(pool).await.unwrap();
    recovery_store::record(
        pool,
        id,
        &format!("validation-{n}"),
        &format!("event-{n}"),
        &failure(&job.workspace.baseline),
    )
    .await
    .unwrap();
    job
}
fn resources() -> Amount {
    Amount {
        tokens: 10,
        turns: 1,
        model_seconds: 5,
    }
}

#[tokio::test]
async fn durable_reservations_settlement_restart_duplicate_events_and_exact_exhaustion() {
    let (mut pool, url, id, draft) = setup().await;
    for n in 1..=3 {
        let job = seed(&pool, id, n).await;
        let event = format!("event-{n}");
        if n > 1 {
            let prior: String =
                sqlx::query_scalar("SELECT decision FROM recovery_failure WHERE event_key=$1")
                    .bind(format!("event-{}", n - 1))
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            assert_eq!(prior, "continued");
        }

        let (a, b) = tokio::join!(
            recovery_store::reserve(&pool, &event, &job, resources()),
            recovery_store::reserve(&pool, &event, &job, resources())
        );
        assert_eq!(u8::from(a.unwrap()) + u8::from(b.unwrap()), 1);
        recovery_store::record(
            &pool,
            id,
            &format!("validation-{n}"),
            &format!("alias-{n}"),
            &failure(&job.workspace.baseline),
        )
        .await
        .unwrap();
        let alias: String =
            sqlx::query_scalar("SELECT decision FROM recovery_failure WHERE event_key=$1")
                .bind(format!("alias-{n}"))
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(alias, "covered");

        pool.close().await;
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "durable_recovery_child", "--nocapture"])
            .env("GH85_RESTART_URL", &url)
            .env("GH85_RESTART_ID", id.to_string())
            .env("GH85_RESTART_ORDINAL", n.to_string())
            .output()
            .unwrap();
        assert!(
            child.status.success(),
            "{}",
            String::from_utf8_lossy(&child.stderr)
        );
        pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(&url)
            .await
            .unwrap();
        assert!(
            !recovery_store::reserve(&pool, &event, &job, resources())
                .await
                .unwrap()
        );
        let reserved: Value = sqlx::query_scalar(
            "SELECT reserved FROM group_budget WHERE draft_id=$1 AND item_id=''",
        )
        .bind(&draft)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(reserved, json!(resources()));
        sqlx::query("INSERT INTO preparation_record(run_id,requirement_id,revision,launch,retry,ready,checked_at) VALUES($1,$2,1,$3,'{}',true,extract(epoch FROM now())::bigint)").bind(&job.launch.key.run_id).bind(id).bind(json!(job.launch)).execute(&pool).await.unwrap();
        sqlx::query("UPDATE requirement SET paused=true WHERE id=$1")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
        assert!(
            !codexsymphony_server::validation_repair::bind(&pool, &job.launch)
                .await
                .unwrap()
        );
        sqlx::query("UPDATE requirement SET paused=false WHERE id=$1")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
        assert!(
            codexsymphony_server::validation_repair::bind(&pool, &job.launch)
                .await
                .unwrap()
        );
        assert!(
            !codexsymphony_server::validation_repair::bind(&pool, &job.launch)
                .await
                .unwrap()
        );
        std::fs::create_dir_all(&job.launch.workspace).unwrap();
        let call = CallIntent {
            key: job.launch.key.clone(),
            turn_id: "turn".into(),
            purpose: Purpose::Repair,
            reserve: resources(),
        };
        assert_eq!(
            budget_store::reserve(&pool, &call).await.unwrap(),
            Admission::Reserved
        );
        assert_eq!(
            budget_store::reserve(&pool, &call).await.unwrap(),
            Admission::Existing
        );
        let usage = Usage {
            input: Some(3),
            cached: Some(0),
            output: Some(2),
            model_seconds: Some(1),
            complete: true,
        };
        budget_store::settle(&pool, &call.key, "turn", "settled", &usage)
            .await
            .unwrap();
        budget_store::settle(&pool, &call.key, "turn", "settled", &usage)
            .await
            .unwrap();
        let (used, reserved): (Value, Value) = sqlx::query_as(
            "SELECT used,reserved FROM group_budget WHERE draft_id=$1 AND item_id=''",
        )
        .bind(&draft)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(used, json!({"tokens":5*n,"turns":n,"model_seconds":n}));
        assert_eq!(reserved, json!(Amount::default()));
        sqlx::query("UPDATE agent_run SET state='Succeeded',quiescent=true WHERE id=$1")
            .bind(&call.key.run_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "UPDATE repair_reservation SET status='failed' WHERE requirement_id=$1 AND ordinal=$2",
        )
        .bind(id)
        .bind(n)
        .execute(&pool)
        .await
        .unwrap();
    }
    let fourth = seed(&pool, id, 4).await;
    assert!(
        !recovery_store::reserve(&pool, "event-4", &fourth, resources())
            .await
            .unwrap()
    );
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM repair_reservation WHERE requirement_id=$1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 3);
    let owner: Option<i64> = sqlx::query_scalar("SELECT requirement_id FROM execution_control")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(owner, Some(id));
    assert_eq!(
        budget_store::inspect(&pool, id).await.unwrap().used.turns,
        3
    );
    pool.close().await;
}

#[tokio::test]
async fn insufficient_parent_balance_and_cancel_do_not_charge_or_launch() {
    let (pool, _, id, draft) = setup().await;
    let job = seed(&pool, id, 1).await;
    sqlx::query("UPDATE group_budget SET limits='{\"tokens\":9,\"turns\":5,\"model_seconds\":60}' WHERE draft_id=$1 AND item_id=''").bind(&draft).execute(&pool).await.unwrap();
    assert!(
        !recovery_store::reserve(&pool, "event-1", &job, resources())
            .await
            .unwrap()
    );
    let job = seed(&pool, id, 2).await;
    codexsymphony_server::delivery_control::cancel(&pool, id)
        .await
        .unwrap();
    assert!(
        !recovery_store::reserve(&pool, "event-2", &job, resources())
            .await
            .unwrap()
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM repair_reservation")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    pool.close().await;
}

#[tokio::test]
async fn durable_recovery_child() {
    let Ok(url) = std::env::var("GH85_RESTART_URL") else {
        return;
    };
    let id: i64 = std::env::var("GH85_RESTART_ID").unwrap().parse().unwrap();
    let n: i64 = std::env::var("GH85_RESTART_ORDINAL")
        .unwrap()
        .parse()
        .unwrap();
    let pool = PgPoolOptions::new().connect(&url).await.unwrap();
    let job = job(id, n);
    assert!(
        !recovery_store::reserve(&pool, &format!("event-{n}"), &job, resources())
            .await
            .unwrap()
    );
    assert_eq!(
        budget_store::inspect(&pool, id)
            .await
            .unwrap()
            .exposure
            .turns,
        n
    );
    pool.close().await;
}

#[tokio::test]
async fn cold_start_barrier_reconciles_unstarted_intent_without_new_ordinal() {
    use codexsymphony_server::{
        git_broker::GitBroker, recovery_worker, run_store, runtime_service::Config,
    };
    let (pool, _, id, _) = setup().await;
    seed(&pool, id, 1).await;
    let (root, repo, _) = source_fixture::fixture();
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
    let config = Config {
        validation: None,
        settings: codexsymphony_server::runtime_client::Settings {
            startup_seconds: 5,
            response_seconds: 5,
            stall_seconds: 5,
            reservation: resources(),
            codex_config: String::new(),
        },
        preparation_adapter: "/bin/true".into(),
        preparation: json!({"launcher":["/bin/true"]}),
    };
    recovery_worker::plan(&pool, &broker, "boot", &config)
        .await
        .unwrap();
    let original: Value = sqlx::query_scalar("SELECT launch FROM repair_reservation")
        .fetch_one(&pool)
        .await
        .unwrap();
    run_store::begin_incarnation(&pool, "restarted")
        .await
        .unwrap();
    recovery_worker::plan(&pool, &broker, "restarted", &config)
        .await
        .unwrap();
    let unchanged: Value = sqlx::query_scalar("SELECT launch FROM repair_reservation")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(original, unchanged);
    assert!(
        run_store::finish_recovery(&pool, "restarted")
            .await
            .unwrap()
    );
    recovery_worker::plan(&pool, &broker, "restarted", &config)
        .await
        .unwrap();
    let (launch, ordinal): (Value, i64) =
        sqlx::query_as("SELECT launch,ordinal FROM repair_reservation")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(ordinal, 1);
    assert_ne!(launch, original);
    assert_eq!(launch["key"]["incarnation"], "restarted");
    assert_eq!(
        budget_store::inspect(&pool, id).await.unwrap().exposure,
        resources()
    );
    let old: Launch = serde_json::from_value(original).unwrap();
    assert!(
        !codexsymphony_server::validation_repair::bind(&pool, &old)
            .await
            .unwrap()
    );
    let launch: Launch = serde_json::from_value(launch).unwrap();
    sqlx::query("INSERT INTO preparation_record(run_id,requirement_id,revision,launch,retry,ready,checked_at) VALUES($1,$2,1,$3,'{}',true,extract(epoch FROM now())::bigint)").bind(&launch.key.run_id).bind(id).bind(json!(launch)).execute(&pool).await.unwrap();
    assert!(
        codexsymphony_server::validation_repair::bind(&pool, &launch)
            .await
            .unwrap()
    );
    sqlx::query("UPDATE agent_run SET state='Succeeded',quiescent=true WHERE id=$1")
        .bind(&launch.key.run_id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        recovery_store::unchanged_candidate(&pool, &launch.key.run_id, "sha-1")
            .await
            .unwrap()
    );
    recovery_worker::plan(&pool, &broker, "restarted", &config)
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM repair_reservation")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    let history: i64 = sqlx::query_scalar("SELECT count(*) FROM repair_intent_history")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(history, 1);
    pool.close().await;
}

#[tokio::test]
async fn ci_admission_requires_current_pr_head_and_original_candidate_identity() {
    let (pool, _, id, _) = setup().await;
    let job = seed(&pool, id, 1).await;
    sqlx::query("UPDATE candidate_validation SET result='succeeded' WHERE id='validation-1'")
        .execute(&pool)
        .await
        .unwrap();
    let mut f = failure("sha-1");
    f.phase = "ci".into();
    f.pr_head = Some("sha-1".into());
    recovery_store::record(&pool, id, "validation-1", "ci-event", &f)
        .await
        .unwrap();
    assert!(
        !recovery_store::reserve(&pool, "ci-event", &job, resources())
            .await
            .unwrap()
    );
    sqlx::query("INSERT INTO github_repository(repository_id,repository_version,policy,probe_pr) VALUES(9,1,'{}',1)").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO github_pr(repository_id,number,requirement_id,observation,last_synced_at,stale) VALUES(9,1,$1,'{\"policy\":{},\"head\":\"external\",\"merge\":\"Unmerged\",\"closed\":false}',extract(epoch FROM now())::bigint,false)").bind(id).execute(&pool).await.unwrap();
    assert!(
        !recovery_store::reserve(&pool, "ci-event", &job, resources())
            .await
            .unwrap()
    );
    sqlx::query("UPDATE github_pr SET observation=jsonb_set(observation,'{head}','\"sha-1\"')")
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        recovery_store::reserve(&pool, "ci-event", &job, resources())
            .await
            .unwrap()
    );
    let context: Value = sqlx::query_scalar("SELECT failure FROM repair_reservation")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(context["failure"]["pr_head"], "sha-1");
    assert!(context["remaining_acceptance"].is_array());
    assert_eq!(context["reserved_resources"], json!(resources()));
    assert_eq!(context["group_budget"].as_array().unwrap().len(), 2);
    pool.close().await;
}

#[tokio::test]
async fn late_parent_exposure_blocks_prepaid_call_without_releasing_its_hold() {
    let (pool, _, id, draft) = setup().await;
    let job = seed(&pool, id, 1).await;
    assert!(
        recovery_store::reserve(&pool, "event-1", &job, resources())
            .await
            .unwrap()
    );
    sqlx::query("INSERT INTO preparation_record(run_id,requirement_id,revision,launch,retry,ready,checked_at) VALUES($1,$2,1,$3,'{}',true,extract(epoch FROM now())::bigint)").bind(&job.launch.key.run_id).bind(id).bind(json!(job.launch)).execute(&pool).await.unwrap();
    assert!(
        codexsymphony_server::validation_repair::bind(&pool, &job.launch)
            .await
            .unwrap()
    );
    // A late accounted sibling usage fact can exceed the parent's remaining
    // resources after the initial repair hold; transfer must recheck exposure.
    sqlx::query("UPDATE group_budget SET used=limits WHERE draft_id=$1 AND item_id=''")
        .bind(draft)
        .execute(&pool)
        .await
        .unwrap();
    std::fs::create_dir_all(&job.launch.workspace).unwrap();
    let call = CallIntent {
        key: job.launch.key,
        turn_id: "late".into(),
        purpose: Purpose::Repair,
        reserve: resources(),
    };
    assert_eq!(
        budget_store::reserve(&pool, &call).await.unwrap(),
        Admission::Blocked
    );
    let transferred: bool =
        sqlx::query_scalar("SELECT resources_transferred FROM repair_reservation")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!transferred);
    assert!(budget_store::inspect(&pool, id).await.unwrap().exhausted);
    assert_eq!(
        budget_store::inspect(&pool, id).await.unwrap().exposure,
        resources()
    );
    pool.close().await;
}

#[tokio::test]
async fn storage_configuration_failure_rolls_back_ordinal_and_group_hold() {
    let (pool, _, id, draft) = setup().await;
    let job = seed(&pool, id, 1).await;
    // Fault injection into this disposable schema: malformed persisted storage
    // configuration is a service/configuration error, never a code repair charge.
    sqlx::raw_sql("INSERT INTO storage_policy(version,document,deployment) VALUES('broken','{}','{}'); UPDATE storage_guard SET policy_version='broken'").execute(&pool).await.unwrap();
    assert!(
        recovery_store::reserve(&pool, "event-1", &job, resources())
            .await
            .is_err()
    );
    assert_eq!(
        budget_store::inspect(&pool, id).await.unwrap().exposure,
        Amount::default()
    );
    let held: Value =
        sqlx::query_scalar("SELECT reserved FROM group_budget WHERE draft_id=$1 AND item_id=''")
            .bind(draft)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(held, json!(Amount::default()));
    sqlx::query("UPDATE storage_guard SET policy_version=NULL")
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        recovery_store::reserve(&pool, "event-1", &job, resources())
            .await
            .unwrap()
    );
    let ordinal: i64 = sqlx::query_scalar("SELECT ordinal FROM repair_reservation")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(ordinal, 1);
    // Corrupt/overflowing retained exposure must fail closed, not wrap to a
    // fresh positive balance or silently discard either reservation.
    sqlx::query("UPDATE repair_reservation SET status='failed'")
        .execute(&pool)
        .await
        .unwrap();
    seed(&pool, id, 2).await;
    sqlx::query("INSERT INTO repair_reservation(requirement_id,ordinal,source_validation_id,failure,status,resources) VALUES($1,2,'validation-2','{}','failed',$2)").bind(id).bind(json!(Amount{tokens:i64::MAX,turns:1,model_seconds:1})).execute(&pool).await.unwrap();
    assert!(budget_store::inspect(&pool, id).await.is_err());
    pool.close().await;
}

#[tokio::test]
async fn paused_repair_resumes_under_same_ordinal_and_prepaid_exact_budget() {
    let (pool, _, id, draft) = setup().await;
    let original = seed(&pool, id, 1).await;
    sqlx::query("UPDATE requirement_budget SET limits=$2 WHERE requirement_id=$1")
        .bind(id)
        .bind(json!(resources()))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE group_budget SET limits=$2 WHERE draft_id=$1")
        .bind(draft)
        .bind(json!(resources()))
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        recovery_store::reserve(&pool, "event-1", &original, resources())
            .await
            .unwrap()
    );
    sqlx::query("INSERT INTO preparation_record(run_id,requirement_id,revision,launch,retry,ready,checked_at) VALUES($1,$2,1,$3,'{}',true,extract(epoch FROM now())::bigint)").bind(&original.launch.key.run_id).bind(id).bind(json!(original.launch)).execute(&pool).await.unwrap();
    assert!(
        codexsymphony_server::validation_repair::bind(&pool, &original.launch)
            .await
            .unwrap()
    );
    sqlx::query("UPDATE agent_run SET state='Failed',quiescent=true,user_paused=true WHERE id=$1")
        .bind(&original.launch.key.run_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO workspace_snapshot(run_id,manifest,candidate) VALUES($1,$2,false)")
        .bind(&original.launch.key.run_id)
        .bind(json!(original.manifest))
        .execute(&pool)
        .await
        .unwrap();
    let mut successor = job(id, 77);
    successor.source = original.launch.key.run_id.clone();
    sqlx::query("INSERT INTO runtime_resume(source_run,job,status) VALUES($1,$2,'prepared')")
        .bind(&successor.source)
        .bind(json!(successor))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO preparation_record(run_id,requirement_id,revision,launch,retry,ready,checked_at) VALUES($1,$2,1,$3,'{}',true,extract(epoch FROM now())::bigint)").bind(&successor.launch.key.run_id).bind(id).bind(json!(successor.launch)).execute(&pool).await.unwrap();
    assert!(
        codexsymphony_server::runtime_resume::bind(&pool, &successor)
            .await
            .unwrap()
    );
    assert!(
        !codexsymphony_server::runtime_resume::bind(&pool, &successor)
            .await
            .unwrap()
    );
    let rows: Vec<(i64, String)> =
        sqlx::query_as("SELECT ordinal,repair_run_id FROM repair_reservation")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(rows, vec![(1, successor.launch.key.run_id.clone())]);
    let history: (String, String) =
        sqlx::query_as("SELECT source_run,successor_run FROM repair_run_history")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        history,
        (
            original.launch.key.run_id.clone(),
            successor.launch.key.run_id.clone()
        )
    );
    assert_eq!(
        budget_store::inspect(&pool, id).await.unwrap().exposure,
        resources()
    );
    std::fs::create_dir_all(&successor.launch.workspace).unwrap();
    let call = CallIntent {
        key: successor.launch.key,
        turn_id: "resumed-turn".into(),
        purpose: Purpose::Repair,
        reserve: resources(),
    };
    assert_eq!(
        budget_store::reserve(&pool, &call).await.unwrap(),
        Admission::Reserved
    );
    let late = CallIntent {
        key: original.launch.key,
        turn_id: "stale-turn".into(),
        purpose: Purpose::Repair,
        reserve: resources(),
    };
    assert_eq!(
        budget_store::reserve(&pool, &late).await.unwrap(),
        Admission::Blocked
    );
    budget_store::settle(
        &pool,
        &call.key,
        "resumed-turn",
        "finished",
        &Usage {
            input: Some(1),
            cached: Some(0),
            output: Some(1),
            model_seconds: Some(1),
            complete: true,
        },
    )
    .await
    .unwrap();
    sqlx::query("UPDATE agent_run SET state='Succeeded',quiescent=true WHERE id=$1")
        .bind(&call.key.run_id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        budget_store::inspect(&pool, id).await.unwrap().used.turns,
        1
    );
    assert!(
        recovery_store::unchanged_candidate(&pool, &call.key.run_id, "sha-1")
            .await
            .unwrap()
    );
    pool.close().await;
}

#[tokio::test]
async fn invalid_phase_evidence_blocks_instead_of_charging_a_repair() {
    let (pool, _, id, _) = setup().await;
    let job = seed(&pool, id, 1).await;
    let mut f = failure("sha-1");
    f.phase = "ci".into();
    f.pr_head = Some("sha-1".into());
    // CI repair requires a previously validated candidate, not local Gate FAIL.
    recovery_store::record(&pool, id, "validation-1", "invalid-ci-binding", &f)
        .await
        .unwrap();
    let decision: String = sqlx::query_scalar(
        "SELECT decision FROM recovery_failure WHERE event_key='invalid-ci-binding'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(decision, "blocked");
    assert!(
        !recovery_store::reserve(&pool, "event-1", &job, resources())
            .await
            .unwrap()
    );
    assert_eq!(
        budget_store::inspect(&pool, id).await.unwrap().exposure,
        Amount::default()
    );
    pool.close().await;
}
