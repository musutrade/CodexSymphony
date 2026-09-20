use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tower::ServiceExt;

async fn fixture() -> (PgPool, Router) {
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&std::env::var("TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
    sqlx::query("TRUNCATE business_request,business_event,requirement_revision,requirement,repository RESTART IDENTITY CASCADE").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO execution_control(id) VALUES(1) ON CONFLICT DO NOTHING")
        .execute(&pool)
        .await
        .unwrap();
    let policy = codexsymphony_server::security::RequestPolicy::new(
        "127.0.0.1:3081".parse().unwrap(),
        "http://localhost:4200".into(),
    )
    .unwrap();
    let app = codexsymphony_server::router(pool.clone(), policy);
    (pool, app)
}
fn contract() -> Value {
    json!({"title":"Future test", "description":"Implement a verifiable change", "acceptance_criteria":[{"description":"Works", "verification_ref":"test"}],"validation_plan":[{"id":"test","check":"cargo_test","selector":"future_test::works","expected_result":"exit 0; assertion passes","timeout_seconds":60}],"network_access":[]})
}
fn repo(version: i64, key: &str) -> Value {
    json!({"request_id":key,"version":version,"repository":{"model":"configured-model","project":"Disposable","remote":"musutrade/disposable","github_repository_id":123,"base_branch":"main","policy":{"allowed_checks":["cargo_test"],"max_timeout_seconds":120,"token_limit":10000,"turn_limit":10,"model_work_seconds":600,"gate_recovery_policy":"one_code_repair"},"revoked":false,"reason":"Initial review"}})
}
async fn request(app: &Router, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path.replace("/api/", "/api/multi/"))
                .header("host", "127.0.0.1:3081")
                .header("origin", "http://localhost:4200")
                .header("x-codexsymphony-csrf", "1")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
async fn ok(app: &Router, method: &str, path: &str, body: Value) -> Value {
    let (status, result) = request(app, method, path, body).await;
    assert!(status.is_success(), "{status}: {result}");
    result
}

#[tokio::test]
async fn registration_review_routing_and_global_ownership() {
    use codexsymphony_server::{github_store, runtime_routes::Deployment};
    let (pool, app) = fixture().await;
    let first = repo(0, "first");
    ok(&app, "PUT", "/api/repository", first.clone()).await;
    let mut second = repo(0, "second");
    second["repository_id"] = json!(2);
    second["repository"]["remote"] = json!("musutrade/disposable-2");
    second["repository"]["github_repository_id"] = json!(124);
    ok(&app, "PUT", "/api/repository", second.clone()).await;
    let listing = ok(&app, "GET", "/api/repository", Value::Null).await;
    assert_eq!(listing["repositories"].as_array().unwrap().len(), 2);
    assert_eq!(listing["repositories"][1]["delivery_ready"], false);
    for id in 1..=2 {
        let draft=ok(&app,"POST","/api/requirements",json!({"request_id":format!("draft-{id}"),"repository_id":id,"version":0,"contract":contract()})).await;
        assert_eq!(draft["repository_id"], id);
        let ready = ok(
            &app,
            "POST",
            &format!("/api/requirements/{id}/ready"),
            json!({"request_id":format!("ready-{id}"),"version":1,"repository_version":1}),
        )
        .await;
        assert_eq!(ready["snapshots"][0]["repository_id"], id);
        let mut tx = pool.begin().await.unwrap();
        assert!(!github_store::claim_ready(&mut tx, id, 1).await.unwrap());
    }
    let config = |baseline: &str| json!({"validation":null,"settings":{"startup_seconds":10,"response_seconds":10,"stall_seconds":10,"reservation":{"tokens":100,"turns":1,"model_seconds":10},"codex_config":""},"preparation_adapter":"/bin/true","preparation":{"launcher":["/bin/true"],"baseline":baseline}});
    let route = |repository: &Value, baseline: &str| json!({"github_repository_id":repository["github_repository_id"],"remote":repository["remote"],"base_branch":"main","version":1,"runtime":config(baseline)});
    let root = std::env::temp_dir().join(codexsymphony_server::process::new_identity().unwrap());
    std::fs::create_dir(&root).unwrap();
    let path = root.join("runtime.json");
    std::fs::write(&path,json!({"repositories":{"1":route(&first["repository"],"first"),"2":route(&second["repository"],"second")}}).to_string()).unwrap();
    let routes = Deployment::load(&path).unwrap();
    assert_eq!(
        routes.selected(&pool).await.unwrap().unwrap().1.preparation["baseline"],
        "first"
    );
    sqlx::query("UPDATE requirement SET state='Submitted',paused=true WHERE id=1")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE execution_control SET requirement_id=1,paused=true")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        routes.selected(&pool).await.unwrap().unwrap().1.preparation["baseline"],
        "first"
    );
    // Only this synthetic fixture explicitly simulates completion of release.
    sqlx::query("UPDATE execution_control SET requirement_id=NULL,paused=false")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        routes.selected(&pool).await.unwrap().unwrap().1.preparation["baseline"],
        "second"
    );
    std::fs::write(&path, config("legacy").to_string()).unwrap();
    assert!(
        Deployment::load(&path)
            .unwrap()
            .selected(&pool)
            .await
            .unwrap()
            .is_none()
    );
    // Revoking one policy cannot invalidate the other repository's review.
    second["version"] = json!(1);
    second["request_id"] = json!("revoke-second");
    second["repository"]["revoked"] = json!(true);
    ok(&app, "PUT", "/api/repository", second).await;
    assert!(
        !ok(&app, "GET", "/api/requirements/2", Value::Null).await["authorization_valid"]
            .as_bool()
            .unwrap()
    );
    assert!(routes.selected(&pool).await.unwrap().is_none());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM agent_run")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    // Bad deployment inputs fail before a worker can claim or call a model.
    for invalid in [
        json!({"repositories":{}}),
        json!({"repositories":{"0":route(&first["repository"],"bad")}}),
        json!({"repositories":{"1":{"github_repository_id":-1,"remote":"bad","base_branch":"main","version":1,"runtime":config("bad")}}}),
        json!({"settings":{},"preparation_adapter":"relative","preparation":{}}),
    ] {
        std::fs::write(&path, invalid.to_string()).unwrap();
        assert!(Deployment::load(&path).is_err());
    }
    sqlx::query("UPDATE requirement SET state='Submitted'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(routes.selected(&pool).await.unwrap().is_none());
    std::fs::remove_dir_all(root).unwrap();
}
