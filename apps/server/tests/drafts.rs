use axum::{
    Router,
    body::{Body, to_bytes},
    http::Request,
};
use codexsymphony_server::{
    draft::{self, Source},
    process,
};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{
    io::{BufRead, BufReader},
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::Duration,
};
use tower::ServiceExt;

fn sample() -> Value {
    serde_json::from_str(include_str!("fixtures/draft-v1.json")).unwrap()
}
fn source(document: Value) -> Value {
    json!({"format":"json","label":"synthetic source; never authorization","text":document.to_string()})
}
fn body(document: Value, version: i64) -> Value {
    json!({"version":version,"source":source(document)})
}
fn app(pool: &PgPool) -> Router {
    codexsymphony_server::router(
        pool.clone(),
        codexsymphony_server::security::RequestPolicy::new(
            "127.0.0.1:3081".parse().unwrap(),
            "http://localhost:4200".into(),
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
#[test]
fn deterministic_parser_and_incomplete_data() {
    let markdown = Source {
        format: "markdown".into(),
        label: "sample".into(),
        text: include_str!("fixtures/draft-v1.md").into(),
    };
    assert_eq!(
        serde_json::to_value(draft::parse(&markdown).unwrap()).unwrap(),
        sample()
    );
    let mut reverse = sample();
    reverse["children"].as_array_mut().unwrap().reverse();
    let reversed: Source = serde_json::from_value(source(reverse.clone())).unwrap();
    assert_eq!(
        serde_json::to_value(draft::parse(&reversed).unwrap()).unwrap(),
        reverse
    );
    for input in [
        Source {
            format: "json".into(),
            label: "invalid syntax".into(),
            text: "{bad JSON".into(),
        },
        Source {
            format: "other".into(),
            ..markdown.clone()
        },
        Source {
            text: "arbitrary prose".into(),
            ..markdown.clone()
        },
        Source {
            label: "x".repeat(1025),
            ..markdown.clone()
        },
        Source {
            text: "x".repeat(262145),
            ..markdown
        },
    ] {
        assert!(draft::parse(&input).is_err());
    }
    let cases = [
        ("/schema", json!("unknown")),
        ("/parent/id", json!("bad ID")),
        ("/children/0/id", json!("")),
        ("/children/0/id", json!("P1")),
        ("/children/0/kind", json!("execute")),
        ("/children/0/parent_id", json!("missing")),
        ("/children/0/repository_id", json!(0)),
        ("/children/1/order", json!(1)),
        ("/children/1/depends_on", json!(["C1", "C1"])),
        ("/children/1/depends_on", json!(["missing"])),
        ("/children/0/depends_on", json!(["C1"])),
        ("/children/0/depends_on", json!(["C2"])),
    ];
    for (pointer, value) in cases {
        let mut doc = sample();
        *doc.pointer_mut(pointer).unwrap() = value;
        let input: Source = serde_json::from_value(source(doc)).unwrap();
        assert!(draft::parse(&input).is_err(), "{pointer}");
    }
    for path in [
        "/parent/acceptance_criteria",
        "/children/0/acceptance_criteria",
    ] {
        let mut doc = sample();
        let ac = doc.pointer(path).unwrap()[0].clone();
        *doc.pointer_mut(path).unwrap() = json!([ac, ac]);
        assert!(draft::parse(&serde_json::from_value(source(doc)).unwrap()).is_err());
    }
    let mut doc = sample();
    doc["children"] = json!(vec![doc["children"][0].clone(); 201]);
    assert!(draft::parse(&serde_json::from_value(source(doc)).unwrap()).is_err());
    let mut doc = sample();
    doc["parent"]["goal"] = json!("");
    doc["parent"]["scope"] = json!("");
    doc["parent"]["acceptance_criteria"] = json!([]);
    doc["children"][0]["repository_id"] = Value::Null;
    doc["children"][0]["goal"] = json!("");
    doc["children"][0]["validation_plan"] = json!("");
    doc["children"][0]["acceptance_criteria"][0]["description"] = json!("");
    let parsed = draft::parse(&serde_json::from_value(source(doc.clone())).unwrap()).unwrap();
    assert_eq!(draft::warnings(&parsed).len(), 7);
    assert_eq!(serde_json::to_value(parsed).unwrap(), doc);
    doc["children"] = json!([]);
    assert!(
        draft::warnings(&draft::parse(&serde_json::from_value(source(doc)).unwrap()).unwrap())
            .iter()
            .any(|x| x.starts_with("children:"))
    );
    assert!(
        draft::warnings(&draft::parse(&serde_json::from_value(source(sample())).unwrap()).unwrap())
            .is_empty()
    );
}

async fn fixture() -> (PgPool, String, PathBuf) {
    upgraded_fixture(16).await
}
async fn upgraded_fixture(last_version: u32) -> (PgPool, String, PathBuf) {
    let database = std::env::var("TEST_DATABASE_URL").unwrap();
    let admin = PgPoolOptions::new().connect(&database).await.unwrap();
    let schema = format!(
        "draft_{}",
        process::new_identity().unwrap().replace('-', "")
    );
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
    let separator = if database.contains('?') { '&' } else { '?' };
    let url = format!("{database}{separator}options=-csearch_path%3D{schema}");
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .unwrap();
    let root = std::env::temp_dir().join(&schema);
    std::fs::create_dir_all(&root).unwrap();
    let old = root.join("old-migrations");
    std::fs::create_dir(&old).unwrap();
    for entry in
        std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../../migrations")).unwrap()
    {
        let entry = entry.unwrap();
        let name = entry.file_name();
        let name = name.to_str().unwrap();
        if name.ends_with(".sql") && name[..4].parse::<u32>().unwrap() <= last_version {
            std::fs::copy(entry.path(), old.join(name)).unwrap();
        }
    }
    sqlx::migrate::Migrator::new(old.as_path())
        .await
        .unwrap()
        .run(&pool)
        .await
        .unwrap();
    sqlx::raw_sql("INSERT INTO repository(id,version,document) VALUES(1,1,'{}'); INSERT INTO requirement(version,state,contract) VALUES(3,'Draft','{\"title\":\"legacy\"}');").execute(&pool).await.unwrap();
    let before: Value = sqlx::query_scalar("SELECT to_jsonb(r) FROM requirement r")
        .fetch_one(&pool)
        .await
        .unwrap();
    let draft_before = if last_version == 17 {
        let document = sample();
        let input = source(document.clone());
        sqlx::query("INSERT INTO imported_draft(id,version,document,source,source_sha256) VALUES('draft-gh59-upgrade',2,$1,$2,'synthetic-source-hash')")
            .bind(&document).bind(&input).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO imported_draft_revision(draft_id,version,document,source,source_sha256) SELECT id,version,document,source,source_sha256 FROM imported_draft")
            .execute(&pool).await.unwrap();
        Some(snapshot(&pool).await)
    } else {
        None
    };
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
    if let Some(before) = draft_before {
        assert_eq!(
            before,
            snapshot(&pool).await,
            "GH59 draft, history, owner and budget must survive M1 upgrade"
        );
    }
    assert_eq!(
        before,
        sqlx::query_scalar::<_, Value>("SELECT to_jsonb(r) FROM requirement r")
            .fetch_one(&pool)
            .await
            .unwrap()
    );
    (pool, url, root)
}
async fn snapshot(pool: &PgPool) -> Value {
    sqlx::query_scalar("SELECT jsonb_build_object('drafts',(SELECT jsonb_agg(to_jsonb(d) ORDER BY id) FROM imported_draft d),'history',(SELECT jsonb_agg(to_jsonb(h) ORDER BY draft_id,version) FROM imported_draft_revision h),'runs',(SELECT count(*) FROM agent_run),'budget',(SELECT count(*) FROM requirement_budget),'owner',(SELECT to_jsonb(c) FROM execution_control c WHERE id=1))").fetch_one(pool).await.unwrap()
}
struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn start(url: &str, root: &std::path::Path) -> (Server, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_codexsymphony-server"))
        .env("DATABASE_URL", url)
        .env("BIND_ADDRESS", "127.0.0.1:0")
        .env("RUST_LOG", "info")
        .env("EXECUTION_DIRECTORY", root.join("execution"))
        .env_remove("RUNTIME_CONFIG")
        .env_remove("GITHUB_APP_CONFIG")
        .env_remove("STORAGE_CONFIG")
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some((_, address)) = line.split_once("API listening at http://") {
                let _ = send.send(address.to_owned());
                break;
            }
        }
    });
    let address = receive.recv_timeout(Duration::from_secs(15)).unwrap();
    (Server(child), format!("http://{}", address.trim()))
}
#[tokio::test]
async fn atomic_import_conflicts_legacy_guards_migration_and_real_restart() {
    let (pool, url, root) = fixture().await;
    let router = app(&pool);
    let baseline = snapshot(&pool).await;
    let imported = request(&router, "POST", "/api/drafts", body(sample(), 0), 200).await;
    assert_eq!(imported["document"], sample());
    assert_eq!(imported["state"], "Draft");
    assert_eq!(imported["version"], 1);
    let path = format!("/api/drafts/{}", imported["id"].as_str().unwrap());
    let source = json!({"format":"markdown","label":"markdown fixture","text":include_str!("fixtures/draft-v1.md")});
    let markdown = request(
        &router,
        "POST",
        "/api/drafts",
        json!({"version":0,"source":source}),
        200,
    )
    .await;
    assert_eq!(markdown["document"], imported["document"]);
    assert_eq!(markdown["source"], source);
    for (pointer, value) in [
        ("/children/0/kind", json!("bad")),
        ("/children/1/id", json!("C1")),
        ("/children/1/depends_on", json!(["unknown"])),
        ("/children/0/depends_on", json!(["C1"])),
        ("/children/0/depends_on", json!(["C2"])),
        ("/children/0/repository_id", json!(99999)),
    ] {
        let mut document = sample();
        *document.pointer_mut(pointer).unwrap() = value;
        let before = snapshot(&pool).await;
        for (method, path, version) in [("POST", "/api/drafts", 0), ("PUT", path.as_str(), 1)] {
            let rejected =
                request(&router, method, path, body(document.clone(), version), 422).await;
            assert!(rejected["error"].as_str().unwrap().len() > 5);
        }
        assert_eq!(before, snapshot(&pool).await);
    }
    let before = snapshot(&pool).await;
    request(&router, "POST", "/api/drafts", body(sample(), 2), 409).await;
    request(&router, "PUT", &path, body(sample(), 0), 409).await;
    request(
        &router,
        "PUT",
        "/api/drafts/draft-missing",
        body(sample(), 1),
        404,
    )
    .await;
    request(
        &router,
        "GET",
        "/api/drafts/draft-missing",
        Value::Null,
        404,
    )
    .await;
    request(
        &router,
        "POST",
        "/api/drafts",
        json!({"version":0,"unexpected":true}),
        422,
    )
    .await;
    assert_eq!(before, snapshot(&pool).await);
    sqlx::raw_sql("CREATE FUNCTION reject_draft_history() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'fixture history failure'; END $$; CREATE TRIGGER reject_history BEFORE INSERT ON imported_draft_revision FOR EACH ROW EXECUTE FUNCTION reject_draft_history();").execute(&pool).await.unwrap();
    let before_failure = snapshot(&pool).await;
    request(&router, "PUT", &path, body(sample(), 1), 503).await;
    request(&router, "POST", "/api/drafts", body(sample(), 0), 503).await;
    assert_eq!(before_failure, snapshot(&pool).await);
    sqlx::raw_sql("DROP TRIGGER reject_history ON imported_draft_revision; DROP FUNCTION reject_draft_history();").execute(&pool).await.unwrap();
    let mut changed = sample();
    changed["parent"]["goal"] = json!("Updated goal");
    changed["children"][0]["repository_id"] = Value::Null;
    let updated = request(&router, "PUT", &path, body(changed.clone(), 1), 200).await;
    assert_eq!(updated["version"], 2);
    assert_eq!(updated["document"], changed);
    assert_eq!(updated["warnings"].as_array().unwrap().len(), 1);
    request(&router, "PUT", &path, body(sample(), 1), 409).await;
    assert_eq!(
        request(&router, "GET", &path, Value::Null, 200).await,
        updated
    );
    assert_eq!(
        request(&router, "GET", "/api/drafts", Value::Null, 200).await["drafts"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    for identity in [
        imported["id"].as_str().unwrap().to_owned(),
        format!("{}:C1", imported["id"].as_str().unwrap()),
    ] {
        for prefix in ["/api", "/api/multi"] {
            request(
                &router,
                "POST",
                &format!("{prefix}/requirements/{identity}/ready"),
                json!({"request_id":"blocked","version":1,"repository_version":1}),
                409,
            )
            .await;
        }
        for action in ["pause", "cancel"] {
            request(
                &router,
                "POST",
                &format!("/api/requirements/{identity}/{action}"),
                json!({"pause":false}),
                409,
            )
            .await;
        }
    }
    let after = snapshot(&pool).await;
    for field in ["runs", "budget", "owner"] {
        assert_eq!(after[field], baseline[field]);
    }
    assert_eq!(after["history"].as_array().unwrap().len(), 3);
    drop(router);
    pool.close().await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let mut process_saved = Value::Null;
    for lifetime in 0..2 {
        let (server, address) = start(&url, &root);
        if lifetime == 0 {
            let response = client
                .post(format!("{address}/api/drafts"))
                .header("origin", &address)
                .header("x-codexsymphony-csrf", "1")
                .json(&body(sample(), 0))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 200);
            let created: Value = response.json().await.unwrap();
            let response = client
                .put(format!(
                    "{address}/api/drafts/{}",
                    created["id"].as_str().unwrap()
                ))
                .header("origin", &address)
                .header("x-codexsymphony-csrf", "1")
                .json(&body(changed.clone(), 1))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 200);
            process_saved = response.json().await.unwrap();
        } else {
            let restored: Value = client
                .get(format!(
                    "{address}/api/drafts/{}",
                    process_saved["id"].as_str().unwrap()
                ))
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            assert_eq!(restored, process_saved);
        }
        let response: Value = client
            .get(format!("{address}{path}"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(response, updated);
        drop(server);
    }
    let pool = PgPoolOptions::new().connect(&url).await.unwrap();
    let after_restart = snapshot(&pool).await;
    assert_eq!(after_restart["runs"], 0);
    assert_eq!(after_restart["budget"], 0);
    assert!(after_restart["owner"]["requirement_id"].is_null());
    pool.close().await;
    std::fs::remove_dir_all(root).unwrap();
    println!(
        "AC02/03/04/05: real PostgreSQL migration, atomic rejection, CAS, two real server lifetimes, no Run/budget/queue admission PASS"
    );
}

#[tokio::test]
async fn gh59_database_upgrades_without_rewriting_draft_history_or_execution_facts() {
    let (pool, url, root) = upgraded_fixture(17).await;
    let before = snapshot(&pool).await;
    let result = request(
        &app(&pool),
        "GET",
        "/api/drafts/draft-gh59-upgrade",
        Value::Null,
        200,
    )
    .await;
    assert_eq!(result["document"], sample());
    assert_eq!(result["version"], 2);
    pool.close().await;
    let reopened = PgPoolOptions::new().connect(&url).await.unwrap();
    sqlx::migrate!("../../migrations")
        .run(&reopened)
        .await
        .unwrap();
    assert_eq!(snapshot(&reopened).await, before);
    reopened.close().await;
    std::fs::remove_dir_all(root).unwrap();
}
