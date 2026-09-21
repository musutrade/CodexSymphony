#[path = "support/auth.rs"]
mod auth_client;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::Request,
};
use codexsymphony_server::{group_review, process};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::path::PathBuf;
use tower::ServiceExt;

fn sample() -> Value {
    let children: Vec<Value> = (1..=4).map(|i| json!({"id":format!("C{i}"),"parent_id":"P1","kind":if i==4 {"validation_only"} else {"code_change"},"order":i,"depends_on":if i==1 {vec![]} else {vec![format!("C{}",i-1)]},"repository_id":1,"goal":format!("Step {i}"),"acceptance_criteria":[{"id":"AC1","description":"observable result"}],"validation_plan":"run the selected automated test"})).collect();
    json!({"schema":"codexsymphony-draft/v1","parent":{"id":"P1","goal":"Complete chain","scope":"test feature only","acceptance_criteria":[{"id":"P-AC1","description":"integrated flow works"}]},"children":children})
}
fn review() -> Value {
    let items:Vec<Value> = (1..=4).map(|i|json!({"child_id":format!("C{i}"),"revision":1,"repository_version":1,"budget":{"tokens":100,"turns":2,"model_seconds":60},"repair_scope":"only this item AC in this repository; no new permissions","merged_baseline_review":"independent regression check on main after predecessors merge; safe without later changes","verification":[{"ac_id":"AC1","step":{"id":"test","check":"cargo_test","selector":"group_review","expected_result":"exit 0","timeout_seconds":30}}]})).collect();
    json!({"parent_revision":1,"full_chain_acs":["P-AC1"],"coverage":[{"parent_ac":"P-AC1","child_id":"C4","child_revision":1,"child_ac":"AC1","step_id":"test"}],"items":items,"group_budget":null,"semantic_review":"Reviewed the integration test against the complete parent flow"})
}
fn repository() -> Value {
    json!({"model":null,"project":"synthetic","remote":"test/group","github_repository_id":123,"base_branch":"main","policy":{"allowed_checks":["cargo_test"],"max_timeout_seconds":60,"token_limit":1000,"turn_limit":20,"model_work_seconds":600,"gate_recovery_policy":"one_code_repair"},"revoked":false,"reason":"synthetic group test"})
}
fn source(document: Value) -> Value {
    json!({"format":"json","label":"synthetic source; never authorization","text":document.to_string()})
}
fn body(document: Value, version: i64) -> Value {
    json!({"version":version,"source":source(document)})
}
fn app(pool: &PgPool) -> Router {
    auth_client::router(
        pool.clone(),
        codexsymphony_server::security::RequestPolicy::new(
            "127.0.0.1:3081".parse().unwrap(),
            "https://localhost:4200".into(),
        )
        .unwrap(),
    )
}
async fn request(app: &Router, method: &str, path: &str, body: Value, expected: u16) -> Value {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("host", "127.0.0.1:3081")
                .header("origin", "https://localhost:4200")
                .header("x-codexsymphony-csrf", "1")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status().as_u16();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(status, expected, "{method} {path}: {value}");
    value
}
#[test]
fn domain_checks_coverage_versions_policy_and_budget() {
    let document = serde_json::from_value(sample()).unwrap();
    let repositories = vec![group_review::RepositorySnapshot {
        id: 1,
        version: 1,
        repository: serde_json::from_value(repository()).unwrap(),
    }];
    let valid: group_review::Review = serde_json::from_value(review()).unwrap();
    assert_eq!(
        group_review::validate(&document, 1, &valid, &repositories)
            .unwrap()
            .tokens,
        400
    );
    let changes = [
        ("/parent_revision", json!(2)),
        ("/semantic_review", json!("")),
        ("/coverage", json!([])),
        ("/coverage/0/parent_ac", json!("missing")),
        ("/coverage/0/child_id", json!("missing")),
        ("/coverage/0/child_revision", json!(2)),
        ("/coverage/0/child_ac", json!("missing")),
        ("/coverage/0/step_id", json!("missing")),
        ("/full_chain_acs/0", json!("missing")),
        ("/items/0/revision", json!(2)),
        ("/items/0/repository_version", json!(2)),
        ("/items/0/verification", json!([])),
        ("/items/0/verification/0/ac_id", json!("missing")),
        ("/items/0/verification/0/step/check", json!("shell")),
        ("/items/0/verification/0/step/timeout_seconds", json!(61)),
        ("/items/0/budget/tokens", json!(1001)),
        ("/items/0/budget/turns", json!(0)),
        ("/items/0/repair_scope", json!("")),
        ("/items/0/merged_baseline_review", json!("")),
        (
            "/group_budget",
            json!({"tokens":401,"turns":8,"model_seconds":240}),
        ),
        ("/items/1/child_id", json!("C1")),
        ("/items/0/child_id", json!("unknown")),
    ];
    for (path, value) in changes {
        let mut input = review();
        *input.pointer_mut(path).unwrap() = value;
        assert!(
            group_review::validate(
                &document,
                1,
                &serde_json::from_value(input).unwrap(),
                &repositories
            )
            .is_err(),
            "{path}"
        );
    }
    for (path, value) in [
        ("/children/1/order", json!(0)),
        ("/children/0/depends_on", json!(["C4"])),
        ("/children/0/depends_on", json!(["missing"])),
        ("/children/3/depends_on", json!([])),
        ("/children/3/kind", json!("code_change")),
        ("/children/0/repository_id", json!(2)),
        ("/parent/scope", json!("")),
        ("/parent/acceptance_criteria/0/description", json!("")),
        ("/children/0/validation_plan", json!("")),
    ] {
        let mut input = sample();
        *input.pointer_mut(path).unwrap() = value;
        assert!(
            group_review::validate(
                &serde_json::from_value(input).unwrap(),
                1,
                &valid,
                &repositories
            )
            .is_err(),
            "{path}"
        );
    }
    let mut revoked = repositories;
    revoked[0].repository.revoked = true;
    assert!(group_review::validate(&document, 1, &valid, &revoked).is_err());
}
async fn fixture() -> (PgPool, String, PathBuf) {
    let database = std::env::var("TEST_DATABASE_URL").unwrap();
    let admin = PgPoolOptions::new().connect(&database).await.unwrap();
    let schema = format!(
        "groups_{}",
        process::new_identity().unwrap().replace('-', "")
    );
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
    let sep = if database.contains('?') { '&' } else { '?' };
    let url = format!("{database}{sep}options=-csearch_path%3D{schema}");
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&url)
        .await
        .unwrap();
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
    sqlx::query("INSERT INTO repository(id,version,document) VALUES(1,1,$1)")
        .bind(repository())
        .execute(&pool)
        .await
        .unwrap();
    (pool, url, std::env::temp_dir().join(schema))
}
async fn counts(pool: &PgPool) -> Value {
    sqlx::query_scalar("SELECT jsonb_build_object('authorizations',(SELECT count(*) FROM group_authorization),'queue',(SELECT count(*) FROM group_queue),'budgets',(SELECT jsonb_agg(to_jsonb(b) ORDER BY item_id) FROM group_budget b),'owner',(SELECT to_jsonb(c) FROM execution_control c),'runs',(SELECT count(*) FROM agent_run),'ready',(SELECT count(*) FROM requirement WHERE state='Ready'))").fetch_one(pool).await.unwrap()
}
async fn save_review(
    app: &Router,
    path: &str,
    version: i64,
    draft_revision: i64,
    review: Value,
) -> Value {
    request(
        app,
        "PUT",
        path,
        json!({"version":version,"draft_revision":draft_revision,"review":review}),
        200,
    )
    .await
}
#[tokio::test]
async fn postgres_atomic_review_replay_races_and_persistent_accounting() {
    let (pool, url, _) = fixture().await;
    let router = app(&pool);
    let draft = request(&router, "POST", "/api/drafts", body(sample(), 0), 200).await;
    let id = draft["id"].as_str().unwrap();
    let path = format!("/api/drafts/{id}/review");
    let authorize = format!("/api/drafts/{id}/authorize");
    request(&router, "GET", "/api/drafts/missing/review", json!({}), 404).await;
    request(&router, "PUT", &path, json!({"unknown":true}), 422).await;
    let empty = request(&router, "GET", &path, json!({}), 200).await;
    assert_eq!(empty["version"], 0);
    assert_eq!(empty["scheduler_available"], true);
    let mut confirmation = json!({"version":1,"draft_revision":1,"request_id":"confirm"});
    request(&router, "POST", &authorize, confirmation.clone(), 409).await;
    let before = counts(&pool).await;
    let mut invalid = review();
    invalid["coverage"] = json!([]);
    save_review(&router, &path, 0, 1, invalid).await;
    request(&router, "POST", &authorize, confirmation.clone(), 422).await;
    assert_eq!(counts(&pool).await, before);
    save_review(&router, &path, 1, 1, review()).await;
    confirmation["version"] = json!(2);
    // A database failure AFTER authorization/budget writes must roll back everything.
    sqlx::raw_sql("CREATE FUNCTION reject_group_queue() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected queue fault'; END $$; CREATE TRIGGER queue_fault BEFORE INSERT ON group_queue FOR EACH ROW EXECUTE FUNCTION reject_group_queue();").execute(&pool).await.unwrap();
    request(&router, "POST", &authorize, confirmation.clone(), 503).await;
    assert_eq!(counts(&pool).await, before);
    sqlx::query("DROP TRIGGER queue_fault ON group_queue")
        .execute(&pool)
        .await
        .unwrap();
    let (first, duplicate) = tokio::join!(
        request(&router, "POST", &authorize, confirmation.clone(), 200),
        request(&router, "POST", &authorize, confirmation.clone(), 200)
    );
    assert_eq!(first, duplicate);
    let after = counts(&pool).await;
    assert_eq!(after["authorizations"], 1);
    assert_eq!(after["queue"], 1);
    assert_eq!(after["runs"], 0);
    assert_eq!(after["ready"], 0);
    assert_eq!(after["owner"], before["owner"]);
    let mut competing = confirmation.clone();
    competing["request_id"] = json!("other");
    request(&router, "POST", &authorize, competing, 409).await;
    let mut mismatched = confirmation.clone();
    mismatched["version"] = json!(3);
    request(&router, "POST", &authorize, mismatched, 409).await;
    request(
        &router,
        "POST",
        &format!("/api/requirements/{id}/ready"),
        json!({}),
        409,
    )
    .await;
    request(
        &router,
        "POST",
        &format!("/api/multi/requirements/{id}/ready"),
        json!({}),
        409,
    )
    .await;
    // Reconnect/recreate router without in-memory review state.
    let reopened = PgPoolOptions::new().connect(&url).await.unwrap();
    let reread = request(&app(&reopened), "GET", &path, json!({}), 200).await;
    assert_eq!(reread["authorizations"][0]["snapshot"]["review"], review());
    assert_eq!(reread["queue"]["state"], "waiting_scheduler");
    // Keep stable counters across both new review AND a new parent/child revision.
    sqlx::query("UPDATE group_budget SET used='{\"tokens\":90,\"turns\":1,\"model_seconds\":20}',reserved='{\"tokens\":20,\"turns\":1,\"model_seconds\":20}' WHERE draft_id=$1").bind(id).execute(&pool).await.unwrap();
    request(
        &router,
        "PUT",
        &format!("/api/drafts/{id}"),
        body(sample(), 1),
        200,
    )
    .await;
    assert_eq!(
        request(&router, "GET", &path, json!({}), 200).await["queue"]["state"],
        "needs_review"
    );
    request(
        &router,
        "POST",
        &authorize,
        json!({"version":2,"draft_revision":1,"request_id":"stale"}),
        409,
    )
    .await;
    let mut next = review();
    next["parent_revision"] = json!(2);
    next["coverage"][0]["child_revision"] = json!(2);
    for item in next["items"].as_array_mut().unwrap() {
        item["revision"] = json!(2);
    }
    save_review(&router, &path, 2, 2, next.clone()).await;
    let exhausted = counts(&pool).await;
    request(
        &router,
        "POST",
        &authorize,
        json!({"version":3,"draft_revision":2,"request_id":"insufficient"}),
        422,
    )
    .await;
    assert_eq!(counts(&pool).await, exhausted);
    for item in next["items"].as_array_mut().unwrap() {
        item["budget"]["tokens"] = json!(120);
    }
    save_review(&router, &path, 3, 2, next).await;
    request(
        &router,
        "POST",
        &authorize,
        json!({"version":4,"draft_revision":2,"request_id":"reauthorize"}),
        200,
    )
    .await;
    let final_view = request(&router, "GET", &path, json!({}), 200).await;
    assert_eq!(final_view["authorizations"].as_array().unwrap().len(), 2);
    assert_eq!(counts(&pool).await["queue"], 1);
    for budget in final_view["budgets"].as_array().unwrap() {
        assert_eq!(budget["used"]["tokens"], 90);
        assert_eq!(budget["reserved"]["tokens"], 20);
    }
    reopened.close().await;
    pool.close().await;
}

