use codexsymphony_server::{group_edit, group_queue_store};
use serde_json::{Value, json};
use sqlx::PgPool;
#[path = "support/groups.rs"]
mod groups;
use groups::*;
async fn authorized(pool: &PgPool) -> String {
    let router = app(pool);
    let value = request(&router, "POST", "/api/drafts", body(sample(), 0), 200).await;
    let id = value["id"].as_str().unwrap();
    request(
        &router,
        "PUT",
        &format!("/api/drafts/{id}/review"),
        json!({"version":0,"draft_revision":1,"review":review()}),
        200,
    )
    .await;
    request(
        &router,
        "POST",
        &format!("/api/drafts/{id}/authorize"),
        json!({"request_id":"initial","version":1,"draft_revision":1}),
        200,
    )
    .await;
    group_queue_store::materialize(pool).await.unwrap();
    id.into()
}
fn changed() -> (Value, Value) {
    let mut document = sample();
    document["children"][1]["goal"] = json!("Changed second scope");
    let mut review = review();
    review["parent_revision"] = json!(2);
    for item in review["items"].as_array_mut().unwrap() {
        item["revision"] = json!(2);
    }
    review["coverage"][0]["child_revision"] = json!(2);
    (document, review)
}
async fn edit(
    pool: &PgPool,
    id: &str,
    key: &str,
    version: i64,
    change: Value,
    status: u16,
) -> Value {
    request(
        &app(pool),
        "POST",
        &format!("/api/drafts/{id}/queue-edit"),
        json!({"request_id":key,"version":version,"change":change}),
        status,
    )
    .await
}
async fn view(pool: &PgPool, id: &str) -> Value {
    request(
        &app(pool),
        "GET",
        &format!("/api/drafts/{id}/review"),
        Value::Null,
        200,
    )
    .await
}
#[test]
fn change_analysis_and_dependency_order() {
    let (new, review2) = changed();
    let before = serde_json::from_value(sample()).unwrap();
    let old = serde_json::from_value(review()).unwrap();
    let after = serde_json::from_value(new).unwrap();
    let review2 = serde_json::from_value(review2).unwrap();
    assert_eq!(
        group_edit::affected(&before, &old, &after, &review2)
            .into_iter()
            .collect::<Vec<_>>(),
        vec!["C2", "C3", "C4"]
    );
    for order in [
        vec!["C1"],
        vec!["C1", "C1", "C3", "C4"],
        vec!["unknown", "C2", "C3", "C4"],
        vec!["C2", "C1", "C3", "C4"],
    ] {
        assert!(
            group_edit::reorder(
                &before,
                &order.into_iter().map(String::from).collect::<Vec<_>>()
            )
            .is_err()
        );
    }
    assert!(group_edit::affected(&before, &old, &before, &old).is_empty());
}
#[tokio::test]
async fn delta_review_is_atomic_idempotent_and_preserves_identity_and_balances() {
    let (pool, url, _) = fixture().await;
    let id = authorized(&pool).await;
    let before:Vec<(String,Option<i64>,i64,Value)>=sqlx::query_as("SELECT child_id,requirement_id,authorization_id,input FROM group_execution_item ORDER BY child_id").fetch_all(&pool).await.unwrap();
    sqlx::query("UPDATE group_budget SET used='{\"tokens\":10,\"turns\":1,\"model_seconds\":5}',reserved='{\"tokens\":5,\"turns\":0,\"model_seconds\":1}' WHERE item_id IN ('','C2')").execute(&pool).await.unwrap();
    sqlx::query("UPDATE requirement SET paused=true WHERE id=$1")
        .bind(before[1].1)
        .execute(&pool)
        .await
        .unwrap();
    let (document, review) = changed();
    let change = json!({"kind":"propose","document":document,"review":review});
    let proposed = edit(&pool, &id, "proposal", 1, change.clone(), 200).await;
    assert_eq!(proposed["affected"], json!(["C2", "C3", "C4"]));
    assert_eq!(
        edit(&pool, &id, "proposal", 1, change.clone(), 200).await,
        proposed
    );
    edit(&pool, &id, "stale", 1, change, 409).await;
    edit(
        &pool,
        &id,
        "proposal",
        2,
        json!({"kind":"approve","edit_version":2}),
        409,
    )
    .await;
    let frozen: Vec<(String, bool)> =
        sqlx::query_as("SELECT child_id,frozen FROM group_execution_item ORDER BY child_id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        frozen,
        vec![
            ("C1".into(), false),
            ("C2".into(), true),
            ("C3".into(), true),
            ("C4".into(), true)
        ]
    );
    let restarted = sqlx::postgres::PgPoolOptions::new()
        .connect(&url)
        .await
        .unwrap();
    assert_eq!(view(&restarted, &id).await["pending_edit"]["version"], 2);
    edit(
        &pool,
        &id,
        "wrong-edit",
        2,
        json!({"kind":"approve","edit_version":1}),
        409,
    )
    .await;
    sqlx::raw_sql("CREATE FUNCTION fail_edit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected authorization failure'; END $$; CREATE TRIGGER fail_edit BEFORE INSERT ON group_authorization FOR EACH ROW EXECUTE FUNCTION fail_edit();").execute(&pool).await.unwrap();
    edit(
        &pool,
        &id,
        "approve",
        2,
        json!({"kind":"approve","edit_version":2}),
        503,
    )
    .await;
    assert_eq!(view(&pool, &id).await["draft_revision"], 1);
    sqlx::query("DROP TRIGGER fail_edit ON group_authorization")
        .execute(&pool)
        .await
        .unwrap();
    let approved = edit(
        &pool,
        &id,
        "approve",
        2,
        json!({"kind":"approve","edit_version":2}),
        200,
    )
    .await;
    assert_eq!(
        edit(
            &pool,
            &id,
            "approve",
            2,
            json!({"kind":"approve","edit_version":2}),
            200
        )
        .await,
        approved
    );
    let after:Vec<(String,Option<i64>,i64,Value)>=sqlx::query_as("SELECT child_id,requirement_id,authorization_id,input FROM group_execution_item ORDER BY child_id").fetch_all(&pool).await.unwrap();
    assert_eq!(before[0], after[0]);
    assert_eq!(before[1].1, after[1].1);
    assert_ne!(before[1].2, after[1].2);
    let result = view(&restarted, &id).await;
    assert_eq!(result["draft_revision"], 2);
    assert!(result["pending_edit"].is_null());
    assert_eq!(result["execution"]["items"][1]["waiting_reason"], "paused");
    assert_eq!(result["budgets"][0]["used"]["tokens"], 10);
    assert_eq!(result["budgets"][0]["reserved"]["tokens"], 5);
    assert_eq!(result["queue"]["version"], 3);
}
#[tokio::test]
async fn invalid_coverage_budget_and_claimed_inputs_never_partially_authorize() {
    let (pool, _, _) = fixture().await;
    let id = authorized(&pool).await;
    let (document, mut review) = changed();
    review["coverage"] = json!([]);
    edit(
        &pool,
        &id,
        "uncovered",
        1,
        json!({"kind":"propose","document":document,"review":review}),
        200,
    )
    .await;
    edit(
        &pool,
        &id,
        "reject",
        2,
        json!({"kind":"approve","edit_version":2}),
        422,
    )
    .await;
    let state = view(&pool, &id).await;
    assert_eq!(state["draft_revision"], 1);
    assert_eq!(state["authorizations"].as_array().unwrap().len(), 1);
    edit(
        &pool,
        &id,
        "reorder-pending",
        2,
        json!({"kind":"reorder","order":["C1","C2","C3","C4"]}),
        409,
    )
    .await;
    let (document, review) = changed();
    sqlx::query("UPDATE requirement SET state='Running' WHERE id=(SELECT requirement_id FROM group_execution_item WHERE child_id='C2')").execute(&pool).await.unwrap();
    edit(
        &pool,
        &id,
        "claimed",
        2,
        json!({"kind":"propose","document":document,"review":review}),
        409,
    )
    .await;
    edit(
        &pool,
        &id,
        "claimed-approve",
        2,
        json!({"kind":"approve","edit_version":2}),
        409,
    )
    .await;
    assert_eq!(view(&pool, &id).await["draft_revision"], 1);
}

