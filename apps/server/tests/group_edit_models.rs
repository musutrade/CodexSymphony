//! Queue edits re-freeze models only for changed, unstarted items.
use codexsymphony_server::group_queue_store;
use serde_json::{Value, json};
use sqlx::PgPool;
#[path = "support/groups.rs"]
mod groups;
use groups::*;

fn old() -> Value {
    json!({"provider":"openai","model":"fixture-old","effort":"low"})
}
fn new() -> Value {
    json!({"provider":"openai","model":"fixture-new","effort":"medium"})
}
fn deploy(path: &std::path::Path, version: &str, models: Value) {
    let registration = json!({"version":version,"repositories":[1],"agent":{"name":"codex","models":models,"reliable_stop":true,"resume":true,"cancel":true,"structured_events":true,"usage_reporting":true}});
    let config = json!({"settings":{"model_capabilities":registration,"startup_seconds":5,"response_seconds":5,"stall_seconds":5,"reservation":{"tokens":100,"turns":1,"model_seconds":30},"codex_config":""},"preparation_adapter":"/bin/true","preparation":{"launcher":["/bin/true"]}});
    std::fs::write(path, config.to_string()).unwrap();
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
async fn requirement(pool: &PgPool, child: &str) -> i64 {
    sqlx::query_scalar("SELECT requirement_id FROM group_execution_item WHERE child_id=$1")
        .bind(child)
        .fetch_one(pool)
        .await
        .unwrap()
}
async fn snapshots(pool: &PgPool, requirement: i64) -> Vec<(i64, Value)> {
    sqlx::query_as("SELECT revision,document FROM requirement_revision WHERE requirement_id=$1 ORDER BY revision")
        .bind(requirement)
        .fetch_all(pool)
        .await
        .unwrap()
}
/// The next delta review exactly as a client resubmits the current one.
fn next(current: &Value) -> (Value, Value) {
    let mut review = current["review"].clone();
    review["parent_revision"] = json!(2);
    for item in review["items"].as_array_mut().unwrap() {
        item["revision"] = json!(2);
    }
    review["coverage"][0]["child_revision"] = json!(2);
    (current["document"].clone(), review)
}
/// Restores RUNTIME_CONFIG and removes the test's deployment directory on drop.
struct Restore(Option<std::ffi::OsString>, std::path::PathBuf);
impl Drop for Restore {
    fn drop(&mut self) {
        unsafe {
            match &self.0 {
                Some(previous) => std::env::set_var("RUNTIME_CONFIG", previous),
                None => std::env::remove_var("RUNTIME_CONFIG"),
            }
        }
        let _ = std::fs::remove_dir_all(&self.1);
    }
}
fn propose(document: &Value, review: &Value) -> Value {
    json!({"kind":"propose","document":document,"review":review})
}

#[tokio::test]
async fn capability_upgrade_reviews_only_the_changed_unstarted_item() {
    let (pool, _, root) = fixture().await;
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("runtime.json");
    deploy(&path, "v1", json!([old()]));
    let _env = Restore(std::env::var_os("RUNTIME_CONFIG"), root.clone());
    unsafe {
        std::env::set_var("RUNTIME_CONFIG", &path);
    }
    let router = app(&pool);
    // C4 becomes a code item so it owns a Requirement; parent coverage still maps to it.
    let mut document = sample();
    document["children"][3]["kind"] = json!("code_change");
    let draft = request(&router, "POST", "/api/drafts", body(document, 0), 200).await;
    let id = draft["id"].as_str().unwrap().to_string();
    let mut reviewed = review();
    reviewed["full_chain_acs"] = json!([]);
    for item in reviewed["items"].as_array_mut().unwrap() {
        item["model_selection"] = json!({"config":old(),"reason":"historical approval"});
    }
    let review_path = format!("/api/drafts/{id}/review");
    request(
        &router,
        "PUT",
        &review_path,
        json!({"version":0,"draft_revision":1,"review":reviewed}),
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
    group_queue_store::materialize(&pool).await.unwrap();

    // Synthetic persisted facts, never a product acceptance claim: C1 completed, C2 claimed.
    let (c1, c2, c3, c4) = (
        requirement(&pool, "C1").await,
        requirement(&pool, "C2").await,
        requirement(&pool, "C3").await,
        requirement(&pool, "C4").await,
    );
    sqlx::query("UPDATE requirement SET state='Submitted' WHERE id=$1")
        .bind(c1)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO group_completion(requirement_id,authorization_id,fact) SELECT requirement_id,authorization_id,jsonb_build_object('requirement_id',requirement_id,'authorization_id',authorization_id,'child_revision',1,'repository_id',1,'github_repository_id',123,'pr_number',1,'head_sha',repeat('a',40),'merged_sha',repeat('b',40),'acceptance_sha',repeat('b',40),'acceptance_plan',input#>'{review,verification}','source','fixture:persistence-only','evidence_sha256',repeat('c',64),'artifact','fixture') FROM group_execution_item WHERE child_id='C1'").execute(&pool).await.unwrap();
    sqlx::query("UPDATE requirement SET state='Running' WHERE id=$1")
        .bind(c2)
        .execute(&pool)
        .await
        .unwrap();
    let history = [
        snapshots(&pool, c1).await,
        snapshots(&pool, c2).await,
        snapshots(&pool, c3).await,
    ];
    let fact: Value = sqlx::query_scalar("SELECT fact FROM group_completion")
        .fetch_one(&pool)
        .await
        .unwrap();

    // The retired model is no longer deployed; only the reviewed new model is.
    deploy(&path, "v2", json!([new()]));
    let current = view(&pool, &id).await;
    let original = current["review"].clone();
    assert_eq!(
        original["items"][0]["frozen_model"]["capability_version"],
        "v1"
    );
    let (document, base) = next(&current);

    // Tampering with historical frozen identity or claimed input is a change to a started item.
    let mut tampered = base.clone();
    tampered["items"][0]["frozen_model"]["capability_version"] = json!("v2");
    edit(
        &pool,
        &id,
        "tamper-frozen",
        1,
        propose(&document, &tampered),
        409,
    )
    .await;
    let mut tampered = base.clone();
    tampered["items"][1]["model_selection"] =
        json!({"config":new(),"reason":"rewrite claimed input"});
    edit(
        &pool,
        &id,
        "tamper-claimed",
        1,
        propose(&document, &tampered),
        409,
    )
    .await;
    assert!(view(&pool, &id).await["pending_edit"].is_null());

    let mut changed = base.clone();
    changed["items"][3]["model_selection"] = json!({"config":new(),"reason":"reviewed new model"});
    let proposed = edit(&pool, &id, "upgrade", 1, propose(&document, &changed), 200).await;
    assert_eq!(proposed["affected"], json!(["C4"]));
    let pending = view(&pool, &id).await["pending_edit"]["review"].clone();
    for index in 0..3 {
        assert_eq!(
            pending["items"][index]["frozen_model"],
            original["items"][index]["frozen_model"]
        );
    }
    let refreshed = &pending["items"][3]["frozen_model"];
    assert_eq!(refreshed["selection"]["config"], new());
    assert_eq!(refreshed["capability_version"], "v2");
    assert_eq!(refreshed["source"], "requirement_override");

    // Approval re-checks current authority for the changed item only.
    deploy(&path, "v3", json!([new()]));
    edit(
        &pool,
        &id,
        "approve-drift",
        2,
        json!({"kind":"approve","edit_version":2}),
        422,
    )
    .await;
    deploy(&path, "v2", json!([new()]));
    edit(
        &pool,
        &id,
        "approve",
        2,
        json!({"kind":"approve","edit_version":2}),
        200,
    )
    .await;

    assert_eq!(
        [
            snapshots(&pool, c1).await,
            snapshots(&pool, c2).await,
            snapshots(&pool, c3).await
        ],
        history
    );
    let saved: Value = sqlx::query_scalar("SELECT fact FROM group_completion")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(saved, fact);
    let upgraded = snapshots(&pool, c4).await;
    let latest = &upgraded.last().unwrap().1["frozen_model"];
    assert_eq!(latest["selection"]["config"], new());
    assert_eq!(latest["capability_version"], "v2");
    let input: Value =
        sqlx::query_scalar("SELECT input FROM group_execution_item WHERE child_id='C1'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        input["review"]["frozen_model"],
        original["items"][0]["frozen_model"]
    );
}
