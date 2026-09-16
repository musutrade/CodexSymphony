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
async fn database_api_acceptance() {
    let (pool, app) = fixture().await;
    assert_eq!(
        ok(&app, "GET", "/api/repository", json!(null)).await["repositories"],
        json!([])
    );
    assert_eq!(
        request(&app, "GET", "/api/requirements/999", json!(null))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let mut invalid_model = repo(0, "invalid-model");
    invalid_model["repository"]["model"] = json!(" ");
    assert_eq!(
        request(&app, "PUT", "/api/repository", invalid_model)
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let draft = json!({"request_id":"create","version":0,"contract":contract()});
    let created = ok(&app, "POST", "/api/requirements", draft.clone()).await;
    assert_eq!(
        created,
        ok(&app, "POST", "/api/requirements", draft.clone()).await
    );
    let mut changed = draft.clone();
    changed["contract"]["title"] = json!("different");
    assert_eq!(
        request(&app, "POST", "/api/requirements", changed).await.0,
        StatusCode::CONFLICT
    );
    let id = created["id"].as_i64().unwrap();
    let path = format!("/api/requirements/{id}");
    let ready = format!("{path}/ready");
    assert_eq!(
        request(
            &app,
            "POST",
            &ready,
            json!({"request_id":"no-repo","version":1,"repository_version":0})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    ok(&app, "PUT", "/api/repository", repo(0, "repo")).await;
    assert_eq!(
        request(&app, "PUT", "/api/repository", repo(0, "stale-repo"))
            .await
            .0,
        StatusCode::CONFLICT
    );
    let edited = ok(
        &app,
        "PATCH",
        &path,
        json!({"request_id":"edit","version":1,"contract":contract()}),
    )
    .await;
    assert_eq!(edited["version"], 2);
    assert_eq!(
        request(
            &app,
            "PATCH",
            &path,
            json!({"request_id":"stale","version":1,"contract":contract()})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let control = json!({"request_id":"review","version":2,"repository_version":1});
    let (one, two) = tokio::join!(
        request(&app, "POST", &ready, control.clone()),
        request(
            &app,
            "POST",
            &ready,
            json!({"request_id":"other-review","version":2,"repository_version":1})
        )
    );
    assert!(one.0.is_success() ^ two.0.is_success());
    let frozen = ok(&app, "GET", &path, json!(null)).await;
    assert_eq!(frozen["state"], "Ready");
    assert_eq!(frozen["snapshots"].as_array().unwrap().len(), 1);
    let winner = if one.0.is_success() {
        control
    } else {
        json!({"request_id":"other-review","version":2,"repository_version":1})
    };
    assert_eq!(ok(&app, "POST", &ready, winner).await, frozen);
    assert_eq!(
        request(
            &app,
            "PATCH",
            &path,
            json!({"request_id":"ready-edit","version":3,"contract":contract()})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let mut updated = repo(1, "update-policy");
    updated["repository"]["policy"]["token_limit"] = json!(20000);
    ok(&app, "PUT", "/api/repository", updated).await;
    assert_eq!(
        ok(&app, "GET", &path, json!(null)).await["snapshots"],
        frozen["snapshots"]
    );
    let withdrawal = json!({"request_id":"withdraw","version":3,"repository_version":1});
    let withdrawn = ok(
        &app,
        "POST",
        &format!("{path}/withdraw"),
        withdrawal.clone(),
    )
    .await;
    assert_eq!(
        withdrawn,
        ok(&app, "POST", &format!("{path}/withdraw"), withdrawal).await
    );
    let mut revoked = repo(2, "revoke");
    revoked["repository"]["revoked"] = json!(true);
    ok(&app, "PUT", "/api/repository", revoked).await;
    assert_eq!(
        request(
            &app,
            "POST",
            &ready,
            json!({"request_id":"revoked","version":4,"repository_version":3})
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let mut reauthorize = repo(3, "reauthorize");
    reauthorize["repository"]["policy"]["token_limit"] = json!(20000);
    ok(&app, "PUT", "/api/repository", reauthorize).await;
    let again = ok(
        &app,
        "POST",
        &ready,
        json!({"request_id":"new-review","version":4,"repository_version":4}),
    )
    .await;
    assert_eq!(again["snapshots"].as_array().unwrap().len(), 2);
    assert_eq!(again["snapshots"][1]["repository_version"], 4);
    assert_eq!(again["authorization_valid"], true);
    let budget = codexsymphony_server::budget_store::inspect(&pool, id)
        .await
        .unwrap();
    assert_eq!(
        budget.limits.tokens, 10000,
        "re-review with a new policy does not refresh the first grant"
    );
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM budget_authorization WHERE requirement_id=$1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 1);
    let mut revoke_ready = repo(4, "revoke-ready");
    revoke_ready["repository"]["revoked"] = json!(true);
    ok(&app, "PUT", "/api/repository", revoke_ready).await;
    assert_eq!(
        ok(&app, "GET", &path, json!(null)).await["authorization_valid"],
        false
    );
    ok(
        &app,
        "PUT",
        "/api/repository",
        repo(5, "restore-repository"),
    )
    .await;
    assert_eq!(
        ok(&app, "GET", &path, json!(null)).await["authorization_valid"],
        false
    );
    assert_eq!(
        ok(&app, "GET", &path, json!(null)).await["snapshots"],
        again["snapshots"]
    );
    for state in ["Running", "Submitted"] {
        sqlx::query("UPDATE requirement SET state=$1 WHERE id=$2")
            .bind(state)
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            request(
                &app,
                "PATCH",
                &path,
                json!({"request_id":state,"version":5,"contract":contract()})
            )
            .await
            .0,
            StatusCode::CONFLICT
        );
    }
    invalid_contracts(&app).await;
    security_has_no_side_effects(&app, &pool, &path).await;
    let listing = ok(&app, "GET", "/api/requirements", json!(null)).await;
    assert_eq!(listing["requirements"].as_array().unwrap().len(), 1);
    let event_count: i64 = sqlx::query_scalar("SELECT count(*) FROM business_event")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(event_count, 11);
    // Independent database reads still see the queue and immutable revisions.
    let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM requirement_revision")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, 2);
    failed_review_is_atomic(&app, &pool, &ready).await;
    sqlx::query("TRUNCATE business_request,business_event,requirement_revision,requirement,repository RESTART IDENTITY CASCADE").execute(&pool).await.unwrap();
    pool.close().await;
    for (method, url, body) in [
        ("GET", "/api/requirements", json!(null)),
        ("GET", "/api/repository", json!(null)),
        ("POST", "/api/requirements", draft),
    ] {
        let response = request(&app, method, url, body).await;
        assert_eq!(response.0, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(response.1, json!({"error":"database unavailable"}));
    }
}
async fn failed_review_is_atomic(app: &Router, pool: &PgPool, ready: &str) {
    // Simulate a damaged persisted Contract: review must fail closed and roll back.
    sqlx::query("UPDATE requirement SET state='Draft',contract='{}'::jsonb")
        .execute(pool)
        .await
        .unwrap();
    let before: (i64, i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM business_event),(SELECT count(*) FROM requirement_revision),(SELECT count(*) FROM business_request)")
        .fetch_one(pool).await.unwrap();
    let response = request(
        app,
        "POST",
        ready,
        json!({"request_id":"damaged-contract","version":5,"repository_version":6}),
    )
    .await;
    assert_eq!(response.0, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.1, json!({"error":"stored record unavailable"}));
    let after: (i64, i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM business_event),(SELECT count(*) FROM requirement_revision),(SELECT count(*) FROM business_request)")
        .fetch_one(pool).await.unwrap();
    assert_eq!(before, after);
    let state: (String, i64) = sqlx::query_as("SELECT state,version FROM requirement")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(state, ("Draft".into(), 5));
}
async fn invalid_contracts(app: &Router) {
    for key in [" ".to_owned(), "x".repeat(201)] {
        assert_eq!(
            request(
                app,
                "POST",
                "/api/requirements",
                json!({"request_id":key,"version":0,"contract":contract()})
            )
            .await
            .0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    let changes = [
        ("/title", json!(" ")),
        ("/description", json!("")),
        ("/acceptance_criteria", json!([])),
        ("/validation_plan", json!([])),
        ("/acceptance_criteria/0/description", json!("")),
        ("/acceptance_criteria/0/verification_ref", json!("missing")),
        ("/validation_plan/0/selector", json!("$(touch /tmp/bad)")),
        ("/validation_plan/0/check", json!("shell")),
        ("/validation_plan/0/expected_result", json!("")),
        ("/validation_plan/0/timeout_seconds", json!(0)),
        ("/network_access", json!(["bad host"])),
    ];
    for (i, (pointer, value)) in changes.into_iter().enumerate() {
        let mut c = contract();
        *c.pointer_mut(pointer).unwrap() = value;
        assert_eq!(
            request(
                app,
                "POST",
                "/api/requirements",
                json!({"request_id":format!("invalid-{i}"),"version":0,"contract":c})
            )
            .await
            .0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    let mut c = contract();
    let duplicate = c["validation_plan"][0].clone();
    c["validation_plan"].as_array_mut().unwrap().push(duplicate);
    assert_eq!(
        request(
            app,
            "POST",
            "/api/requirements",
            json!({"request_id":"duplicate-step","version":0,"contract":c})
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let mut c = contract();
    c["gate_recovery_policy"] = json!("unlimited");
    assert_eq!(
        request(
            app,
            "POST",
            "/api/requirements",
            json!({"request_id":"override","version":0,"contract":c})
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
}
async fn security_has_no_side_effects(app: &Router, pool: &PgPool, path: &str) {
    let original: String =
        sqlx::query_scalar("SELECT jsonb_agg(to_jsonb(r))::text FROM requirement r")
            .fetch_one(pool)
            .await
            .unwrap();
    let events: i64 = sqlx::query_scalar("SELECT count(*) FROM business_event")
        .fetch_one(pool)
        .await
        .unwrap();
    for (method, url, body) in [
        (
            "POST",
            "/api/requirements".to_string(),
            json!({"request_id":"attack-create","version":0,"contract":contract()}),
        ),
        (
            "PATCH",
            path.to_string(),
            json!({"request_id":"attack-edit","version":5,"contract":contract()}),
        ),
        (
            "POST",
            format!("{path}/ready"),
            json!({"request_id":"attack-ready","version":5,"repository_version":6}),
        ),
        (
            "POST",
            format!("{path}/withdraw"),
            json!({"request_id":"attack-withdraw","version":5,"repository_version":1}),
        ),
        ("PUT", "/api/repository".to_string(), repo(6, "attack-repo")),
    ] {
        // Exercise each boundary with an otherwise valid lifecycle state.
        let state = if url.ends_with("/withdraw") {
            "Ready"
        } else {
            "Draft"
        };
        sqlx::query("UPDATE requirement SET state=$1")
            .bind(state)
            .execute(pool)
            .await
            .unwrap();
        let before: String =
            sqlx::query_scalar("SELECT jsonb_agg(to_jsonb(r))::text FROM requirement r")
                .fetch_one(pool)
                .await
                .unwrap();
        for (host, origin, csrf) in [
            ("evil.test", Some("http://localhost:4200"), Some("1")),
            ("127.0.0.1:3081", Some("http://evil.test"), Some("1")),
            ("127.0.0.1:3081", None, Some("1")),
            ("127.0.0.1:3081", Some("http://localhost:4200"), None),
            ("127.0.0.1:3081", Some("http://localhost:4200"), Some("bad")),
        ] {
            let mut req = Request::builder()
                .method(method)
                .uri(&url)
                .header("host", host)
                .header("content-type", "application/json");
            if let Some(value) = origin {
                req = req.header("origin", value);
            }
            if let Some(value) = csrf {
                req = req.header("x-codexsymphony-csrf", value);
            }
            let response = app
                .clone()
                .oneshot(req.body(Body::from(body.to_string())).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
        }
        assert_eq!(
            before,
            sqlx::query_scalar::<_, String>(
                "SELECT jsonb_agg(to_jsonb(r))::text FROM requirement r"
            )
            .fetch_one(pool)
            .await
            .unwrap()
        );
    }
    sqlx::query("UPDATE requirement SET state='Submitted'")
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        original,
        sqlx::query_scalar::<_, String>("SELECT jsonb_agg(to_jsonb(r))::text FROM requirement r")
            .fetch_one(pool)
            .await
            .unwrap()
    );
    assert_eq!(
        events,
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM business_event")
            .fetch_one(pool)
            .await
            .unwrap()
    );
}

#[test]
fn policy_and_authorization_reject_invalid_or_expanded_inputs() {
    use codexsymphony_server::contract::{Contract, Repository, authorize, validate_repository};
    let valid: Repository = serde_json::from_value(repo(0, "test")["repository"].clone()).unwrap();
    let input: Contract = serde_json::from_value(contract()).unwrap();
    assert!(validate_repository(&valid).is_ok());
    let changes = [
        ("/project", json!("")),
        ("/reason", json!("")),
        ("/remote", json!("not/a/repository")),
        ("/github_repository_id", json!(0)),
        ("/base_branch", json!("bad branch")),
        ("/policy/allowed_checks", json!([])),
        ("/policy/allowed_checks", json!(["shell"])),
        ("/policy/max_timeout_seconds", json!(0)),
        ("/policy/token_limit", json!(0)),
        ("/policy/turn_limit", json!(0)),
        ("/policy/model_work_seconds", json!(0)),
        ("/policy/gate_recovery_policy", json!("unlimited")),
    ];
    for (pointer, value) in changes {
        let mut candidate = serde_json::to_value(&valid).unwrap();
        *candidate.pointer_mut(pointer).unwrap() = value;
        let candidate: Repository = serde_json::from_value(candidate).unwrap();
        assert!(validate_repository(&candidate).is_err(), "{pointer}");
    }
    let mut changed = valid.clone();
    changed.policy.allowed_checks = vec!["npm_test".into()];
    assert!(authorize(&input, &changed).is_err());
    changed = valid.clone();
    changed.policy.max_timeout_seconds = 1;
    assert!(authorize(&input, &changed).is_err());
    changed = valid.clone();
    changed.revoked = true;
    assert!(authorize(&input, &changed).is_err());
    assert!(authorize(&input, &valid).is_ok());
}
