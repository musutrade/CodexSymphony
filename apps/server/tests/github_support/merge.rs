//! Real HTTP + durable Broker + real independent local validation. The HTTP peer
//! is synthetic; only the separate disposable-host run certifies GitHub writes.
use super::*;
use codexsymphony_server::{
    git_broker::GitBroker, github_contract::*, group_queue_store, validation_runner,
    validation_service, workspace::Workspace,
};
#[path = "../support/groups.rs"]
mod groups;

struct Case {
    f: Fixture,
    pool: PgPool,
    root: PathBuf,
    head: String,
    merged: String,
    pr: Value,
    broker: GitBroker,
    policy: Policy,
}
async fn setup() -> Case {
    setup_method("squash", false).await
}
async fn setup_method(method: &str, failing_post: bool) -> Case {
    setup_source(method, failing_post, false).await
}
async fn setup_source(method: &str, failing_post: bool, test_merge: bool) -> Case {
    setup_contract(method, failing_post, test_merge, false).await
}
async fn setup_contract(method: &str, failing_post: bool, test_merge: bool, checks: bool) -> Case {
    setup_variant(method, failing_post, test_merge, checks, None).await
}
async fn setup_variant(
    method: &str,
    failing_post: bool,
    test_merge: bool,
    checks: bool,
    post_argument: Option<&str>,
) -> Case {
    let (pool, _, _) = groups::fixture().await;
    let mut repository = groups::repository();
    repository["github_repository_id"] = json!(99);
    repository["remote"] = json!("owner/repo");
    sqlx::query("UPDATE repository SET document=$1 WHERE id=1")
        .bind(repository)
        .execute(&pool)
        .await
        .unwrap();
    let router = groups::app(&pool);
    let draft = groups::request(
        &router,
        "POST",
        "/api/drafts",
        groups::body(groups::sample(), 0),
        200,
    )
    .await;
    let id = draft["id"].as_str().unwrap();
    groups::request(
        &router,
        "PUT",
        &format!("/api/drafts/{id}/review"),
        json!({"version":0,"draft_revision":1,"review":groups::review()}),
        200,
    )
    .await;
    groups::request(
        &router,
        "POST",
        &format!("/api/drafts/{id}/authorize"),
        json!({"version":1,"draft_revision":1,"request_id":"merge-case"}),
        200,
    )
    .await;
    group_queue_store::materialize(&pool).await.unwrap();
    let (root, repo, plan) = delivery_source::fixture();
    let head = validation_runner::candidate(&repo).unwrap().sha;
    std::fs::write(repo.join("source"), "actual merged checkout\n").unwrap();
    git_fixture(&repo, &["commit", "-am", "independent merged identity"]);
    let merged = validation_runner::candidate(&repo).unwrap().sha;
    assert_ne!(head, merged);
    git_fixture(
        &repo,
        &[
            "commit",
            "--allow-empty",
            "-m",
            "independent test merge identity",
        ],
    );
    let test_sha = validation_runner::candidate(&repo).unwrap().sha;
    let bundle = root.join("seed.bundle");
    git_fixture(
        &repo,
        &["bundle", "create", bundle.to_str().unwrap(), "--all"],
    );
    let broker = GitBroker::initialize(&root.join("workspaces"), &bundle).unwrap();
    let workspace = Workspace {
        key: RunKey {
            run_id: "run".into(),
            request_id: "request".into(),
            incarnation: "boot".into(),
        },
        identity: "owned".into(),
        requirement: 1,
        revision: 1,
        phase: "validation".into(),
        baseline: head.clone(),
        branch: "ai/req-1-run".into(),
        path: broker.path("run").unwrap().to_string_lossy().into_owned(),
    };
    broker.prepare(&workspace, true).unwrap();
    let manifest = broker.preserve(&workspace).unwrap();
    sqlx::raw_sql("UPDATE requirement SET state='Running' WHERE id=1; UPDATE execution_control SET requirement_id=1,recovery_complete=true,incarnation='boot'; INSERT INTO agent_run(id,requirement_id,revision,incarnation,request_id,workspace,workspace_identity,launch,state,quiescent) VALUES('run',1,1,'boot','request','/tmp','owned','{}','Succeeded',true);").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO workspace_snapshot(run_id,manifest,candidate) VALUES('run',$1,true)")
        .bind(json!(manifest))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO group_claim_input(requirement_id,authorization_id,baseline,dependencies) SELECT 1,authorization_id,$1,'[]'::jsonb FROM group_execution_item WHERE requirement_id=1").bind(&head).execute(&pool).await.unwrap();
    let checkout = PathBuf::from(&workspace.path);
    let candidate = validation_runner::candidate(&checkout).unwrap();
    assert!(
        validation_service::validate(
            &pool,
            validation_service::Request {
                id: "actual-validation",
                source_run: "run",
                requirement: 1,
                revision: 1,
                checkout: &checkout,
                directory: &root.join("original-evidence"),
                candidate: &candidate,
                plan: &plan
            }
        )
        .await
        .unwrap()
    );
    let f = Fixture::new().await;
    let plan_file = root.join("post-plan.json");
    let mut post_plan = plan.clone();
    if failing_post {
        post_plan.steps[0].command.push("fail".into());
    }
    if let Some(argument) = post_argument {
        post_plan.steps[0].command.push(argument.into());
        post_plan.steps[0].timeout_seconds = 5;
    }
    std::fs::write(&plan_file, serde_json::to_vec(&post_plan).unwrap()).unwrap();
    let selector = Selector {
        name: "ci".into(),
        source: Source::CheckRun { app_id: 42 },
    };
    let mut policy = Policy {
        repository_id: 99,
        repository: "owner/repo".into(),
        default_branch: "main".into(),
        version: 1,
        required: vec![selector.clone()],
        wait_seconds: 900,
        delivery: Some(DeliveryContract {
            schema_version: 1,
            pre_merge: PreMerge {
                source: if test_merge {
                    PreMergeSource::TestMerge
                } else {
                    PreMergeSource::Head
                },
                checkout: if test_merge {
                    PreMergeSource::TestMerge
                } else {
                    PreMergeSource::Head
                },
                checks: vec![RequiredCheck {
                    selector,
                    applicability: "always".into(),
                    trigger: Trigger::External {
                        event: "pull_request".into(),
                        path: "ci.yml".into(),
                        blob_sha: "a".repeat(40),
                    },
                    job: None,
                }],
                wait_seconds: 900,
            },
            post_merge: PostMerge::FixedValidation {
                plan_id: plan_file.to_string_lossy().into_owned(),
                configuration_sha256: post_plan.identity().unwrap().config_sha256,
                authorization: "isolated synthetic source acceptance".into(),
                wait_seconds: 900,
            },
            actions: Actions {
                merge: true,
                merge_method: Some(method.into()),
                rerun_actions: false,
                rerequest_checks: false,
                read_logs: false,
            },
            protection: json!({"required_status_checks":{"strict":true,"checks":[{"context":"ci","app_id":42}]},"enforce_admins":{"enabled":true}}),
            rules: vec![],
        }),
    };
    if checks {
        let mut selector = action_policy().required.remove(0);
        if let Source::Actions { event, branch, .. } = &mut selector.source {
            *event = "workflow_dispatch".into();
            *branch = "main".into();
        }
        policy.delivery.as_mut().unwrap().post_merge = PostMerge::Checks {
            checks: vec![RequiredCheck {
                selector,
                applicability: "always".into(),
                trigger: Trigger::WorkflowDispatch,
                job: Some("build".into()),
            }],
            wait_seconds: 900,
            probe_pr: 2,
        };
        f.put(
            "/repos/owner/repo/actions/workflows/8",
            json!({"id":8,"path":".github/workflows/ci.yml","state":"active"}),
        );
        f.put("/repos/owner/repo/contents/%2E%67%69%74%68%75%62%2F%77%6F%72%6B%66%6C%6F%77%73%2F%63%69%2E%79%6D%6C", json!({"sha":"pinned"}));
        f.put(
            "POST /repos/owner/repo/actions/workflows/8/dispatches",
            json!({}),
        );
        f.put(
            "/repos/owner/repo/git/ref/heads/%6D%61%69%6E",
            json!({"object":{"sha":merged}}),
        );
        post_checks(&f, &merged, "success");
    }
    v1::grants(&f, &policy);
    f.put("/repos/owner/repo",json!({"id":99,"full_name":"owner/repo","default_branch":"main","archived":false,"private":true,"allow_squash_merge":true,"allow_rebase_merge":true,"allow_merge_commit":true}));
    f.put(
        "/repos/owner/repo/branches/%6D%61%69%6E",
        json!({"name":"main","protected":true}),
    );
    f.put(
        "/repos/owner/repo/branches/%6D%61%69%6E/protection",
        policy.delivery.as_ref().unwrap().protection.clone(),
    );
    f.put(
        "/repos/owner/repo/contents/%63%69%2E%79%6D%6C",
        json!({"sha":"a".repeat(40)}),
    );
    let mut check = check(2, "success");
    check["head_sha"] = json!(head);
    f.put(
        &format!("/repos/owner/repo/commits/{head}/check-suites"),
        json!({"check_suites":[{"id":9}]}),
    );
    f.put(
        "/repos/owner/repo/check-suites/9/check-runs",
        json!({"check_runs":[check]}),
    );
    if test_merge {
        let mut selected = super::check(3, "success");
        selected["head_sha"] = json!(test_sha);
        selected["check_suite"]["id"] = json!(10);
        f.put(
            &format!("/repos/owner/repo/commits/{test_sha}/check-suites"),
            json!({"check_suites":[{"id":10}]}),
        );
        f.put(
            "/repos/owner/repo/check-suites/10/check-runs",
            json!({"check_runs":[selected]}),
        );
    }
    let key: String = sqlx::query_scalar("SELECT action_key FROM delivery")
        .fetch_one(&pool)
        .await
        .unwrap();
    let pr = json!({"number":1,"state":"open","merged":false,"merged_at":null,"draft":false,"mergeable":true,"mergeable_state":"clean","merge_commit_sha":test_sha,"head":{"repo":{"id":99},"ref":workspace.branch,"sha":head},"base":{"repo":{"id":99,"full_name":"owner/repo"},"ref":"main","sha":"b".repeat(40)},"body":format!("<!-- codexsymphony-delivery:{key} -->")});
    f.put("/repos/owner/repo/pulls/1", pr.clone());
    f.put("/repos/owner/repo/pulls", json!([{"number":1}]));
    f.put(
        "PUT /repos/owner/repo/pulls/1/merge",
        json!({"merged":true,"sha":merged}),
    );
    f.put("expected-merge", json!({"sha":head,"merge_method":method}));
    if checks {
        let mut probe = pr.clone();
        probe["number"] = json!(2);
        probe["state"] = json!("closed");
        probe["merged"] = json!(true);
        probe["merged_at"] = json!("2026-09-22T00:00:00Z");
        probe["merge_commit_sha"] = json!(merged);
        f.put("/repos/owner/repo/pulls/2", probe);
    }
    github_store::configure(&pool, &policy, 1).await.unwrap();
    let cap = github_observe::preflight(&mut f.client(), &policy, 1, github_service::now())
        .await
        .unwrap_or_else(|error| panic!("{error:?}; requests: {:?}", f.data.lock().unwrap().seen));
    assert!(cap.blockers.is_empty(), "{:?}", cap.blockers);
    github_store::save_capability(&pool, &cap).await.unwrap();
    sqlx::raw_sql("UPDATE delivery SET pr_number=1; UPDATE delivery_action SET state='confirmed'; UPDATE requirement SET state='Submitted' WHERE id=1; INSERT INTO github_pr(repository_id,number,requirement_id) VALUES(99,1,1);").execute(&pool).await.unwrap();
    Case {
        f,
        pool,
        root,
        head,
        merged,
        pr,
        broker,
        policy,
    }
}
impl Case {
    async fn tick(&self, client: &mut AppClient) {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        sqlx::query("UPDATE merge_operation SET next_attempt_at=0")
            .execute(&self.pool)
            .await
            .unwrap();
        github_service::deliver(&self.pool, client, &self.root, github_service::now())
            .await
            .unwrap();
    }
    fn merged(&mut self) {
        self.pr["merged"] = json!(true);
        self.pr["state"] = json!("closed");
        self.pr["merged_at"] = json!("2026-09-22T00:00:00Z");
        self.pr["merge_commit_sha"] = json!(self.merged);
        self.f.put("/repos/owner/repo/pulls/1", self.pr.clone());
    }
    async fn state(&self) -> String {
        sqlx::query_scalar("SELECT state FROM requirement WHERE id=1")
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }
    async fn close(self) {
        self.pool.close().await;
        std::fs::remove_dir_all(self.root).unwrap();
    }
}
#[tokio::test]
async fn broker_merge_real_merged_checkout_completes_child_and_advances_successor() {
    let _guard = recovery_acceptance::DATABASE_TEST.lock().await;
    for method in ["squash", "rebase", "merge"] {
        let mut case = setup_method(method, false).await;
        let mut client = case.f.client();
        case.tick(&mut client).await;
        case.tick(&mut client).await;
        assert_eq!(case.state().await, "Submitted");
        case.merged();
        case.tick(&mut client).await;
        assert_eq!(case.state().await, "Done", "{}", sqlx::query_scalar::<_,Value>("SELECT jsonb_build_object('state',state,'blocker',blocker,'method',merge_method) FROM merge_operation").fetch_one(&case.pool).await.unwrap());
        let fact: Value =
            sqlx::query_scalar("SELECT fact FROM group_completion WHERE requirement_id=1")
                .fetch_one(&case.pool)
                .await
                .unwrap();
        assert_eq!(fact["merged_sha"], case.merged);
        assert_eq!(fact["acceptance_sha"], case.merged);
        assert_eq!(fact["head_sha"], case.head);
        let acceptance: Value = sqlx::query_scalar("SELECT acceptance FROM merge_operation")
            .fetch_one(&case.pool)
            .await
            .unwrap();
        assert_eq!(acceptance["evidence"]["candidate"]["sha"], case.merged);
        assert_ne!(acceptance["evidence"]["candidate"]["sha"], case.head);
        codexsymphony_server::delivery_control::settle(&case.pool)
            .await
            .unwrap();
        let selected = codexsymphony_server::runtime_initial::plan(
            &case.pool,
            &case.broker,
            "boot",
            &["/bin/true".into()],
            &case.head,
        )
        .await
        .unwrap();
        let (_, workspace) =
            selected.expect("Done predecessor must not remain hidden behind Submitted filtering");
        assert_eq!(workspace.requirement, 2);
        assert_eq!(workspace.baseline, case.merged);
        assert_eq!(
            case.f
                .data
                .lock()
                .unwrap()
                .seen
                .iter()
                .filter(|request| request.starts_with("PUT /repos/owner/repo/pulls/1/merge"))
                .count(),
            1
        );
        case.close().await;
    }
}
#[tokio::test]
async fn post_merge_failure_preserves_merge_and_owner_without_business_done() {
    let _guard = recovery_acceptance::DATABASE_TEST.lock().await;
    let mut case = setup().await;
    let mut client = case.f.client();
    case.tick(&mut client).await;
    case.tick(&mut client).await;
    case.merged();
    // Changing the host-owned entry invalidates its pinned plan, never the merge fact.
    let PostMerge::FixedValidation { plan_id, .. } =
        &case.policy.delivery.as_ref().unwrap().post_merge
    else {
        unreachable!()
    };
    let plan: validation_runner::Plan =
        serde_json::from_slice(&std::fs::read(plan_id).unwrap()).unwrap();
    std::fs::write(plan.entry, "changed entry").unwrap();
    case.tick(&mut client).await;
    assert_eq!(case.state().await, "Submitted");
    let row: (String, String) = sqlx::query_as("SELECT state,merged_sha FROM merge_operation")
        .fetch_one(&case.pool)
        .await
        .unwrap();
    assert_eq!(row, ("blocked".into(), case.merged.clone()));
    codexsymphony_server::delivery_control::settle(&case.pool)
        .await
        .unwrap();
    let owner: Option<i64> = sqlx::query_scalar("SELECT requirement_id FROM execution_control")
        .fetch_one(&case.pool)
        .await
        .unwrap();
    assert_eq!(owner, Some(1));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM group_completion")
            .fetch_one(&case.pool)
            .await
            .unwrap(),
        0
    );
    case.close().await;
}

