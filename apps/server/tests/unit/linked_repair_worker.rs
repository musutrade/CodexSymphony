//! Unit fixtures exercise persistence boundaries; these are not B05/B06 receipts.
use super::*;
use crate::merge_test_support::fixture;
// These fixtures share PostgreSQL's real global coordinator lock.
pub(crate) static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
pub(crate) async fn setup() -> (PgPool, std::path::PathBuf, GitBroker, Config, String) {
    let (pool, root, intent, mut remote) = fixture::fixture().await;
    crate::merge_worker::tick(&pool, &mut remote, 100)
        .await
        .unwrap();
    sqlx::query("UPDATE merge_operation SET state='merged',merged_sha=$1")
        .bind(&intent.head)
        .execute(&pool)
        .await
        .unwrap();
    let plan = crate::merge_validation::plan(&pool, &intent).await.unwrap();
    let mut evidence = fixture::evidence(&root, &root.join("repo"), &plan);
    evidence.steps[0].exit_code = Some(1);
    evidence.steps[0].output = "AssertionError: source".into();
    evidence.steps[0].output_sha256 = crate::validation::sha256(&evidence.steps[0].output);
    crate::linked_failure_store::post_merge(&pool, &intent, &evidence, &["test".into()])
        .await
        .unwrap();
    let id = format!("post-merge:{}", intent.action_key());
    let mut tx = crate::run_store::lock(&pool).await.unwrap();
    crate::linked_failure_store::record(
        &mut tx,
        &id,
        1,
        1,
        Some(&intent.action_key()),
        None,
        &evidence,
        &["test".into()],
    )
    .await
    .unwrap();
    assert!(
        crate::linked_failure_store::record(
            &mut tx,
            &id,
            1,
            2,
            Some(&intent.action_key()),
            None,
            &evidence,
            &["test".into()]
        )
        .await
        .is_err()
    );
    tx.rollback().await.unwrap();
    let bundle = root.join("seed.bundle");
    assert!(
        std::process::Command::new("git")
            .arg("-C")
            .arg(root.join("repo"))
            .args(["bundle", "create"])
            .arg(&bundle)
            .arg("--all")
            .status()
            .unwrap()
            .success()
    );
    let broker = GitBroker::initialize(&root.join("workspaces"), &bundle).unwrap();
    let key = crate::execution::RunKey {
        run_id: "base".into(),
        request_id: "base".into(),
        incarnation: "boot".into(),
    };
    let workspace = crate::workspace::Workspace {
        key,
        identity: "base".into(),
        requirement: 1,
        revision: 1,
        phase: "post_merge".into(),
        baseline: intent.head.clone(),
        branch: "ai/req-1-base".into(),
        path: broker.path("base").unwrap().to_string_lossy().into_owned(),
    };
    broker.prepare(&workspace, true).unwrap();
    let manifest = broker.preserve(&workspace).unwrap();
    sqlx::query("UPDATE delivery SET manifest=$1")
        .bind(json!(manifest))
        .execute(&pool)
        .await
        .unwrap();
    let repo: Value = sqlx::query_scalar("SELECT document FROM repository WHERE id=1")
        .fetch_one(&pool)
        .await
        .unwrap();
    let document = json!({"repository_id":1,"repository_version":1,"repository":repo});
    sqlx::query("UPDATE linked_failure SET repository_id=1,document=$2,paths='[\"source\"]',baseline=$3,manifest=$4,source_run='run' WHERE id=$1")
        .bind(&id).bind(document).bind(&intent.head).bind(json!(manifest)).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO repair_authorization VALUES(1,2,'bounded_v1') ON CONFLICT(requirement_id) DO UPDATE SET repair_limit=2,policy='bounded_v1'").execute(&pool).await.unwrap();
    let config = Config {
        validation: Some(plan),
        settings: crate::runtime_client::Settings {
            startup_seconds: 5,
            response_seconds: 5,
            stall_seconds: 5,
            reservation: crate::budget::Amount {
                tokens: 10,
                turns: 1,
                model_seconds: 10,
            },
            codex_config: String::new(),
        },
        preparation_adapter: "/bin/true".into(),
        preparation: json!({"launcher":["/bin/true"],"deployment_identity":"fixture"}),
    };
    (pool, root, broker, config, id)
}
#[tokio::test]
async fn duplicate_reservation_pause_restart_bind_and_scope_remain_on_original_item() {
    let _serial = crate::linked_repair_worker::tests::SERIAL.lock().await;
    let (pool, root, broker, config, id) = setup().await;
    sqlx::query("UPDATE requirement SET paused=true")
        .execute(&pool)
        .await
        .unwrap();
    reserve(&pool, &broker, "boot", &config).await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM repair_reservation")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    sqlx::query("UPDATE requirement SET paused=false")
        .execute(&pool)
        .await
        .unwrap();
    reserve(&pool, &broker, "boot", &config).await.unwrap();
    reserve(&pool, &broker, "boot", &config).await.unwrap();
    let (count, ordinal): (i64, i64) =
        sqlx::query_as("SELECT count(*),max(ordinal) FROM repair_reservation")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((count, ordinal), (1, 1));
    let (launch,workspace,manifest,source):(Value,Value,Value,String)=sqlx::query_as("SELECT p.launch,p.workspace,f.manifest,f.source_run FROM repair_reservation p JOIN linked_failure f ON f.id=p.linked_failure_id").fetch_one(&pool).await.unwrap();
    let job = Job {
        launch: serde_json::from_value(launch).unwrap(),
        workspace: serde_json::from_value(workspace).unwrap(),
        manifest: serde_json::from_value(manifest).unwrap(),
        source,
    };
    assert!(!bind(&pool, &job.launch).await.unwrap());
    // A successful adapter exit without preparation evidence must not start a Run.
    prepare_job(&pool, &root, &broker, &config, &job)
        .await
        .unwrap();
    let started: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_run WHERE id=$1)")
        .bind(&job.launch.key.run_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(!started);
    let status: String = sqlx::query_scalar("SELECT status FROM repair_reservation")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "reserved");
    sqlx::query("UPDATE requirement SET paused=true")
        .execute(&pool)
        .await
        .unwrap();
    prepare_job(&pool, &root, &broker, &config, &job)
        .await
        .unwrap();
    sqlx::query("UPDATE requirement SET paused=false")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE execution_control SET incarnation='boot2'")
        .execute(&pool)
        .await
        .unwrap();
    tick(&pool, &root, &broker, "boot2", &config).await.unwrap();
    let (_, job) = pending_job(&pool).await.unwrap().unwrap();
    sqlx::query("INSERT INTO preparation_record(run_id,requirement_id,revision,launch,retry,ready,checked_at) VALUES($1,1,1,$2,'{}',true,extract(epoch FROM now())::bigint) ON CONFLICT(run_id) DO UPDATE SET ready=true,checked_at=EXCLUDED.checked_at").bind(&job.launch.key.run_id).bind(json!(job.launch)).execute(&pool).await.unwrap();
    tick(&pool, &root, &broker, "boot2", &config).await.unwrap();
    let bound:(String,i64,String)=sqlx::query_as("SELECT p.status,a.requirement_id,i.failure_id FROM repair_reservation p JOIN agent_run a ON a.id=p.repair_run_id JOIN linked_run_input i ON i.run_id=a.id").fetch_one(&pool).await.unwrap();
    assert_eq!(bound, ("started".into(), 1, id.clone()));
    tick(&pool, &root, &broker, "boot2", &config).await.unwrap();
    let snapshot = broker.preserve(&job.workspace).unwrap();
    assert!(
        !candidate_allowed(&pool, &broker, &job.launch.key.run_id, &snapshot)
            .await
            .unwrap()
    );
    let blocker: String = sqlx::query_scalar("SELECT blocker FROM linked_failure WHERE id=$1")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(blocker.contains("no progress"));
    let owner: Option<i64> = sqlx::query_scalar("SELECT requirement_id FROM execution_control")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(owner, Some(1));
    pool.close().await;
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn committed_repair_diff_accepts_only_reviewed_paths_and_rejects_baseline_drift() {
    let _serial = SERIAL.lock().await;
    for (file, permitted) in [("source", true), ("outside", false)] {
        let (pool, root, broker, config, id) = setup().await;
        reserve(&pool, &broker, "boot", &config).await.unwrap();
        let (_, job) = pending_job(&pool).await.unwrap().unwrap();
        restore(&broker, &job).unwrap();
        sqlx::query("INSERT INTO preparation_record(run_id,requirement_id,revision,launch,retry,ready,checked_at) VALUES($1,1,1,$2,'{}',true,extract(epoch FROM now())::bigint) ON CONFLICT(run_id) DO UPDATE SET ready=true,checked_at=EXCLUDED.checked_at")
            .bind(&job.launch.key.run_id).bind(json!(job.launch)).execute(&pool).await.unwrap();
        assert!(bind(&pool, &job.launch).await.unwrap());
        let path = Path::new(&job.workspace.path);
        std::fs::write(path.join(file), "repair change").unwrap();
        for args in [
            vec!["add", file],
            vec![
                "-c",
                "user.name=fixture",
                "-c",
                "user.email=fixture@example.com",
                "commit",
                "-m",
                "repair diff",
            ],
        ] {
            assert!(
                std::process::Command::new("git")
                    .arg("-C")
                    .arg(path)
                    .args(args)
                    .status()
                    .unwrap()
                    .success()
            );
        }
        assert!(
            restore(&broker, &job)
                .unwrap_err()
                .to_string()
                .contains("baseline drift")
        );
        let manifest = broker.preserve(&job.workspace).unwrap();
        assert_eq!(
            candidate_allowed(&pool, &broker, &job.launch.key.run_id, &manifest)
                .await
                .unwrap(),
            permitted
        );
        let (state, evidence): (String, Value) =
            sqlx::query_as("SELECT state,evidence FROM linked_failure WHERE id=$1")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(state, if permitted { "reserved" } else { "blocked" });
        assert_eq!(evidence["steps"][0]["exit_code"], 1);
        pool.close().await;
        std::fs::remove_dir_all(root).unwrap();
    }
}
#[tokio::test]
async fn quota_counts_prior_repair_stages_and_keeps_original_failure() {
    let _serial = crate::linked_repair_worker::tests::SERIAL.lock().await;
    let (pool, root, broker, config, id) = setup().await;
    sqlx::query("INSERT INTO repair_reservation(requirement_id,ordinal,source_validation_id,failure,status) VALUES(1,2,'validation','{}','failed')").execute(&pool).await.unwrap();
    reserve(&pool, &broker, "boot", &config).await.unwrap();
    let (state, evidence): (String, Value) =
        sqlx::query_as("SELECT state,evidence FROM linked_failure WHERE id=$1")
            .bind(&id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(state, "blocked");
    assert_eq!(evidence["steps"][0]["exit_code"], 1);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM repair_reservation")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    assert!(
        !bind(
            &pool,
            &Launch {
                key: crate::execution::RunKey {
                    run_id: "missing".into(),
                    request_id: "missing".into(),
                    incarnation: "boot".into()
                },
                workspace: String::new(),
                workspace_identity: String::new(),
                program: String::new(),
                args: vec![]
            }
        )
        .await
        .unwrap()
    );
    pool.close().await;
    std::fs::remove_dir_all(root).unwrap();
}
