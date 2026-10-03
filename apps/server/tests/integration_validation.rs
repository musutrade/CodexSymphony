//! Actual PostgreSQL + supervised validation commands. No GitHub/model writes.
use codexsymphony_server::{
    budget::Amount,
    git_broker::GitBroker,
    group_budget_increase::{self, ChildIncrease, GroupIncrease},
    group_queue_store, integration_worker, process, run_store,
    validation_runner::{self, Plan},
};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::io::AsyncWriteExt;
#[path = "support/diagnostics.rs"]
mod diagnostic_source;
#[path = "support/groups.rs"]
mod groups;
#[path = "support/validation_runner.rs"]
mod source;
struct Fixture {
    pool: PgPool,
    url: String,
    root: PathBuf,
    broker: GitBroker,
    plan: Plan,
    draft: String,
}

async fn budget_admin(f: &Fixture, command: &str, payload: Value) -> bool {
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_codexsymphony-server"))
        .args(["budget", command, "--stdin-json"])
        .env("DATABASE_URL", &f.url)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.to_string().as_bytes())
        .await
        .unwrap();
    child.wait().await.unwrap().success()
}

#[tokio::test]
async fn in_flight_group_increase_preserves_exposure_and_original_accounts() {
    let f = fixture("dependencies", false).await;
    assert!(!budget_admin(&f, "repair-recheck", json!({})).await);
    let used = json!({"tokens":125,"turns":1,"model_seconds":10});
    let reserved = json!({"tokens":0,"turns":0,"model_seconds":40});
    sqlx::query(
        "UPDATE group_budget SET used=$2,reserved=$3 WHERE draft_id=$1 AND item_id IN ('','C1')",
    )
    .bind(&f.draft)
    .bind(&used)
    .bind(&reserved)
    .execute(&f.pool)
    .await
    .unwrap();
    sqlx::query("UPDATE requirement_budget SET exhausted=true WHERE requirement_id IN (SELECT requirement_id FROM group_execution_item WHERE draft_id=$1)")
        .bind(&f.draft).execute(&f.pool).await.unwrap();
    let bound: Vec<(String, i64, i64)> = sqlx::query_as("SELECT i.child_id,i.requirement_id,b.version FROM group_execution_item i JOIN requirement_budget b ON b.requirement_id=i.requirement_id WHERE i.draft_id=$1 ORDER BY i.child_id")
        .bind(&f.draft).fetch_all(&f.pool).await.unwrap();
    assert_eq!(bound.len(), 4);
    let delta = Amount {
        tokens: 100,
        turns: 2,
        model_seconds: 60,
    };
    let children: Vec<_> = bound
        .iter()
        .map(
            |(child_id, requirement_id, expected_version)| ChildIncrease {
                child_id: child_id.clone(),
                requirement_id: *requirement_id,
                expected_version: *expected_version,
                delta,
            },
        )
        .collect();
    let grant = GroupIncrease {
        request_id: "reviewed-live-group-increase".into(),
        draft_id: f.draft.clone(),
        expected_queue_version: 1,
        actor: "local-user".into(),
        reason: "explicit cumulative increase".into(),
        parent_delta: Amount {
            tokens: 400,
            turns: 8,
            model_seconds: 240,
        },
        children,
    };
    let mut invalid = grant.clone();
    invalid.parent_delta.tokens -= 1;
    assert!(
        group_budget_increase::increase(&f.pool, &invalid)
            .await
            .is_err()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM budget_authorization WHERE version=2")
            .fetch_one(&f.pool)
            .await
            .unwrap(),
        0
    );
    let mut overflow = grant.clone();
    overflow.children[0].delta.tokens = i64::MAX;
    assert!(
        group_budget_increase::increase(&f.pool, &overflow)
            .await
            .is_err()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM budget_authorization WHERE version=2")
            .fetch_one(&f.pool)
            .await
            .unwrap(),
        0
    );
    assert!(!budget_admin(&f, "increase", json!({})).await);
    assert!(budget_admin(&f, "group-increase", json!(grant)).await);
    group_budget_increase::increase(&f.pool, &grant)
        .await
        .unwrap();
    let ledgers: Vec<(String, Value, Value, Value)> = sqlx::query_as(
        "SELECT item_id,limits,used,reserved FROM group_budget WHERE draft_id=$1 ORDER BY item_id",
    )
    .bind(&f.draft)
    .fetch_all(&f.pool)
    .await
    .unwrap();
    assert_eq!(ledgers.len(), 5);
    assert_eq!(
        ledgers[0].1,
        json!({"tokens":800,"turns":16,"model_seconds":480})
    );
    for row in &ledgers[1..] {
        assert_eq!(row.1, json!({"tokens":200,"turns":4,"model_seconds":120}));
    }
    assert_eq!(ledgers[0].2, used);
    assert_eq!(ledgers[0].3, reserved);
    assert_eq!(ledgers[1].2, used);
    assert_eq!(ledgers[1].3, reserved);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM requirement_budget WHERE version=2 AND NOT exhausted"
        )
        .fetch_one(&f.pool)
        .await
        .unwrap(),
        4
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM budget_authorization WHERE version=2")
            .fetch_one(&f.pool)
            .await
            .unwrap(),
        4
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM group_queue_event WHERE request_id=$1")
            .bind(&grant.request_id)
            .fetch_one(&f.pool)
            .await
            .unwrap(),
        1
    );
    let mut stale = grant.clone();
    stale.request_id = "stale-live-group-increase".into();
    assert!(
        group_budget_increase::increase(&f.pool, &stale)
            .await
            .is_err()
    );
    let mut conflict = grant.clone();
    conflict.reason = "different approval".into();
    assert!(
        group_budget_increase::increase(&f.pool, &conflict)
            .await
            .is_err()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM budget_authorization WHERE version>2")
            .fetch_one(&f.pool)
            .await
            .unwrap(),
        0
    );
    let mut selective = grant.clone();
    selective.request_id = "c1-tokens-only-increase".into();
    selective.parent_delta = Amount {
        tokens: 100,
        ..Amount::default()
    };
    for child in &mut selective.children {
        child.expected_version = 2;
        child.delta = if child.child_id == "C1" {
            selective.parent_delta
        } else {
            Amount::default()
        };
    }
    let mut missing = selective.clone();
    missing.children.pop();
    assert!(
        group_budget_increase::increase(&f.pool, &missing)
            .await
            .is_err()
    );
    let mut empty = selective.clone();
    empty.parent_delta = Amount::default();
    for child in &mut empty.children {
        child.delta = Amount::default();
    }
    assert!(
        group_budget_increase::increase(&f.pool, &empty)
            .await
            .is_err()
    );
    let mut negative = selective.clone();
    negative.children[1].delta.tokens = -1;
    negative.parent_delta.tokens -= 1;
    assert!(
        group_budget_increase::increase(&f.pool, &negative)
            .await
            .is_err()
    );
    assert!(budget_admin(&f, "group-increase", json!(selective)).await);
    assert!(budget_admin(&f, "group-increase", json!(selective)).await);
    let after: Vec<(String, Value, Value, Value)> = sqlx::query_as(
        "SELECT item_id,limits,used,reserved FROM group_budget WHERE draft_id=$1 ORDER BY item_id",
    )
    .bind(&f.draft)
    .fetch_all(&f.pool)
    .await
    .unwrap();
    for (before, after) in ledgers.iter().zip(&after) {
        let mut expected = before.1.clone();
        if before.0.is_empty() || before.0 == "C1" {
            expected["tokens"] = json!(before.1["tokens"].as_i64().unwrap() + 100);
        }
        assert_eq!(after.1, expected);
        assert_eq!(after.2, before.2);
        assert_eq!(after.3, before.3);
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM budget_authorization WHERE version=3")
            .fetch_one(&f.pool)
            .await
            .unwrap(),
        4
    );
    f.pool.close().await;
    std::fs::remove_dir_all(f.root).unwrap();
}
fn git(path: &Path, args: &[&str]) {
    let result = std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}
