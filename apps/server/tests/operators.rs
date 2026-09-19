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
async fn versioned_operations_and_independent_durable_views() {
    let (pool, app) = fixture().await;
    sqlx::query("UPDATE storage_guard SET blocked=false,error=NULL")
        .execute(&pool)
        .await
        .unwrap();
    ok(&app, "PUT", "/api/repository", repo(0, "operator-repo")).await;
    let created = ok(
        &app,
        "POST",
        "/api/requirements",
        json!({"version":0,"request_id":"operator-create","contract":contract()}),
    )
    .await;
    let id = created["id"].as_i64().unwrap();
    let path = format!("/api/requirements/{id}/operations");
    let initial = ok(&app, "GET", &path, json!(null)).await;
    assert_eq!(initial["metrics"]["model_calls"], 0);
    assert!(initial["metrics"]["input"].is_null());
    assert_eq!(initial["metrics"]["zero_intervention"]["denominator"], 0);
    assert_eq!(initial["storage_lifecycle"], "not_ready");
    assert_eq!(
        request(
            &app,
            "GET",
            "/api/requirements/99999/operations",
            json!(null)
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(
            &app,
            "POST",
            &path,
            json!({"version":1,"request_id":"pause-draft","action":"pause"})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let ready = ok(
        &app,
        "POST",
        &format!("/api/requirements/{id}/ready"),
        json!({"version":1,"repository_version":1,"request_id":"operator-ready"}),
    )
    .await;
    let ready = verify_recheck(&pool, &app, id, &path, &ready).await;
    let pause = json!({"version":ready["version"],"request_id":"operator-pause","action":"pause"});
    let paused = ok(&app, "POST", &path, pause.clone()).await;
    assert_eq!(ok(&app, "POST", &path, pause.clone()).await, paused);
    let mut changed = pause.clone();
    changed["action"] = json!("cancel");
    assert_eq!(
        request(&app, "POST", &path, changed).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        request(
            &app,
            "POST",
            &path,
            json!({"version":1,"request_id":"stale","action":"cancel"})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        request(&app, "POST", &path, json!({"action":"cancel"}))
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        request(
            &app,
            "POST",
            &path,
            json!({"version":1,"request_id":"","action":"cancel"})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        ok(&app, "GET", "/api/inbox", json!(null)).await["requirement_ids"],
        json!([id])
    );
    sqlx::query("UPDATE execution_control SET recovery_complete=false")
        .execute(&pool)
        .await
        .unwrap();
    let resume =
        json!({"version":paused["version"],"request_id":"operator-resume","action":"resume"});
    assert_eq!(
        request(&app, "POST", &path, resume.clone()).await.0,
        StatusCode::CONFLICT
    );
    sqlx::query("UPDATE execution_control SET recovery_complete=true")
        .execute(&pool)
        .await
        .unwrap();
    let resumed = ok(&app, "POST", &path, resume).await;
    assert_eq!(
        ok(&app, "GET", &path, json!(null)).await["requirement"]["paused"],
        false
    );
    // Persist a real Run and question; views and answers do not own its lifecycle.
    sqlx::query("UPDATE requirement SET state='Running',paused=true WHERE id=$1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO agent_run(id,requirement_id,revision,incarnation,request_id,workspace,workspace_identity,launch,state) VALUES('operator-run',$1,1,'fixture','request','fixture','fixture','{}','Running')").bind(id).execute(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO runtime_session(run_id,created_at,last_progress) VALUES('operator-run',1,1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO runtime_question(id,requirement_id,revision,run_id,rpc_id,original,created_at) VALUES('operator-question',$1,1,'operator-run','1',$2,1)")
        .bind(id).bind(json!({"params":{"questions":[{"id":"choice","question":"Which option?","options":[{"label":"yes"}]}]}})).execute(&pool).await.unwrap();
    let question = ok(&app, "GET", &path, json!(null)).await;
    assert_eq!(
        question["questions"][0]["questions"][0]["question"],
        "Which option?"
    );
    let answer_path = "/api/operator/questions/operator-question/answer";
    assert_eq!(
        request(&app, "POST", answer_path, json!({})).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(request(&app,"POST",answer_path,json!({"version":1,"answers":[{"id":"choice","text":"yes"},{"id":"choice","text":"no"}]})).await.0,StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        request(
            &app,
            "POST",
            answer_path,
            json!({"version":2,"answers":[{"id":"choice","text":"yes"}]})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let answer = json!({"version":1,"answers":[{"id":"choice","text":"yes"}]});
    assert_eq!(
        ok(&app, "POST", answer_path, answer.clone()).await["saved"],
        true
    );
    assert_eq!(
        request(&app, "POST", answer_path, answer).await.0,
        StatusCode::CONFLICT
    );
    let after = ok(&app, "GET", &path, json!(null)).await;
    assert_eq!(after["requirement"]["paused"], true);
    assert_eq!(after["runs"][0]["state"], "Running");
    assert_eq!(after["questions"][0]["resume_state"], "pending");
    assert!(after["metrics"]["interventions"].as_i64().unwrap() >= 3);
    // Only owned database evidence is readable; no filesystem path parameter exists.
    sqlx::query("INSERT INTO runtime_evidence(run_id,channel,kept_bytes,truncated) VALUES('operator-run','stdout',50,true)").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO runtime_evidence_chunk(run_id,channel,sequence,payload) VALUES('operator-run','stdout',1,$1)").bind(b"safe output\nAuthorization: Bearer private-fixture\nend".as_slice()).execute(&pool).await.unwrap();
    let evidence = ok(
        &app,
        "GET",
        &format!("/api/requirements/{id}/evidence/operator-run/stdout"),
        json!(null),
    )
    .await;
    assert!(evidence["text"].as_str().unwrap().contains("safe output"));
    assert!(!evidence.to_string().contains("private-fixture"));
    assert_eq!(
        request(
            &app,
            "GET",
            "/api/requirements/99999/evidence/operator-run/stdout",
            json!(null)
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let facts = ok(&app, "GET", &path, json!(null)).await;
    assert_eq!(facts["materials"][0]["status"], "truncated");
    verify_metrics(&pool, &app, id, &path).await;
    verify_negative_requests(&app, id).await;
    let cancelled = ok(
        &app,
        "POST",
        &path,
        json!({"version":resumed["version"],"request_id":"operator-cancel","action":"cancel"}),
    )
    .await;
    assert!(cancelled["version"].as_i64().unwrap() > resumed["version"].as_i64().unwrap());
    let facts = ok(&app, "GET", &path, json!(null)).await;
    assert_eq!(facts["requirement"]["state"], "Cancelled");
    assert_eq!(facts["requirement"]["cleanup_complete"], false);
    assert_eq!(facts["runs"][0]["state"], "Running");
    assert_eq!(
        request(
            &app,
            "POST",
            &path,
            json!({"version":cancelled["version"],"request_id":"cancelled-pause","action":"pause"})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        ok(&app, "GET", "/api/inbox", json!(null)).await["requirement_ids"],
        json!([id])
    );
    sqlx::query("UPDATE agent_run SET quiescent=true WHERE requirement_id=$1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    codexsymphony_server::delivery_control::settle(&pool)
        .await
        .unwrap();
    assert_eq!(
        ok(&app, "GET", &path, json!(null)).await["requirement"]["cleanup_complete"],
        true
    );
    assert_eq!(
        ok(&app, "GET", "/api/inbox", json!(null)).await["requirement_ids"],
        json!([])
    );
    verify_zero_intervention(&pool, &app).await;
    pool.close().await;
    assert_eq!(
        request(&app, "GET", &path, json!(null)).await.0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        request(&app, "GET", "/api/inbox", json!(null)).await.0,
        StatusCode::SERVICE_UNAVAILABLE
    );
}
#[test]
fn control_availability_and_redaction() {
    use codexsymphony_server::{
        operator_control::{Action, allowed},
        operator_view::{redact, redact_text},
    };
    assert!(allowed("Ready", false, Action::Pause));
    assert!(!allowed("Ready", true, Action::Pause));
    assert!(allowed("Running", true, Action::Resume));
    assert!(!allowed("Draft", true, Action::Resume));
    assert!(allowed("Failed", false, Action::Cancel));
    assert!(!allowed("Cancelled", false, Action::Cancel));
    assert!(allowed("Ready", false, Action::Recheck));
    assert!(!allowed("Draft", false, Action::Recheck));
    let mut value = json!({"array":["password=private","ordinary",2,null,true]});
    redact(&mut value);
    assert_eq!(value["array"][0], "[redacted]");
    assert_eq!(value["array"][1], "ordinary");
    assert_eq!(
        redact_text("safe\nBearer private\nend"),
        "safe\n[redacted]\nend"
    );
}

#[test]
fn operator_previews_never_expose_multiline_private_keys() {
    use codexsymphony_server::operator_view::redact_text;
    for text in [
        "-----BEGIN PRIVATE KEY-----\nprivate-body\n-----END PRIVATE KEY-----",
        "Authorization: Basic private",
        "{\"token\":\"private\"}",
        "credentials: private",
    ] {
        assert_eq!(redact_text(text), "[redacted]");
    }
}

async fn verify_negative_requests(app: &Router, id: i64) {
    for (method, path, body, status) in [
        (
            "POST",
            "/api/operator/questions/operator-question/answer".into(),
            json!({"version":1,"answers":"wrong"}),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "POST",
            "/api/operator/questions/missing/answer".into(),
            json!({"version":1,"answers":[]}),
            StatusCode::NOT_FOUND,
        ),
        (
            "GET",
            format!("/api/requirements/{id}/evidence/operator-run/other"),
            json!(null),
            StatusCode::NOT_FOUND,
        ),
        (
            "GET",
            format!("/api/requirements/{id}/evidence/foreign-run/stdout"),
            json!(null),
            StatusCode::NOT_FOUND,
        ),
    ] {
        assert_eq!(request(app, method, &path, body).await.0, status);
    }
}

async fn verify_metrics(pool: &PgPool, app: &Router, id: i64, path: &str) {
    sqlx::query("INSERT INTO agent_run(id,requirement_id,revision,incarnation,request_id,workspace,workspace_identity,launch,state,quiescent,waiting) VALUES('metrics-old',$1,1,'old','metrics-old','fixture','fixture','{}','Succeeded',true,'{\"human_seconds\":7}')")
        .bind(id).execute(pool).await.unwrap();
    sqlx::query("UPDATE agent_run SET waiting='{\"human_seconds\":11}' WHERE id='operator-run'")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO model_call(run_id,turn_id,requirement_id,intent,reserved,usage) VALUES('operator-run','call',$1,'{}','{}','{\"input\":100,\"cached\":20,\"output\":30}'),('metrics-old','call',$1,'{}','{}','{\"input\":50,\"cached\":10,\"output\":null}')")
        .bind(id).execute(pool).await.unwrap();
    let metrics = ok(app, "GET", path, json!(null)).await["metrics"].clone();
    assert_eq!(metrics["model_calls"], 2);
    assert_eq!(metrics["input"], 150);
    assert_eq!(metrics["cached"], 30);
    assert!(
        metrics["output"].is_null(),
        "unknown usage must not be counted as zero"
    );
    assert_eq!(metrics["human_seconds"], 18);
    assert!(metrics["to_pr_seconds"].is_null());
    assert_eq!(metrics["zero_intervention"]["denominator"], 0);
    assert!(!metrics["phases"].as_array().unwrap().is_empty());
    sqlx::query(
        "UPDATE model_call SET usage=jsonb_set(usage,'{output}','10') WHERE run_id='metrics-old'",
    )
    .execute(pool)
    .await
    .unwrap();
    assert_eq!(
        ok(app, "GET", path, json!(null)).await["metrics"]["output"],
        40
    );
}

async fn verify_recheck(pool: &PgPool, app: &Router, id: i64, path: &str, ready: &Value) -> Value {
    use codexsymphony_server::preparation::{Failure, Retry};
    let mut retry = Retry::new("preparation", 1);
    retry.todo = true;
    retry.attempts = 3;
    retry.last_failure = Some(Failure::new(
        "preparation_dependency_missing",
        "token=fixture-private",
        "probe",
    ));
    sqlx::query("INSERT INTO preparation_record(run_id,requirement_id,revision,launch,retry,checked_at) VALUES('operator-preflight',$1,1,'{}',$2,1)")
        .bind(id).bind(json!(retry)).execute(pool).await.unwrap();
    let facts = ok(app, "GET", path, json!(null)).await;
    assert_eq!(facts["preparation"][0]["detail"], "[redacted]");
    assert_eq!(facts["preparation"][0]["attempts"], 3);
    let recheck =
        json!({"version":ready["version"],"request_id":"operator-recheck","action":"recheck"});
    let result = ok(app, "POST", path, recheck.clone()).await;
    assert_eq!(ok(app, "POST", path, recheck).await, result);
    let facts = ok(app, "GET", path, json!(null)).await;
    assert_eq!(facts["preparation"][0]["todo"], false);
    assert_eq!(facts["preparation"][0]["attempts"], 3);
    assert_eq!(facts["metrics"]["repair_count"], 0);
    assert_eq!(facts["metrics"]["reasons"], json!(["preparation_recheck"]));
    assert_eq!(
        request(
            app,
            "POST",
            path,
            json!({"version":result["version"],"request_id":"invalid-recheck","action":"recheck"})
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    result
}

async fn verify_zero_intervention(pool: &PgPool, app: &Router) {
    let mut ids = Vec::new();
    for name in ["untouched", "withdrawn"] {
        let created = ok(
            app,
            "POST",
            "/api/requirements",
            json!({"version":0,"request_id":format!("metric-{name}"),"contract":contract()}),
        )
        .await;
        let id = created["id"].as_i64().unwrap();
        ids.push(id);
        let ready = ok(app, "POST", &format!("/api/requirements/{id}/ready"), json!({"version":created["version"],"repository_version":1,"request_id":format!("metric-ready-{name}")})).await;
        if name == "withdrawn" {
            let draft = ok(app, "POST", &format!("/api/requirements/{id}/withdraw"), json!({"version":ready["version"],"repository_version":1,"request_id":"metric-withdraw"})).await;
            ok(app, "POST", &format!("/api/requirements/{id}/ready"), json!({"version":draft["version"],"repository_version":1,"request_id":"metric-rereview"})).await;
        }
        // Metrics-only persistent fixture, not an assertion of PR delivery/A01.
        sqlx::query("UPDATE requirement SET state='Submitted' WHERE id=$1")
            .bind(id)
            .execute(pool)
            .await
            .unwrap();
    }
    let facts = ok(
        app,
        "GET",
        &format!("/api/requirements/{}/operations", ids[1]),
        json!(null),
    )
    .await;
    assert_eq!(facts["metrics"]["reasons"], json!(["review_withdrawal"]));
    assert_eq!(
        facts["metrics"]["zero_intervention"],
        json!({"phase":"reviewed_to_submitted","denominator":2,"numerator":1})
    );
    assert!(facts["metrics"]["to_pr_seconds"].is_null());
}