#[tokio::test]
async fn response_loss_validates_actual_merge_without_guessing_method_or_resending() {
    let _guard = recovery_acceptance::DATABASE_TEST.lock().await;
    let mut case = setup().await;
    let mut client = case.f.client();
    case.tick(&mut client).await;
    case.f.fail("/repos/owner/repo/pulls/1/merge", vec![500]);
    case.tick(&mut client).await;
    case.merged();
    case.tick(&mut client).await;
    let saved: Value = sqlx::query_scalar("SELECT to_jsonb(m) FROM merge_operation m")
        .fetch_one(&case.pool)
        .await
        .unwrap();
    assert_eq!(saved["merged_sha"], case.merged);
    assert_eq!(saved["state"], "blocked");
    assert!(saved["merge_method"].is_null());
    assert_eq!(
        saved["acceptance"]["evidence"]["candidate"]["sha"],
        case.merged
    );
    assert_eq!(case.state().await, "Submitted");
    case.tick(&mut client).await;
    assert_eq!(
        case.f
            .data
            .lock()
            .unwrap()
            .seen
            .iter()
            .filter(|r| r.starts_with("PUT /repos/owner/repo/pulls/1/merge"))
            .count(),
        1
    );
    case.close().await;
}
#[tokio::test]
async fn pause_and_cancel_racing_a_sent_merge_retain_fact_without_false_done() {
    let _guard = recovery_acceptance::DATABASE_TEST.lock().await;
    for cancel in [false, true] {
        let mut case = setup().await;
        let mut client = case.f.client();
        case.tick(&mut client).await;
        case.tick(&mut client).await;
        if cancel {
            codexsymphony_server::delivery_control::cancel(&case.pool, 1)
                .await
                .unwrap();
        } else {
            sqlx::query("UPDATE requirement SET paused=true WHERE id=1")
                .execute(&case.pool)
                .await
                .unwrap();
        }
        case.merged();
        case.tick(&mut client).await;
        let sha: String = sqlx::query_scalar("SELECT merged_sha FROM merge_operation")
            .fetch_one(&case.pool)
            .await
            .unwrap();
        assert_eq!(sha, case.merged);
        assert_ne!(case.state().await, "Done");
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM group_completion")
                .fetch_one(&case.pool)
                .await
                .unwrap(),
            0
        );
        if cancel {
            codexsymphony_server::delivery_control::settle(&case.pool)
                .await
                .unwrap();
            let owner: Option<i64> =
                sqlx::query_scalar("SELECT requirement_id FROM execution_control")
                    .fetch_one(&case.pool)
                    .await
                    .unwrap();
            assert_eq!(owner, None);
        } else {
            sqlx::query("UPDATE requirement SET paused=false WHERE id=1")
                .execute(&case.pool)
                .await
                .unwrap();
            case.tick(&mut client).await;
            assert_eq!(case.state().await, "Done");
        }
        case.close().await;
    }
}
#[tokio::test]
async fn failed_merged_command_keeps_original_output_and_blocks_successors() {
    let _guard = recovery_acceptance::DATABASE_TEST.lock().await;
    let mut case = setup_method("squash", true).await;
    let mut client = case.f.client();
    case.tick(&mut client).await;
    case.tick(&mut client).await;
    case.merged();
    case.tick(&mut client).await;
    let (key, state): (String, String) =
        sqlx::query_as("SELECT action_key,state FROM merge_operation")
            .fetch_one(&case.pool)
            .await
            .unwrap();
    assert_eq!(state, "blocked");
    assert_eq!(case.state().await, "Submitted");
    let dir = case
        .root
        .join("validations")
        .join(format!("post-merge-{key}"));
    let binding: Value =
        serde_json::from_slice(&std::fs::read(dir.join("binding.json")).unwrap()).unwrap();
    assert_eq!(binding["candidate"]["sha"], case.merged);
    let output = std::fs::read_to_string(dir.join("step-0.log")).unwrap();
    assert!(output.contains("actual merged checkout"));
    codexsymphony_server::delivery_control::settle(&case.pool)
        .await
        .unwrap();
    let owner: Option<i64> = sqlx::query_scalar("SELECT requirement_id FROM execution_control")
        .fetch_one(&case.pool)
        .await
        .unwrap();
    assert_eq!(owner, Some(1));
    case.close().await;
}