async fn fixture(mode: &str, two: bool) -> Fixture {
    let (pool, url, _) = groups::fixture().await;
    let (root, repo, mut plan) = source::fixture();
    if mode == "diagnostics" {
        let diagnostic_root = root.join("diagnostic-source");
        std::fs::create_dir(&diagnostic_root).unwrap();
        let (_, diagnostic_plan) = diagnostic_source::subprocess(&diagnostic_root);
        plan = diagnostic_plan;
        plan.steps[0].id = "test".into();
    }
    if mode == "linked" {
        std::fs::write(
            &plan.entry,
            "#!/bin/sh\necho 'AssertionError: integration source invariant'\nexit 1\n",
        )
        .unwrap();
        plan.entry_sha256 =
            codexsymphony_server::validation::sha256(std::fs::read(&plan.entry).unwrap());
    }
    if mode == "infrastructure" {
        std::fs::write(&plan.entry, "#!/bin/sh\nif [ ! -f \"$(dirname \"$0\")/service-ready\" ]; then echo service unavailable; exit 1; fi\ncat source\n").unwrap();
        plan.entry_sha256 =
            codexsymphony_server::validation::sha256(std::fs::read(&plan.entry).unwrap());
    }
    plan.steps[0]
        .command
        .extend((!mode.is_empty()).then(|| mode.to_owned()));
    let sha = validation_runner::candidate(&repo).unwrap().sha;
    let bundle = root.join("seed.bundle");
    git(
        &repo,
        &["bundle", "create", bundle.to_str().unwrap(), "--all"],
    );
    let broker = GitBroker::initialize(&root.join("workspaces"), &bundle).unwrap();
    let mut repositories = vec![
        json!({"repository_id":1,"repository_version":1,"repair_scope":"only reviewed AC","selection":{"kind":"fixed","sha":sha}}),
    ];
    if two {
        let mut second = groups::repository();
        second["remote"] = json!("test/second");
        second["github_repository_id"] = json!(456);
        sqlx::query("INSERT INTO repository(id,version,document) VALUES(2,1,$1)")
            .bind(second)
            .execute(&pool)
            .await
            .unwrap();
        let (other, other_repo, _) = source::fixture();
        std::fs::write(other_repo.join("source"), "independent repository").unwrap();
        git(&other_repo, &["commit", "-am", "second version"]);
        let second_sha = validation_runner::candidate(&other_repo).unwrap().sha;
        git(
            &root.join("workspaces/canonical.git"),
            &["fetch", other_repo.to_str().unwrap(), &second_sha],
        );
        repositories.push(json!({"repository_id":2,"repository_version":1,"repair_scope":"none","selection":{"kind":"fixed","sha":second_sha}}));
        std::fs::remove_dir_all(other).unwrap();
    }
    let dependencies = mode == "dependencies";
    let mut document = groups::sample();
    let mut child = document["children"][3].clone();
    child["depends_on"] = json!([]);
    child["order"] = json!(1);
    if !dependencies {
        document["children"] = json!([child]);
    }
    let mut review = groups::review();
    let mut item = review["items"][3].clone();
    item["integration"] = json!({"configuration_sha256":plan.identity().unwrap().config_sha256,"repositories":repositories});
    if dependencies {
        item["integration"]["repositories"][0]["selection"] =
            json!({"kind":"completed_dependencies"});
    }
    if dependencies {
        review["items"][3] = item;
    } else {
        review["items"] = json!([item]);
    }
    if mode == "chain" {
        let mut successor = document["children"][0].clone();
        successor["id"] = json!("C5");
        successor["order"] = json!(2);
        successor["depends_on"] = json!(["C4"]);
        document["children"].as_array_mut().unwrap().push(successor);
        let mut successor = review["items"][0].clone();
        successor["child_id"] = json!("C5");
        review["items"].as_array_mut().unwrap().push(successor);
        review["coverage"][0]["child_id"] = json!("C5");
    }
    let app = groups::app(&pool);
    let saved = groups::request(&app, "POST", "/api/drafts", groups::body(document, 0), 200).await;
    let draft = saved["id"].as_str().unwrap().to_owned();
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
        json!({"version":1,"draft_revision":1,"request_id":process::new_identity().unwrap()}),
        200,
    )
    .await;
    run_store::begin_incarnation(&pool, "boot").await.unwrap();
    sqlx::query("UPDATE execution_control SET recovery_complete=true")
        .execute(&pool)
        .await
        .unwrap();
    group_queue_store::materialize(&pool).await.unwrap();
    Fixture {
        pool,
        url,
        root,
        broker,
        plan,
        draft,
    }
}
async fn tick(f: &Fixture) {
    codexsymphony_server::runtime_service::tick(
        &f.pool,
        &f.root,
        Path::new(env!("CARGO_BIN_EXE_codexsymphony-server")),
        &f.broker,
        "boot",
        &codexsymphony_server::runtime_service::Config {
            validation: Some(f.plan.clone()),
            settings: codexsymphony_server::runtime_client::Settings {
                model_capabilities: None,
                startup_seconds: 5,
                response_seconds: 5,
                stall_seconds: 5,
                reservation: codexsymphony_server::budget::Amount {
                    tokens: 100,
                    turns: 1,
                    model_seconds: 30,
                },
                codex_config: String::new(),
            },
            preparation_adapter: "/bin/true".into(),
            preparation: json!({"launcher":["/bin/true"]}),
        },
    )
    .await
    .unwrap();
    let view = groups::request(
        &groups::app(&f.pool),
        "GET",
        &format!("/api/drafts/{}/review", f.draft),
        Value::Null,
        200,
    )
    .await;
    if view["execution"]["completed"] != view["execution"]["total"] {
        assert_ne!(view["execution"]["parent_state"], "Done");
    }
}
async fn complete(f: &Fixture, expected: &str) {
    for _ in 0..200 {
        tick(f).await;
        let state: Option<String> = sqlx::query_scalar(
            "SELECT state FROM integration_validation ORDER BY created_at DESC LIMIT 1",
        )
        .fetch_optional(&f.pool)
        .await
        .unwrap();
        if state.as_deref() == Some(expected) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("integration did not reach {expected}");
}
async fn counts(f: &Fixture) -> (i64, i64, i64) {
    sqlx::query_as("SELECT (SELECT count(*) FROM agent_run),(SELECT count(*) FROM delivery),(SELECT count(*) FROM group_acceptance)").fetch_one(&f.pool).await.unwrap()
}
#[tokio::test]
async fn no_code_difference_completes_child_parent_and_releases_owner() {
    let f = fixture("", false).await;
    complete(&f, "passed").await;
    assert_eq!(counts(&f).await, (0, 0, 1));
    let owner: Option<i64> = sqlx::query_scalar("SELECT requirement_id FROM execution_control")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(owner, None);
    tick(&f).await;
    assert_eq!(counts(&f).await, (0, 0, 1));
    let view = groups::request(
        &groups::app(&f.pool),
        "GET",
        &format!("/api/drafts/{}/review", f.draft),
        Value::Null,
        200,
    )
    .await;
    assert_eq!(view["execution"]["parent_state"], "Done");
    assert_eq!(view["business_complete"], true);
}
#[tokio::test]
async fn independent_repositories_freeze_distinct_versions_without_cross_ancestry() {
    let f = fixture("", true).await;
    complete(&f, "passed").await;
    let b: Value = sqlx::query_scalar("SELECT binding FROM integration_validation")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(b["versions"].as_array().unwrap().len(), 2);
    assert_ne!(
        b["versions"][0]["candidate"]["sha"],
        b["versions"][1]["candidate"]["sha"]
    );
    assert_eq!(counts(&f).await, (0, 0, 1));
}
#[tokio::test]
async fn failed_required_check_retains_owner_without_parent_done_or_repair_child() {
    let f = fixture("fail", false).await;
    complete(&f, "failed").await;
    assert_eq!(counts(&f).await, (0, 0, 0));
    let owner: Option<i64> = sqlx::query_scalar("SELECT requirement_id FROM execution_control")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert!(owner.is_some());
    tick(&f).await;
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM group_execution_item")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(n, 1);
}

async fn diagnostic_api_read(pool: &PgPool, path: &str) -> (u16, Value) {
    use tower::ServiceExt;
    let response = codexsymphony_server::diagnostic_api::routes()
        .with_state(pool.clone())
        .oneshot(
            axum::http::Request::builder()
                .uri(path)
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn integration_reports_survive_cleanup_and_only_the_linked_repair_can_read_them() {
    use codexsymphony_server::{
        diagnostic_store,
        diagnostics::{Chunk, Page},
        execution::RunKey,
        runtime_store, runtime_tools,
        validation::sha256,
    };
    let f = fixture("diagnostics", true).await;
    complete(&f, "failed").await;
    let (invocation, requirement, original): (String, i64, Value) =
        sqlx::query_as("SELECT id,requirement_id,result FROM integration_validation")
            .fetch_one(&f.pool)
            .await
            .unwrap();
    let mut tx = f.pool.begin().await.unwrap();
    let page = diagnostic_store::list_in(&mut tx, requirement, None, 0)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(page.artifacts.len(), 4);
    let expected = diagnostic_source::binding(&invocation, 1);
    for artifact in &page.artifacts {
        assert_eq!(artifact.binding.identity.run_id, None);
        assert_eq!(
            artifact.binding.identity.invocation_id,
            expected.identity.invocation_id
        );
        assert_eq!(artifact.binding.identity.attempt, expected.identity.attempt);
        assert_eq!(artifact.binding.generation, expected.generation);
        assert_eq!(
            artifact.binding.validation_id.as_deref(),
            Some(invocation.as_str())
        );
    }
    // Reconciliation repeats capture idempotently without replacing raw bytes.
    tick(&f).await;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM diagnostic_artifact")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(count, 4);
    std::fs::remove_dir_all(f.root.join(&invocation).join("checks")).unwrap();
    let key = RunKey {
        run_id: "diagnostic-repair".into(),
        request_id: "diagnostic-request".into(),
        incarnation: "boot".into(),
    };
    sqlx::query("INSERT INTO agent_run(id,requirement_id,revision,incarnation,request_id,workspace,workspace_identity,launch,state,model) VALUES($1,$2,1,'boot',$3,'/fixture','fixture','{}','Created','fixture-model')")
        .bind(&key.run_id).bind(requirement).bind(&key.request_id).execute(&f.pool).await.unwrap();
    runtime_store::open(&f.pool, &key, 100).await.unwrap();
    runtime_store::thread(&f.pool, &key, "diagnostic-thread", 100)
        .await
        .unwrap();
    runtime_store::turn(&f.pool, &key, "turn", "call", 100)
        .await
        .unwrap();
    let request = |tool: &str, args: Value| json!({"id":"diagnostic-rpc","method":"item/tool/call","params":{"threadId":"diagnostic-thread","turnId":"turn","callId":"read","tool":tool,"arguments":args}});
    let response = runtime_tools::handle(
        &f.pool,
        &f.broker,
        &key,
        &request("list_diagnostics", json!({"after":0})),
    )
    .await
    .unwrap();
    let denied: Page =
        serde_json::from_str(response["contentItems"][0]["text"].as_str().unwrap()).unwrap();
    assert!(denied.artifacts.is_empty());
    let response = runtime_tools::handle(
        &f.pool,
        &f.broker,
        &key,
        &request(
            "read_diagnostic",
            json!({"artifact_id":page.artifacts[0].artifact_id,"offset":0,"limit":8192}),
        ),
    )
    .await
    .unwrap();
    assert_eq!(response["success"], false);
    // Explicit persisted fixture authorization; no paid Runtime or new delivery.
    sqlx::query("INSERT INTO linked_failure(id,requirement_id,revision,integration_id,evidence,required_steps,state) VALUES('diagnostic-linked',$1,1,$2,'{}','[]','reserved')")
        .bind(requirement).bind(&invocation).execute(&f.pool).await.unwrap();
    sqlx::query("INSERT INTO repair_reservation(requirement_id,ordinal,linked_failure_id,repair_run_id,failure,status) VALUES($1,1,'diagnostic-linked',$2,'{}','started')")
        .bind(requirement).bind(&key.run_id).execute(&f.pool).await.unwrap();
    for (source, call) in [
        (&invocation[..], "unrelated"),
        ("unrelated", &invocation[..]),
    ] {
        let allowed: bool = sqlx::query_scalar("SELECT diagnostic_run_allows($1,$2,$3)")
            .bind(&key.run_id)
            .bind(source)
            .bind(call)
            .fetch_one(&f.pool)
            .await
            .unwrap();
        assert!(
            !allowed,
            "integration repair must bind both source and invocation"
        );
    }
    let context = codexsymphony_server::diagnostic_tools::context(&f.pool, &key)
        .await
        .unwrap();
    assert_eq!(context["manifest"]["artifacts"], json!(page.artifacts));
    // The paid Runtime prompt carries identities/digests, never the long original
    // report. The same retained failure and all original output bytes stay intact.
    let repair_failure = json!({"original_failure":"diagnostic-linked","evidence":original["evidence"],"authorized_paths":["index.js"],"baseline":"original"});
    sqlx::query("UPDATE repair_reservation SET failure=$2 WHERE repair_run_id=$1")
        .bind(&key.run_id)
        .bind(&repair_failure)
        .execute(&f.pool)
        .await
        .unwrap();
    let prompt = runtime_store::input(&f.pool, &key).await.unwrap();
    let projected: Value = serde_json::from_str(&prompt).unwrap();
    assert!(projected["repair_context"]["evidence"].is_null());
    assert_eq!(
        projected["repair_context"]["evidence_metadata"]["candidate"],
        original["evidence"]["candidate"]
    );
    assert_eq!(
        projected["diagnostics"]["manifest"]["artifacts"],
        json!(page.artifacts)
    );
    for (metadata, original_step) in projected["repair_context"]["evidence_metadata"]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .zip(original["evidence"]["steps"].as_array().unwrap())
    {
        assert!(metadata.get("output").is_none());
        assert_eq!(metadata["output_sha256"], original_step["output_sha256"]);
        assert_eq!(
            metadata["output_bytes"],
            original_step["output"].as_str().unwrap().len()
        );
    }
    assert!(prompt.len() < original["evidence"].to_string().len() / 2);
    let retained: Value =
        sqlx::query_scalar("SELECT failure FROM repair_reservation WHERE repair_run_id=$1")
            .bind(&key.run_id)
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(retained, repair_failure);
    let retained_result: Value =
        sqlx::query_scalar("SELECT result FROM integration_validation WHERE id=$1")
            .bind(&invocation)
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(retained_result, original);
    let mut records = Vec::new();
    for artifact in &page.artifacts {
        let mut bytes = Vec::new();
        let mut offset = 0;
        loop {
            let response = runtime_tools::handle(
                &f.pool,
                &f.broker,
                &key,
                &request(
                    "read_diagnostic",
                    json!({"artifact_id":artifact.artifact_id,"offset":offset,"limit":8192}),
                ),
            )
            .await
            .unwrap();
            assert_eq!(response["success"], true);
            let chunk: Chunk =
                serde_json::from_str(response["contentItems"][0]["text"].as_str().unwrap())
                    .unwrap();
            assert_eq!(chunk.artifact, *artifact);
            assert_eq!(chunk.offset, offset);
            bytes.extend_from_slice(chunk.text.as_bytes());
            offset = chunk.next;
            if chunk.end {
                break;
            }
        }
        assert_eq!(bytes.len() as u64, artifact.export_bytes);
        assert_eq!(Some(sha256(&bytes)), artifact.export_sha256);
        if bytes.len() > 8192 {
            let text = std::str::from_utf8(&bytes).unwrap();
            assert!(text.contains("FIRST") && text.contains("LAST"));
        }
        records.push(json!({"artifact":artifact,"bytes":bytes.len(),"sha256":sha256(&bytes)}));
    }
    let secondary: Value = sqlx::query_scalar("SELECT document FROM repository WHERE id=2")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    for (revoked, version, cutoff) in [(true, 1_i64, 0_i64), (false, 2, 1)] {
        sqlx::query("UPDATE repository SET document=jsonb_set(document,'{revoked}',$1),version=$2,revoked_through_version=$3 WHERE id=2")
            .bind(json!(revoked)).bind(version).bind(cutoff).execute(&f.pool).await.unwrap();
        let response = runtime_tools::handle(
            &f.pool,
            &f.broker,
            &key,
            &request("list_diagnostics", json!({"after":0})),
        )
        .await
        .unwrap();
        let denied: Page =
            serde_json::from_str(response["contentItems"][0]["text"].as_str().unwrap()).unwrap();
        assert!(denied.artifacts.is_empty());
        let response = runtime_tools::handle(
            &f.pool,
            &f.broker,
            &key,
            &request(
                "read_diagnostic",
                json!({"artifact_id":page.artifacts[0].artifact_id,"offset":0,"limit":8192}),
            ),
        )
        .await
        .unwrap();
        assert_eq!(response["success"], false);
        let (status, denied) = diagnostic_api_read(
            &f.pool,
            &format!("/api/requirements/{requirement}/diagnostics/0"),
        )
        .await;
        assert_eq!(status, 200);
        assert!(denied["artifacts"].as_array().unwrap().is_empty());
        let (status, _) = diagnostic_api_read(
            &f.pool,
            &format!(
                "/api/requirements/{requirement}/diagnostic-artifacts/{}/0/8192",
                page.artifacts[0].artifact_id
            ),
        )
        .await;
        assert_eq!(status, 409);
    }
    // Restore only this isolated fixture's grant; live historical revocations are immutable.
    sqlx::query("UPDATE repository SET document=$1,version=1,revoked_through_version=0 WHERE id=2")
        .bind(secondary)
        .execute(&f.pool)
        .await
        .unwrap();
    let context = codexsymphony_server::diagnostic_tools::context(&f.pool, &key)
        .await
        .unwrap();
    assert_eq!(context["manifest"]["artifacts"], json!(page.artifacts));
    sqlx::query("UPDATE repair_reservation SET status='failed' WHERE requirement_id=$1")
        .bind(requirement)
        .execute(&f.pool)
        .await
        .unwrap();
    let response = runtime_tools::handle(
        &f.pool,
        &f.broker,
        &key,
        &request(
            "read_diagnostic",
            json!({"artifact_id":page.artifacts[0].artifact_id,"offset":0,"limit":8192}),
        ),
    )
    .await
    .unwrap();
    assert_eq!(response["success"], false);
    let retained: Value =
        sqlx::query_scalar("SELECT result FROM integration_validation WHERE id=$1")
            .bind(&invocation)
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(retained, original);
    std::fs::write(f.root.join("integration-diagnostic-evidence.json"), serde_json::to_vec_pretty(&json!({"kind":"controlled subprocess and PostgreSQL fixture; no paid model","invocation":invocation,"artifacts":records,"original_result_preserved":true,"nonprimary_revocation_denied":true,"historical_grant_denied_after_reauthorization":true})).unwrap()).unwrap();
    println!(
        "GH-90 integration diagnostics: {}",
        f.root
            .join("integration-diagnostic-evidence.json")
            .display()
    );
    f.pool.close().await;
}

#[tokio::test]
async fn missing_step_and_changed_configuration_never_claim() {
    let mut f = fixture("", false).await;
    f.plan.steps[0].id = "wrong".into();
    assert!(
        integration_worker::tick(
            &f.pool,
            &f.root,
            Path::new(env!("CARGO_BIN_EXE_codexsymphony-server")),
            &f.broker,
            "boot",
            &f.plan
        )
        .await
        .is_err()
    );
    assert_eq!(counts(&f).await, (0, 0, 0));
    let owner: Option<i64> = sqlx::query_scalar("SELECT requirement_id FROM execution_control")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(owner, None);
}
#[tokio::test]
async fn cancellation_and_unknown_spawn_keep_outcomes_separate() {
    let f = fixture("", false).await;
    tick(&f).await;
    let id: String = sqlx::query_scalar("SELECT id FROM integration_validation")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE integration_validation SET state='executing'")
        .execute(&f.pool)
        .await
        .unwrap();
    tick(&f).await;
    let state: String = sqlx::query_scalar("SELECT state FROM integration_validation")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(state, "unknown");
    assert!(!f.root.join(&id).exists());
    let launch: Value = sqlx::query_scalar("SELECT launch FROM integration_validation")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    let mut identity = process::identity(std::process::id()).unwrap();
    identity.pid = u32::MAX;
    std::fs::create_dir(f.root.join(&id)).unwrap();
    process::durable_write(
        &f.root.join(&id).join("identity.json"),
        &json!({"key":launch["key"],"process":identity}),
    )
    .unwrap();
    tick(&f).await;
    let blocker: String = sqlx::query_scalar("SELECT blocker FROM integration_validation")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert!(blocker.contains("descendant quiescence unknown"));
    process::durable_write(
        &f.root.join(&id).join("quiescent.json"),
        &json!({"key":launch["key"],"process":process::identity(std::process::id()).unwrap()}),
    )
    .unwrap();
    assert!(
        codexsymphony_server::coordinator::recover(&f.pool, &f.root, "boot")
            .await
            .is_err()
    );
    sqlx::query("UPDATE storage_guard SET blocked=true")
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(
        codexsymphony_server::coordinator::recover(&f.pool, &f.root, "boot")
            .await
            .is_err()
    );
    sqlx::query("UPDATE storage_guard SET blocked=false")
        .execute(&f.pool)
        .await
        .unwrap();
    let requirement: i64 = sqlx::query_scalar("SELECT requirement_id FROM integration_validation")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    codexsymphony_server::delivery_control::cancel(&f.pool, requirement)
        .await
        .unwrap();
    codexsymphony_server::delivery_control::settle(&f.pool)
        .await
        .unwrap();
    let owner: Option<i64> = sqlx::query_scalar("SELECT requirement_id FROM execution_control")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(owner, Some(requirement));
    assert_eq!(counts(&f).await, (0, 0, 0));
}
#[tokio::test]
async fn source_mutation_and_forged_binding_cannot_complete() {
    let f = fixture("mutate", false).await;
    complete(&f, "failed").await;
    assert_eq!(counts(&f).await, (0, 0, 0));
    let id: String = sqlx::query_scalar("SELECT id FROM integration_validation")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    let path = f.root.join(id).join("outcome.json");
    let mut outcome: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    outcome["binding_sha256"] = json!("f".repeat(64));
    process::durable_write(&path, &outcome).unwrap();
    assert!(
        integration_worker::tick(
            &f.pool,
            &f.root,
            Path::new(env!("CARGO_BIN_EXE_codexsymphony-server")),
            &f.broker,
            "boot",
            &f.plan
        )
        .await
        .is_err()
    );
    assert_eq!(counts(&f).await, (0, 0, 0));
}
#[tokio::test]
async fn pause_stops_supervised_work_and_restart_never_unpauses() {
    let mut f = fixture("timeout", false).await;
    f.plan.steps[0].timeout_seconds = 5;
    // This fixture changes the reviewed configuration before any claim.
    sqlx::query("UPDATE group_execution_item SET input=jsonb_set(input,'{review,integration,configuration_sha256}',$1)").bind(json!(f.plan.identity().unwrap().config_sha256)).execute(&f.pool).await.unwrap();
    tick(&f).await;
    tick(&f).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    tick(&f).await;
    sqlx::query("UPDATE execution_control SET paused=true")
        .execute(&f.pool)
        .await
        .unwrap();
    for _ in 0..100 {
        tick(&f).await;
        let quiet: bool = sqlx::query_scalar("SELECT quiescent FROM integration_validation")
            .fetch_one(&f.pool)
            .await
            .unwrap();
        if quiet {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    run_store::begin_incarnation(&f.pool, "restart")
        .await
        .unwrap();
    assert!(
        codexsymphony_server::coordinator::recover(&f.pool, &f.root, "restart")
            .await
            .unwrap()
    );
    let paused: bool = sqlx::query_scalar("SELECT paused FROM execution_control")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert!(paused);
    assert_eq!(counts(&f).await, (0, 0, 0));
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM integration_validation")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(n, 1);
}

#[test]
fn explicit_authorization_rejects_stale_scopes_and_missing_repository_coverage() {
    use codexsymphony_server::{
        draft::Document,
        group_review::{Item, RepositorySnapshot},
        integration::{self, Authorization},
    };
    let document: Document = serde_json::from_value(groups::sample()).unwrap();
    let item: Item = serde_json::from_value(groups::review()["items"][3].clone()).unwrap();
    let repository: RepositorySnapshot =
        serde_json::from_value(json!({"id":1,"version":1,"repository":groups::repository()}))
            .unwrap();
    let valid = json!({"configuration_sha256":"a".repeat(64),"repositories":[{"repository_id":1,"repository_version":1,"repair_scope":"reviewed AC","selection":{"kind":"fixed","sha":"b".repeat(40)}}]});
    let auth: Authorization = serde_json::from_value(valid.clone()).unwrap();
    assert!(
        integration::validate(&auth, &item, &document, std::slice::from_ref(&repository)).is_ok()
    );
    for (path, value) in [
        ("/configuration_sha256", json!("old")),
        ("/repositories/0/repository_version", json!(2)),
        ("/repositories/0/repair_scope", json!(" ")),
        ("/repositories/0/selection/sha", json!("short")),
        ("/repositories/0/repository_id", json!(2)),
        ("/repositories", json!([])),
    ] {
        let mut value_auth = valid.clone();
        *value_auth.pointer_mut(path).unwrap() = value;
        let invalid: Authorization = serde_json::from_value(value_auth).unwrap();
        assert!(
            integration::validate(
                &invalid,
                &item,
                &document,
                std::slice::from_ref(&repository)
            )
            .is_err()
        );
    }
    let mut duplicate = auth.clone();
    duplicate
        .repositories
        .push(duplicate.repositories[0].clone());
    assert!(
        integration::validate(
            &duplicate,
            &item,
            &document,
            std::slice::from_ref(&repository)
        )
        .is_err()
    );
    let mut wrong = item.clone();
    wrong.child_id = "absent".into();
    assert!(
        integration::validate(&auth, &wrong, &document, std::slice::from_ref(&repository)).is_err()
    );
    wrong.child_id = "C1".into();
    assert!(
        integration::validate(&auth, &wrong, &document, std::slice::from_ref(&repository)).is_err()
    );
    let mut cross = document.clone();
    cross.children[0].repository_id = Some(2);
    assert!(
        integration::validate(&auth, &item, &cross, std::slice::from_ref(&repository)).is_err()
    );
    let mut revoked = repository.clone();
    revoked.repository.revoked = true;
    assert!(integration::validate(&auth, &item, &document, &[revoked]).is_err());
    let mut primary = document.clone();
    primary.children[3].repository_id = None;
    assert!(integration::validate(&auth, &item, &primary, &[repository]).is_err());
}

#[tokio::test]
async fn infrastructure_retry_preserves_originals_and_uses_a_new_validation_identity() {
    let f = fixture("infrastructure", false).await;
    complete(&f, "failed").await;
    let first: (String, Value) = sqlx::query_as("SELECT id,result FROM integration_validation")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    tick(&f).await;
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM integration_validation")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(n, 1);
    std::fs::write(f.root.join("service-ready"), "restored fixture service").unwrap();
    sqlx::query(
        "UPDATE integration_validation SET next_attempt_at=extract(epoch FROM now())::bigint",
    )
    .execute(&f.pool)
    .await
    .unwrap();
    complete(&f, "passed").await;
    let rows: Vec<(String, Value)> =
        sqlx::query_as("SELECT id,result FROM integration_validation ORDER BY created_at")
            .fetch_all(&f.pool)
            .await
            .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0], first);
    assert_ne!(rows[0].0, rows[1].0);
    assert_ne!(rows[0].1["binding_sha256"], rows[1].1["binding_sha256"]);
    assert_eq!(counts(&f).await, (0, 0, 1));
}
#[tokio::test]
async fn prepared_restart_and_cancel_racing_success_never_create_business_done() {
    let f = fixture("", false).await;
    tick(&f).await;
    run_store::begin_incarnation(&f.pool, "restart")
        .await
        .unwrap();
    assert!(
        codexsymphony_server::coordinator::recover(&f.pool, &f.root, "restart")
            .await
            .unwrap()
    );
    for _ in 0..100 {
        integration_worker::tick(
            &f.pool,
            &f.root,
            Path::new(env!("CARGO_BIN_EXE_codexsymphony-server")),
            &f.broker,
            "restart",
            &f.plan,
        )
        .await
        .unwrap();
        let quiet: bool = sqlx::query_scalar("SELECT quiescent FROM integration_validation")
            .fetch_one(&f.pool)
            .await
            .unwrap();
        if quiet {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let id: i64 = sqlx::query_scalar("SELECT requirement_id FROM integration_validation")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    codexsymphony_server::delivery_control::cancel(&f.pool, id)
        .await
        .unwrap();
    tick(&f).await;
    codexsymphony_server::delivery_control::settle(&f.pool)
        .await
        .unwrap();
    assert_eq!(counts(&f).await, (0, 0, 0));
    let owner: Option<i64> = sqlx::query_scalar("SELECT requirement_id FROM execution_control")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(owner, None);
}

#[tokio::test]
async fn storage_stop_receipt_can_be_reconciled_before_clearing_storage_latch() {
    let f = fixture("timeout", false).await;
    install_storage(&f).await;
    tick(&f).await;
    tick(&f).await;
    sqlx::query("UPDATE storage_guard SET blocked=true,error='fixture disk unavailable'")
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(
        !codexsymphony_server::storage::recover(&f.pool, &f.root)
            .await
            .unwrap()
    );
    let mut quiet = false;
    for _ in 0..100 {
        assert!(
            !codexsymphony_server::coordinator::recover(&f.pool, &f.root, "boot")
                .await
                .unwrap()
        );
        quiet = sqlx::query_scalar("SELECT quiescent FROM integration_validation")
            .fetch_one(&f.pool)
            .await
            .unwrap();
        if quiet {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(quiet);
    assert!(
        codexsymphony_server::storage::recover(&f.pool, &f.root)
            .await
            .unwrap()
    );
    assert_eq!(counts(&f).await, (0, 0, 0));
}
#[tokio::test]
async fn authorization_edit_and_repository_revocation_races_keep_parent_incomplete() {
    let f = fixture("", false).await;
    tick(&f).await;
    let id: i64 = sqlx::query_scalar("SELECT requirement_id FROM integration_validation")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    let rejected = codexsymphony_server::runtime_initial::plan(
        &f.pool,
        &f.broker,
        "boot",
        &["/bin/false".into()],
        &"a".repeat(40),
    )
    .await
    .unwrap();
    assert!(rejected.is_none());
    sqlx::query("UPDATE group_execution_item SET frozen=true WHERE requirement_id=$1")
        .bind(id)
        .execute(&f.pool)
        .await
        .unwrap();
    tick(&f).await;
    let state: String = sqlx::query_scalar("SELECT state FROM integration_validation")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(state, "prepared");
    sqlx::query("UPDATE group_execution_item SET frozen=false")
        .execute(&f.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE repository SET document=jsonb_set(document,'{revoked}','true')")
        .execute(&f.pool)
        .await
        .unwrap();
    tick(&f).await;
    assert_eq!(counts(&f).await, (0, 0, 0));
    let state: String = sqlx::query_scalar("SELECT state FROM integration_validation")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(state, "prepared");
}

#[tokio::test]
async fn completed_dependency_versions_are_checked_before_claim() {
    // Synthetic upstream completion records exercise the dependency protocol;
    // actual merge provenance is covered by the existing GH-86 adapter tests.
    let f = fixture("dependencies", false).await;
    tick(&f).await;
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM integration_validation")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
    let sha = validation_runner::candidate(&f.root.join("repo"))
        .unwrap()
        .sha;
    let rows:Vec<(i64,i64,Value)>=sqlx::query_as("SELECT requirement_id,authorization_id,input FROM group_execution_item WHERE input#>>'{child,kind}'='code_change'").fetch_all(&f.pool).await.unwrap();
    for (requirement, authorization, input) in rows {
        let fact = codexsymphony_server::group_dependency::Fact {
            requirement_id: requirement,
            authorization_id: authorization,
            child_revision: 1,
            repository_id: 1,
            github_repository_id: 123,
            pr_number: requirement,
            head_sha: sha.clone(),
            merged_sha: sha.clone(),
            acceptance_sha: sha.clone(),
            acceptance_plan: input["review"]["verification"].clone(),
            source: "synthetic-upstream-protocol".into(),
            evidence_sha256: "a".repeat(64),
            artifact: format!("fixture:{requirement}"),
        };
        sqlx::query(
            "INSERT INTO group_completion(requirement_id,authorization_id,fact) VALUES($1,$2,$3)",
        )
        .bind(requirement)
        .bind(authorization)
        .bind(json!(fact))
        .execute(&f.pool)
        .await
        .unwrap();
        sqlx::query("UPDATE requirement SET state='Done' WHERE id=$1")
            .bind(requirement)
            .execute(&f.pool)
            .await
            .unwrap();
    }
    complete(&f, "passed").await;
    let binding: Value = sqlx::query_scalar("SELECT binding FROM integration_validation")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(binding["versions"][0]["candidate"]["sha"], sha);
    assert_eq!(
        binding["versions"][0]["artifacts"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(counts(&f).await, (0, 0, 1));
}

#[tokio::test]
async fn changed_parent_revision_cannot_borrow_old_integration_acceptance() {
    let f = fixture("", false).await;
    complete(&f, "passed").await;
    let before: Value = sqlx::query_scalar("SELECT fact FROM group_completion")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE imported_draft SET version=version+1 WHERE id=$1")
        .bind(&f.draft)
        .execute(&f.pool)
        .await
        .unwrap();
    let view = groups::request(
        &groups::app(&f.pool),
        "GET",
        &format!("/api/drafts/{}/review", f.draft),
        Value::Null,
        200,
    )
    .await;
    assert_eq!(view["business_complete"], false);
    assert_eq!(
        view["execution"]["parent_state"],
        "waiting_business_acceptance"
    );
    let after: Value = sqlx::query_scalar("SELECT fact FROM group_completion")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(before, after);
}

#[tokio::test]
async fn zero_model_validation_may_finish_at_exact_group_budget_but_not_above_it() {
    let f = fixture("", false).await;
    sqlx::query("UPDATE group_budget SET used=limits || jsonb_build_object('tokens',(limits->>'tokens')::bigint+1) WHERE item_id=''").execute(&f.pool).await.unwrap();
    tick(&f).await;
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM integration_validation")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
    sqlx::query("UPDATE group_budget SET used=limits WHERE item_id=''")
        .execute(&f.pool)
        .await
        .unwrap();
    complete(&f, "passed").await;
    assert_eq!(counts(&f).await, (0, 0, 1));
}

#[tokio::test]
async fn reviewed_two_validation_chain_keeps_parent_waiting_and_reuses_no_child_fact() {
    let f = fixture("chain", true).await;
    let app = groups::app(&f.pool);
    let view = groups::request(
        &app,
        "GET",
        &format!("/api/drafts/{}/review", f.draft),
        Value::Null,
        200,
    )
    .await;
    let mut document = view["document"].clone();
    document["children"][0]["goal"] = json!("Reviewed verification scope change");
    let mut review = view["review"].clone();
    review["parent_revision"] = json!(2);
    for item in review["items"].as_array_mut().unwrap() {
        item["revision"] = json!(2);
    }
    review["coverage"][0]["child_revision"] = json!(2);
    let path = format!("/api/drafts/{}/queue-edit", f.draft);
    groups::request(&app,"POST",&path,json!({"request_id":"validation-change","version":1,"change":{"kind":"propose","document":document,"review":review}}),200).await;
    groups::request(&app,"POST",&path,json!({"request_id":"validation-approve","version":2,"change":{"kind":"approve","edit_version":2}}),200).await;
    complete(&f, "passed").await;
    assert_eq!(counts(&f).await, (0, 0, 0));
    let old: Value = sqlx::query_scalar("SELECT fact FROM group_completion")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    tick(&f).await;
    complete(&f, "passed").await;
    assert_eq!(counts(&f).await, (0, 0, 1));
    let preserved: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM group_completion WHERE fact=$1)")
            .bind(old)
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert!(preserved);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM group_completion")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(count, 2);
}

#[tokio::test]
async fn clean_commit_drift_before_execution_cannot_be_relabelled() {
    let f = fixture("", false).await;
    tick(&f).await;
    let job: Value = sqlx::query_scalar("SELECT job FROM integration_validation")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    let checkout = Path::new(job["checkouts"][0].as_str().unwrap());
    std::fs::write(checkout.join("source"), "different committed source").unwrap();
    git(
        checkout,
        &[
            "-c",
            "user.name=test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "-am",
            "drift",
        ],
    );
    complete(&f, "failed").await;
    assert_eq!(counts(&f).await, (0, 0, 0));
}

async fn install_storage(f: &Fixture) {
    use codexsymphony_server::{
        storage_files::Directory,
        storage_lifecycle::{CATEGORIES, Limit, Policy},
        storage_store::{Deployment, Root},
    };
    let cold = f.root.with_extension("cold");
    std::fs::create_dir(&cold).unwrap();
    let root = |path: PathBuf| Root {
        identity: Directory::open(&path).unwrap().identity().unwrap(),
        path,
    };
    let config = Deployment {
        policy: Policy {
            version: "integration-fixture-v1".into(),
            reason: "real validation storage accounting".into(),
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
        execution: root(f.root.clone()),
        cold: root(cold),
        database_filesystem: root(f.root.clone()),
        database_extras: vec![],
    };
    codexsymphony_server::storage_store::install(&f.pool, &config)
        .await
        .unwrap();
}

#[tokio::test]
async fn native_integration_failure_persists_once_without_changing_kind_or_releasing_owner() {
    let f = fixture("linked", true).await;
    complete(&f, "failed").await;
    let (id, original): (String, Value) = sqlx::query_as("SELECT id,evidence FROM linked_failure")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    tick(&f).await;
    tick(&f).await;
    let (count, evidence): (i64, Value) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM linked_failure),evidence FROM linked_failure WHERE id=$1",
    )
    .bind(id)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(count, 1);
    assert_eq!(evidence, original);
    assert_eq!(counts(&f).await, (0, 0, 0));
    let (kind,owner):(String,Option<i64>)=sqlx::query_as("SELECT i.input#>>'{child,kind}',c.requirement_id FROM group_execution_item i JOIN execution_control c ON c.requirement_id=i.requirement_id").fetch_one(&f.pool).await.unwrap();
    assert_eq!(kind, "validation_only");
    assert!(owner.is_some());
}

// Adapter-boundary test: the merged candidate is supplied locally here. Real
// GitHub merge/PR evidence is separately required by the GH-88 B05/B06 runbook.
#[tokio::test]
async fn serial_repair_versions_rerun_all_checks_and_preserve_failed_combinations() {
    let f = fixture("linked", true).await;
    install_storage(&f).await;
    complete(&f, "failed").await;
    let original: Value =
        sqlx::query_scalar("SELECT result FROM integration_validation ORDER BY created_at LIMIT 1")
            .fetch_one(&f.pool)
            .await
            .unwrap();
    let mut expected: Value = sqlx::query_scalar("SELECT binding FROM integration_validation")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    for repository in [1_i64, 2] {
        let (failed, job): (String, Value) = sqlx::query_as(
            "SELECT id,job FROM integration_validation ORDER BY created_at DESC,id DESC LIMIT 1",
        )
        .fetch_one(&f.pool)
        .await
        .unwrap();
        let index = job["binding"]["versions"]
            .as_array()
            .unwrap()
            .iter()
            .position(|v| v["repository_id"] == repository)
            .unwrap();
        let checkout = Path::new(job["checkouts"][index].as_str().unwrap());
        std::fs::write(
            checkout.join("source"),
            format!("repaired repository {repository}"),
        )
        .unwrap();
        git(
            checkout,
            &[
                "-c",
                "user.name=repair fixture",
                "-c",
                "user.email=fixture@example.com",
                "commit",
                "-am",
                "local adapter candidate",
            ],
        );
        let candidate = validation_runner::candidate(checkout).unwrap();
        expected["versions"][index]["candidate"] = json!(candidate);
        sqlx::query("UPDATE linked_failure SET state='merged',repository_id=$2,final_version=$3 WHERE integration_id=$1").bind(&failed).bind(repository).bind(json!({"candidate":candidate})).execute(&f.pool).await.unwrap();
        complete(&f, "failed").await;
        let (latest,binding):(String,Value)=sqlx::query_as("SELECT id,binding FROM integration_validation ORDER BY created_at DESC,id DESC LIMIT 1").fetch_one(&f.pool).await.unwrap();
        assert_ne!(latest, failed);
        for i in 0..2 {
            assert_eq!(
                binding["versions"][i]["candidate"],
                expected["versions"][i]["candidate"]
            );
        }
        let original_saved: Value = sqlx::query_scalar(
            "SELECT result FROM integration_validation ORDER BY created_at LIMIT 1",
        )
        .fetch_one(&f.pool)
        .await
        .unwrap();
        assert_eq!(original_saved, original);
        assert_eq!(counts(&f).await, (0, 0, 0));
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM integration_validation")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(count, 3);
}

#[tokio::test]
async fn integration_supervisor_growth_uses_its_preallocated_hot_budget() {
    let f = fixture("linked", true).await;
    install_storage(&f).await;
    let mut tx = f.pool.begin().await.unwrap();
    sqlx::query("UPDATE storage_guard SET blocked=true")
        .execute(&mut *tx)
        .await
        .unwrap();
    assert!(
        !codexsymphony_server::storage_service::reserve_integration(&mut tx, "blocked-validation")
            .await
            .unwrap()
    );
    tx.rollback().await.unwrap();
    complete(&f, "failed").await;
    let id: String = sqlx::query_scalar("SELECT id FROM integration_validation")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    let allocated: i64 =
        sqlx::query_scalar("SELECT allocated FROM storage_allocation WHERE request_id=$1")
            .bind(format!("{id}-hot"))
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(allocated, 1 << 20);
    let directory = f.root.join(&id);
    for size in [4096, 131072] {
        std::fs::write(directory.join("growth.log"), vec![b'x'; size]).unwrap();
        codexsymphony_server::storage_cleanup::scan(&f.pool, 100)
            .await
            .unwrap();
        let blocked: bool = sqlx::query_scalar("SELECT blocked FROM storage_guard")
            .fetch_one(&f.pool)
            .await
            .unwrap();
        assert!(!blocked);
    }
    let allocations: Vec<String> = sqlx::query_scalar(
        "SELECT category FROM storage_allocation WHERE run_id=$1 ORDER BY category",
    )
    .bind(format!("{id}-repo-1"))
    .fetch_all(&f.pool)
    .await
    .unwrap();
    assert_eq!(allocations, vec!["workspace"]);
    assert_eq!(counts(&f).await, (0, 0, 0));
}

#[tokio::test]
async fn repository_scope_checks_every_integration_input_before_start() {
    let f = fixture("", true).await;
    tick(&f).await;
    sqlx::query("UPDATE plugin_scope SET kind='repositories',repository_ids='{1}' WHERE plugin_id='validation:native'").execute(&f.pool).await.unwrap();
    let error = integration_worker::tick(
        &f.pool,
        &f.root,
        Path::new(env!("CARGO_BIN_EXE_codexsymphony-server")),
        &f.broker,
        "boot",
        &f.plan,
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("scope unavailable"));
    let state: String = sqlx::query_scalar("SELECT state FROM integration_validation")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(state, "prepared");
    assert_eq!(counts(&f).await, (0, 0, 0));
    // Restoring explicit authorization resumes the same saved job, with both
    // repository bindings retained, without replacing its input combination.
    sqlx::query(
        "UPDATE plugin_scope SET repository_ids='{1,2}' WHERE plugin_id='validation:native'",
    )
    .execute(&f.pool)
    .await
    .unwrap();
    complete(&f, "passed").await;
    let bindings:Vec<i64>=sqlx::query_scalar("SELECT repository_id FROM plugin_scope_invocation WHERE plugin_id='validation:native' ORDER BY repository_id").fetch_all(&f.pool).await.unwrap();
    assert_eq!(bindings, vec![1, 2]);
}
