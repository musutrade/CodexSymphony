//! Protocol acceptance uses fixed synthetic completion fixtures, not M3 acceptance.
use codexsymphony_server::{
    budget::{Amount, Purpose, Usage},
    budget_store, delivery_control,
    git_broker::GitBroker,
    group_completion::{self, Fact, Verifier},
    group_queue_store as queue, run_store, runtime_initial,
};
use serde_json::{Value, json};
use sqlx::PgPool;
#[path = "support/groups.rs"]
mod groups;
#[path = "support/validation_runner.rs"]
mod source;
use groups::*;

async fn authorized(pool: &PgPool, key: &str) -> String {
    authorized_document(pool, key, sample()).await
}
async fn authorized_document(pool: &PgPool, key: &str, document: Value) -> String {
    authorized_review(pool, key, document, review()).await
}
async fn authorized_review(pool: &PgPool, key: &str, document: Value, review: Value) -> String {
    let router = app(pool);
    let draft = request(&router, "POST", "/api/drafts", body(document, 0), 200).await;
    let id = draft["id"].as_str().unwrap().to_owned();
    request(
        &router,
        "PUT",
        &format!("/api/drafts/{id}/review"),
        json!({"version":0,"draft_revision":1,"review":review}),
        200,
    )
    .await;
    request(
        &router,
        "POST",
        &format!("/api/drafts/{id}/authorize"),
        json!({"version":1,"draft_revision":1,"request_id":key}),
        200,
    )
    .await;
    id
}
async fn bootstrap(pool: &PgPool) {
    sqlx::query("INSERT INTO github_repository(repository_id,repository_version,policy,probe_pr,capability,checked_at,stale) VALUES($1,1,'{}',1,'{\"policy\":{},\"blockers\":[]}',extract(epoch FROM now())::bigint,false)")
        .bind(repository()["github_repository_id"].as_i64().unwrap()).execute(pool).await.unwrap();
    run_store::begin_incarnation(pool, "boot").await.unwrap();
    sqlx::query("UPDATE execution_control SET recovery_complete=true")
        .execute(pool)
        .await
        .unwrap();
}
fn broker() -> (std::path::PathBuf, GitBroker, String) {
    let (root, repo, _) = source::fixture();
    std::fs::write(repo.join("source"), root.to_string_lossy().as_bytes()).unwrap();
    assert!(
        std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["commit", "-am", "unique synthetic baseline"])
            .output()
            .unwrap()
            .status
            .success()
    );
    let baseline = codexsymphony_server::validation_runner::candidate(&repo)
        .unwrap()
        .sha;
    let bundle = root.join("seed.bundle");
    assert!(
        std::process::Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["bundle", "create"])
            .arg(&bundle)
            .arg("--all")
            .status()
            .unwrap()
            .success()
    );
    let broker = GitBroker::initialize(&root.join("workspaces"), &bundle).unwrap();
    (root, broker, baseline)
}
async fn plan(
    pool: &PgPool,
    broker: &GitBroker,
    base: &str,
) -> Option<(
    codexsymphony_server::execution::Launch,
    codexsymphony_server::workspace::Workspace,
)> {
    let incarnation: String =
        sqlx::query_scalar("SELECT incarnation FROM execution_control WHERE id=1")
            .fetch_one(pool)
            .await
            .unwrap();
    runtime_initial::plan(pool, broker, &incarnation, &["/usr/bin/codex".into()], base)
        .await
        .unwrap()
}
async fn prepared(pool: &PgPool, launch: &codexsymphony_server::execution::Launch, id: i64) {
    std::fs::create_dir_all(&launch.workspace).unwrap();
    sqlx::query("INSERT INTO preparation_record(run_id,requirement_id,revision,launch,retry,ready,checked_at) VALUES($1,$2,1,$3,'{}',true,extract(epoch FROM now())::bigint)")
        .bind(&launch.key.run_id).bind(id).bind(json!(launch)).execute(pool).await.unwrap();
}
async fn count(pool: &PgPool, table: &str) -> i64 {
    sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
        .fetch_one(pool)
        .await
        .unwrap()
}
async fn owner(pool: &PgPool) -> Option<i64> {
    sqlx::query_scalar("SELECT requirement_id FROM execution_control WHERE id=1")
        .fetch_one(pool)
        .await
        .unwrap()
}
struct FixtureVerifier(Fact);
impl Verifier for FixtureVerifier {
    fn verify(&self, fact: &Fact) -> bool {
        self.0 == *fact && fact.source == "fixture:confirmed-acceptance"
    }
}