#[tokio::test]
async fn test_merge_is_independently_validated_and_never_relabelled_as_merged() {
    let _guard = recovery_acceptance::DATABASE_TEST.lock().await;
    let mut case = setup_source("squash", false, true).await;
    let test_sha = case.pr["merge_commit_sha"].clone();
    assert_ne!(test_sha, case.head);
    assert_ne!(test_sha, case.merged);
    let mut client = case.f.client();
    case.tick(&mut client).await;
    case.tick(&mut client).await;
    let evidence: Value = sqlx::query_scalar("SELECT pre_validation FROM merge_operation")
        .fetch_one(&case.pool)
        .await
        .unwrap();
    assert_eq!(evidence["candidate"]["sha"], test_sha);
    case.merged();
    case.tick(&mut client).await;
    assert_eq!(case.state().await, "Done");
    let evidence: Value = sqlx::query_scalar("SELECT acceptance FROM merge_operation")
        .fetch_one(&case.pool)
        .await
        .unwrap();
    assert_eq!(evidence["evidence"]["candidate"]["sha"], case.merged);
    case.close().await;
}

fn post_checks(f: &Fixture, sha: &str, conclusion: &str) {
    f.put(
        "/repos/owner/repo/actions/runs",
        json!({"workflow_runs":[]}),
    );
    let mut check = super::check(22, conclusion);
    check["head_sha"] = json!(sha);
    check["check_suite"]["id"] = json!(19);
    f.put(
        &format!("/repos/owner/repo/commits/{sha}/check-suites"),
        json!({"check_suites":[{"id":19}]}),
    );
    f.put(
        "/repos/owner/repo/check-suites/19/check-runs",
        json!({"check_runs":[check]}),
    );
    let mut run = super::run();
    run["id"] = json!(11);
    run["head_sha"] = json!(sha);
    run["event"] = json!("workflow_dispatch");
    run["head_branch"] = json!("main");
    run["check_suite_id"] = json!(19);
    f.put(
        &format!("/repos/owner/repo/actions/runs?head_sha={sha}&per_page=100&page=1"),
        json!({"workflow_runs":[run]}),
    );
    let mut job = super::job();
    job["id"] = json!(21);
    job["run_id"] = json!(11);
    job["name"] = json!("build");
    job["check_run_url"] = json!("https://api.github.com/repos/owner/repo/check-runs/22");
    f.put(
        "/repos/owner/repo/actions/runs/11/attempts/2/jobs",
        json!({"jobs":[job]}),
    );
}