#[tokio::test]
async fn reorder_restarts_without_reauthorizing_and_content_edits_keep_completed_facts() {
    let (pool, url, _) = fixture().await;
    let id = authorized(&pool).await;
    let (mut document, mut review) = changed();
    document["children"][2]["depends_on"] = json!(["C1"]);
    document["children"][3]["depends_on"] = json!(["C2", "C3"]);
    document["children"][1]["order"] = json!(3);
    document["children"][2]["order"] = json!(2);
    edit(
        &pool,
        &id,
        "independent",
        1,
        json!({"kind":"propose","document":document,"review":review}),
        200,
    )
    .await;
    edit(
        &pool,
        &id,
        "independent-approve",
        2,
        json!({"kind":"approve","edit_version":2}),
        200,
    )
    .await;
    let before: Vec<(String, i64, Value)> = sqlx::query_as(
        "SELECT child_id,authorization_id,input FROM group_execution_item ORDER BY child_id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    let order = json!({"kind":"reorder","order":["C1","C2","C3","C4"]});
    let result = edit(&pool, &id, "reorder", 3, order.clone(), 200).await;
    assert_eq!(
        edit(&pool, &id, "reorder", 3, order.clone(), 200).await,
        result
    );
    edit(&pool, &id, "old-order", 3, order, 409).await;
    edit(
        &pool,
        &id,
        "duplicate",
        4,
        json!({"kind":"reorder","order":["C1","C1","C3","C4"]}),
        422,
    )
    .await;
    edit(
        &pool,
        &id,
        "dependency",
        4,
        json!({"kind":"reorder","order":["C4","C1","C2","C3"]}),
        422,
    )
    .await;
    let restart = sqlx::postgres::PgPoolOptions::new()
        .connect(&url)
        .await
        .unwrap();
    assert_eq!(
        view(&restart, &id).await["execution"]["items"][1]["child_id"],
        "C2"
    );
    let after: Vec<(String, i64, Value)> = sqlx::query_as(
        "SELECT child_id,authorization_id,input FROM group_execution_item ORDER BY child_id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(before, after);
    assert_eq!(
        view(&pool, &id).await["authorizations"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    // Synthetic persisted completion fixture, never a product acceptance claim.
    sqlx::query("UPDATE requirement SET state='Submitted' WHERE id=(SELECT requirement_id FROM group_execution_item WHERE child_id='C1')").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO group_completion(requirement_id,authorization_id,fact) SELECT requirement_id,authorization_id,jsonb_build_object('requirement_id',requirement_id,'authorization_id',authorization_id,'child_revision',1,'repository_id',1,'github_repository_id',123,'pr_number',1,'head_sha',repeat('a',40),'merged_sha',repeat('b',40),'acceptance_sha',repeat('b',40),'acceptance_plan',input#>'{review,verification}','source','fixture:persistence-only','evidence_sha256',repeat('c',64),'artifact','fixture') FROM group_execution_item WHERE child_id='C1'").execute(&pool).await.unwrap();
    let current = view(&pool, &id).await;
    document = current["document"].clone();
    review = current["review"].clone();
    document["children"][1]["goal"] = json!("Another unstarted revision");
    review["parent_revision"] = json!(3);
    for item in review["items"].as_array_mut().unwrap() {
        item["revision"] = json!(3);
    }
    review["coverage"][0]["child_revision"] = json!(3);
    edit(
        &pool,
        &id,
        "after-completion",
        4,
        json!({"kind":"propose","document":document,"review":review}),
        200,
    )
    .await;
    edit(
        &pool,
        &id,
        "after-completion-approve",
        5,
        json!({"kind":"approve","edit_version":5}),
        200,
    )
    .await;
    assert_eq!(view(&pool, &id).await["execution"]["completed"], 1);
    let saved: Value = sqlx::query_scalar("SELECT fact FROM group_completion")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(saved["source"], "fixture:persistence-only");
}
#[tokio::test]
async fn adding_removing_and_budget_reduction_preserve_ledgers_and_reject_policy_drift() {
    let (pool, _, _) = fixture().await;
    let id = authorized(&pool).await;
    let (mut document, mut review) = changed();
    let mut child = document["children"][1].clone();
    child["id"] = json!("C5");
    child["order"] = json!(2);
    document["children"][1] = child;
    document["children"][2]["depends_on"] = json!(["C5"]);
    review["items"][1]["child_id"] = json!("C5");
    sqlx::query("UPDATE group_budget SET used='{\"tokens\":250,\"turns\":1,\"model_seconds\":5}' WHERE item_id=''").execute(&pool).await.unwrap();
    let mut low = review.clone();
    low["group_budget"] = json!({"tokens":100,"turns":4,"model_seconds":100});
    edit(
        &pool,
        &id,
        "low-budget",
        1,
        json!({"kind":"propose","document":document,"review":low}),
        200,
    )
    .await;
    edit(
        &pool,
        &id,
        "low-approve",
        2,
        json!({"kind":"approve","edit_version":2}),
        422,
    )
    .await;
    edit(
        &pool,
        &id,
        "replace",
        2,
        json!({"kind":"propose","document":document,"review":review}),
        200,
    )
    .await;
    sqlx::query("UPDATE repository SET version=2")
        .execute(&pool)
        .await
        .unwrap();
    edit(
        &pool,
        &id,
        "policy-drift",
        3,
        json!({"kind":"approve","edit_version":3}),
        409,
    )
    .await;
    sqlx::query("UPDATE repository SET version=1")
        .execute(&pool)
        .await
        .unwrap();
    edit(
        &pool,
        &id,
        "replace-approve",
        3,
        json!({"kind":"approve","edit_version":3}),
        200,
    )
    .await;
    let row: (bool, bool) =
        sqlx::query_as("SELECT removed,frozen FROM group_execution_item WHERE child_id='C2'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(row, (true, true));
    let members: Vec<String> = sqlx::query_scalar(
        "SELECT child_id FROM group_execution_item WHERE NOT removed ORDER BY child_id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(members, vec!["C1", "C3", "C4", "C5"]);
    let ledgers: i64 = sqlx::query_scalar("SELECT count(*) FROM group_budget")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(ledgers, 6);
    assert_eq!(view(&pool, &id).await["budgets"][0]["used"]["tokens"], 250);
    edit(
        &pool,
        &id,
        "no-pending",
        4,
        json!({"kind":"approve","edit_version":3}),
        409,
    )
    .await;
    let current = view(&pool, &id).await;
    edit(
        &pool,
        &id,
        "bad-revision",
        4,
        json!({"kind":"propose","document":current["document"],"review":current["review"]}),
        409,
    )
    .await;
}

#[tokio::test]
async fn invalid_envelopes_and_unapproved_queues_do_not_create_partial_membership() {
    let (pool, _, _) = fixture().await;
    let router = app(&pool);
    let draft = request(&router, "POST", "/api/drafts", body(sample(), 0), 200).await;
    let id = draft["id"].as_str().unwrap();
    let path = format!("/api/drafts/{id}/queue-edit");
    request(&router, "POST", &path, json!({}), 422).await;
    edit(
        &pool,
        id,
        "missing",
        0,
        json!({"kind":"reorder","order":["C1","C2","C3","C4"]}),
        409,
    )
    .await;
    request(
        &router,
        "PUT",
        &format!("/api/drafts/{id}/review"),
        json!({"version":0,"draft_revision":1,"review":review()}),
        200,
    )
    .await;
    request(
        &router,
        "POST",
        &format!("/api/drafts/{id}/authorize"),
        json!({"request_id":"before-projection","version":1,"draft_revision":1}),
        200,
    )
    .await;
    request(
        &router,
        "PUT",
        &format!("/api/drafts/{id}/review"),
        json!({"version":1,"draft_revision":1,"review":review()}),
        200,
    )
    .await;
    let (document, review) = changed();
    edit(
        &pool,
        id,
        "not-currently-authorized",
        2,
        json!({"kind":"propose","document":document,"review":review}),
        409,
    )
    .await;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM group_execution_item")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    request(
        &router,
        "POST",
        &format!("/api/drafts/{id}/authorize"),
        json!({"request_id":"reapproved","version":2,"draft_revision":1}),
        200,
    )
    .await;
    let mut review = review.clone();
    review["items"][1]["child_id"] = json!("C2");
    let mut unchanged = groups::review();
    unchanged["parent_revision"] = json!(2);
    for item in unchanged["items"].as_array_mut().unwrap() {
        item["revision"] = json!(2);
    }
    unchanged["coverage"][0]["child_revision"] = json!(2);
    edit(
        &pool,
        id,
        "no-content",
        3,
        json!({"kind":"propose","document":sample(),"review":unchanged}),
        422,
    )
    .await;
    edit(
        &pool,
        id,
        "first-edit",
        3,
        json!({"kind":"propose","document":document,"review":review}),
        200,
    )
    .await;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM group_execution_item")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 4);
}

#[test]
fn dependency_closure_matches_same_repository_baseline_binding() {
    let mut original = sample();
    original["children"][2]["depends_on"] = json!(["C1"]);
    original["children"][3]["depends_on"] = json!(["C2", "C3"]);
    for same_repo in [true, false] {
        if !same_repo {
            original["children"][2]["repository_id"] = json!(2);
        }
        let mut updated = original.clone();
        updated["children"][1]["goal"] = json!("new scope");
        let before = serde_json::from_value(original.clone()).unwrap();
        let after = serde_json::from_value(updated).unwrap();
        let review = serde_json::from_value(review()).unwrap();
        let changed = group_edit::affected(&before, &review, &after, &review);
        assert_eq!(changed.contains("C3"), same_repo);
        assert!(changed.contains("C4"));
        assert!(!changed.contains("C1"));
    }
}

#[test]
fn shared_scope_and_group_ceiling_invalidate_all_outstanding_inputs() {
    let before = serde_json::from_value(sample()).unwrap();
    let old = serde_json::from_value(review()).unwrap();
    let mut after = sample();
    after["parent"]["scope"] = json!("expanded shared scope");
    assert_eq!(
        group_edit::affected(&before, &old, &serde_json::from_value(after).unwrap(), &old).len(),
        4
    );
    let mut new = review();
    new["group_budget"] = json!({"tokens":300,"turns":4,"model_seconds":100});
    assert_eq!(
        group_edit::affected(
            &before,
            &old,
            &before,
            &serde_json::from_value(new).unwrap()
        )
        .len(),
        4
    );
    let mut missing = review();
    missing["items"].as_array_mut().unwrap().remove(1);
    assert!(
        group_edit::affected(
            &before,
            &old,
            &before,
            &serde_json::from_value(missing).unwrap()
        )
        .contains("C2")
    );
}
#[tokio::test]
async fn kind_rebinding_is_rejected_and_added_validation_items_never_create_a_run() {
    let (pool, _, _) = fixture().await;
    let id = authorized(&pool).await;
    let (mut document, review) = changed();
    document["children"][1]["kind"] = json!("validation_only");
    edit(
        &pool,
        &id,
        "kind-change",
        1,
        json!({"kind":"propose","document":document,"review":review}),
        200,
    )
    .await;
    edit(
        &pool,
        &id,
        "kind-approve",
        2,
        json!({"kind":"approve","edit_version":2}),
        422,
    )
    .await;
    assert_eq!(view(&pool, &id).await["draft_revision"], 1);
    let mut document = sample();
    let mut child = document["children"][3].clone();
    child["id"] = json!("C5");
    child["order"] = json!(5);
    child["depends_on"] = json!(["C4"]);
    document["children"].as_array_mut().unwrap().push(child);
    let mut review = review;
    let mut item = review["items"][3].clone();
    item["child_id"] = json!("C5");
    review["items"].as_array_mut().unwrap().push(item);
    review["coverage"][0]["child_id"] = json!("C5");
    edit(
        &pool,
        &id,
        "add-validation",
        2,
        json!({"kind":"propose","document":document,"review":review}),
        200,
    )
    .await;
    edit(
        &pool,
        &id,
        "add-validation-approve",
        3,
        json!({"kind":"approve","edit_version":3}),
        200,
    )
    .await;
    let saved = view(&pool, &id).await;
    assert_eq!(
        saved["execution"]["items"][4]["waiting_reason"],
        "waiting_explicit_validation_authorization"
    );
    assert!(saved["execution"]["items"][4]["requirement_id"].is_null());
    let runs: i64 = sqlx::query_scalar("SELECT count(*) FROM agent_run")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(runs, 0);
    let requirements: i64 = sqlx::query_scalar("SELECT count(*) FROM requirement")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(requirements, 3);
}