#[tokio::test]
async fn postgres_rejects_invalid_coverage_references_plans_policies_and_order() {
    let (pool, _, _) = fixture().await;
    let router = app(&pool);
    let draft = request(&router, "POST", "/api/drafts", body(sample(), 0), 200).await;
    let id = draft["id"].as_str().unwrap();
    let path = format!("/api/drafts/{id}/review");
    let authorize = format!("/api/drafts/{id}/authorize");
    let before = counts(&pool).await;
    let failures = [
        ("/coverage", json!([])),
        ("/coverage/0/child_id", json!("missing")),
        ("/coverage/0/child_revision", json!(2)),
        ("/items/0/revision", json!(2)),
        ("/items/0/verification", json!([])),
        ("/items/0/repository_version", json!(2)),
        ("/items/0/budget/tokens", json!(1001)),
    ];
    let mut version = 0;
    for (pointer, value) in failures {
        let mut bad = review();
        *bad.pointer_mut(pointer).unwrap() = value;
        save_review(&router, &path, version, 1, bad).await;
        version += 1;
        request(
            &router,
            "POST",
            &authorize,
            json!({"version":version,"draft_revision":1,"request_id":format!("bad-{version}")}),
            422,
        )
        .await;
        assert_eq!(counts(&pool).await, before);
    }
    save_review(&router, &path, version, 1, review()).await;
    version += 1;
    sqlx::query("UPDATE repository SET document=jsonb_set(document,'{revoked}','true') WHERE id=1")
        .execute(&pool)
        .await
        .unwrap();
    request(
        &router,
        "POST",
        &authorize,
        json!({"version":version,"draft_revision":1,"request_id":"revoked"}),
        422,
    )
    .await;
    sqlx::query("DELETE FROM repository WHERE id=1")
        .execute(&pool)
        .await
        .unwrap();
    request(
        &router,
        "POST",
        &authorize,
        json!({"version":version,"draft_revision":1,"request_id":"missing-repo"}),
        422,
    )
    .await;
    sqlx::query("INSERT INTO repository(id,version,document) VALUES(1,1,$1)")
        .bind(repository())
        .execute(&pool)
        .await
        .unwrap();
    // Drafts can express an unresolved order, but confirmation cannot authorize it.
    let mut bad = sample();
    bad["children"][0]["order"] = json!(8);
    request(
        &router,
        "PUT",
        &format!("/api/drafts/{id}"),
        body(bad, 1),
        200,
    )
    .await;
    let mut next = review();
    next["parent_revision"] = json!(2);
    next["coverage"][0]["child_revision"] = json!(2);
    for item in next["items"].as_array_mut().unwrap() {
        item["revision"] = json!(2);
    }
    save_review(&router, &path, version, 2, next).await;
    version += 1;
    request(
        &router,
        "POST",
        &authorize,
        json!({"version":version,"draft_revision":2,"request_id":"bad-order"}),
        422,
    )
    .await;
    assert_eq!(counts(&pool).await, before);
    for dependencies in [json!(["missing"]), json!(["C4"])] {
        let mut document = sample();
        document["children"][0]["depends_on"] = dependencies;
        request(
            &router,
            "PUT",
            &format!("/api/drafts/{id}"),
            body(document, 2),
            422,
        )
        .await;
        assert_eq!(counts(&pool).await, before);
    }
    pool.close().await;
}
#[tokio::test]
async fn competing_confirmations_create_one_authorization() {
    let (pool, _, _) = fixture().await;
    let router = app(&pool);
    let draft = request(&router, "POST", "/api/drafts", body(sample(), 0), 200).await;
    let id = draft["id"].as_str().unwrap();
    save_review(&router, &format!("/api/drafts/{id}/review"), 0, 1, review()).await;
    async fn confirm(router: Router, id: &str, key: &str) -> u16 {
        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/drafts/{id}/authorize"))
                    .header("host", "127.0.0.1:3081")
                    .header("origin", "https://localhost:4200")
                    .header("x-codexsymphony-csrf", "1")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({"version":1,"draft_revision":1,"request_id":key}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        response.status().as_u16()
    }
    let (a, b) = tokio::join!(
        confirm(router.clone(), id, "one"),
        confirm(router, id, "two")
    );
    let mut results = [a, b];
    results.sort();
    assert_eq!(results, [200, 409]);
    assert_eq!(counts(&pool).await["authorizations"], 1);
    assert_eq!(counts(&pool).await["queue"], 1);
}
