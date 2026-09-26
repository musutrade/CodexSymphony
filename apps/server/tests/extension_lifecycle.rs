use codexsymphony_server::{
    controlled_contract::Verdict, extension_feedback as feedback, extension_recovery as recovery,
    validation::*, validation_runner, validation_service,
};
use serde_json::{Value, json};
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{fs, path::Path, process::Command};
#[allow(dead_code)]
#[path = "support/automatic_merge.rs"]
mod delivery_fixture;
use delivery_fixture::source as runner;

async fn database() -> PgPool {
    let options: PgConnectOptions = std::env::var("TEST_DATABASE_URL").unwrap().parse().unwrap();
    let admin = PgPoolOptions::new()
        .connect_with(options.clone())
        .await
        .unwrap();
    let schema = format!(
        "extension_{}",
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
fn step(value: Value) -> StepEvidence {
    let output = value.to_string();
    StepEvidence {
        id: "test".into(),
        command: vec!["/gate-entry".into(), feedback::SELECTOR.into()],
        exit_code: Some(0),
        output_sha256: sha256(&output),
        output,
        log_ref: "controlled/log".into(),
        consumer: "validation".into(),
        code_failure: true,
    }
}
fn unsupported() -> Value {
    json!({"protocol_version":1,"check_id":"test","verdict":"unknown","fault":{"class":"unsupported","code":"fixture.closure","message":"Closure projection cannot be measured","owner":"gate_maintainer","scope":["source"],"resume_condition":"Approved collector supports projection closures"}})
}
fn constraint() -> codexsymphony_server::development_constraints::Constraint {
    serde_json::from_value(json!({"id":"no-closure","version":"1","source":"reviewed project convention","reason":"collector limitation","instruction":"Use ordinary named functions in changed production functions","paths":["source"],"code_scope":"changed_production","release_condition":"Approved collector regression and complete Gate pass"})).unwrap()
}

#[test]
fn feedback_distinguishes_business_failure_capability_and_execution() {
    for (word, verdict) in [("pass", Verdict::Pass), ("fail", Verdict::Fail)] {
        let s = step(json!({"protocol_version":1,"check_id":"test","verdict":word,"fault":null}));
        assert_eq!(feedback::verdict(&s), verdict);
        assert_eq!(feedback::code_failure(&s), word == "fail");
        let mut crash = s.clone();
        crash.exit_code = None;
        assert_eq!(feedback::verdict(&crash), Verdict::Unknown);
    }
    let s = step(unsupported());
    assert_eq!(feedback::status(&s), "unknown");
    assert!(!feedback::code_failure(&s));
    assert_eq!(
        codexsymphony_server::extension_failure::normalized(&s)["source"],
        "reviewed_plugin"
    );
    for class in [
        "input",
        "resource",
        "internal",
        "dependency",
        "unsupported",
        "protocol",
        "unknown",
    ] {
        let mut v = unsupported();
        v["fault"]["class"] = json!(class);
        assert!(feedback::decode(&step(v)).unwrap().is_some());
    }
    let mut legacy = s.clone();
    legacy.command = vec!["legacy".into()];
    assert!(feedback::decode(&legacy).unwrap().is_none());
    assert_eq!(feedback::verdict(&legacy), Verdict::Pass);
    legacy.exit_code = Some(2);
    assert_eq!(feedback::verdict(&legacy), Verdict::Fail);
    legacy.exit_code = None;
    assert_eq!(feedback::verdict(&legacy), Verdict::Unknown);
    legacy.output.clear();
    assert_eq!(feedback::verdict(&legacy), Verdict::Unknown);
}

#[test]
fn malformed_conflicting_feedback_never_authorizes_delivery() {
    let mut cases = vec![
        json!({}),
        json!({"protocol_version":2}),
        json!({"unknown_field":true}),
    ];
    for (pointer, value) in [
        ("/protocol_version", json!(2)),
        ("/check_id", json!("other")),
        ("/verdict", json!("pass")),
        ("/fault", Value::Null),
        ("/fault/code", json!("")),
        ("/fault/scope", json!([""])),
        ("/fault/class", json!("new_unknown_class")),
    ] {
        let mut v = unsupported();
        *v.pointer_mut(pointer).unwrap() = value;
        cases.push(v);
    }
    let mut too_many = unsupported();
    too_many["fault"]["scope"] = json!(vec!["file"; 65]);
    cases.push(too_many);
    for v in cases {
        let s = step(v);
        assert!(feedback::decode(&s).is_err());
        assert_eq!(feedback::verdict(&s), Verdict::Unknown);
        assert_eq!(
            codexsymphony_server::extension_failure::normalized(&s)["source"],
            "host"
        );
    }
    let mut tampered = step(unsupported());
    tampered.output.push('x');
    assert!(feedback::decode(&tampered).is_err());
    tampered.output = " ".repeat(65537);
    tampered.output_sha256 = sha256(&tampered.output);
    assert!(feedback::decode(&tampered).is_err());
}

#[test]
fn scoped_constraints_have_provenance_and_explicit_release_conditions() {
    use codexsymphony_server::development_constraints::validate;
    let c = constraint();
    assert!(validate(std::slice::from_ref(&c)).is_ok());
    assert!(recovery::path_allowed("source", std::slice::from_ref(&c)));
    assert!(recovery::path_allowed(
        "source/file",
        std::slice::from_ref(&c)
    ));
    assert!(!recovery::path_allowed(
        "source-other",
        std::slice::from_ref(&c)
    ));
    assert!(validate(&vec![c.clone(); 33]).is_err());
    assert!(validate(&[c.clone(), c.clone()]).is_err());
    for paths in [
        vec![],
        vec!["../escape".into()],
        vec!["/absolute".into()],
        vec!["a\\b".into()],
        vec!["".into()],
        vec!["a".repeat(1025)],
        vec!["a".into(); 65],
    ] {
        let mut bad = c.clone();
        bad.paths = paths;
        assert!(validate(&[bad]).is_err());
    }
    let mut bad = c;
    bad.release_condition.clear();
    assert!(validate(&[bad]).is_err());
}

#[tokio::test]
async fn lifecycle_commits_all_transitions_and_dispatches_with_durable_ack() {
    let pool = database().await;
    sqlx::query("INSERT INTO plugin_scope(plugin_id,kind,repository_ids,enabled) VALUES('notification:bark','all','{}',true)").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO notification_plugin VALUES('bark',session_user,true)")
        .execute(&pool)
        .await
        .unwrap();
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("INSERT INTO requirement(version,state,contract) VALUES(1,'Draft','{}')")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("UPDATE requirement SET state='Ready',revision=1 WHERE id=1")
        .execute(&mut *tx)
        .await
        .unwrap();
    let invisible: i64 = sqlx::query_scalar("SELECT count(*) FROM lifecycle_event")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(invisible, 0);
    tx.commit().await.unwrap();
    sqlx::query("UPDATE requirement SET state='Running' WHERE id=1")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE requirement SET version=version+1 WHERE id=1")
        .execute(&pool)
        .await
        .unwrap();
    let events = codexsymphony_server::lifecycle_api::events(&pool, 1, 0)
        .await
        .unwrap();
    assert_eq!(events["events"].as_array().unwrap().len(), 3);
    let claim: Value = sqlx::query_scalar("SELECT notification_claim('bark')")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(claim["sequence"], 1);
    assert_eq!(claim["facts"]["status"]["state"], "Draft");
    let blocked: Option<Value> = sqlx::query_scalar("SELECT notification_claim('bark')")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(blocked.is_none());
    let event = claim["event_id"].as_i64().unwrap();
    let stale: bool = sqlx::query_scalar("SELECT notification_ack('bark',$1,2,'accepted')")
        .bind(event)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(!stale);
    let ack: bool = sqlx::query_scalar("SELECT notification_ack('bark',$1,1,'ignored')")
        .bind(event)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(ack);
    let next: Value = sqlx::query_scalar("SELECT notification_claim('bark')")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(next["sequence"], 2);
    sqlx::query("UPDATE notification_delivery SET next_attempt_at=clock_timestamp()-interval '1 second' WHERE state='unknown'").execute(&pool).await.unwrap();
    let retry: Value = sqlx::query_scalar("SELECT notification_claim('bark')")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(retry["event_id"], next["event_id"]);
    assert_eq!(retry["attempt"], 2);
    assert!(
        sqlx::query("SELECT notification_claim('other')")
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("SELECT notification_ack('bark',1,1,'pass')")
            .execute(&pool)
            .await
            .is_err()
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM notification_attempt")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 3);
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("UPDATE requirement SET state='Done'")
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM lifecycle_event")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 3);
    pool.close().await;
}

async fn seed(pool: &PgPool, candidate: &Candidate, manifest: &Value) {
    let contract = json!({"title":"fixture","description":"scope","acceptance_criteria":[{"description":"passes","verification_ref":"test"}],"validation_plan":[{"id":"test","check":"cargo_test","selector":"fixture","expected_result":"pass","timeout_seconds":30}],"network_access":[],"development_constraints":[constraint()]});
    sqlx::query("INSERT INTO repository(id,version,document) VALUES(1,1,'{\"revoked\":false}')")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO requirement(version,state,contract,revision) VALUES(1,'Running',$1,1)",
    )
    .bind(&contract)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO requirement_revision VALUES(1,1,$1)").bind(json!({"repository_id":1,"repository_version":1,"contract":contract,"repository":{"model":"gpt-6-astra","github_repository_id":7,"remote":"owner/repo","base_branch":"main"}})).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO requirement_budget(requirement_id,limits) VALUES(1,'{\"tokens\":100,\"turns\":10,\"model_seconds\":100}')").execute(pool).await.unwrap();
    sqlx::query("INSERT INTO repair_authorization VALUES(1,3,'bounded_v1')")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE execution_control SET requirement_id=1,incarnation='boot',recovery_complete=true",
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO agent_run(id,requirement_id,revision,incarnation,request_id,workspace,workspace_identity,launch,state,quiescent,phase) VALUES('source',1,1,'boot','source','/tmp','owned','{}','Succeeded',true,'validation')").execute(pool).await.unwrap();
    sqlx::query(
        "INSERT INTO workspace_snapshot(run_id,manifest,candidate) VALUES('source',$1,true)",
    )
    .bind(manifest)
    .execute(pool)
    .await
    .unwrap();
    assert_eq!(manifest["head"], candidate.sha);
}

fn plugin(plan: &mut validation_runner::Plan, language: &str, response: Value) {
    let content = if language == "shell" {
        format!("#!/bin/sh\nprintf '%s' '{}'\n", response)
    } else {
        format!(
            "#!/usr/bin/env python3\nprint({:?})\n",
            response.to_string()
        )
    };
    fs::write(&plan.entry, &content).unwrap();
    plan.entry_sha256 = sha256(&content);
    plan.steps[0].command = vec!["/gate-entry".into(), feedback::SELECTOR.into()];
}

fn broker(
    root: &Path,
    repo: &Path,
) -> (
    codexsymphony_server::git_broker::GitBroker,
    codexsymphony_server::workspace::Manifest,
) {
    let bundle = root.join("seed.bundle");
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["bundle", "create"])
            .arg(&bundle)
            .arg("--all")
            .status()
            .unwrap()
            .success()
    );
    let broker =
        codexsymphony_server::git_broker::GitBroker::initialize(&root.join("workspaces"), &bundle)
            .unwrap();
    let candidate = validation_runner::candidate(repo).unwrap();
    let workspace = codexsymphony_server::workspace::Workspace {
        key: codexsymphony_server::execution::RunKey {
            run_id: "source".into(),
            request_id: "source".into(),
            incarnation: "boot".into(),
        },
        identity: "source".into(),
        requirement: 1,
        revision: 1,
        phase: "validation".into(),
        baseline: candidate.sha,
        branch: "ai/req-1-source".into(),
        path: broker
            .path("source")
            .unwrap()
            .to_string_lossy()
            .into_owned(),
    };
    broker.prepare(&workspace, true).unwrap();
    let manifest = broker.preserve(&workspace).unwrap();
    (broker, manifest)
}

