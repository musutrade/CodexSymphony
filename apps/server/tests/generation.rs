#[path = "support/auth.rs"]
mod auth_client;
use codexsymphony_server::{
    budget::Usage,
    generation::{self, Request},
    generation_store as store, process,
};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
fn request(id: &str) -> Request {
    Request {
        request_id: id.into(),
        draft_id: None,
        version: 0,
        label: "test input".into(),
        text: format!("synthetic {id}"),
    }
}
fn sample() -> String {
    include_str!("fixtures/draft-v1.json").into()
}
async fn pool() -> PgPool {
    let url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL required");
    let admin = PgPoolOptions::new().connect(&url).await.unwrap();
    let schema = format!(
        "generation_{}",
        process::new_identity().unwrap().replace('-', "")
    );
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
    let separator = if url.contains('?') { '&' } else { '?' };
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&format!("{url}{separator}options=-csearch_path%3D{schema}"))
        .await
        .unwrap();
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
    sqlx::query("INSERT INTO repository(id,version,document) VALUES(1,1,'{}')")
        .execute(&pool)
        .await
        .unwrap();
    pool
}
#[test]
fn request_validation_prompt_data_and_usage() {
    let valid = request("valid");
    assert!(valid.validate().is_ok());
    for bad in [
        Request {
            request_id: "../escape".into(),
            ..valid.clone()
        },
        Request {
            request_id: "".into(),
            ..valid.clone()
        },
        Request {
            version: -1,
            ..valid.clone()
        },
        Request {
            version: 1,
            ..valid.clone()
        },
        Request {
            label: " ".into(),
            ..valid.clone()
        },
        Request {
            text: "".into(),
            ..valid.clone()
        },
        Request {
            text: "a".repeat(16001),
            ..valid.clone()
        },
        Request {
            label: "a".repeat(513),
            ..valid.clone()
        },
    ] {
        assert!(bad.validate().is_err());
    }
    let prompt = generation::prompt(&valid, &json!([]), &Value::Null);
    assert!(prompt.contains("untrusted requirement data"));
    assert!(prompt.contains("synthetic valid"));
    let usage = codexsymphony_server::generation_runtime::usage(
        &json!({"params":{"tokenUsage":{"total":{"inputTokens":5,"cachedInputTokens":2,"outputTokens":3}}}}),
        4,
    );
    assert!(usage.complete);
    assert!(usage.valid());
    assert_eq!(usage.input, Some(5));
    assert!(!codexsymphony_server::generation_runtime::usage(&json!({}), 0).complete);
    assert!(generation::document("valid", sample()).is_ok());
    assert!(generation::document("invalid", "{bad".into()).is_err());
    assert_eq!(store::error(9999, "x").0.as_u16(), 500);
}
#[tokio::test]
async fn durable_admission_atomic_commit_edit_conflict_and_recovery() {
    let pool = pool().await;
    let before:Value=sqlx::query_scalar("SELECT jsonb_build_object('owner',(SELECT to_jsonb(c) FROM execution_control c),'runs',(SELECT count(*) FROM agent_run),'budgets',(SELECT count(*) FROM requirement_budget))").fetch_one(&pool).await.unwrap();
    let input = request("first");
    let (a, b) = tokio::join!(store::admit(&pool, &input), store::admit(&pool, &input));
    let a = a.unwrap();
    let b = b.unwrap();
    assert_ne!(a.1, b.1);
    assert_eq!(a.0["id"], b.0["id"]);
    let same = Request {
        request_id: "different-id".into(),
        ..input.clone()
    };
    assert!(!store::admit(&pool, &same).await.unwrap().1);
    assert_eq!(
        store::admit(
            &pool,
            &Request {
                text: "different".into(),
                ..input.clone()
            }
        )
        .await
        .unwrap_err()
        .0
        .as_u16(),
        409
    );
    assert_eq!(
        store::admit(&pool, &request("busy"))
            .await
            .unwrap_err()
            .0
            .as_u16(),
        409
    );
    let usage = Usage {
        input: Some(20),
        cached: Some(5),
        output: Some(30),
        model_seconds: Some(1),
        complete: true,
    };
    store::progress(
        &pool,
        "first",
        &usage,
        &json!({"thread_id":"actual-fixture-thread"}),
    )
    .await
    .unwrap();
    store::complete(&pool, "first", sample()).await.unwrap();
    let completed = store::read(&pool, "first").await.unwrap();
    assert_eq!(completed["status"], "succeeded");
    assert_eq!(completed["output_version"], 1);
    assert_eq!(completed["usage"]["input"], 20);
    let draft: Value =
        sqlx::query_scalar("SELECT document FROM imported_draft WHERE id='draft-first'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(draft["children"][0]["id"], "C1");
    let edit = Request {
        draft_id: Some("draft-first".into()),
        version: 1,
        ..request("edit")
    };
    store::admit(&pool, &edit).await.unwrap();
    sqlx::query("UPDATE imported_draft SET version=2, document=jsonb_set(document,'{parent,goal}','\"user edit\"') WHERE id='draft-first'").execute(&pool).await.unwrap();
    assert_eq!(
        store::complete(&pool, "edit", sample())
            .await
            .unwrap_err()
            .0
            .as_u16(),
        409
    );
    store::fail(
        &pool,
        "edit",
        "conflict",
        "version conflict",
        Some(&sample()),
    )
    .await
    .unwrap();
    let preserved: String = sqlx::query_scalar(
        "SELECT document->'parent'->>'goal' FROM imported_draft WHERE id='draft-first'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(preserved, "user edit");
    assert_eq!(
        store::admit(
            &pool,
            &Request {
                request_id: "stale".into(),
                text: "stale".into(),
                ..edit
            }
        )
        .await
        .unwrap_err()
        .0
        .as_u16(),
        409
    );
    store::admit(&pool, &request("restart")).await.unwrap();
    sqlx::query(r#"UPDATE draft_generation SET usage='{"input":12,"cached":0,"output":3,"model_seconds":1,"complete":true}' WHERE id='restart'"#)
        .execute(&pool).await.unwrap();
    store::recover(&pool).await.unwrap();
    assert_eq!(
        store::read(&pool, "restart").await.unwrap()["status"],
        "interrupted"
    );
    let interrupted = store::read(&pool, "restart").await.unwrap();
    assert_eq!(interrupted["usage"]["complete"], false);
    assert_eq!(interrupted["usage"]["input"], 12);
    assert!(!store::admit(&pool, &request("restart")).await.unwrap().1);
    assert!(store::read(&pool, "missing").await.is_err());
    assert_eq!(
        store::list(&pool).await.unwrap()["generations"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    let after:Value=sqlx::query_scalar("SELECT jsonb_build_object('owner',(SELECT to_jsonb(c) FROM execution_control c),'runs',(SELECT count(*) FROM agent_run),'budgets',(SELECT count(*) FROM requirement_budget))").fetch_one(&pool).await.unwrap();
    assert_eq!(before, after);
}
#[tokio::test]
async fn invalid_output_never_partially_writes_a_draft() {
    let pool = pool().await;
    let base: Value = serde_json::from_str(&sample()).unwrap();
    let mut cases = vec!["{malformed".to_string()];
    for (field, value) in [
        ("kind", json!("execute")),
        ("repository_id", json!(99999)),
        ("id", json!("P1")),
        ("depends_on", json!(["missing"])),
    ] {
        let mut document = base.clone();
        document["children"][0][field] = value;
        cases.push(document.to_string());
    }
    let mut cycle = base.clone();
    cycle["children"][0]["depends_on"] = json!(["C2"]);
    cases.push(cycle.to_string());
    let mut missing = base.clone();
    missing["parent"].as_object_mut().unwrap().remove("goal");
    cases.push(missing.to_string());
    let mut unknown = base.clone();
    unknown["execute"] = json!(true);
    cases.push(unknown.to_string());
    for (i, output) in cases.into_iter().enumerate() {
        let id = format!("invalid-{i}");
        store::admit(&pool, &request(&id)).await.unwrap();
        let failure = store::complete(&pool, &id, output.clone())
            .await
            .unwrap_err();
        assert_eq!(failure.0.as_u16(), 422);
        store::fail(
            &pool,
            &id,
            "failed",
            failure.1.0["error"].as_str().unwrap(),
            Some(&output),
        )
        .await
        .unwrap();
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM imported_draft")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM imported_draft_revision")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn pinned_runtime_success_invalid_output_token_limit_and_timeout() {
    use codexsymphony_server::generation_runtime::{self, Config};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let pool = pool().await;
    for (index, mode) in [
        "ok",
        "malformed",
        "token-limit",
        "timeout",
        "failure",
        "missing-usage",
    ]
    .iter()
    .enumerate()
    {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let calls = Arc::new(AtomicUsize::new(0));
        let captured = calls.clone();
        let mode = *mode;
        let app=axum::Router::new().route("/responses",axum::routing::post(move || {let captured=captured.clone();async move {
   captured.fetch_add(1,Ordering::SeqCst);
   if mode=="timeout" {tokio::time::sleep(std::time::Duration::from_secs(5)).await;}
   let text=if mode=="malformed" {"bad".to_string()}else{sample()};
   let tokens=if mode=="token-limit" {30001}else{25};
   let usage=if mode=="missing-usage" {Value::Null}else{json!({"input_tokens":tokens,"output_tokens":10,"total_tokens":tokens+10})};
   let events=if mode=="failure" {vec![json!({"type":"error","code":"fixture_failure","message":"scripted provider failure"})]}else{vec![json!({"type":"response.created","response":{"id":"fixture-generation"}}),json!({"type":"response.output_item.done","item":{"type":"message","id":"fixture-message","role":"assistant","content":[{"type":"output_text","text":text}]}}),json!({"type":"response.completed","response":{"id":"fixture-generation","usage":usage}})]};
   ([("content-type","text/event-stream")],events.iter().map(|v|format!("data: {v}\n\n")).collect::<String>())
  }}));
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let home = std::env::temp_dir().join(format!(
            "generation-runtime-{}",
            process::new_identity().unwrap()
        ));
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join("config.toml"),format!("model = \"gpt-6-astra\"\nmodel_provider = \"fixture\"\n[model_providers.fixture]\nname = \"Scripted test fixture\"\nbase_url = \"http://127.0.0.1:{port}\"\nwire_api = \"responses\"\nrequires_openai_auth = false\nsupports_websockets = false\nrequest_max_retries = 0\nstream_max_retries = 0\n")).unwrap();
        let request = request(&format!("runtime-{index}"));
        store::admit(&pool, &request).await.unwrap();
        generation_runtime::run(
            pool.clone(),
            Config {
                codex_home: home,
                model: "gpt-6-astra".into(),
                timeout_seconds: if mode == "timeout" { 1 } else { 10 },
            },
            request.clone(),
        )
        .await;
        let record = store::read(&pool, &request.request_id).await.unwrap();
        assert_eq!(
            record["status"],
            if mode == "ok" { "succeeded" } else { "failed" },
            "{mode}: {record}"
        );
        assert!(calls.load(Ordering::SeqCst) <= 1, "provider must not retry");
        if mode == "ok" {
            assert_eq!(record["usage"]["input"], 25);
            assert!(
                record["evidence"]["runtime_user_agent"]
                    .as_str()
                    .unwrap()
                    .contains("0.156.1")
            );
        }
        server.abort();
    }
}

#[tokio::test]
async fn http_records_replay_validation_and_unconfigured_generation() {
    use axum::{
        body::{Body, to_bytes},
        http::Request as HttpRequest,
    };
    use tower::ServiceExt;
    let pool = pool().await;
    let input = request("api-replay");
    store::admit(&pool, &input).await.unwrap();
    store::fail(
        &pool,
        &input.request_id,
        "failed",
        "scripted persisted failure",
        None,
    )
    .await
    .unwrap();
    let app = auth_client::router(
        pool.clone(),
        codexsymphony_server::security::RequestPolicy::new(
            "127.0.0.1:3081".parse().unwrap(),
            "https://localhost:4200".into(),
        )
        .unwrap(),
    );
    for (method, path, body, status) in [
        ("GET", "/api/draft-generations", json!({}), 200),
        ("GET", "/api/draft-generations/api-replay", json!({}), 200),
        ("GET", "/api/draft-generations/missing", json!({}), 404),
        ("POST", "/api/draft-generations", json!(input), 200),
        (
            "POST",
            "/api/draft-generations",
            json!(Request {
                text: "changed".into(),
                ..input.clone()
            }),
            409,
        ),
        (
            "POST",
            "/api/draft-generations",
            json!({"unknown":true}),
            422,
        ),
        (
            "POST",
            "/api/draft-generations",
            json!(Request {
                text: "".into(),
                ..input.clone()
            }),
            422,
        ),
        (
            "POST",
            "/api/draft-generations",
            json!(request("unconfigured")),
            503,
        ),
    ] {
        let response = app
            .clone()
            .oneshot(
                HttpRequest::builder()
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
        assert_eq!(response.status().as_u16(), status);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert!(serde_json::from_slice::<Value>(&bytes).is_ok());
    }
    pool.close().await;
    assert_eq!(
        store::read(&pool, "api-replay")
            .await
            .unwrap_err()
            .0
            .as_u16(),
        503
    );
}

#[test]
fn operator_configuration_is_explicit_and_bounded() {
    use codexsymphony_server::generation_runtime::Config;
    let directory = std::env::temp_dir().join(format!(
        "generation-config-{}",
        process::new_identity().unwrap()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("settings.json");
    assert!(Config::read(&path).is_err());
    std::fs::write(&path, "{invalid").unwrap();
    assert!(Config::read(&path).is_err());
    for value in [
        json!({"codex_home":"relative","model":"model","timeout_seconds":120}),
        json!({"codex_home":"/tmp","model":"","timeout_seconds":120}),
        json!({"codex_home":"/tmp","model":"model","timeout_seconds":121}),
    ] {
        std::fs::write(&path, value.to_string()).unwrap();
        assert!(Config::read(&path).is_err());
    }
    std::fs::write(
        &path,
        json!({"codex_home":"/tmp","model":"model","timeout_seconds":1}).to_string(),
    )
    .unwrap();
    assert_eq!(Config::read(&path).unwrap().timeout_seconds, 1);
}

#[test]
fn parent_identity_binding_runs_in_an_isolated_child() {
    use codexsymphony_server::generation_runtime::parent_guard;
    if let Ok(parent) = std::env::var("GH61_PARENT_TEST") {
        assert!(parent_guard(0)().is_err());
        parent_guard(parent.parse().unwrap())().unwrap();
        return;
    }
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "parent_identity_binding_runs_in_an_isolated_child",
            "--nocapture",
        ])
        .env("GH61_PARENT_TEST", std::process::id().to_string())
        .status()
        .unwrap();
    assert!(result.success());
}

struct TestApi {
    database: String,
    client: tokio::sync::OnceCell<reqwest::Client>,
    child: std::process::Child,
    url: String,
}
impl Drop for TestApi {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl TestApi {
    fn start(database: &str, root: &std::path::Path, config: &std::path::Path) -> Self {
        use std::io::BufRead;
        let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_codexsymphony-server"))
            .env("DATABASE_URL", database)
            .env("BIND_ADDRESS", "127.0.0.1:0")
            .env("WEB_ORIGIN", "https://localhost:4200")
            .env("AUTH_CONFIG", server_auth::config(root))
            .env("WEB_ORIGIN", "https://localhost:4200")
            .env("EXECUTION_DIRECTORY", root.join("execution"))
            .env("DRAFT_GENERATION_CONFIG", config)
            .env("RUST_LOG", "info")
            .env_remove("RUNTIME_CONFIG")
            .env_remove("GITHUB_APP_CONFIG")
            .env_remove("STORAGE_CONFIG")
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let reader = std::io::BufReader::new(child.stdout.take().unwrap());
        for line in reader.lines() {
            let line = line.unwrap();
            if let Some((_, address)) = line.split_once("API listening at http://") {
                return Self {
                    database: database.into(),
                    client: tokio::sync::OnceCell::new(),
                    child,
                    url: format!("http://{}", address.trim()),
                };
            }
        }
        let _ = child.kill();
        let _ = child.wait();
        panic!("API did not start");
    }
    fn stop(&mut self) {
        assert!(
            std::process::Command::new("kill")
                .args(["-INT", &self.child.id().to_string()])
                .status()
                .unwrap()
                .success()
        );
        assert!(self.child.wait().unwrap().success());
    }
    async fn call(&self, method: &str, path: &str, body: Value, status: u16) -> Value {
        let client = self
            .client
            .get_or_init(|| server_auth::client(&self.url, &self.database))
            .await;
        let response = client
            .request(method.parse().unwrap(), format!("{}{path}", self.url))
            .json(&body)
            .send()
            .await
            .unwrap();
        let actual = response.status().as_u16();
        let result = response.json::<Value>().await.unwrap();
        assert_eq!(actual, status, "{result}");
        result
    }
    async fn finished(&self, id: &str) -> Value {
        for _ in 0..200 {
            let record = self
                .call(
                    "GET",
                    &format!("/api/draft-generations/{id}"),
                    json!({}),
                    200,
                )
                .await;
            if record["status"] != "running" {
                return record;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!("generation did not finish");
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_http_generation_edit_race_process_crash_and_failed_recovery() {
    use codexsymphony_server::generation_runtime::{self, Config};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let pool = pool().await;
    let schema: String = sqlx::query_scalar("SELECT current_schema()")
        .fetch_one(&pool)
        .await
        .unwrap();
    let base = std::env::var("TEST_DATABASE_URL").unwrap();
    let separator = if base.contains('?') { '&' } else { '?' };
    let database = format!("{base}{separator}options=-csearch_path%3D{schema}");
    let root = std::env::temp_dir().join(format!(
        "generation-http-{}",
        process::new_identity().unwrap()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let resume = Arc::new(tokio::sync::Notify::new());
    let gate = resume.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let provider=axum::Router::new().route("/responses",axum::routing::post(move ||{let count=count.clone();let gate=gate.clone();async move{
  let ordinal=count.fetch_add(1,Ordering::SeqCst);if ordinal>0{gate.notified().await;}
  let events=[json!({"type":"response.created","response":{"id":"http-fixture"}}),json!({"type":"response.output_item.done","item":{"type":"message","id":"http-message","role":"assistant","content":[{"type":"output_text","text":sample()}]}}),json!({"type":"response.completed","response":{"id":"http-fixture","usage":{"input_tokens":10,"output_tokens":20,"total_tokens":30}}})];
  ([("content-type","text/event-stream")],events.iter().map(|v|format!("data: {v}\n\n")).collect::<String>())
 }}));
    let provider = tokio::spawn(async move { axum::serve(listener, provider).await.unwrap() });
    let home = root.join("home");
    std::fs::create_dir(&home).unwrap();
    std::fs::write(home.join("config.toml"),format!("model_provider=\"fixture\"\n[model_providers.fixture]\nname=\"Explicit HTTP test fixture\"\nbase_url=\"http://127.0.0.1:{port}\"\nwire_api=\"responses\"\nrequires_openai_auth=false\nsupports_websockets=false\nrequest_max_retries=0\nstream_max_retries=0\n")).unwrap();
    let config = Config {
        codex_home: home,
        model: "gpt-6-astra".into(),
        timeout_seconds: 30,
    };
    let path = root.join("config.json");
    std::fs::write(&path, json!(config).to_string()).unwrap();
    let mut api = TestApi::start(&database, &root, &path);
    api.call("GET", "/api/draft-generations", json!({}), 200)
        .await;
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    std::fs::write(&path, "{broken").unwrap();
    api.call(
        "POST",
        "/api/draft-generations",
        json!(request("bad-config")),
        503,
    )
    .await;
    std::fs::write(&path, json!(config).to_string()).unwrap();
    api.call(
        "POST",
        "/api/draft-generations",
        json!(request("http-first")),
        200,
    )
    .await;
    assert_eq!(api.finished("http-first").await["status"], "succeeded");
    let mut draft = api
        .call("GET", "/api/drafts/draft-http-first", json!({}), 200)
        .await;
    let edit = Request {
        draft_id: Some("draft-http-first".into()),
        version: 1,
        ..request("http-edit")
    };
    api.call("POST", "/api/draft-generations", json!(edit), 200)
        .await;
    while calls.load(Ordering::SeqCst) < 2 {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    api.call(
        "POST",
        "/api/draft-generations",
        json!(request("other-running")),
        409,
    )
    .await;
    draft["document"]["parent"]["goal"] = json!("concurrent user edit");
    let edited=api.call("PUT","/api/drafts/draft-http-first",json!({"version":1,"source":{"format":"json","label":"user edit","text":draft["document"].to_string()}}),200).await;
    resume.notify_one();
    assert_eq!(api.finished("http-edit").await["status"], "conflict");
    assert_eq!(
        api.call("GET", "/api/drafts/draft-http-first", json!({}), 200)
            .await,
        edited
    );
    api.stop();
    api = TestApi::start(&database, &root, &path);
    api.call(
        "POST",
        "/api/draft-generations",
        json!(request("http-crash")),
        200,
    )
    .await;
    while calls.load(Ordering::SeqCst) < 3 {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let children = std::fs::read_dir(format!("/proc/{}/task", api.child.id()))
        .unwrap()
        .map(|entry| {
            std::fs::read_to_string(entry.unwrap().path().join("children")).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ");
    assert!(!children.trim().is_empty());
    api.child.kill().unwrap();
    api.child.wait().unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    for child in children.split_whitespace() {
        if let Ok(stat) = std::fs::read_to_string(format!("/proc/{child}/stat")) {
            assert!(
                stat.split(')').nth(1).unwrap().trim().starts_with('Z'),
                "old model process remains active"
            );
        }
    }
    api = TestApi::start(&database, &root, &path);
    assert_eq!(api.finished("http-crash").await["status"], "interrupted");
    api.call(
        "POST",
        "/api/draft-generations",
        json!(request("http-crash")),
        200,
    )
    .await;
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    api.stop();
    // A database failure retains the intent; restart recovery does not retry it.
    let lost = request("lost-database");
    store::admit(&pool, &lost).await.unwrap();
    assert!(
        store::admit(
            &pool,
            &Request {
                text: "".into(),
                ..request("invalid")
            }
        )
        .await
        .is_err()
    );
    pool.close().await;
    generation_runtime::run(pool, config, lost).await;
    let reopened = PgPoolOptions::new().connect(&database).await.unwrap();
    assert_eq!(
        store::read(&reopened, "lost-database").await.unwrap()["status"],
        "running"
    );
    store::recover(&reopened).await.unwrap();
    sqlx::query("DROP TABLE draft_generation")
        .execute(&reopened)
        .await
        .unwrap();
    let startup = std::process::Command::new(env!("CARGO_BIN_EXE_codexsymphony-server"))
        .env("DATABASE_URL", database)
        .env("BIND_ADDRESS", "127.0.0.1:0")
        .env("WEB_ORIGIN", "https://localhost:4200")
        .env("AUTH_CONFIG", server_auth::config(&root))
        .output()
        .unwrap();
    assert!(!startup.status.success());
    assert!(String::from_utf8_lossy(&startup.stderr).contains("generation recovery failed"));
    provider.abort();
}

#[path = "support/server_auth.rs"]
mod server_auth;
