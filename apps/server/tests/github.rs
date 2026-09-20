//! Real HTTP fixtures: credentials stay on this side of the Agent boundary.
use axum::{
    Json, Router,
    extract::{Request, State},
    http::StatusCode,
    response::IntoResponse,
};
use codexsymphony_server::{
    execution::{Launch, RunKey},
    github::{self, *},
    github_http::AppClient,
    github_observe, github_service, github_store, run_store,
};
use serde_json::{Value, json};
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[derive(Default)]
struct Data {
    routes: HashMap<String, Value>,
    errors: HashMap<String, Vec<u16>>,
    redirects: HashMap<String, (u16, String)>,
    seen: Vec<String>,
    grants: usize,
}
struct Fixture {
    data: Arc<Mutex<Data>>,
    url: String,
    task: tokio::task::JoinHandle<()>,
    root: PathBuf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
async fn handler(
    State(data): State<Arc<Mutex<Data>>>,
    request: Request,
) -> axum::response::Response {
    let path = request.uri().path().to_owned();
    let query = request.uri().query().unwrap_or("").to_owned();
    let method = request.method().to_string();
    let body = axum::body::to_bytes(request.into_body(), 65536)
        .await
        .unwrap();
    let mut data = data.lock().unwrap();
    data.seen.push(format!("{method} {path}?{query}"));
    if let Some((status, location)) = data.redirects.get(&path) {
        return (
            StatusCode::from_u16(*status).unwrap(),
            [("location", location.clone())],
        )
            .into_response();
    }
    if let Some(errors) = data.errors.get_mut(&path)
        && !errors.is_empty()
    {
        return StatusCode::from_u16(errors.remove(0))
            .unwrap()
            .into_response();
    }
    if method == "POST" && path == "/app/installations/7/access_tokens" {
        assert_eq!(path, "/app/installations/7/access_tokens");
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            body["repository_ids"],
            json!([data.routes["/repos/owner/repo"]["id"]])
        );
        assert_eq!(
            body["permissions"],
            json!({"contents":"write","pull_requests":"write","checks":"read","actions":"read"})
        );
        data.grants += 1;
        return Json(data.routes[&path].clone()).into_response();
    }
    if let Some(value) = data.routes.get(&format!("{method} {path}")) {
        return Json(value.clone()).into_response();
    }
    assert_eq!(method, "GET", "preflight must not publish or rerun");
    let key = format!("{path}?{query}");
    if let Some(value) = data.routes.get(&key) {
        return Json(value.clone()).into_response();
    }
    if query.contains("page=") && !query.contains("page=1&") && !query.ends_with("page=1") {
        let value = data.routes.get(&path).unwrap_or(&Value::Null);
        let empty = match value.as_object() {
            Some(map) => json!({map.keys().next().unwrap():[]}),
            None => json!([]),
        };
        return Json(empty).into_response();
    }
    match data.routes.get(&path) {
        Some(value) => Json(value.clone()).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
impl Fixture {
    async fn new() -> Self {
        let root =
            std::env::temp_dir().join(codexsymphony_server::process::new_identity().unwrap());
        std::fs::create_dir(&root).unwrap();
        let key = root.join("fixture.pem");
        let output = std::process::Command::new("openssl")
            .args(["genrsa", "-out"])
            .arg(&key)
            .arg("2048")
            .output()
            .unwrap();
        assert!(output.status.success());
        let data = Arc::new(Mutex::new(Data::default()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let router = Router::new().fallback(handler).with_state(data.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let fixture = Self {
            data,
            url,
            task,
            root,
        };
        fixture.put(
            "/repos/owner/repo/installation",
            json!({"id":7,"app_id":42}),
        );
        fixture.grant(
            json!({"contents":"write","pull_requests":"write","checks":"read","actions":"read"}),
        );
        fixture.put("/repos/owner/repo", json!({"id":99,"full_name":"owner/repo","default_branch":"main","archived":false,"private":true}));
        fixture.put(
            "/repos/owner/repo/branches/%6D%61%69%6E",
            json!({"name":"main","protected":false}),
        );
        fixture.put("/repos/owner/repo/rules/branches/%6D%61%69%6E", json!([]));
        fixture.put("/repos/owner/repo/pulls/1", pr());
        fixture.put(
            "/repos/owner/repo/commits/abc/check-suites",
            json!({"check_suites":[{"id":9}]}),
        );
        fixture.put(
            "/repos/owner/repo/check-suites/9/check-runs",
            json!({"check_runs":[]}),
        );
        fixture.put(
            "/repos/owner/repo/commits/abc/statuses",
            json!([status(1, "success")]),
        );
        fixture
    }
    fn put(&self, path: &str, value: Value) {
        self.data.lock().unwrap().routes.insert(path.into(), value);
    }
    fn grant(&self, permissions: Value) {
        self.put("/app/installations/7/access_tokens", json!({"token":"synthetic-installation-fixture","expires_at":"2030-01-01T00:00:00Z","permissions":permissions}));
    }
    fn client(&self) -> AppClient {
        AppClient::new(
            &self.url,
            42,
            &std::fs::read(self.root.join("fixture.pem")).unwrap(),
        )
        .unwrap()
    }
    fn fail(&self, path: &str, errors: Vec<u16>) {
        self.data.lock().unwrap().errors.insert(path.into(), errors);
    }
    fn actions(&self) {
        self.put(
            "/repos/owner/repo/actions/workflows/8",
            json!({"id":8,"path":".github/workflows/ci.yml","state":"active"}),
        );
        self.put("/repos/owner/repo/contents/%2E%67%69%74%68%75%62%2F%77%6F%72%6B%66%6C%6F%77%73%2F%63%69%2E%79%6D%6C",json!({"sha":"pinned"}));
        self.put(
            "/repos/owner/repo/actions/runs",
            json!({"workflow_runs":[run()]}),
        );
        self.put(
            "/repos/owner/repo/actions/runs/10/attempts/2/jobs",
            json!({"jobs":[job()]}),
        );
        self.put(
            "/repos/owner/repo/check-suites/9/check-runs",
            json!({"check_runs":[check(1, "failure")]}),
        );
        self.put(
            "/repos/owner/repo/check-suites/9/check-runs?filter=all&per_page=100&page=2",
            json!({"check_runs":[check(2, "success")]}),
        );
    }
}
fn policy() -> Policy {
    Policy {
        repository_id: 99,
        repository: "owner/repo".into(),
        default_branch: "main".into(),
        version: 1,
        wait_seconds: 3600,
        required: vec![Selector {
            name: "ci".into(),
            source: Source::Status { creator_id: 5 },
        }],
    }
}
fn action_policy() -> Policy {
    let mut p = policy();
    p.required[0].source = Source::Actions {
        app_id: 42,
        workflow_id: 8,
        workflow_sha: "pinned".into(),
        event: "pull_request".into(),
        branch: "feature".into(),
        branch_from_pr: None,
    };
    p
}
fn pr() -> Value {
    json!({"number":1,"state":"open","merged":false,"merged_at":null,"merge_commit_sha":"test-merge","head":{"sha":"abc","ref":"feature"},"base":{"sha":"def","ref":"main","repo":{"id":99}}})
}
fn status(id: u64, state: &str) -> Value {
    json!({"id":id,"context":"ci","creator":{"id":5},"state":state})
}
fn check(id: u64, result: &str) -> Value {
    json!({"id":id,"name":"ci","head_sha":"abc","app":{"id":42},"check_suite":{"id":9},"status":"completed","conclusion":result})
}
fn run() -> Value {
    json!({"id":10,"head_sha":"abc","head_branch":"feature","workflow_id":8,"event":"pull_request","run_attempt":2,"run_number":4,"check_suite_id":9})
}
fn job() -> Value {
    json!({"id":20,"run_id":10,"run_attempt":2,"check_run_url":"https://api.github.com/repos/owner/repo/check-runs/2"})
}

#[tokio::test]
async fn generated_pr_branch_requires_explicit_same_repository_source_binding() {
    let f = Fixture::new().await;
    f.actions();
    let mut c = f.client();
    let mut p = action_policy();
    let now = github_service::now();
    let mut actual = pr();
    actual["head"]["ref"] = json!("ai/req-1-real-run");
    actual["head"]["repo"] = json!({"id":99});
    f.put("/repos/owner/repo/pulls/1", actual.clone());
    let mut workflow = run();
    workflow["head_branch"] = actual["head"]["ref"].clone();
    f.put(
        "/repos/owner/repo/actions/runs",
        json!({"workflow_runs":[workflow.clone()]}),
    );
    let fixed = github_observe::observe(&mut c, &p, 1, now).await.unwrap();
    assert_eq!(fixed.checks[0].state, CheckState::Missing);
    if let Source::Actions { branch_from_pr, .. } = &mut p.required[0].source {
        *branch_from_pr = Some(true);
    }
    let observed = github_observe::observe(&mut c, &p, 1, now).await.unwrap();
    assert_eq!(observed.checks[0].state, CheckState::Success);
    assert_eq!(observed.policy, p);
    assert_eq!(observed.checks[0].selector, p.required[0]);
    assert_eq!(
        observed.checks[0].evidence[0]["workflow_run"]["head_branch"],
        actual["head"]["ref"]
    );
    for (field, wrong) in [
        ("event", json!("push")),
        ("workflow_id", json!(77)),
        ("head_branch", json!("feature")),
    ] {
        let mut unrelated = workflow.clone();
        unrelated[field] = wrong;
        f.put(
            "/repos/owner/repo/actions/runs",
            json!({"workflow_runs":[unrelated]}),
        );
        assert_eq!(
            github_observe::observe(&mut c, &p, 1, now)
                .await
                .unwrap()
                .checks[0]
                .state,
            CheckState::Missing
        );
    }
    f.put(
        "/repos/owner/repo/actions/runs",
        json!({"workflow_runs":[workflow]}),
    );
    for head in [
        json!({"repo":{"id":100},"ref":"ai/req-1-real-run","sha":"abc"}),
        json!({"repo":{"id":99},"ref":"","sha":"abc"}),
        json!({"repo":{"id":99},"sha":"abc"}),
    ] {
        actual["head"] = head;
        f.put("/repos/owner/repo/pulls/1", actual.clone());
        assert!(github_observe::observe(&mut c, &p, 1, now).await.is_err());
    }
    let mut legacy = serde_json::to_value(action_policy().required[0].clone()).unwrap();
    legacy["source"]
        .as_object_mut()
        .unwrap()
        .remove("branch_from_pr");
    assert_eq!(
        serde_json::from_value::<Selector>(legacy).unwrap(),
        action_policy().required[0]
    );
}

#[tokio::test]
async fn http_token_scope_pagination_and_separate_sources() {
    let f = Fixture::new().await;
    let mut c = f.client();
    let p = policy();
    let now = github_service::now();
    let ready = github_observe::preflight(&mut c, &p, 1, now).await.unwrap();
    assert_eq!(ready.blockers.len(), 1);
    assert_eq!(f.data.lock().unwrap().grants, 1);
    github_observe::preflight(&mut c, &p, 1, now + 301)
        .await
        .unwrap();
    assert_eq!(f.data.lock().unwrap().grants, 2);
    f.fail("/repos/owner/repo", vec![401]);
    github_observe::preflight(&mut c, &p, 1, now + 302)
        .await
        .unwrap();
    assert_eq!(f.data.lock().unwrap().grants, 3);
    f.actions();
    let p = action_policy();
    let observed = github_observe::preflight(&mut c, &p, 1, now + 303)
        .await
        .unwrap();
    assert!(observed.blockers.is_empty());
    let obs: Observation = serde_json::from_value(observed.configuration["probe"].clone()).unwrap();
    assert_eq!(obs.checks[0].state, CheckState::Success);
    assert_eq!(obs.checks[0].evidence[0]["id"], 2);
    assert_eq!(obs.merge, MergeFact::Unmerged);
    assert_eq!(obs.merged_sha, None);
    assert!(
        f.data
            .lock()
            .unwrap()
            .seen
            .iter()
            .all(|s| s.starts_with("GET ")
                || s.starts_with("POST /app/installations/7/access_tokens"))
    );
    f.put("/repos/owner/repo/commits/abc/statuses", json!([]));
    let observed = github_observe::observe(&mut c, &p, 1, now).await.unwrap();
    assert_eq!(observed.checks[0].state, CheckState::Success);
    let observed = github_observe::observe(&mut c, &policy(), 1, now)
        .await
        .unwrap();
    assert_eq!(observed.checks[0].state, CheckState::Missing);
}
#[tokio::test]
async fn private_permissions_access_unknown_and_configuration_changes() {
    let f = Fixture::new().await;
    let p = action_policy();
    f.actions();
    let now = github_service::now();
    f.grant(json!({"contents":"read"}));
    let mut c = f.client();
    let cap = github_observe::preflight(&mut c, &p, 1, now).await.unwrap();
    assert_eq!(cap.blockers.len(), 4);
    f.put(
        "/repos/owner/repo/actions/workflows/8",
        json!({"path":".github/workflows/ci.yml","state":"disabled_manually"}),
    );
    f.put("/repos/owner/repo/rules/branches/%6D%61%69%6E",json!([{"type":"required_status_checks","parameters":{"required_status_checks":[{"context":"missing","integration_id":42}]}}]));
    let cap = github_observe::preflight(&mut c, &p, 1, now).await.unwrap();
    assert!(cap.blockers.iter().any(|s| s.contains("workflow")));
    assert!(cap.blockers.iter().any(|s| s.contains("unmapped")));
    for code in [403, 404] {
        f.fail("/repos/owner/repo/pulls/1", vec![code]);
        let error = github_observe::observe(&mut c, &p, 1, now)
            .await
            .unwrap_err();
        assert_eq!(error.status, Some(code));
        assert!(!error.to_string().contains("synthetic-installation"));
    }
    let mut closed = pr();
    closed["state"] = json!("closed");
    f.put("/repos/owner/repo/pulls/1", closed);
    let obs = github_observe::observe(&mut c, &p, 1, now).await.unwrap();
    assert_eq!(obs.merge, MergeFact::Unmerged);
    assert!(obs.closed);
    f.put(
        "/repos/owner/repo/installation",
        json!({"id":7,"app_id":43}),
    );
    assert!(
        github_observe::preflight(&mut f.client(), &p, 1, now)
            .await
            .is_err()
    );
    f.put(
        "/repos/owner/repo/installation",
        json!({"id":7,"app_id":42}),
    );
    f.put(
        "/app/installations/7/access_tokens",
        json!({"expires_at":"2020-01-01T00:00:00Z","token":"expired"}),
    );
    assert!(
        github_observe::preflight(&mut f.client(), &p, 1, now)
            .await
            .is_err()
    );
}
#[test]
fn fact_and_source_resolution() {
    assert_eq!(github::merge_fact(&json!({})), MergeFact::Unknown);
    assert_eq!(
        github::merge_fact(&json!({"merged":true})),
        MergeFact::Merged
    );
    assert_eq!(
        github::merge_fact(&json!({"merged_at":"2026-09-15T00:00:00Z"})),
        MergeFact::Merged
    );
    assert_eq!(
        (0..5).map(github::retry_delay).collect::<Vec<_>>(),
        vec![60, 120, 240, 300, 300]
    );
    let mut p = policy();
    assert!(github::validate_policy(&p));
    p.repository_id = 0;
    assert!(!github::validate_policy(&p));
    let selector = &policy().required[0];
    for (state, expected) in [
        ("success", CheckState::Success),
        ("pending", CheckState::Pending),
        ("failure", CheckState::Failure),
    ] {
        assert_eq!(
            github::resolve(
                selector,
                &[],
                &[status(1, "failure"), status(2, state)],
                &[],
                &[]
            )
            .state,
            expected
        );
    }
    let selector = Selector {
        name: "ci".into(),
        source: Source::CheckRun { app_id: 42 },
    };
    let mut spoof = check(3, "success");
    spoof["app"]["id"] = json!(100);
    assert_eq!(
        github::resolve(&selector, &[spoof], &[], &[], &[]).state,
        CheckState::Missing
    );
    assert_eq!(
        github::resolve(
            &selector,
            &[check(1, "failure"), check(2, "success")],
            &[],
            &[],
            &[]
        )
        .state,
        CheckState::Ambiguous
    );
    let mut pending = check(1, "success");
    pending["status"] = json!("in_progress");
    assert_eq!(
        github::resolve(&selector, &[pending], &[], &[], &[]).state,
        CheckState::Pending
    );
    assert_eq!(
        github::resolve(&selector, &[check(1, "success")], &[], &[], &[]).state,
        CheckState::Success
    );
    let selector = &action_policy().required[0];
    assert_eq!(
        github::resolve(
            selector,
            &[check(1, "failure"), check(2, "success")],
            &[],
            &[run()],
            &[job()]
        )
        .state,
        CheckState::Success
    );
    let mut old = job();
    old["run_attempt"] = json!(1);
    assert_eq!(
        github::resolve(selector, &[check(2, "success")], &[], &[run()], &[old]).state,
        CheckState::Missing
    );
}

async fn database() -> PgPool {
    let url = std::env::var("TEST_DATABASE_URL").expect("disposable database required");
    let schema = format!(
        "gh16_{}",
        codexsymphony_server::process::new_identity()
            .unwrap()
            .replace('-', "")
    );
    let admin = PgPoolOptions::new().connect(&url).await.unwrap();
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .unwrap();
    let options: PgConnectOptions = url.parse().unwrap();
    let pool = PgPoolOptions::new()
        .connect_with(options.options([("search_path", schema.as_str())]))
        .await
        .unwrap();
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
    sqlx::raw_sql("INSERT INTO repository(id,version,document) VALUES (1,1,'{\"revoked\":false,\"github_repository_id\":99,\"remote\":\"owner/repo\",\"base_branch\":\"main\"}'); INSERT INTO requirement(version,state,contract,revision) VALUES (1,'Ready','{}',1); INSERT INTO requirement_revision(requirement_id,revision,document) VALUES (1,1,'{\"repository_version\":1,\"repository\":{\"github_repository_id\":99}}'); UPDATE execution_control SET incarnation='current',recovery_complete=true;")
        .execute(&pool).await.unwrap();
    pool
}
async fn count(pool: &PgPool, query: &str) -> i64 {
    sqlx::query_scalar(query).fetch_one(pool).await.unwrap()
}
#[tokio::test]
async fn persistent_polling_blocks_claims_without_starting_a_model() {
    let pool = database().await;
    let f = Fixture::new().await;
    let mut c = f.client();
    let p = action_policy();
    f.actions();
    let now = github_service::now();
    let launch = Launch {
        key: RunKey {
            run_id: "run".into(),
            request_id: "request".into(),
            incarnation: "current".into(),
        },
        workspace: std::env::temp_dir().to_str().unwrap().into(),
        workspace_identity: "fixture".into(),
        program: "/should-never-start".into(),
        args: vec![],
    };
    assert!(!run_store::reserve_prepared(&pool, &launch).await.unwrap());
    assert_eq!(count(&pool, "SELECT count(*) FROM agent_run").await, 0);
    assert!(github_store::configure(&pool, &p, 1).await.unwrap());
    assert!(!github_store::configure(&pool, &p, 1).await.unwrap());
    assert!(github_store::link(&pool, 99, 1, 1).await.unwrap());
    assert!(!github_store::link(&pool, 99, 1, 1).await.unwrap());
    github_service::tick(&pool, &mut c, now).await.unwrap();
    assert_eq!(count(&pool,"SELECT count(*) FROM github_repository WHERE NOT stale AND capability->'blockers'='[]'::jsonb").await,1);
    let requests = f.data.lock().unwrap().seen.len();
    github_service::tick(&pool, &mut c, now + 59).await.unwrap();
    assert_eq!(requests, f.data.lock().unwrap().seen.len());
    for code in [403, 404] {
        f.fail("/repos/owner/repo/pulls/1", vec![code, code]);
        github_service::tick(&pool, &mut c, now + 60).await.unwrap();
        assert_eq!(
            count(
                &pool,
                "SELECT count(*) FROM github_pr WHERE stale AND observation->>'merge'='Unmerged'"
            )
            .await,
            1
        );
        assert!(!run_store::reserve_prepared(&pool, &launch).await.unwrap());
        assert_eq!(count(&pool, "SELECT count(*) FROM agent_run").await, 0);
        sqlx::raw_sql("UPDATE github_repository SET next_attempt_at=0; UPDATE github_pr SET next_attempt_at=0;").execute(&pool).await.unwrap();
    }
    assert_eq!(
        count(&pool, "SELECT next_attempt_at FROM github_pr").await,
        0
    );
    let mut closed = pr();
    closed["state"] = json!("closed");
    f.put("/repos/owner/repo/pulls/1", closed);
    github_service::tick(&pool, &mut c, now + 61).await.unwrap();
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM github_pr WHERE NOT stale AND observation->>'merge'='Unmerged'"
        )
        .await,
        1
    );
    // This test isolates repository authorization; preparation admission is
    // separately exercised against real probe results in preparation tests.
    sqlx::query("INSERT INTO preparation_record(run_id,requirement_id,revision,launch,retry,ready,checked_at) VALUES($1,1,1,$2,'{}',true,extract(epoch FROM now())::bigint)")
        .bind(&launch.key.run_id).bind(json!(launch)).execute(&pool).await.unwrap();
    assert!(run_store::reserve_prepared(&pool, &launch).await.unwrap());
    assert_eq!(
        count(&pool, "SELECT requirement_id FROM execution_control").await,
        1
    );
    let mut merged = pr();
    merged["merged"] = json!(true);
    merged["merged_at"] = json!("2026-09-15T00:00:00Z");
    merged["merge_commit_sha"] = json!("actual");
    f.put("/repos/owner/repo/pulls/1", merged);
    github_service::tick(&pool, &mut c, now + 122)
        .await
        .unwrap();
    assert_eq!(
        count(&pool, "SELECT requirement_id FROM execution_control").await,
        1
    );
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM requirement WHERE state='Running'"
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM agent_run WHERE state='Created'"
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM github_pr WHERE observation->>'merged_sha'='actual'"
        )
        .await,
        1
    );
    let mut altered = p.clone();
    altered.wait_seconds = 900;
    assert!(github_store::configure(&pool, &altered, 1).await.unwrap());
    let old = github_observe::preflight(&mut c, &p, 1, now).await.unwrap();
    github_store::save_capability(&pool, &old).await.unwrap();
    assert_eq!(
        count(&pool, "SELECT count(*) FROM github_repository WHERE stale").await,
        1
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn control_plane_configuration_and_readonly_cli() {
    let f = Fixture::new().await;
    f.actions();
    let config = json!({"app_id":42,"api_url":f.url,"private_key_path":f.root.join("fixture.pem"),"policy":action_policy(),"probe_pr":1});
    let file = f.root.join("config.json");
    std::fs::write(&file, serde_json::to_vec(&config).unwrap()).unwrap();
    let pool = database().await;
    let worker = github_service::start_path(&pool, &file).await.unwrap();
    // Capability is an intermediate result. Require complete PR sync cycles,
    // including a fresh poll after the worker's sleep, before stopping it.
    assert!(github_store::link(&pool, 99, 1, 1).await.unwrap());
    for _ in 0..2 {
        sqlx::query("UPDATE github_pr SET stale=true,next_attempt_at=0")
            .execute(&pool)
            .await
            .unwrap();
        for _ in 0..100 {
            if count(&pool, "SELECT count(*) FROM github_pr WHERE NOT stale").await == 1 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert_eq!(
            count(&pool, "SELECT count(*) FROM github_pr WHERE NOT stale AND observation->>'merge'='Unmerged'").await,
            1
        );
    }
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM github_repository WHERE NOT stale"
        )
        .await,
        1
    );
    worker.abort();
    assert!(worker.await.unwrap_err().is_cancelled());
    // The same App config installs both exact policies; no second credential path.
    let original: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    let mut second = action_policy();
    second.repository_id = 100;
    second.repository = "owner/second".into();
    sqlx::query("INSERT INTO repository(id,version,document) SELECT 2,version,jsonb_set(jsonb_set(document,'{github_repository_id}','100'),'{remote}','\"owner/second\"') FROM repository WHERE id=1").execute(&pool).await.unwrap();
    std::fs::write(
        &file,
        json!({"app":original,"repositories":[{"policy":second,"probe_pr":1}]}).to_string(),
    )
    .unwrap();
    let multi = github_service::start_path(&pool, &file).await.unwrap();
    multi.abort();
    assert!(multi.await.unwrap_err().is_cancelled());
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM github_repository WHERE repository_id IN (99,100)"
        )
        .await,
        2
    );
    std::fs::write(&file, original.to_string()).unwrap();
    let binary = env!("CARGO_BIN_EXE_codexsymphony-server");
    let out = std::process::Command::new(binary)
        .arg("--github-inspect")
        .arg(&file)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let cap: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(cap["blockers"], json!([]));
    assert!(!String::from_utf8_lossy(&out.stdout).contains("synthetic-installation"));
    assert!(AppClient::new("https://untrusted.invalid/", 42, b"invalid").is_err());
    assert!(AppClient::new("not a url", 42, b"invalid").is_err());
    assert!(AppClient::new("https://api.github.com/", 42, b"invalid").is_err());
}