#[tokio::test]
async fn post_checks_dispatch_once_then_validate_actual_merged_source() {
    let _guard = recovery_acceptance::DATABASE_TEST.lock().await;
    let mut case = setup_contract("squash", false, false, true).await;
    let mut client = case.f.client();
    case.tick(&mut client).await;
    case.tick(&mut client).await;
    case.merged();
    case.f.put(
        &format!(
            "/repos/owner/repo/actions/runs?head_sha={}&per_page=100&page=1",
            case.merged
        ),
        json!({"workflow_runs":[]}),
    );
    for _ in 0..2 {
        case.tick(&mut client).await;
    }
    assert_eq!(case.state().await, "Submitted");
    assert_eq!(
        case.f
            .data
            .lock()
            .unwrap()
            .seen
            .iter()
            .filter(|r| r.starts_with("POST /repos/owner/repo/actions/workflows/8/dispatches"))
            .count(),
        1
    );
    post_checks(&case.f, &case.merged, "success");
    case.tick(&mut client).await;
    assert_eq!(case.state().await, "Done");
    case.close().await;
}

#[tokio::test]
async fn post_checks_failure_and_timeout_keep_owner_and_block_successors() {
    let _guard = recovery_acceptance::DATABASE_TEST.lock().await;
    for timeout in [false, true] {
        let mut case = setup_contract("squash", false, false, true).await;
        let mut client = case.f.client();
        case.tick(&mut client).await;
        case.tick(&mut client).await;
        case.merged();
        if timeout {
            case.f.put(
                &format!(
                    "/repos/owner/repo/actions/runs?head_sha={}&per_page=100&page=1",
                    case.merged
                ),
                json!({"workflow_runs":[]}),
            );
            sqlx::query("UPDATE merge_operation SET merged_at=1,created_at=1")
                .execute(&case.pool)
                .await
                .unwrap();
        } else {
            post_checks(&case.f, &case.merged, "failure");
        }
        case.tick(&mut client).await;
        assert_eq!(case.state().await, "Submitted");
        let (state, reason): (String, String) =
            sqlx::query_as("SELECT state,blocker FROM merge_operation")
                .fetch_one(&case.pool)
                .await
                .unwrap();
        assert_eq!(state, "blocked", "{reason}");
        assert!(
            reason.contains(if timeout {
                "deadline"
            } else {
                "required check failed"
            }),
            "{reason}"
        );
        case.close().await;
    }
}

