use axum::{
    Router,
    body::{Body, to_bytes},
    http::Request,
};
use codexsymphony_server::process;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::path::PathBuf;
use tower::ServiceExt;

pub fn sample() -> Value {
    let children: Vec<Value> = (1..=4).map(|i| json!({"id":format!("C{i}"),"parent_id":"P1","kind":if i==4 {"validation_only"} else {"code_change"},"order":i,"depends_on":if i==1 {vec![]} else {vec![format!("C{}",i-1)]},"repository_id":1,"goal":format!("Step {i}"),"acceptance_criteria":[{"id":"AC1","description":"observable result"}],"validation_plan":"run the selected automated test"})).collect();
    json!({"schema":"codexsymphony-draft/v1","parent":{"id":"P1","goal":"Complete chain","scope":"test feature only","acceptance_criteria":[{"id":"P-AC1","description":"integrated flow works"}]},"children":children})
}
pub fn review() -> Value {
    let items:Vec<Value> = (1..=4).map(|i|json!({"child_id":format!("C{i}"),"revision":1,"repository_version":1,"budget":{"tokens":100,"turns":2,"model_seconds":60},"repair_scope":"only this item AC in this repository; no new permissions","merged_baseline_review":"independent regression check on main after predecessors merge; safe without later changes","verification":[{"ac_id":"AC1","step":{"id":"test","check":"cargo_test","selector":"group_review","expected_result":"exit 0","timeout_seconds":30}}]})).collect();
    json!({"parent_revision":1,"full_chain_acs":["P-AC1"],"coverage":[{"parent_ac":"P-AC1","child_id":"C4","child_revision":1,"child_ac":"AC1","step_id":"test"}],"items":items,"group_budget":null,"semantic_review":"Reviewed the integration test against the complete parent flow"})
}
pub fn repository() -> Value {
    json!({"model":"fixture-model","project":"synthetic","remote":"test/group","github_repository_id":123,"base_branch":"main","policy":{"allowed_checks":["cargo_test"],"max_timeout_seconds":60,"token_limit":1000,"turn_limit":20,"model_work_seconds":600,"gate_recovery_policy":"one_code_repair"},"revoked":false,"reason":"synthetic group test"})
}
fn source(document: Value) -> Value {
    json!({"format":"json","label":"synthetic source; never authorization","text":document.to_string()})
}
pub fn body(document: Value, version: i64) -> Value {
    json!({"version":version,"source":source(document)})
}
pub fn app(pool: &PgPool) -> Router {
    codexsymphony_server::router(
        pool.clone(),
        codexsymphony_server::security::RequestPolicy::new(
            "127.0.0.1:3081".parse().unwrap(),
            "http://localhost:4200".into(),
        )
        .unwrap(),
    )
}
pub async fn request(app: &Router, method: &str, path: &str, body: Value, expected: u16) -> Value {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("host", "127.0.0.1:3081")
                .header("origin", "http://localhost:4200")
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
pub async fn fixture() -> (PgPool, String, PathBuf) {
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