#[tokio::test]
async fn identities_rules_and_incomplete_payloads_fail_closed() {
    let f = Fixture::new().await;
    let mut c = f.client();
    let now = github_service::now();
    let mut p = policy();
    f.put(
        "/repos/owner/repo/branches/%6D%61%69%6E",
        json!({"protection":{"required_status_checks":{"contexts":["other"]}}}),
    );
    let cap = github_observe::preflight(&mut c, &p, 1, now).await.unwrap();
    assert!(!cap.blockers.is_empty());
    f.put(
        "/repos/owner/repo/branches/%6D%61%69%6E",
        json!({"protection":{"required_status_checks":{"checks":[{"context":"ci","app_id":42}]}}}),
    );
    let cap = github_observe::preflight(&mut c, &p, 1, now).await.unwrap();
    assert!(!cap.blockers.is_empty());
    f.actions();
    p = action_policy();
    assert!(
        github_observe::preflight(&mut c, &p, 1, now)
            .await
            .unwrap()
            .blockers
            .is_empty()
    );
    f.put(
        "/repos/owner/repo/rules/branches/%6D%61%69%6E",
        json!([{"type":"other"},{"type":"required_status_checks"}]),
    );
    assert!(
        !github_observe::preflight(&mut c, &p, 1, now)
            .await
            .unwrap()
            .blockers
            .is_empty()
    );
    let mut other = run();
    other["id"] = json!(100);
    f.put(
        "/repos/owner/repo/actions/runs",
        json!({"workflow_runs":[run(),other]}),
    );
    f.put(
        "/repos/owner/repo/actions/runs/100/attempts/2/jobs",
        json!({"jobs":[]}),
    );
    assert_eq!(
        github_observe::observe(&mut c, &p, 1, now)
            .await
            .unwrap()
            .checks[0]
            .state,
        CheckState::Ambiguous
    );
    f.put(
        "/repos/owner/repo/actions/runs",
        json!({"workflow_runs":[]}),
    );
    assert_eq!(
        github_observe::observe(&mut c, &p, 1, now)
            .await
            .unwrap()
            .checks[0]
            .state,
        CheckState::Missing
    );
    f.put(
        "/repos/owner/repo/actions/runs",
        json!({"total_count":1000,"workflow_runs":[]}),
    );
    assert_eq!(
        github_observe::observe(&mut c, &p, 1, now)
            .await
            .unwrap_err()
            .code,
        "github_pagination_incomplete"
    );
    f.put(
        "/repos/owner/repo/commits/abc/statuses",
        json!({"invalid":[]}),
    );
    assert!(github_observe::observe(&mut c, &p, 1, now).await.is_err());
    f.put("/repos/owner/repo", json!({"id":100}));
    assert!(github_observe::preflight(&mut c, &p, 1, now).await.is_err());
    p.repository = "../bad".into();
    assert!(github_observe::preflight(&mut c, &p, 1, now).await.is_err());
    assert!(
        c.get(&policy(), "https://untrusted.invalid/", now)
            .await
            .is_err()
    );
    let missing = Selector {
        name: "ci".into(),
        source: Source::Status { creator_id: 5 },
    };
    let mut bad = status(0, "success");
    bad["id"] = Value::Null;
    assert_eq!(
        github::resolve(&missing, &[], &[bad], &[], &[]).state,
        CheckState::Ambiguous
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn configured_server_and_transport_failure() {
    use sqlx::ConnectOptions;
    let pool = database().await;
    let f = Fixture::new().await;
    f.actions();
    let mut client = f.client();
    let config = json!({"app_id":42,"api_url":f.url,"private_key_path":f.root.join("fixture.pem"),"policy":action_policy(),"probe_pr":1});
    let file = f.root.join("server-config.json");
    std::fs::write(&file, serde_json::to_vec(&config).unwrap()).unwrap();
    let schema: String = sqlx::query_scalar("SELECT current_schema()")
        .fetch_one(&pool)
        .await
        .unwrap();
    let mut url = pool.connect_options().to_url_lossy();
    url.query_pairs_mut()
        .append_pair("options", &format!("-c search_path={schema}"));
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_codexsymphony-server"))
        .env("DATABASE_URL", url.as_str())
        .env("GITHUB_APP_CONFIG", &file)
        .env("BIND_ADDRESS", "127.0.0.1:0")
        .env("RUST_LOG", "off")
        .env("EXECUTION_DIRECTORY", f.root.join("execution"))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    for _ in 0..100 {
        if count(
            &pool,
            "SELECT count(*) FROM github_repository WHERE NOT stale",
        )
        .await
            == 1
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    }
    let observed = count(
        &pool,
        "SELECT count(*) FROM github_repository WHERE NOT stale",
    )
    .await;
    std::process::Command::new("/bin/kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(child.wait().unwrap().success());
    assert_eq!(observed, 1);
    f.put(
        "/repos/owner/repo/actions/runs",
        json!({"workflow_runs":[]}),
    );
    let cap = github_observe::preflight(&mut client, &action_policy(), 1, github_service::now())
        .await
        .unwrap();
    assert!(!cap.blockers.is_empty());
    f.task.abort();
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    assert!(
        client
            .get(&policy(), "/repos/owner/repo", github_service::now())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn authenticated_http_never_follows_redirects() {
    let source = Fixture::new().await;
    let sink = Fixture::new().await;
    let mut client = source.client();
    let now = 1_800_000_000;
    // Acquire the token normally, then test authenticated repository requests.
    client.permissions(&policy(), now).await.unwrap();
    for status in [301, 302, 303, 307, 308] {
        for destination in [
            format!("{}destination", source.url),
            format!("{}destination", sink.url),
        ] {
            source.data.lock().unwrap().seen.clear();
            source
                .data
                .lock()
                .unwrap()
                .redirects
                .insert("/repos/owner/repo".into(), (status, destination));
            let error = client
                .get(&policy(), "/repos/owner/repo", now)
                .await
                .unwrap_err();
            assert_eq!(error.status, Some(status));
            assert_eq!(source.data.lock().unwrap().seen.len(), 1);
            assert!(sink.data.lock().unwrap().seen.is_empty());
        }
    }
}

#[path = "support/validation_runner.rs"]
mod delivery_source;
#[tokio::test]
async fn delivery_adapter_reconciles_exact_branch_and_rechecks_before_close() {
    use codexsymphony_server::{
        delivery_remote::Github, delivery_store::Pending, delivery_worker::Remote,
        git_broker::GitBroker, workspace::Workspace,
    };
    let fixture = Fixture::new().await;
    let (root, repo, _) = delivery_source::fixture();
    let baseline = codexsymphony_server::validation_runner::candidate(&repo)
        .unwrap()
        .sha;
    let bundle = root.join("seed.bundle");
    assert!(
        std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["bundle", "create"])
            .arg(&bundle)
            .arg("--all")
            .status()
            .unwrap()
            .success()
    );
    let broker = GitBroker::initialize(&root.join("workspaces"), &bundle).unwrap();
    let workspace = Workspace {
        key: RunKey {
            run_id: "delivery".into(),
            request_id: "request".into(),
            incarnation: "boot".into(),
        },
        identity: "owned".into(),
        requirement: 1,
        revision: 1,
        phase: "handoff".into(),
        baseline: baseline.clone(),
        branch: "ai/req-1-delivery".into(),
        path: broker
            .path("delivery")
            .unwrap()
            .to_string_lossy()
            .into_owned(),
    };
    broker.prepare(&workspace, true).unwrap();
    let manifest = broker.preserve(&workspace).unwrap();
    let mut job = Pending {
        action_key: "test".into(),
        kind: "publish".into(),
        state: "pending".into(),
        attempts: 0,
        requirement_id: 1,
        revision: 1,
        repository_id: 99,
        repository: "owner/repo".into(),
        branch: workspace.branch.clone(),
        base_branch: "main".into(),
        head_sha: baseline.clone(),
        manifest: json!(manifest),
        pr_number: None,
    };
    let mut client = fixture.client();
    let mut remote = Github {
        client: &mut client,
        policy: policy(),
        broker: &broker,
        now: 100,
    };
    fixture.put("/repos/owner/repo/pulls", json!([]));
    assert!(remote.find(&job).await.unwrap().is_none());
    assert!(remote.head(&job).await.unwrap().is_none());
    fixture.put(
        &format!("/repos/owner/repo/git/ref/heads/{}", job.branch),
        json!({"object":{"sha":baseline}}),
    );
    assert_eq!(remote.head(&job).await.unwrap(), Some(baseline.clone()));
    let mut value = pr();
    value["body"] = json!(job.identity().marker());
    value["head"] = json!({"repo":{"id":99},"ref":job.branch,"sha":baseline});
    value["base"]["repo"]["full_name"] = json!("owner/repo");
    fixture.put("POST /repos/owner/repo/pulls", value.clone());
    assert_eq!(remote.create(&job).await.unwrap(), value);
    fixture.put("/repos/owner/repo/pulls", json!([{"number":1}]));
    fixture.put("/repos/owner/repo/pulls/1", value.clone());
    assert_eq!(remote.find(&job).await.unwrap(), Some(value.clone()));
    job.pr_number = Some(2);
    assert!(remote.find(&job).await.is_err());
    job.pr_number = Some(1);
    fixture.put(
        "/repos/owner/repo/pulls",
        json!([{"number":1},{"number":2}]),
    );
    assert!(remote.find(&job).await.is_err());
    let mut closed = value.clone();
    closed["state"] = json!("closed");
    fixture.put("PATCH /repos/owner/repo/pulls/1", closed.clone());
    assert_eq!(remote.close(&job, 1).await.unwrap(), closed);
    value["merged"] = json!(true);
    fixture.put("/repos/owner/repo/pulls/1", value.clone());
    assert_eq!(remote.close(&job, 1).await.unwrap(), value);
    // A valid local candidate still cannot push when App authentication fails.
    fixture.fail("/repos/owner/repo/installation", vec![403]);
    remote.now = 10_000;
    assert!(remote.push(&job).await.is_err());
    let saved_manifest = job.manifest.clone();
    job.manifest = json!({});
    assert!(remote.create(&job).await.is_err());
    job.manifest = saved_manifest;
    // Revalidate the canonical candidate before every external write.
    std::fs::write(std::path::Path::new(&workspace.path).join("source"), "next").unwrap();
    broker.commit(&workspace, "advance candidate").unwrap();
    assert!(remote.create(&job).await.is_err());
    job.head_sha = "changed".into();
    assert!(remote.create(&job).await.is_err());
    assert!(remote.push(&job).await.is_err());
    job.repository_id = 100;
    assert!(remote.head(&job).await.is_err());
    let seen = fixture.data.lock().unwrap().seen.clone();
    assert!(
        seen.iter()
            .any(|s| s.contains("state=all") && s.contains("head=owner%3Aai%2Freq-1-delivery"))
    );
    assert_eq!(seen.iter().filter(|s| s.starts_with("PATCH")).count(), 1);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn delivery_basic_header_uses_rfc4648_padding() {
    for (input, expected) in [
        ("", ""),
        ("f", "Zg=="),
        ("fo", "Zm8="),
        ("foo", "Zm9v"),
        ("foobar", "Zm9vYmFy"),
    ] {
        assert_eq!(
            codexsymphony_server::github_http::encode_basic(input),
            expected
        );
    }
}

#[path = "support/delivery.rs"]
mod delivery_database;
#[tokio::test]
async fn configured_delivery_service_and_real_fast_forward_git() {
    use codexsymphony_server::{
        delivery_control, delivery_store, git_broker::GitBroker, workspace::Workspace,
    };
    let pool = delivery_database::database().await;
    let fixture = Fixture::new().await;
    fixture.put("/repos/owner/repo", json!({"id":7}));
    let (root, repo, _) = delivery_source::fixture();
    let baseline = codexsymphony_server::validation_runner::candidate(&repo)
        .unwrap()
        .sha;
    let bundle = root.join("seed.bundle");
    git_fixture(
        &repo,
        &["bundle", "create", bundle.to_str().unwrap(), "--all"],
    );
    let broker = GitBroker::initialize(&root.join("workspaces"), &bundle).unwrap();
    let workspace = Workspace {
        key: RunKey {
            run_id: "run".into(),
            request_id: "request".into(),
            incarnation: "boot".into(),
        },
        identity: "owned".into(),
        requirement: 1,
        revision: 1,
        phase: "handoff".into(),
        baseline: baseline.clone(),
        branch: "ai/req-1-run".into(),
        path: broker.path("run").unwrap().to_string_lossy().into_owned(),
    };
    broker.prepare(&workspace, true).unwrap();
    let manifest = broker.preserve(&workspace).unwrap();
    let mut policy = policy();
    policy.repository_id = 7;
    sqlx::raw_sql("DELETE FROM delivery_action; DELETE FROM delivery;")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE candidate_validation SET candidate_sha=$1")
        .bind(&baseline)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE workspace_snapshot SET manifest=$1")
        .bind(json!(manifest))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE github_repository SET policy=$1,capability=jsonb_build_object('blockers','[]'::jsonb,'policy',$1::jsonb)").bind(json!(policy)).execute(&pool).await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    delivery_store::enqueue(&mut tx, "validation")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let job = delivery_store::due(&pool, 0).await.unwrap().remove(0);
    let mut pr = json!({"number":12,"body":job.identity().marker(),"base":{"repo":{"id":7,"full_name":"owner/repo"},"ref":"main"},"head":{"repo":{"id":7},"ref":workspace.branch,"sha":baseline},"merged":false,"merged_at":null,"state":"open"});
    fixture.put("/repos/owner/repo/pulls", json!([]));
    fixture.put(
        &format!("/repos/owner/repo/git/ref/heads/{}", workspace.branch),
        json!({"object":{"sha":baseline}}),
    );
    fixture.put("POST /repos/owner/repo/pulls", pr.clone());
    let mut client = fixture.client();
    github_service::deliver(&pool, &mut client, &root, 0)
        .await
        .unwrap();
    fixture.put("/repos/owner/repo/pulls", json!([{"number":12}]));
    fixture.put("/repos/owner/repo/pulls/12", pr.clone());
    github_service::deliver(&pool, &mut client, &root, 60)
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT state FROM requirement")
            .fetch_one(&pool)
            .await
            .unwrap(),
        "Submitted"
    );
    delivery_control::cancel(&pool, 1).await.unwrap();
    pr["state"] = json!("closed");
    fixture.put("PATCH /repos/owner/repo/pulls/12", pr.clone());
    github_service::deliver(&pool, &mut client, &root, 120)
        .await
        .unwrap();
    fixture.put("/repos/owner/repo/pulls/12", pr);
    github_service::deliver(&pool, &mut client, &root, 180)
        .await
        .unwrap();
    github_service::deliver(&pool, &mut client, &root, 240)
        .await
        .unwrap();
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT requirement_id IS NULL FROM execution_control")
            .fetch_one(&pool)
            .await
            .unwrap()
    );
    // Real local Git verifies that an accepted push cannot overwrite divergence.
    let destination = root.join("remote.git");
    git_fixture(&root, &["init", "--bare", destination.to_str().unwrap()]);
    let command = || {
        let mut command = tokio::process::Command::new("git");
        command
            .arg("--git-dir")
            .arg(root.join("workspaces/canonical.git"))
            .args(["push", "--no-verify", "--"])
            .arg(&destination);
        command
    };
    let mut first = command();
    first.arg(format!("{baseline}:refs/heads/check"));
    codexsymphony_server::github_http::push_git(first, &baseline)
        .await
        .unwrap();
    std::fs::write(
        std::path::Path::new(&workspace.path).join("source"),
        "changed",
    )
    .unwrap();
    let next = broker.commit(&workspace, "next").unwrap();
    let mut advance = command();
    advance.arg(format!("{next}:refs/heads/check"));
    codexsymphony_server::github_http::push_git(advance, &next)
        .await
        .unwrap();
    let mut divergent = command();
    divergent.arg(format!("{baseline}:refs/heads/check"));
    assert!(
        codexsymphony_server::github_http::push_git(divergent, &baseline)
            .await
            .is_err()
    );
    assert_eq!(
        git_fixture(&destination, &["rev-parse", "refs/heads/check"]),
        next
    );
    std::fs::remove_dir_all(root).unwrap();
}
fn git_fixture(root: &std::path::Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().into()
}

#[tokio::test]
async fn app_push_reports_local_git_failures_without_remote_writes_or_token_disclosure() {
    let fixture = Fixture::new().await;
    let mut client = fixture.client();
    let missing = fixture.root.join("missing.git");
    // Git rejects this absent local directory before contacting any remote.
    let error = client
        .push(&policy(), &missing, "abc", "ai/test", 100)
        .await
        .unwrap_err();
    assert_eq!(error.code, "github_transient_or_unknown");
    assert!(!error.to_string().contains("token"));
    let error =
        codexsymphony_server::github_http::push_git(tokio::process::Command::new(missing), "abc")
            .await
            .unwrap_err();
    assert_eq!(error.code, "github_identity_conflict");
    assert_eq!(fixture.data.lock().unwrap().grants, 1);
}

#[test]
fn git_push_inherits_only_service_proxy_routing() {
    use std::ffi::OsString;
    let mut command = tokio::process::Command::new("git");
    command.env_clear();
    let input = [
        ("HTTPS_PROXY", "http://127.0.0.1:3128"),
        ("no_proxy", "localhost,127.0.0.1"),
        ("GIT_CONFIG_GLOBAL", "/untrusted/config"),
        ("GITHUB_TOKEN", "synthetic-not-a-token"),
        ("DATABASE_URL", "synthetic-database"),
    ];
    codexsymphony_server::github_http::configure_proxy(
        &mut command,
        input
            .into_iter()
            .map(|(key, value)| (OsString::from(key), OsString::from(value))),
    );
    let actual: Vec<_> = command.as_std().get_envs().collect();
    assert_eq!(actual.len(), 2);
    assert_eq!(
        actual[0],
        (
            std::ffi::OsStr::new("HTTPS_PROXY"),
            Some(std::ffi::OsStr::new(input[0].1))
        )
    );
    assert_eq!(
        actual[1],
        (
            std::ffi::OsStr::new("no_proxy"),
            Some(std::ffi::OsStr::new(input[1].1))
        )
    );
}

#[tokio::test]
async fn private_actions_do_not_require_unselected_legacy_status_permission() {
    let f = Fixture::new().await;
    f.actions();
    f.fail("/repos/owner/repo/commits/abc/statuses", vec![403]);
    let now = github_service::now();
    let mut client = f.client();
    let capability = github_observe::preflight(&mut client, &action_policy(), 1, now)
        .await
        .unwrap();
    assert!(capability.blockers.is_empty());
    assert!(
        !f.data
            .lock()
            .unwrap()
            .seen
            .iter()
            .any(|request| request.contains("/statuses"))
    );
    // An explicitly selected Status source still fails closed on denied access.
    let error = github_observe::observe(&mut client, &policy(), 1, now)
        .await
        .unwrap_err();
    assert_eq!(error.status, Some(403));
}