#[tokio::test]
async fn transient_post_observation_retries_and_validation_honors_pause() {
    let _guard = recovery_acceptance::DATABASE_TEST.lock().await;
    let mut case = setup().await;
    let mut client = case.f.client();
    case.tick(&mut client).await;
    case.tick(&mut client).await;
    case.merged();
    // The merge response is already persisted. Skip the reconciliation worker
    // so the error is exercised specifically by post-merge observation.
    sqlx::query("UPDATE requirement SET paused=true WHERE id=1")
        .execute(&case.pool)
        .await
        .unwrap();
    case.tick(&mut client).await;
    sqlx::query("UPDATE requirement SET paused=false WHERE id=1")
        .execute(&case.pool)
        .await
        .unwrap();
    case.f.fail("/repos/owner/repo/pulls/1", vec![503]);
    case.tick(&mut client).await;
    let receipts: Value = sqlx::query_scalar("SELECT receipts FROM merge_operation")
        .fetch_one(&case.pool)
        .await
        .unwrap();
    assert!(
        receipts
            .to_string()
            .contains("post_merge_observation_error")
    );
    assert_eq!(case.state().await, "Submitted");
    sqlx::query("UPDATE requirement SET paused=true WHERE id=1")
        .execute(&case.pool)
        .await
        .unwrap();
    case.tick(&mut client).await;
    assert_eq!(case.state().await, "Submitted");
    sqlx::query("UPDATE requirement SET paused=false WHERE id=1")
        .execute(&case.pool)
        .await
        .unwrap();
    case.tick(&mut client).await;
    assert_eq!(case.state().await, "Done");
    case.close().await;
}