#[tokio::test]
async fn global_claim_restart_budget_and_completion_protocol() {
    let (pool, url, _) = fixture().await;
    bootstrap(&pool).await;
    let draft = authorized(&pool, "first").await;
    let _other = authorized(&pool, "second").await;
    queue::materialize(&pool).await.unwrap();
    queue::materialize(&pool).await.unwrap();
    assert_eq!(count(&pool, "requirement").await, 6);
    assert_eq!(count(&pool, "group_execution_item").await, 8);
    assert_eq!(owner(&pool).await, None);
    let (root, broker, base) = broker();
    sqlx::raw_sql("CREATE FUNCTION fail_plan() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected initial intent failure'; END $$; CREATE TRIGGER fail_plan BEFORE INSERT ON initial_run FOR EACH ROW EXECUTE FUNCTION fail_plan();").execute(&pool).await.unwrap();
    assert!(
        runtime_initial::plan(&pool, &broker, "boot", &["/usr/bin/codex".into()], &base)
            .await
            .is_err()
    );
    assert_eq!(count(&pool, "group_claim_input").await, 0);
    assert_eq!(count(&pool, "initial_run").await, 0);
    sqlx::query("DROP TRIGGER fail_plan ON initial_run")
        .execute(&pool)
        .await
        .unwrap();
    let (launch, workspace) = plan(&pool, &broker, &base).await.unwrap();
    assert_eq!(workspace.requirement, 1);
    let saved = plan(&pool, &broker, &base).await.unwrap();
    assert_eq!(saved.0.key, launch.key);
    prepared(&pool, &launch, 1).await;
    // Roll back after Run insert and before owner/state commit.
    sqlx::raw_sql("CREATE FUNCTION fail_claim() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected state persistence failure'; END $$; CREATE TRIGGER fail_claim BEFORE UPDATE ON requirement FOR EACH ROW EXECUTE FUNCTION fail_claim();").execute(&pool).await.unwrap();
    assert!(run_store::reserve_prepared(&pool, &launch).await.is_err());
    assert_eq!(count(&pool, "agent_run").await, 0);
    assert_eq!(owner(&pool).await, None);
    sqlx::query("DROP TRIGGER fail_claim ON requirement")
        .execute(&pool)
        .await
        .unwrap();
    let (one, two) = tokio::join!(
        run_store::reserve_prepared(&pool, &launch),
        run_store::reserve_prepared(&pool, &launch)
    );
    assert_ne!(one.unwrap(), two.unwrap());
    assert_eq!(owner(&pool).await, Some(1));
    assert_eq!(count(&pool, "agent_run").await, 1);
    assert!(plan(&pool, &broker, &base).await.is_none());
    let intent = budget_store::CallIntent {
        key: launch.key.clone(),
        turn_id: "turn".into(),
        purpose: Purpose::Coding,
        reserve: Amount {
            tokens: 50,
            turns: 1,
            model_seconds: 10,
        },
    };
    assert_eq!(
        budget_store::reserve(&pool, &intent).await.unwrap(),
        budget_store::Admission::Reserved
    );
    assert_eq!(
        budget_store::reserve(&pool, &intent).await.unwrap(),
        budget_store::Admission::Existing
    );
    budget_store::settle(
        &pool,
        &launch.key,
        "turn",
        "end",
        &Usage {
            input: Some(5),
            cached: Some(0),
            output: Some(5),
            model_seconds: Some(1),
            complete: true,
        },
    )
    .await
    .unwrap();
    let used: i64 = sqlx::query_scalar(
        "SELECT (used->>'tokens')::bigint FROM group_budget WHERE draft_id=$1 AND item_id=''",
    )
    .bind(&draft)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(used, 10);
    run_store::pause(&pool, Some(1)).await.unwrap();
    let reopened = sqlx::postgres::PgPoolOptions::new()
        .connect(&url)
        .await
        .unwrap();
    assert_eq!(owner(&reopened).await, Some(1));
    assert!(
        !run_store::reserve_prepared(&reopened, &launch)
            .await
            .unwrap()
    );
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT paused FROM requirement WHERE id=1")
            .fetch_one(&reopened)
            .await
            .unwrap()
    );
    let fact = merged_fixture(&pool, &launch, &base).await;
    run_store::begin_incarnation(&reopened, "restarted")
        .await
        .unwrap();
    assert!(
        run_store::finish_recovery(&reopened, "restarted")
            .await
            .unwrap()
    );
    assert!(plan(&reopened, &broker, &base).await.is_none());
    assert_eq!(count(&reopened, "agent_run").await, 1);
    // Merge without an applicable acceptance verifier retains the owner.
    delivery_control::settle(&pool).await.unwrap();
    assert_eq!(owner(&pool).await, Some(1));
    let mut invalid = fact.clone();
    invalid.acceptance_sha = "0".repeat(40);
    assert!(
        group_completion::record(&pool, &FixtureVerifier(fact.clone()), &invalid)
            .await
            .is_err()
    );
    assert!(
        group_completion::record(&pool, &FixtureVerifier(invalid), &fact)
            .await
            .is_err()
    );
    for (pointer, value) in [
        ("/child_revision", json!(2)),
        ("/repository_id", json!(2)),
        ("/github_repository_id", json!(999)),
        ("/acceptance_plan", json!([{"wrong":"plan"}])),
        ("/acceptance_plan", json!([])),
        ("/acceptance_plan", json!(null)),
        ("/head_sha", json!("f".repeat(40))),
    ] {
        let mut value_fact = json!(fact);
        *value_fact.pointer_mut(pointer).unwrap() = value;
        let wrong: Fact = serde_json::from_value(value_fact).unwrap();
        assert!(
            group_completion::record(&pool, &FixtureVerifier(wrong.clone()), &wrong)
                .await
                .is_err()
        );
    }
    sqlx::query("UPDATE github_pr SET observation=jsonb_set(observation,'{merge}','\"Unmerged\"')")
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        group_completion::record(&pool, &FixtureVerifier(fact.clone()), &fact)
            .await
            .is_err()
    );
    delivery_control::settle(&pool).await.unwrap();
    assert_eq!(owner(&pool).await, Some(1));
    sqlx::query("UPDATE github_pr SET observation=jsonb_set(observation,'{merge}','\"Merged\"')")
        .execute(&pool)
        .await
        .unwrap();
    group_completion::record(&pool, &FixtureVerifier(fact.clone()), &fact)
        .await
        .unwrap();
    group_completion::record(&pool, &FixtureVerifier(fact.clone()), &fact)
        .await
        .unwrap();
    delivery_control::settle(&pool).await.unwrap();
    assert_eq!(owner(&pool).await, Some(1)); // pause still authoritative
    assert!(delivery_control::resume(&pool, Some(1)).await.unwrap());
    delivery_control::settle(&pool).await.unwrap();
    assert_eq!(owner(&pool).await, None);
    assert!(plan(&pool, &broker, &"0".repeat(40)).await.is_none());
    assert_eq!(count(&pool, "initial_run").await, 1);
    let next = plan(&pool, &broker, &base).await.unwrap();
    assert_eq!(next.1.requirement, 2);
    let dependencies: Value =
        sqlx::query_scalar("SELECT dependencies FROM group_claim_input WHERE requirement_id=2")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(dependencies, json!([fact]));
    request(
        &app(&pool),
        "POST",
        "/api/multi/requirements/2/withdraw",
        json!({"request_id":"bypass","version":1,"repository_version":1}),
        409,
    )
    .await;
    let view = request(
        &app(&pool),
        "GET",
        &format!("/api/drafts/{draft}/review"),
        json!({}),
        200,
    )
    .await;
    assert_eq!(view["execution"]["completed"], 1);
    assert_eq!(
        view["execution"]["items"][3]["waiting_reason"],
        "waiting_validation_only_execution_not_implemented"
    );
    assert_eq!(view["business_complete"], false);
    delivery_control::cancel(&pool, 1).await.unwrap();
    let cancelled = request(
        &app(&pool),
        "GET",
        &format!("/api/drafts/{draft}/review"),
        json!({}),
        200,
    )
    .await;
    assert_eq!(cancelled["execution"]["completed"], 0);
    assert_eq!(
        cancelled["execution"]["items"][0]["waiting_reason"],
        "cancelled_not_success"
    );

    assert_eq!(count(&pool, "agent_run").await, 1);
    reopened.close().await;
    pool.close().await;
    std::fs::remove_dir_all(root).unwrap();
}
async fn merged_fixture(
    pool: &PgPool,
    launch: &codexsymphony_server::execution::Launch,
    base: &str,
) -> Fact {
    sqlx::query("UPDATE agent_run SET quiescent=true,state='Succeeded' WHERE id=$1")
        .bind(&launch.key.run_id)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("UPDATE requirement SET state='Submitted' WHERE id=1")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO candidate_validation(id,requirement_id,revision,source_run_id,candidate_sha,candidate_tree,trusted,required_steps,source_before,source_after,entry_before,entry_after,stage,result) VALUES('fixture',1,1,$1,$2,'tree','{}','[]','tree','tree','entry','entry','done','succeeded')")
        .bind(&launch.key.run_id).bind(base).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO workspace_snapshot(run_id,manifest,candidate) VALUES($1,$2,true)")
        .bind(&launch.key.run_id)
        .bind(json!({"head":base,"workspace":{"path":launch.workspace}}))
        .execute(pool)
        .await
        .unwrap();
    let github = repository()["github_repository_id"].as_i64().unwrap();
    sqlx::query("INSERT INTO delivery(action_key,validation_id,requirement_id,revision,repository_id,repository,branch,base_branch,head_sha,manifest,policy,pr_number) VALUES('fixture','fixture',1,1,$1,'owner/repo','ai/fixture','main',$2,'{}','{}',17)").bind(github).bind(base).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO github_pr(repository_id,number,requirement_id,observation,last_synced_at,stale) VALUES($1,17,1,$2,extract(epoch FROM now())::bigint,false)").bind(github).bind(json!({"merge":"Merged","merged_sha":base,"head":base,"head_ref":"ai/fixture","base_ref":"main"})).execute(pool).await.unwrap();
    let authorization: i64 = sqlx::query_scalar(
        "SELECT authorization_id FROM group_execution_item WHERE requirement_id=1",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    Fact {
        requirement_id: 1,
        authorization_id: authorization,
        child_revision: 1,
        repository_id: 1,
        github_repository_id: github,
        pr_number: 17,
        head_sha: base.into(),
        merged_sha: base.into(),
        acceptance_sha: base.into(),
        acceptance_plan: review()["items"][0]["verification"].clone(),
        source: "fixture:confirmed-acceptance".into(),
        evidence_sha256: "a".repeat(64),
        artifact: "fixture://protocol-only/not-m3".into(),
    }
}

#[tokio::test]
async fn unauthorized_stale_unavailable_cancelled_and_unsupported_never_claim() {
    let (pool, _, _) = fixture().await;
    bootstrap(&pool).await;
    let draft = authorized(&pool, "reject").await;
    queue::materialize(&pool).await.unwrap();
    let (root, broker, base) = broker();
    for mutation in [
        "UPDATE group_queue SET state='needs_review'",
        "UPDATE imported_draft SET version=2",
        "UPDATE repository SET document=jsonb_set(document,'{revoked}','true')",
        "UPDATE github_repository SET stale=true",
        "UPDATE requirement SET paused=true WHERE id=1",
        "UPDATE requirement SET state='Cancelled',cancel_requested=true WHERE id=1",
        "UPDATE requirement SET state='Failed' WHERE id=1",
    ] {
        let mut tx = pool.begin().await.unwrap();
        // Persist each negative input, then explicitly restore fixture state.
        sqlx::query(mutation).execute(&mut *tx).await.unwrap();
        tx.commit().await.unwrap();
        assert!(plan(&pool, &broker, &base).await.is_none(), "{mutation}");
        sqlx::raw_sql("UPDATE group_queue SET state='waiting_scheduler'; UPDATE imported_draft SET version=1; UPDATE repository SET document=jsonb_set(document,'{revoked}','false'); UPDATE github_repository SET stale=false; UPDATE requirement SET state='Ready',paused=false,cancel_requested=false;").execute(&pool).await.unwrap();
    }
    assert_eq!(count(&pool, "agent_run").await, 0);
    // A dependency with no completion cannot be skipped just by changing order.
    sqlx::query("UPDATE group_execution_item SET input=jsonb_set(input,'{child,depends_on}','[\"C2\"]') WHERE requirement_id=1").execute(&pool).await.unwrap();
    assert!(plan(&pool, &broker, &base).await.is_none());
    let view = request(
        &app(&pool),
        "GET",
        &format!("/api/drafts/{draft}/review"),
        json!({}),
        200,
    )
    .await;
    assert_eq!(
        view["execution"]["items"][0]["waiting_reason"],
        "waiting_dependency_completion"
    );
    assert_eq!(count(&pool, "initial_run").await, 0);
    pool.close().await;
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn cross_repository_versions_do_not_use_git_ancestry_and_share_legacy_owner() {
    let (pool, _, _) = fixture().await;
    bootstrap(&pool).await;
    let mut repo = repository();
    repo["github_repository_id"] = json!(124);
    repo["remote"] = json!("test/other");
    sqlx::query("INSERT INTO repository(id,version,document) VALUES(2,1,$1)")
        .bind(repo)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO github_repository(repository_id,repository_version,policy,probe_pr,capability,checked_at,stale) VALUES(124,1,'{}',1,'{\"policy\":{},\"blockers\":[]}',extract(epoch FROM now())::bigint,false)").execute(&pool).await.unwrap();
    let mut document = sample();
    document["children"][1]["repository_id"] = json!(2);
    authorized_document(&pool, "cross", document).await;
    authorized(&pool, "other-group").await;
    queue::materialize(&pool).await.unwrap();
    let contract = json!({"title":"legacy independent","description":"synthetic regression","acceptance_criteria":[{"description":"works","verification_ref":"test"}],"validation_plan":[{"id":"test","check":"cargo_test","selector":"group_queue","expected_result":"exit 0","timeout_seconds":10}],"network_access":[]});
    let old = request(
        &app(&pool),
        "POST",
        "/api/multi/requirements",
        json!({"repository_id":1,"version":0,"request_id":"legacy-create","contract":contract}),
        201,
    )
    .await;
    let old_id = old["id"].as_i64().unwrap();
    request(
        &app(&pool),
        "POST",
        &format!("/api/multi/requirements/{old_id}/ready"),
        json!({"repository_version":1,"version":1,"request_id":"legacy-review"}),
        200,
    )
    .await;
    let (root, broker, base) = broker();
    let (launch, _) = plan(&pool, &broker, &base).await.unwrap();
    prepared(&pool, &launch, 1).await;
    assert!(run_store::reserve_prepared(&pool, &launch).await.unwrap());
    let fact = merged_fixture(&pool, &launch, &base).await;
    group_completion::record(&pool, &FixtureVerifier(fact.clone()), &fact)
        .await
        .unwrap();
    delivery_control::settle(&pool).await.unwrap();
    let (other_root, other_broker, other_base) = self::broker();
    assert_ne!(base, other_base);
    assert!(!other_broker.contains_commit(&other_base, &base));
    let (second, workspace) = plan(&pool, &other_broker, &other_base).await.unwrap();
    assert_eq!(workspace.requirement, 2);
    prepared(&pool, &second, 2).await;
    let (a, b, c) = tokio::join!(
        run_store::reserve_prepared(&pool, &second),
        run_store::reserve_prepared(&pool, &second),
        run_store::reserve_prepared(&pool, &launch)
    );
    assert_eq!(
        usize::from(a.unwrap()) + usize::from(b.unwrap()) + usize::from(c.unwrap()),
        1
    );
    assert_eq!(owner(&pool).await, Some(2));
    assert!(
        runtime_initial::plan_selected(
            &pool,
            &broker,
            "boot",
            &["/usr/bin/codex".into()],
            &base,
            Some((old_id, 1))
        )
        .await
        .unwrap()
        .is_none()
    );
    let bound: Value =
        sqlx::query_scalar("SELECT dependencies FROM group_claim_input WHERE requirement_id=2")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(bound[0]["artifact"], "fixture://protocol-only/not-m3");
    assert_eq!(bound[0]["merged_sha"], base);
    assert_eq!(count(&pool, "agent_run").await, 2);
    pool.close().await;
    std::fs::remove_dir_all(root).unwrap();
    std::fs::remove_dir_all(other_root).unwrap();
}

#[tokio::test]
async fn group_ceiling_inflight_reservations_and_late_usage_survive_restart() {
    let (pool, url, _) = fixture().await;
    bootstrap(&pool).await;
    let mut review = review();
    review["group_budget"] = json!({"tokens":60,"turns":3,"model_seconds":30});
    let draft = authorized_review(&pool, "ceiling", sample(), review).await;
    queue::materialize(&pool).await.unwrap();
    let (root, broker, base) = broker();
    let (launch, _) = plan(&pool, &broker, &base).await.unwrap();
    prepared(&pool, &launch, 1).await;
    assert!(run_store::reserve_prepared(&pool, &launch).await.unwrap());
    let intent = budget_store::CallIntent {
        key: launch.key.clone(),
        turn_id: "first".into(),
        purpose: Purpose::Coding,
        reserve: Amount {
            tokens: 60,
            turns: 1,
            model_seconds: 10,
        },
    };
    // Synthetic persisted-accounting fault: reject without reserving a call
    // or releasing ownership, then restore the fixture for normal settlement.
    for dimension in ["tokens", "turns", "model_seconds"] {
        let mut used = json!(Amount::default());
        let mut reserved = json!(Amount::default());
        used[dimension] = json!(i64::MAX);
        reserved[dimension] = json!(1);
        sqlx::query("UPDATE group_budget SET used=$2,reserved=$3 WHERE draft_id=$1 AND item_id=''")
            .bind(&draft)
            .bind(used)
            .bind(reserved)
            .execute(&pool)
            .await
            .unwrap();
        let error = budget_store::reserve(&pool, &intent).await.unwrap_err();
        assert!(
            matches!(error, sqlx::Error::Protocol(ref message) if message == "group accounting overflow")
        );
        assert_eq!(count(&pool, "model_call").await, 0);
        assert_eq!(owner(&pool).await, Some(1));
    }
    sqlx::query("UPDATE group_budget SET used=$2,reserved=$2 WHERE draft_id=$1 AND item_id=''")
        .bind(&draft)
        .bind(json!(Amount::default()))
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        budget_store::reserve(&pool, &intent).await.unwrap(),
        budget_store::Admission::Reserved
    );
    budget_store::settle(
        &pool,
        &launch.key,
        "first",
        "partial",
        &Usage {
            input: Some(1),
            ..Usage::default()
        },
    )
    .await
    .unwrap();
    assert!(
        !sqlx::query_scalar::<_, bool>(
            "SELECT stop_requested FROM agent_run WHERE requirement_id=1"
        )
        .fetch_one(&pool)
        .await
        .unwrap()
    );
    let mut next = intent.clone();
    next.turn_id = "second".into();
    next.reserve.tokens = 1;
    assert_eq!(
        budget_store::reserve(&pool, &next).await.unwrap(),
        budget_store::Admission::Blocked
    );
    assert_eq!(owner(&pool).await, Some(1));
    let reopened = sqlx::postgres::PgPoolOptions::new()
        .connect(&url)
        .await
        .unwrap();
    let exposure:Value=sqlx::query_scalar("SELECT jsonb_build_object('used',used,'reserved',reserved) FROM group_budget WHERE draft_id=$1 AND item_id=''").bind(&draft).fetch_one(&reopened).await.unwrap();
    assert_eq!(exposure["used"]["tokens"], 1);
    assert_eq!(exposure["reserved"]["tokens"], 59);
    budget_store::settle(
        &reopened,
        &launch.key,
        "first",
        "late",
        &Usage {
            input: Some(61),
            output: Some(0),
            cached: Some(0),
            model_seconds: Some(1),
            complete: true,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM requirement_budget WHERE exhausted")
            .fetch_one(&reopened)
            .await
            .unwrap(),
        3
    );
    let increased = budget_store::Increase {
        request_id: "no-bypass".into(),
        requirement_id: 1,
        expected_version: 1,
        actor: "fixture".into(),
        reason: "group review required".into(),
        delta: Amount {
            tokens: 1,
            turns: 1,
            model_seconds: 1,
        },
    };
    assert!(budget_store::increase(&reopened, &increased).await.is_err());
    assert_eq!(count(&pool, "model_call").await, 1);
    pool.close().await;
    reopened.close().await;
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn legacy_same_transaction_queue_keeps_numeric_order() {
    let (pool, _, _) = fixture().await;
    bootstrap(&pool).await;
    sqlx::raw_sql("INSERT INTO requirement(version,state,contract,revision) SELECT 1,'Ready','{}',1 FROM generate_series(1,12); INSERT INTO requirement_revision SELECT id,1,'{\"repository_version\":1}' FROM requirement; UPDATE requirement SET state='Submitted' WHERE id=1;").execute(&pool).await.unwrap();
    let (root, broker, base) = broker();
    assert_eq!(plan(&pool, &broker, &base).await.unwrap().1.requirement, 2);
    pool.close().await;
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn queue_wait_reasons_and_bound_review_conflict_preserve_authorization() {
    let (pool, _, _) = fixture().await;
    bootstrap(&pool).await;
    let draft = authorized(&pool, "view-reasons").await;
    queue::materialize(&pool).await.unwrap();
    let router = app(&pool);
    let path = format!("/api/drafts/{draft}/review");
    let before = request(&router, "GET", &path, json!({}), 200).await;
    assert_eq!(
        before["execution"]["items"][0]["waiting_reason"],
        "waiting_repository_baseline_or_preparation"
    );
    request(
        &router,
        "PUT",
        &path,
        json!({"version":1,"draft_revision":1,"review":review()}),
        409,
    )
    .await;
    let after = request(&router, "GET", &path, json!({}), 200).await;
    assert_eq!(before, after);
    assert_eq!(count(&pool, "group_review_revision").await, 1);
    assert_eq!(count(&pool, "group_authorization").await, 1);
    for (mutation, expected) in [
        (
            "UPDATE group_queue SET state='needs_review'",
            "needs_review",
        ),
        (
            "UPDATE requirement SET state='Submitted' WHERE id=1",
            "waiting_confirmed_merge_and_applicable_acceptance",
        ),
        (
            "UPDATE requirement SET state='Failed' WHERE id=1",
            "occupied_execution_or_blocker",
        ),
        (
            "UPDATE github_repository SET stale=true",
            "repository_unavailable",
        ),
        (
            "INSERT INTO requirement(version,state,contract,revision,created_at) VALUES(1,'Ready','{}',1,now()-interval '1 day')",
            "waiting_queue_order",
        ),
    ] {
        sqlx::query(mutation).execute(&pool).await.unwrap();
        let view = request(&router, "GET", &path, json!({}), 200).await;
        assert_eq!(
            view["execution"]["items"][0]["waiting_reason"], expected,
            "{mutation}"
        );
        assert_eq!(view["execution"]["completed"], 0);
        assert_eq!(view["business_complete"], false);
        sqlx::raw_sql("UPDATE group_queue SET state='waiting_scheduler'; UPDATE requirement SET state='Ready'; UPDATE github_repository SET stale=false;").execute(&pool).await.unwrap();
    }
    assert_eq!(count(&pool, "agent_run").await, 0);
    assert_eq!(owner(&pool).await, None);
    pool.close().await;
}