#[tokio::test]
async fn same_candidate_plugin_upgrade_revalidates_without_erasing_failure_or_budget() {
    let (root, repo, mut plan) = runner::fixture();
    plugin(&mut plan, "shell", unsupported());
    let (broker, manifest) = broker(&root, &repo);
    let candidate = validation_runner::candidate(&repo).unwrap();
    let pool = database().await;
    seed(&pool, &candidate, &json!(manifest)).await;
    let directory = root.join("first");
    let request = validation_service::Request {
        id: "first",
        source_run: "source",
        requirement: 1,
        revision: 1,
        checkout: &repo,
        directory: &directory,
        candidate: &candidate,
        plan: &plan,
    };
    assert!(!validation_service::validate(&pool, request).await.unwrap());
    let state: String =
        sqlx::query_scalar("SELECT result FROM candidate_validation WHERE id='first'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(state, "blocked");
    let failures = recovery::view(&pool, 1).await.unwrap();
    assert_eq!(
        failures["failures"][0]["facts"]["feedback"]["fault"]["class"],
        "unsupported"
    );
    let old_plan = plan.identity().unwrap().config_sha256;
    plugin(
        &mut plan,
        "python",
        json!({"protocol_version":1,"check_id":"test","verdict":"pass","fault":null}),
    );
    assert_ne!(old_plan, plan.identity().unwrap().config_sha256);
    let decision = recovery::Decision {
        request_id: "recover-1".into(),
        version: 1,
        revision: 1,
        validation_id: "first".into(),
        reason: "Reviewed fixture collector now supports the source".into(),
        action: recovery::Action::Revalidate {
            plan_digest: plan.identity().unwrap().config_sha256,
            resume_condition: "Approved replacement completes required check".into(),
        },
    };
    assert!(recovery::decide(&pool, 2, &decision).await.is_err());
    let (first, concurrent) = tokio::join!(
        recovery::decide(&pool, 1, &decision),
        recovery::decide(&pool, 1, &decision)
    );
    let accepted = first.unwrap();
    assert_eq!(accepted, concurrent.unwrap());
    assert_eq!(accepted["started"], false);
    assert_eq!(
        accepted,
        recovery::decide(&pool, 1, &decision).await.unwrap()
    );
    let mut stale = decision.clone();
    stale.request_id = "other".into();
    assert!(recovery::decide(&pool, 1, &stale).await.is_err());
    assert!(
        codexsymphony_server::extension_revalidation::tick(
            &pool,
            &root,
            &broker,
            &plan,
            &Value::Null
        )
        .await
        .unwrap()
    );
    let result = recovery::view(&pool, 1).await.unwrap();
    assert_eq!(
        result["failures"][0]["resolution_state"], "complete",
        "{result}"
    );
    let rows: Vec<(String, String, String)> =
        sqlx::query_as("SELECT id,result,candidate_sha FROM candidate_validation ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].1, "blocked");
    assert_eq!(rows[1].1, "succeeded");
    assert_eq!(rows[0].2, rows[1].2);
    assert!(
        !codexsymphony_server::extension_revalidation::tick(
            &pool,
            &root,
            &broker,
            &plan,
            &Value::Null
        )
        .await
        .unwrap()
    );
    let runs: i64 = sqlx::query_scalar("SELECT count(*) FROM agent_run")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(runs, 1);
    let version: i64 =
        sqlx::query_scalar("SELECT version FROM requirement_budget WHERE requirement_id=1")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(version, 1);
    let deliveries: i64 = sqlx::query_scalar("SELECT count(*) FROM delivery")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(deliveries, 1);
    pool.close().await;
}

#[tokio::test]
async fn silent_legacy_check_recovers_same_candidate_and_explicit_post_merge_plan() {
    let (root, repo, mut plan) = runner::fixture();
    // The command really completes, but legacy evidence requires output.
    let script = "#!/bin/sh\ntest -f source || exit 1\nif [ \"$1\" = report ]; then printf 'source verified\\n'; fi\n";
    fs::write(&plan.entry, script).unwrap();
    plan.entry_sha256 = sha256(script);
    let (broker, manifest) = broker(&root, &repo);
    let candidate = validation_runner::candidate(&repo).unwrap();
    let pool = database().await;
    seed(&pool, &candidate, &json!(manifest)).await;
    let policy = delivery_fixture::policy(&plan, &root.join("original-plan.json"));
    fs::write(
        root.join("original-plan.json"),
        serde_json::to_vec(&plan).unwrap(),
    )
    .unwrap();
    sqlx::query("UPDATE repository SET document=document||'{\"github_repository_id\":7}'::jsonb")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO github_repository(repository_id,repository_version,policy,probe_pr) VALUES(7,1,$1,1)")
        .bind(json!(policy)).execute(&pool).await.unwrap();
    let directory = root.join("silent");
    assert!(
        !validation_service::validate(
            &pool,
            validation_service::Request {
                id: "silent",
                source_run: "source",
                requirement: 1,
                revision: 1,
                checkout: &repo,
                directory: &directory,
                candidate: &candidate,
                plan: &plan,
            }
        )
        .await
        .unwrap()
    );
    let retained: Value = sqlx::query_scalar(
        "SELECT to_jsonb(s) FROM validation_step s WHERE validation_id='silent'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(retained["exit_code"], 0);
    assert_eq!(retained["output"], "");
    assert_eq!(retained["status"], "unknown");
    let before: Value =
        sqlx::query_scalar("SELECT to_jsonb(b) FROM requirement_budget b WHERE requirement_id=1")
            .fetch_one(&pool)
            .await
            .unwrap();
    plan.steps[0].command.push("report".into());
    let mut decision = recovery::Decision {
        request_id: "recover-silent".into(),
        version: 1,
        revision: 1,
        validation_id: "silent".into(),
        reason: "reviewed script reports actual successful assertion".into(),
        action: recovery::Action::RevalidateDelivery {
            plan_digest: plan.identity().unwrap().config_sha256,
            resume_condition: "same required assertion emits verified evidence".into(),
            policy_digest: sha256(serde_json::to_vec(&policy).unwrap()),
        },
    };
    let mut wrong = decision.clone();
    if let recovery::Action::RevalidateDelivery { policy_digest, .. } = &mut wrong.action {
        *policy_digest = "0".repeat(64);
    }
    assert!(recovery::decide(&pool, 1, &wrong).await.is_err());
    sqlx::query("UPDATE requirement SET paused=true")
        .execute(&pool)
        .await
        .unwrap();
    assert!(recovery::decide(&pool, 1, &decision).await.is_err());
    sqlx::query("UPDATE requirement SET paused=false")
        .execute(&pool)
        .await
        .unwrap();
    let accepted = recovery::decide(&pool, 1, &decision).await.unwrap();
    assert_eq!(accepted["started"], false);
    assert_eq!(
        accepted,
        recovery::decide(&pool, 1, &decision).await.unwrap()
    );
    decision.request_id = "stale-silent".into();
    assert!(recovery::decide(&pool, 1, &decision).await.is_err());
    assert!(
        codexsymphony_server::extension_revalidation::tick(
            &pool,
            &root,
            &broker,
            &plan,
            &Value::Null
        )
        .await
        .unwrap()
    );
    let recovered = recovery::view(&pool, 1).await.unwrap();
    let event = recovered["failures"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["event_key"] == accepted["event_key"])
        .unwrap();
    assert_eq!(event["resolution_state"], "complete", "{recovered}");
    let preserved: Value = sqlx::query_scalar(
        "SELECT to_jsonb(s) FROM validation_step s WHERE validation_id='silent'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(retained, preserved);
    let after: Value =
        sqlx::query_scalar("SELECT to_jsonb(b) FROM requirement_budget b WHERE requirement_id=1")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(before, after);
    let (runs, calls): (i64, i64) =
        sqlx::query_as("SELECT (SELECT count(*) FROM agent_run),(SELECT count(*) FROM model_call)")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((runs, calls), (1, 0));
    let saved_policy: Value =
        sqlx::query_scalar("SELECT policy FROM github_repository WHERE repository_id=7")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(saved_policy, json!(policy));
    let (sha, tree): (String, String) = sqlx::query_as(
        "SELECT candidate_sha,candidate_tree FROM candidate_validation WHERE retry_of='silent'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!((sha, tree), (candidate.sha, candidate.tree));
    recover_invalidated_pending_delivery(
        &pool,
        &root,
        &broker,
        &plan,
        &decision,
        event["successor_validation"].as_str().unwrap(),
    )
    .await;
    pool.close().await;
}

async fn recover_invalidated_pending_delivery(
    pool: &PgPool,
    root: &Path,
    broker: &codexsymphony_server::git_broker::GitBroker,
    plan: &validation_runner::Plan,
    first: &recovery::Decision,
    prior: &str,
) {
    // Persisted output of a control interruption; never clear this old flag.
    sqlx::query("UPDATE candidate_validation SET hook_invalidated=true WHERE id=$1")
        .bind(prior)
        .execute(pool)
        .await
        .unwrap();
    let original: Value =
        sqlx::query_scalar("SELECT to_jsonb(d) FROM delivery d WHERE validation_id=$1")
            .bind(prior)
            .fetch_one(pool)
            .await
            .unwrap();
    let mut decision = first.clone();
    decision.request_id = "recover-before-first-send".into();
    decision.version = 2;
    decision.validation_id = prior.into();
    let mut ordinary = decision.clone();
    ordinary.action = recovery::Action::Revalidate {
        plan_digest: plan.identity().unwrap().config_sha256,
        resume_condition: "ordinary revalidation cannot rebind delivery".into(),
    };
    assert!(recovery::decide(pool, 1, &ordinary).await.is_err());
    for (change, restore) in [
        (
            "UPDATE delivery_action SET attempts=1",
            "UPDATE delivery_action SET attempts=0",
        ),
        (
            "UPDATE delivery_action SET state='unknown'",
            "UPDATE delivery_action SET state='pending'",
        ),
        (
            "UPDATE delivery SET pr_number=42",
            "UPDATE delivery SET pr_number=NULL",
        ),
        (
            "INSERT INTO delivery_attempt(action_key,kind,ordinal,operation) SELECT action_key,'publish',1,'push' FROM delivery",
            "DELETE FROM delivery_attempt",
        ),
    ] {
        sqlx::query(change).execute(pool).await.unwrap();
        assert!(recovery::decide(pool, 1, &decision).await.is_err());
        sqlx::query(restore).execute(pool).await.unwrap();
    }
    let accepted = recovery::decide(pool, 1, &decision).await.unwrap();
    assert_eq!(
        accepted,
        recovery::decide(pool, 1, &decision).await.unwrap()
    );
    assert!(
        codexsymphony_server::extension_revalidation::tick(pool, root, broker, plan, &Value::Null)
            .await
            .unwrap()
    );
    let (successor, state): (String, String) = sqlx::query_as(
        "SELECT successor_validation,resolution_state FROM recovery_failure WHERE event_key=$1",
    )
    .bind(accepted["event_key"].as_str().unwrap())
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(state, "complete");
    assert_ne!(successor, prior);
    let current: Value = sqlx::query_scalar("SELECT to_jsonb(d) FROM delivery d")
        .fetch_one(pool)
        .await
        .unwrap();
    let mut expected = original;
    expected["validation_id"] = json!(successor);
    assert_eq!(current, expected, "only current proof binding may change");
    let preserved: (String, bool, Option<String>) = sqlx::query_as(
        "SELECT result,hook_invalidated,superseded_by FROM candidate_validation WHERE id=$1",
    )
    .bind(prior)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(
        preserved,
        ("succeeded".into(), true, Some(successor.clone()))
    );
    let fact: Value =
        sqlx::query_scalar("SELECT fact FROM delivery_observation WHERE kind='validation_rebound'")
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(fact["previous_validation"], prior);
    assert_eq!(fact["validation"], successor);
    assert_eq!(fact["recovery_event"], accepted["event_key"]);
    let mut tx = pool.begin().await.unwrap();
    codexsymphony_server::delivery_store::enqueue(&mut tx, &successor)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let facts: (i64,i64,i64,i64,i64) = sqlx::query_as("SELECT (SELECT count(*) FROM delivery),(SELECT count(*) FROM delivery_attempt),(SELECT count(*) FROM delivery_observation WHERE kind='validation_rebound'),(SELECT count(*) FROM agent_run),(SELECT count(*) FROM model_call)")
        .fetch_one(pool).await.unwrap();
    assert_eq!(facts, (1, 0, 1, 1, 0));
    let budget: i64 =
        sqlx::query_scalar("SELECT version FROM requirement_budget WHERE requirement_id=1")
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(budget, 1);
}

#[tokio::test]
async fn approved_adaptation_uses_real_codex_runtime_and_preserves_fault_identity() {
    use codexsymphony_server::{budget::Amount, execution::RunKey, runtime_client, runtime_resume};
    let (root, repo, mut plan) = runner::fixture();
    let failure = unsupported().to_string();
    let pass =
        json!({"protocol_version":1,"check_id":"test","verdict":"pass","fault":null}).to_string();
    let script = format!(
        "#!/bin/sh\nif grep -q 'ordinary named function' source; then printf '%s' '{pass}'; else printf '%s' '{failure}'; fi\n"
    );
    fs::write(&plan.entry, &script).unwrap();
    plan.entry_sha256 = sha256(&script);
    plan.steps[0].command = vec!["/gate-entry".into(), feedback::SELECTOR.into()];
    let (broker, manifest) = broker(&root, &repo);
    let candidate = validation_runner::candidate(&repo).unwrap();
    let pool = database().await;
    seed(&pool, &candidate, &json!(manifest)).await;
    let directory = root.join("unsupported");
    assert!(
        !validation_service::validate(
            &pool,
            validation_service::Request {
                id: "unsupported",
                source_run: "source",
                requirement: 1,
                revision: 1,
                checkout: &repo,
                directory: &directory,
                candidate: &candidate,
                plan: &plan
            }
        )
        .await
        .unwrap()
    );
    let decision = recovery::Decision {
        request_id: "adapt-1".into(),
        version: 1,
        revision: 1,
        validation_id: "unsupported".into(),
        reason: "Approve local source adaptation only".into(),
        action: recovery::Action::AdaptCode {
            constraints: vec![constraint()],
        },
    };
    let accepted = recovery::decide(&pool, 1, &decision).await.unwrap();
    let codex = String::from_utf8(Command::new("which").arg("codex").output().unwrap().stdout)
        .unwrap()
        .trim()
        .to_owned();
    assert!(!codex.is_empty());
    let key = RunKey {
        run_id: "adapt-run".into(),
        request_id: "adapt-launch".into(),
        incarnation: "boot".into(),
    };
    let workspace = codexsymphony_server::workspace::Workspace {
        key: key.clone(),
        identity: "adapt-workspace".into(),
        requirement: 1,
        revision: 1,
        phase: "execution".into(),
        baseline: manifest.head.clone(),
        branch: "ai/req-1-adapt-run".into(),
        path: broker
            .path(&key.run_id)
            .unwrap()
            .to_string_lossy()
            .into_owned(),
    };
    let job = runtime_resume::Job {
        source: "source".into(),
        launch: codexsymphony_server::execution::Launch {
            key,
            program: codex,
            args: vec!["app-server".into()],
            workspace: workspace.path.clone(),
            workspace_identity: workspace.identity.clone(),
        },
        workspace,
        manifest: manifest.clone(),
    };
    let resources = Amount {
        tokens: 20,
        turns: 1,
        model_seconds: 30,
    };
    assert!(
        codexsymphony_server::recovery_store::reserve(
            &pool,
            accepted["event_key"].as_str().unwrap(),
            &job,
            resources
        )
        .await
        .unwrap()
    );
    broker.restore_candidate(&job.workspace, &manifest).unwrap();
    sqlx::query("INSERT INTO preparation_record(run_id,requirement_id,revision,launch,retry,ready,checked_at) VALUES($1,1,1,$2,'{}',true,extract(epoch FROM now())::bigint)").bind(&job.launch.key.run_id).bind(json!(job.launch)).execute(&pool).await.unwrap();
    assert!(
        codexsymphony_server::validation_repair::bind(&pool, &job.launch)
            .await
            .unwrap()
    );
    let prompt = codexsymphony_server::runtime_store::input(&pool, &job.launch.key)
        .await
        .unwrap();
    assert!(prompt.contains("no-closure"));
    assert!(prompt.contains("approved_adaptation_constraints"));
    let counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let calls = counter.clone();
    let path = job.workspace.path.clone();
    let app=axum::Router::new().route("/responses",axum::routing::post(move || {
        let counter=counter.clone();let path=path.clone();
        async move {
            let index=counter.fetch_add(1,std::sync::atomic::Ordering::SeqCst);
            let (name,args)=match index {
                0=>("exec_command",json!({"cmd":"printf 'ordinary named function\\n' > source","yield_time_ms":1000,"max_output_tokens":1000})),
                1=>("create_local_commit",json!({"message":"Adapt fixture to supported named function"})),
                _=>{
                    let sha=String::from_utf8(Command::new("git").arg("-C").arg(path).args(["rev-parse","HEAD"]).output().unwrap().stdout).unwrap();
                    ("report_completion",json!({"candidate_sha":sha.trim(),"summary":"Approved source adaptation completed"}))
                }
            };
            let events=[json!({"type":"response.created","response":{"id":format!("adapt-{index}")}}),json!({"type":"response.output_item.done","item":{"type":"function_call","call_id":format!("call-{index}"),"name":name,"arguments":args.to_string()}}),json!({"type":"response.completed","response":{"id":format!("adapt-{index}"),"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}})];
            ([("content-type","text/event-stream")],events.iter().map(|v|format!("data: {v}\n\n")).collect::<String>())
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let settings = runtime_client::Settings {
        startup_seconds: 30,
        response_seconds: 30,
        stall_seconds: 30,
        reservation: resources,
        codex_config: format!(
            r#"model = "gpt-6-astra"
model_provider = "adapt_fixture"
approval_policy = "never"
sandbox_mode = "danger-full-access"
[features]
apps = false
plugins = false
remote_plugin = false
goals = false
[model_providers.adapt_fixture]
name = "Local adaptation fixture"
base_url = "http://127.0.0.1:{port}"
wire_api = "responses"
requires_openai_auth = false
supports_websockets = false
request_max_retries = 0
stream_max_retries = 0
"#
        ),
    };
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        runtime_client::execute(
            &pool,
            &root,
            Path::new(env!("CARGO_BIN_EXE_codexsymphony-server")),
            &broker,
            &job.launch,
            &settings,
        ),
    )
    .await;
    server.abort();
    result.unwrap().unwrap();
    assert!(calls.load(std::sync::atomic::Ordering::SeqCst) >= 3);
    assert_eq!(
        fs::read_to_string(Path::new(&job.workspace.path).join("source")).unwrap(),
        "ordinary named function\n"
    );
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while !root
            .join(&job.launch.key.run_id)
            .join("quiescent.json")
            .exists()
        {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    // Consume actual supervisor stop and preserve the actual Runtime candidate.
    codexsymphony_server::coordinator::recover(&pool, &root, "boot")
        .await
        .unwrap();
    let saved: Value =
        sqlx::query_scalar("SELECT manifest FROM workspace_snapshot WHERE run_id=$1")
            .bind(&job.launch.key.run_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    let saved: codexsymphony_server::workspace::Manifest = serde_json::from_value(saved).unwrap();
    assert_ne!(saved.head, candidate.sha);
    assert!(
        recovery::check_scope(&pool, &broker, &job.launch.key.run_id, &saved)
            .await
            .unwrap()
    );
    // Exercise the actual worker guard with an incompatible approved path fixture.
    sqlx::query("UPDATE recovery_failure SET resolution=jsonb_set(resolution,'{command,action,constraints,0,paths}','[\"outside\"]') WHERE resolution_state='adaptation'").execute(&pool).await.unwrap();
    assert!(
        codexsymphony_server::validation_worker::tick(&pool, &root, &broker, &plan)
            .await
            .unwrap()
    );
    let scope_blocked:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM recovery_failure WHERE event_key='scope:'||$1 AND decision='blocked')").bind(&job.launch.key.run_id).fetch_one(&pool).await.unwrap();
    assert!(scope_blocked);
    let before: i64 =
        sqlx::query_scalar("SELECT count(*) FROM candidate_validation WHERE source_run_id=$1")
            .bind(&job.launch.key.run_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(before, 0);
    // Restore this disposable fixture to exercise the successful validation path too.
    sqlx::query("UPDATE recovery_failure SET resolution=jsonb_set(resolution,'{command,action,constraints,0,paths}','[\"source\"]') WHERE resolution_state='adaptation'").execute(&pool).await.unwrap();
    let adapted = validation_runner::candidate(Path::new(&job.workspace.path)).unwrap();
    assert!(
        validation_service::validate(
            &pool,
            validation_service::Request {
                id: "adapted",
                source_run: &job.launch.key.run_id,
                requirement: 1,
                revision: 1,
                checkout: Path::new(&job.workspace.path),
                directory: &root.join("adapted"),
                candidate: &adapted,
                plan: &plan
            }
        )
        .await
        .unwrap()
    );
    let facts = recovery::view(&pool, 1).await.unwrap();
    assert_eq!(
        facts["failures"][0]["facts"]["feedback"]["fault"]["class"],
        "unsupported"
    );
    assert_eq!(facts["failures"][0]["decision"], "repaired");
    // A restart-equivalent prompt still includes the approved scoped decision.
    let key = RunKey {
        run_id: job.launch.key.run_id.clone(),
        ..job.launch.key.clone()
    };
    assert!(
        codexsymphony_server::runtime_store::input(&pool, &key)
            .await
            .unwrap()
            .contains("no-closure")
    );
    let used: i64 = sqlx::query_scalar("SELECT count(*) FROM model_call")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(used > 0);
    pool.close().await;
}

#[tokio::test]
async fn concurrent_notifications_are_isolated_by_requirement_and_plugin_role() {
    let pool = database().await;
    let schema: String = sqlx::query_scalar("SELECT current_schema()")
        .fetch_one(&pool)
        .await
        .unwrap();
    let role = format!(
        "notify_{}",
        codexsymphony_server::process::new_identity()
            .unwrap()
            .replace('-', "")
    );
    sqlx::raw_sql(&format!("CREATE ROLE {role} LOGIN PASSWORD 'synthetic-notification-only'; GRANT USAGE ON SCHEMA {schema} TO {role}; GRANT EXECUTE ON FUNCTION notification_claim(text), notification_ack(text,bigint,integer,text) TO {role};")).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO plugin_scope(plugin_id,kind,repository_ids,enabled) VALUES('notification:isolated','all','{}',true)").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO notification_plugin VALUES('isolated',$1,true)")
        .bind(&role)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::raw_sql("INSERT INTO requirement(version,state,contract) VALUES(1,'Draft','{}'),(1,'Draft','{}'); UPDATE requirement SET state='Ready' WHERE id=1;").execute(&pool).await.unwrap();
    let options: PgConnectOptions = std::env::var("TEST_DATABASE_URL").unwrap().parse().unwrap();
    let login = PgPoolOptions::new()
        .max_connections(2)
        .connect_with(
            options
                .username(&role)
                .password("synthetic-notification-only")
                .options([("search_path", schema)]),
        )
        .await
        .unwrap();
    for forbidden in [
        "SELECT * FROM plugin_scope",
        "UPDATE plugin_scope SET kind='all',repository_ids='{}'",
        "SELECT plugin_scope_admit('agent:codex','forged',1,0,1)",
    ] {
        assert!(sqlx::query(forbidden).execute(&login).await.is_err());
    }
    let (a, b) = tokio::join!(
        sqlx::query_scalar::<_, Value>("SELECT notification_claim('isolated')").fetch_one(&login),
        sqlx::query_scalar::<_, Value>("SELECT notification_claim('isolated')").fetch_one(&login)
    );
    let a = a.unwrap();
    let b = b.unwrap();
    assert_ne!(a["requirement_id"], b["requirement_id"]);
    assert_eq!(a["sequence"], 1);
    assert_eq!(b["sequence"], 1);
    assert!(
        sqlx::query("SELECT * FROM requirement")
            .execute(&login)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE requirement SET state='Done'")
            .execute(&login)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("SELECT notification_claim('another')")
            .execute(&login)
            .await
            .is_err()
    );
    let blocked: Option<Value> = sqlx::query_scalar("SELECT notification_claim('isolated')")
        .fetch_one(&login)
        .await
        .unwrap();
    assert!(blocked.is_none());
    let first = if a["requirement_id"] == 1 { a } else { b };
    let ack: bool = sqlx::query_scalar("SELECT notification_ack('isolated',$1,1,'accepted')")
        .bind(first["event_id"].as_i64().unwrap())
        .fetch_one(&login)
        .await
        .unwrap();
    assert!(ack);
    let mut spoof = login.acquire().await.unwrap();
    sqlx::raw_sql("CREATE TEMP TABLE plugin_scope(plugin_id text,kind text,repository_ids bigint[],enabled boolean,version bigint); INSERT INTO plugin_scope VALUES('notification:isolated','all','{}',true,1);")
        .execute(&mut *spoof).await.unwrap();
    sqlx::query("UPDATE plugin_scope SET enabled=false WHERE plugin_id='notification:isolated'")
        .execute(&pool)
        .await
        .unwrap();
    let denied: Option<Value> = sqlx::query_scalar("SELECT notification_claim('isolated')")
        .fetch_one(&mut *spoof)
        .await
        .unwrap();
    assert!(denied.is_none());
    drop(spoof);
    sqlx::query("UPDATE plugin_scope SET enabled=true WHERE plugin_id='notification:isolated'")
        .execute(&pool)
        .await
        .unwrap();
    let next: Value = sqlx::query_scalar("SELECT notification_claim('isolated')")
        .fetch_one(&login)
        .await
        .unwrap();
    assert_eq!(next["requirement_id"], 1);
    assert_eq!(next["sequence"], 2);
    login.close().await;
    sqlx::raw_sql(&format!("DROP OWNED BY {role}; DROP ROLE {role};"))
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}

#[tokio::test]
async fn recovery_and_notification_http_contracts_reject_bad_inputs_and_storage_failure() {
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    let pool = database().await;
    let routes = codexsymphony_server::extension_api::routes()
        .merge(codexsymphony_server::lifecycle_api::routes())
        .with_state(pool.clone());
    for path in ["extension-recovery", "lifecycle"] {
        let response = routes
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/requirements/1/{path}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
    }
    let post = |path: &str, body: &str| {
        Request::builder()
            .method("POST")
            .uri(format!("/api/requirements/1/{path}"))
            .header("content-type", "application/json")
            .body(Body::from(body.to_owned()))
            .unwrap()
    };
    assert_eq!(
        routes
            .clone()
            .oneshot(post("extension-recovery", "{}"))
            .await
            .unwrap()
            .status(),
        422
    );
    let decision = json!({"request_id":"missing","version":1,"revision":1,"validation_id":"absent","reason":"review","action":{"kind":"adapt_code","constraints":[constraint()]}});
    assert_eq!(
        routes
            .clone()
            .oneshot(post("extension-recovery", &decision.to_string()))
            .await
            .unwrap()
            .status(),
        409
    );
    let body = "{\"event_id\":1,\"plugin_id\":\"absent\"}";
    assert_eq!(
        routes
            .clone()
            .oneshot(post("notifications/replay", body))
            .await
            .unwrap()
            .status(),
        200
    );
    pool.close().await;
    for path in ["extension-recovery", "lifecycle"] {
        assert_eq!(
            routes
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(format!("/api/requirements/1/{path}"))
                        .body(Body::empty())
                        .unwrap()
                )
                .await
                .unwrap()
                .status(),
            503
        );
    }
    assert_eq!(
        routes
            .oneshot(post("notifications/replay", body))
            .await
            .unwrap()
            .status(),
        503
    );
}

#[tokio::test]
async fn real_plugin_crash_timeout_protocol_and_quality_failure_preserve_distinct_facts() {
    for (script, verdict, decision, result) in [
        (
            "#!/bin/sh\nkill -KILL $$\n",
            "unknown",
            "blocked",
            "blocked",
        ),
        ("#!/bin/sh\nsleep 3\n", "unknown", "blocked", "blocked"),
        (
            "#!/bin/sh\nprintf 'invalid-json'\n",
            "unknown",
            "blocked",
            "blocked",
        ),
        (
            "#!/bin/sh\nprintf '%s' '{\"protocol_version\":99,\"check_id\":\"test\",\"verdict\":\"pass\",\"fault\":null}'\n",
            "unknown",
            "blocked",
            "blocked",
        ),
        (
            "#!/bin/sh\nprintf '%s' '{\"protocol_version\":1,\"check_id\":\"test\",\"verdict\":\"fail\",\"fault\":null}'\n",
            "fail",
            "code",
            "gate_failed",
        ),
    ] {
        let (root, repo, mut plan) = runner::fixture();
        fs::write(&plan.entry, script).unwrap();
        plan.entry_sha256 = sha256(script);
        plan.steps[0].command = vec!["/gate-entry".into(), feedback::SELECTOR.into()];
        plan.steps[0].timeout_seconds = 1;
        let (_, manifest) = broker(&root, &repo);
        let candidate = validation_runner::candidate(&repo).unwrap();
        let pool = database().await;
        seed(&pool, &candidate, &json!(manifest)).await;
        let passed = validation_service::validate(
            &pool,
            validation_service::Request {
                id: "failure",
                source_run: "source",
                requirement: 1,
                revision: 1,
                checkout: &repo,
                directory: &root.join("evidence"),
                candidate: &candidate,
                plan: &plan,
            },
        )
        .await
        .unwrap();
        assert!(!passed);
        let view = recovery::view(&pool, 1).await.unwrap();
        assert_eq!(
            view["failures"][0]["facts"]["feedback"]["verdict"], verdict,
            "{view}"
        );
        assert_eq!(view["failures"][0]["decision"], decision);
        let saved: String =
            sqlx::query_scalar("SELECT result FROM candidate_validation WHERE id='failure'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(saved, result);
        let deliveries: i64 = sqlx::query_scalar("SELECT count(*) FROM delivery")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(deliveries, 0);
        pool.close().await;
    }
}

#[test]
fn old_frozen_inputs_keep_their_serialized_identity_without_optional_constraints() {
    let original = json!({"title":"old","description":"frozen","acceptance_criteria":[],"validation_plan":[],"network_access":[]});
    let mut contract: codexsymphony_server::contract::Contract =
        serde_json::from_value(original.clone()).unwrap();
    assert_eq!(json!(contract), original);
    contract.development_constraints = Some(vec![constraint()]);
    assert_eq!(
        json!(contract)["development_constraints"][0]["id"],
        "no-closure"
    );
    serialization_failures(&contract);
    let original = json!({"integration":null,"child_id":"old","revision":1,"repository_version":1,"budget":{"tokens":1,"turns":1,"model_seconds":1},"repair_scope":"source","merged_baseline_review":"frozen","verification":[]});
    let mut item: codexsymphony_server::group_review::Item =
        serde_json::from_value(original.clone()).unwrap();
    assert_eq!(json!(item), original);
    item.development_constraints = Some(vec![constraint()]);
    serialization_failures(&item);
    assert_eq!(
        json!(item)["development_constraints"][0]["id"],
        "no-closure"
    );
}

#[tokio::test]
async fn environment_events_capture_blocking_changes_without_polling_noise() {
    let pool = database().await;
    sqlx::query(
        "INSERT INTO requirement(version,state,contract,revision) VALUES(1,'Running','{}',1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    for (passed, digest) in [
        (true, "one"),
        (true, "one"),
        (false, "one"),
        (false, "one"),
        (true, "two"),
    ] {
        let report = json!({"response":{},"differences":[],"error":if passed {Value::Null} else {json!("private diagnostic")},"actual_digest":digest});
        sqlx::query("INSERT INTO environment_observation(requirement_id,revision,stage,report) VALUES(1,1,'validation',$1)").bind(report).execute(&pool).await.unwrap();
    }
    let events = codexsymphony_server::lifecycle_api::events(&pool, 1, 0)
        .await
        .unwrap();
    assert_eq!(events["events"].as_array().unwrap().len(), 4);
    assert_eq!(events["events"][2]["phase"], "environment");
    assert_eq!(events["events"][2]["facts"]["status"]["passed"], false);
    assert!(!events.to_string().contains("private diagnostic"));
    pool.close().await;
}

#[tokio::test]
async fn interrupted_revalidation_remains_recoverable_and_repeated_unsupported_does_not_loop() {
    let (root, repo, mut plan) = runner::fixture();
    plugin(&mut plan, "shell", unsupported());
    let (broker, manifest) = broker(&root, &repo);
    let candidate = validation_runner::candidate(&repo).unwrap();
    let pool = database().await;
    seed(&pool, &candidate, &json!(manifest)).await;
    assert!(
        !validation_service::validate(
            &pool,
            validation_service::Request {
                id: "initial",
                source_run: "source",
                requirement: 1,
                revision: 1,
                checkout: &repo,
                directory: &root.join("initial"),
                candidate: &candidate,
                plan: &plan
            }
        )
        .await
        .unwrap()
    );
    fs::write(&plan.entry, "#!/bin/sh\nyes flood\n").unwrap();
    plan.entry_sha256 = sha256("#!/bin/sh\nyes flood\n");
    let mut decision = recovery::Decision {
        request_id: "first-retry".into(),
        version: 1,
        revision: 1,
        validation_id: "initial".into(),
        reason: "restore approved prerequisites".into(),
        action: recovery::Action::Revalidate {
            plan_digest: plan.identity().unwrap().config_sha256,
            resume_condition: "reviewed implementation available".into(),
        },
    };
    recovery::decide(&pool, 1, &decision).await.unwrap();
    // A real output-limit failure occurs after the successor intent exists.
    assert!(
        codexsymphony_server::extension_revalidation::tick(
            &pool,
            &root,
            &broker,
            &plan,
            &Value::Null
        )
        .await
        .unwrap()
    );
    let view = recovery::view(&pool, 1).await.unwrap();
    let first = &view["failures"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["validation_id"] == "initial")
        .unwrap();
    assert_eq!(first["resolution_state"], "blocked", "{view}");
    plugin(&mut plan, "python", unsupported());
    decision.action = recovery::Action::Revalidate {
        plan_digest: plan.identity().unwrap().config_sha256,
        resume_condition: "output contract restored".into(),
    };
    decision.validation_id = first["successor_validation"].as_str().unwrap().into();
    decision.request_id = "second-retry".into();
    decision.version = 2;
    recovery::decide(&pool, 1, &decision).await.unwrap();
    assert!(
        codexsymphony_server::extension_revalidation::tick(
            &pool,
            &root,
            &broker,
            &plan,
            &Value::Null
        )
        .await
        .unwrap()
    );
    assert!(
        !codexsymphony_server::extension_revalidation::tick(
            &pool,
            &root,
            &broker,
            &plan,
            &Value::Null
        )
        .await
        .unwrap()
    );
    let statuses: Vec<String> =
        sqlx::query_scalar("SELECT result FROM candidate_validation ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(statuses, vec!["blocked", "blocked", "blocked"]);
    let runs: i64 = sqlx::query_scalar("SELECT count(*) FROM agent_run")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(runs, 1);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM delivery")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    pool.close().await;
}

#[tokio::test]
async fn notification_replay_extends_once_without_resetting_attempt_history_or_business_state() {
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use tower::ServiceExt;
    let pool = database().await;
    sqlx::query("INSERT INTO plugin_scope(plugin_id,kind,repository_ids,enabled) VALUES('notification:bark','all','{}',true)").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO notification_plugin VALUES('bark',session_user,true)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO requirement(version,state,contract) VALUES(1,'Draft','{}')")
        .execute(&pool)
        .await
        .unwrap();
    let routes = codexsymphony_server::lifecycle_api::routes().with_state(pool.clone());
    for ordinal in 1..=6 {
        if ordinal == 4 {
            for accepted in [true, false] {
                let response = routes
                    .clone()
                    .oneshot(
                        Request::builder()
                            .method("POST")
                            .uri("/api/requirements/1/notifications/replay")
                            .header("content-type", "application/json")
                            .body(Body::from("{\"event_id\":1,\"plugin_id\":\"bark\"}"))
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                assert_eq!(response.status(), 200);
                let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
                assert_eq!(
                    serde_json::from_slice::<Value>(&bytes).unwrap()["accepted"],
                    accepted
                );
            }
        }
        let claim: Value = sqlx::query_scalar("SELECT notification_claim('bark')")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(claim["event_id"], 1);
        assert_eq!(claim["attempt"], ordinal);
        sqlx::query("UPDATE notification_delivery SET next_attempt_at=clock_timestamp()-interval '1 second'").execute(&pool).await.unwrap();
        if ordinal == 3 || ordinal == 6 {
            let exhausted: Option<Value> = sqlx::query_scalar("SELECT notification_claim('bark')")
                .fetch_one(&pool)
                .await
                .unwrap();
            assert!(exhausted.is_none());
        }
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM notification_attempt")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 6);
    let state: String = sqlx::query_scalar("SELECT state FROM requirement WHERE id=1")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(state, "Draft");
    let late: bool = sqlx::query_scalar("SELECT notification_ack('bark',1,1,'accepted')")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(!late);
    pool.close().await;
}

#[tokio::test]
async fn retained_workspaces_do_not_allow_stale_recovery_after_a_new_run() {
    let (root, repo, mut plan) = runner::fixture();
    plugin(&mut plan, "shell", unsupported());
    let (broker, manifest) = broker(&root, &repo);
    let candidate = validation_runner::candidate(&repo).unwrap();
    let pool = database().await;
    seed(&pool, &candidate, &json!(manifest)).await;
    assert!(
        !validation_service::validate(
            &pool,
            validation_service::Request {
                id: "old",
                source_run: "source",
                requirement: 1,
                revision: 1,
                checkout: &repo,
                directory: &root.join("old"),
                candidate: &candidate,
                plan: &plan
            }
        )
        .await
        .unwrap()
    );
    let mut newer = manifest.workspace.clone();
    newer.key.run_id = "later".into();
    newer.key.request_id = "later".into();
    newer.identity = "later".into();
    newer.branch = "ai/req-1-later".into();
    newer.path = broker.path("later").unwrap().to_string_lossy().into_owned();
    broker.prepare(&newer, true).unwrap();
    fs::write(
        Path::new(&newer.path).join("source"),
        "independent retained work",
    )
    .unwrap();
    sqlx::query("INSERT INTO agent_run(id,requirement_id,revision,incarnation,request_id,workspace,workspace_identity,launch,state,quiescent) VALUES('later',1,1,'boot','later',$1,'later','{}','Interrupted',true)").bind(&newer.path).execute(&pool).await.unwrap();
    let decision = recovery::Decision {
        request_id: "old-recovery".into(),
        version: 1,
        revision: 1,
        validation_id: "old".into(),
        reason: "attempt to revive old workspace".into(),
        action: recovery::Action::Revalidate {
            plan_digest: plan.identity().unwrap().config_sha256,
            resume_condition: "reviewed".into(),
        },
    };
    assert!(recovery::decide(&pool, 1, &decision).await.is_err());
    broker.verify(&manifest).unwrap();
    assert_eq!(validation_runner::candidate(&repo).unwrap(), candidate);
    assert_eq!(
        fs::read_to_string(Path::new(&newer.path).join("source")).unwrap(),
        "independent retained work"
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM candidate_validation")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    pool.close().await;
}

struct FailingWriter(usize);
impl std::io::Write for FailingWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.0 == 0 {
            return Err(std::io::Error::other("fixture output failure"));
        }
        let n = bytes.len().min(self.0);
        self.0 -= n;
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn serialization_failures<T: serde::Serialize>(value: &T) {
    let complete = serde_json::to_vec(value).unwrap();
    for limit in 0..complete.len() {
        assert!(
            serde_json::to_writer(FailingWriter(limit), value).is_err(),
            "truncated output at {limit} must not be accepted"
        );
    }
}

#[test]
fn repository_scopes_reject_ambiguous_references() {
    use codexsymphony_server::plugin_scope::{contains, repository_revision};
    for scope in ["all", "repository:42", "repositories:1,42"] {
        assert!(contains(scope, 42));
        assert!(!contains(scope, 0));
    }
    for scope in [
        "",
        "repository:01",
        "repository:2",
        "repositories:",
        "repositories:42,42",
        "repositories:0,42",
        "repositories:-1,42",
        "repositories:42,",
        "repositories:42,x",
        "repositories:01,42",
        "workspace:a",
    ] {
        assert!(!contains(scope, 42), "{scope}");
    }
    assert_eq!(repository_revision("repository:42@3"), Some((42, 3)));
    for value in [
        "",
        "x@1",
        "repository:1",
        "repository:x@1",
        "repository:1@x",
        "repository:0@1",
        "repository:1@0",
    ] {
        assert_eq!(repository_revision(value), None);
    }
}

#[tokio::test]
async fn repository_scopes_filter_before_outbox_and_recheck_revocation() {
    let pool = database().await;
    sqlx::raw_sql("INSERT INTO repository(id,version,document) VALUES(1,1,'{}'),(42,1,'{}');
        INSERT INTO plugin_scope(plugin_id,kind,repository_ids,enabled) VALUES('notification:A','all','{}',true),('notification:B','repositories','{42}',true);
        INSERT INTO notification_plugin VALUES('A','unused_scope_a',true),('B',session_user,true);
        INSERT INTO requirement(version,state,contract,repository_id) VALUES(1,'Draft','{}',1),(1,'Draft','{}',42),(1,'Draft','{}',42);")
        .execute(&pool).await.unwrap();
    let deliveries: Vec<(String,i64)> = sqlx::query_as("SELECT d.plugin_id,e.requirement_id FROM notification_delivery d JOIN lifecycle_event e ON e.id=d.event_id ORDER BY 1,2").fetch_all(&pool).await.unwrap();
    assert_eq!(
        deliveries,
        vec![
            ("A".into(), 1),
            ("A".into(), 2),
            ("A".into(), 3),
            ("B".into(), 2),
            ("B".into(), 3)
        ]
    );
    let event: Value = sqlx::query_scalar("SELECT notification_claim('B')")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(event["repository_id"], 42);
    assert_eq!(event["scope_version"], 1);
    let id = event["event_id"].as_i64().unwrap();
    sqlx::query("SELECT notification_ack('B',$1,1,'failed')")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE plugin_scope SET repository_ids='{1}' WHERE plugin_id='notification:B'")
        .execute(&pool)
        .await
        .unwrap();
    let blocked: Option<Value> = sqlx::query_scalar("SELECT notification_claim('B')")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(blocked.is_none());
    let replay: bool = sqlx::query_scalar("SELECT notification_replay(2,$1,'B')")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(!replay);
    let unchanged: (i32,i32,i64)=sqlx::query_as("SELECT attempts,attempt_limit,scope_version FROM notification_delivery WHERE plugin_id='B' AND event_id=$1").bind(id).fetch_one(&pool).await.unwrap();
    assert_eq!(unchanged, (1, 3, 1));
    // Expansion never backfills old events. all also applies to future repositories.
    sqlx::raw_sql("UPDATE plugin_scope SET kind='all',repository_ids='{}' WHERE plugin_id='notification:B'; INSERT INTO repository VALUES(77,1,'{}',0); INSERT INTO requirement(version,state,contract,repository_id) VALUES(1,'Draft','{}',77);").execute(&pool).await.unwrap();
    let old: i64=sqlx::query_scalar("SELECT count(*) FROM notification_delivery d JOIN lifecycle_event e ON e.id=d.event_id WHERE plugin_id='B' AND e.requirement_id=1").fetch_one(&pool).await.unwrap();
    assert_eq!(old, 0);
    let future: i64=sqlx::query_scalar("SELECT count(*) FROM notification_delivery d JOIN lifecycle_event e ON e.id=d.event_id WHERE e.repository_id=77").fetch_one(&pool).await.unwrap();
    assert_eq!(future, 2);
    for statement in [
        "INSERT INTO plugin_scope(plugin_id,kind,repository_ids,enabled) VALUES('bad','repositories','{}',true)",
        "INSERT INTO plugin_scope(plugin_id,kind,repository_ids,enabled) VALUES('bad','repositories','{42,42}',true)",
        "INSERT INTO plugin_scope(plugin_id,kind,repository_ids,enabled) VALUES('bad','repositories','{999}',true)",
        "INSERT INTO plugin_scope(plugin_id,kind,repository_ids,enabled) VALUES('bad','repositories','{NULL}',true)",
        "INSERT INTO plugin_scope(plugin_id,kind,repository_ids,enabled) VALUES('bad','all','{42}',true)",
        "INSERT INTO notification_plugin VALUES('missing','missing_scope_role',true)",
        "UPDATE plugin_scope SET plugin_id='renamed' WHERE plugin_id='notification:B'",
    ] {
        assert!(
            sqlx::query(statement).execute(&pool).await.is_err(),
            "{statement}"
        );
    }
    pool.close().await;
}

#[tokio::test]
async fn execution_scope_freezes_repository_and_version_across_runs() {
    use codexsymphony_server::plugin_scope::admit;
    let pool = database().await;
    sqlx::raw_sql("INSERT INTO repository(id,version,document) VALUES(1,1,'{}'),(42,1,'{}'); INSERT INTO requirement(version,state,contract,repository_id) VALUES(1,'Draft','{}',1),(1,'Draft','{}',42); UPDATE plugin_scope SET kind='repositories',repository_ids='{42}' WHERE plugin_id='agent:codex';").execute(&pool).await.unwrap();
    assert!(admit(&pool, "agent:codex", "run-a", 1, 0).await.is_err());
    for run in ["run-a", "run-b"] {
        admit(&pool, "agent:codex", run, 2, 0).await.unwrap();
    }
    admit(&pool, "agent:codex", "run-a", 2, 0).await.unwrap();
    assert!(admit(&pool, "missing", "run-c", 2, 0).await.is_err());
    assert!(admit(&pool, "agent:codex", "run-a", 2, 1).await.is_err());
    sqlx::query(
        "UPDATE plugin_scope SET kind='all',repository_ids='{}' WHERE plugin_id='agent:codex'",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert!(admit(&pool, "agent:codex", "run-a", 1, 0).await.is_err());
    admit(&pool, "agent:codex", "run-c", 1, 0).await.unwrap();
    let frozen: Vec<i64>=sqlx::query_scalar("SELECT scope_version FROM plugin_scope_invocation WHERE invocation_id IN ('run-a','run-b') ORDER BY invocation_id").fetch_all(&pool).await.unwrap();
    assert_eq!(frozen, vec![2, 2]);
    sqlx::query("UPDATE plugin_scope SET enabled=false WHERE plugin_id='agent:codex'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(admit(&pool, "agent:codex", "run-a", 2, 0).await.is_err());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM plugin_scope_invocation")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 3);
    pool.close().await;
}

#[tokio::test]
async fn scope_migration_preserves_existing_global_subscriptions_and_frozen_hooks() {
    let options: PgConnectOptions = std::env::var("TEST_DATABASE_URL").unwrap().parse().unwrap();
    let admin = PgPoolOptions::new()
        .connect_with(options.clone())
        .await
        .unwrap();
    let schema = format!(
        "scope_upgrade_{}",
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
        .connect_with(options.options([("search_path", schema)]))
        .await
        .unwrap();
    for migration in sqlx::migrate!("../../migrations").iter() {
        if migration.version >= 37 {
            break;
        }
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
    }
    sqlx::raw_sql("INSERT INTO notification_plugin VALUES('legacy',session_user,true);
        INSERT INTO repository(id,version,document) VALUES(42,1,'{\"hooks\":[{\"name\":\"installed\"}]}');
        INSERT INTO requirement(version,state,contract,repository_id) VALUES(1,'Draft','{}',42);
        INSERT INTO requirement_revision VALUES(1,1,'{\"repository_id\":42,\"repository\":{\"hooks\":[{\"name\":\"retained\"}]}}');").execute(&pool).await.unwrap();
    sqlx::raw_sql(include_str!(
        "../../../migrations/0037_plugin_repository_scope.sql"
    ))
    .execute(&pool)
    .await
    .unwrap();
    let scopes:Vec<(String,String)>=sqlx::query_as("SELECT plugin_id,kind FROM plugin_scope WHERE plugin_id LIKE 'hook:%' OR plugin_id='notification:legacy' ORDER BY 1").fetch_all(&pool).await.unwrap();
    assert_eq!(
        scopes,
        vec![
            ("hook:installed".into(), "all".into()),
            ("hook:retained".into(), "all".into()),
            ("notification:legacy".into(), "all".into())
        ]
    );
    let event: Value = sqlx::query_scalar("SELECT notification_claim('legacy')")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(event["repository_id"], 42);
    assert_eq!(event["scope_version"], 1);
    assert_eq!(event["attempt"], 1);
    pool.close().await;
}

#[tokio::test]
async fn controlled_scope_registration_rejects_unknown_and_duplicate_repository_ids() {
    let pool = database().await;
    sqlx::query("INSERT INTO repository(id,version,document) VALUES(1,1,'{}'),(42,1,'{}')")
        .execute(&pool)
        .await
        .unwrap();
    for (scope, valid) in [
        ("all", true),
        ("repository:42", true),
        ("repositories:1,42", true),
        ("repositories:42,42", false),
        ("repositories:42,999", false),
        ("", false),
        ("repositories:", false),
        ("repository:01", false),
    ] {
        let plan = json!({"controlled":{"extensions":[{"scope_ref":scope}]}}).to_string();
        let result = sqlx::query(
            "UPDATE repository SET document=jsonb_build_object('environment',$1::text) WHERE id=42",
        )
        .bind(plan)
        .execute(&pool)
        .await;
        assert_eq!(result.is_ok(), valid, "{scope}: {result:?}");
    }
    pool.close().await;
}