#[tokio::test]
async fn pause_during_actual_merged_validation_stops_process_and_retains_evidence() {
    let _guard = recovery_acceptance::DATABASE_TEST.lock().await;
    let mut case = setup_variant("squash", false, false, false, Some("timeout")).await;
    let mut client = case.f.client();
    case.tick(&mut client).await;
    case.tick(&mut client).await;
    case.merged();
    let pool = case.pool.clone();
    let pause = tokio::spawn(async move {
        for _ in 0..100 {
            let started: bool =
                sqlx::query_scalar("SELECT acceptance_started FROM merge_operation")
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            if started {
                tokio::time::sleep(std::time::Duration::from_millis(350)).await;
                sqlx::query("UPDATE requirement SET paused=true WHERE id=1")
                    .execute(&pool)
                    .await
                    .unwrap();
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!("acceptance never started");
    });
    case.tick(&mut client).await;
    pause.await.unwrap();
    assert_eq!(case.state().await, "Submitted");
    let (key, state): (String, String) =
        sqlx::query_as("SELECT action_key,state FROM merge_operation")
            .fetch_one(&case.pool)
            .await
            .unwrap();
    assert_eq!(state, "blocked");
    let directory = case
        .root
        .join("validations")
        .join(format!("post-merge-{key}"));
    assert!(directory.join("binding.json").exists());
    assert!(!directory.join("result.json").exists());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM group_completion")
            .fetch_one(&case.pool)
            .await
            .unwrap(),
        0
    );
    case.close().await;
}

#[tokio::test]
async fn test_merge_missing_plan_blocks_before_merge_and_preserves_owner() {
    let _guard = recovery_acceptance::DATABASE_TEST.lock().await;
    let case = setup_source("squash", false, true).await;
    let mut client = case.f.client();
    case.tick(&mut client).await;
    sqlx::query("UPDATE candidate_validation SET approved_plan=NULL")
        .execute(&case.pool)
        .await
        .unwrap();
    case.tick(&mut client).await;
    let (state, blocker, evidence): (String, String, Option<Value>) =
        sqlx::query_as("SELECT state,blocker,pre_validation FROM merge_operation")
            .fetch_one(&case.pool)
            .await
            .unwrap();
    assert_eq!(state, "blocked");
    assert!(
        blocker.starts_with("pre_merge checkout validation blocked:"),
        "{blocker}"
    );
    assert!(evidence.is_none());
    assert!(
        !case
            .f
            .data
            .lock()
            .unwrap()
            .seen
            .iter()
            .any(|r| r.starts_with("PUT /repos/owner/repo/pulls/1/merge"))
    );
    let owner: Option<i64> = sqlx::query_scalar("SELECT requirement_id FROM execution_control")
        .fetch_one(&case.pool)
        .await
        .unwrap();
    assert_eq!(owner, Some(1));
    case.close().await;
}

#[tokio::test]
async fn dispatch_branch_movement_and_failed_send_cannot_repeat_a_write() {
    let _guard = recovery_acceptance::DATABASE_TEST.lock().await;
    for moved in [true, false] {
        let mut case = setup_contract("squash", false, false, true).await;
        let mut client = case.f.client();
        case.tick(&mut client).await;
        case.tick(&mut client).await;
        case.merged();
        case.f.put(
            &format!(
                "/repos/owner/repo/actions/runs?head_sha={}&per_page=100&page=1",
                case.merged
            ),
            json!({"workflow_runs":[]}),
        );
        if moved {
            case.f.put(
                "/repos/owner/repo/git/ref/heads/%6D%61%69%6E",
                json!({"object":{"sha":"f".repeat(40)}}),
            );
        } else {
            case.f.fail(
                "/repos/owner/repo/actions/workflows/8/dispatches",
                vec![503],
            );
        }
        case.tick(&mut client).await;
        case.tick(&mut client).await;
        let sent = case
            .f
            .data
            .lock()
            .unwrap()
            .seen
            .iter()
            .filter(|r| r.starts_with("POST /repos/owner/repo/actions/workflows/8/dispatches"))
            .count();
        assert_eq!(sent, usize::from(!moved));
        assert_ne!(case.state().await, "Done");
        case.close().await;
    }
}
