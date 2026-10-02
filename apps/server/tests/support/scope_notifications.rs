use super::{Value, database};
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

#[tokio::test]
async fn rapid_scoped_events_are_transactional_ordered_and_retry_bounded() {
    let owner = database().await;
    sqlx::raw_sql("INSERT INTO repository(id,version,document) VALUES(42,1,'{}');
        INSERT INTO plugin_scope(plugin_id,kind,repository_ids,enabled) VALUES('notification:rapid','repositories','{42}',true)")
        .execute(&owner).await.unwrap();
    let receiver = login(&owner, "rapid").await;
    let mut transaction = owner.begin().await.unwrap();
    sqlx::query(
        "INSERT INTO requirement(version,state,contract,repository_id) VALUES(1,'Draft','{}',42)",
    )
    .execute(&mut *transaction)
    .await
    .unwrap();
    transaction.rollback().await.unwrap();
    assert!(claim(&receiver, "rapid").await.is_none());
    let id: i64 = sqlx::query_scalar("INSERT INTO requirement(version,state,contract,repository_id) VALUES(1,'Draft','{}',42) RETURNING id")
        .fetch_one(&owner).await.unwrap();
    let mut transaction = owner.begin().await.unwrap();
    for state in ["Ready", "Running", "Blocked"] {
        sqlx::query("UPDATE requirement SET state=$1,version=version+1 WHERE id=$2")
            .bind(state)
            .bind(id)
            .execute(&mut *transaction)
            .await
            .unwrap();
    }
    transaction.commit().await.unwrap();
    let events = codexsymphony_server::lifecycle_api::events(&owner, id, 0)
        .await
        .unwrap();
    assert_eq!(events["events"].as_array().unwrap().len(), 4);
    let first = claim(&receiver, "rapid").await.unwrap();
    assert_eq!(first["sequence"], 1);
    // Keep an unacknowledged event blocking only its own later sequence.
    for attempt in 1..=3 {
        let current = if attempt == 1 {
            first.clone()
        } else {
            claim(&receiver, "rapid").await.unwrap()
        };
        assert_eq!(current["event_id"], first["event_id"]);
        assert_eq!(current["attempt"], attempt);
        assert!(ack(&receiver, "rapid", &current, "unknown").await);
        assert!(claim(&receiver, "rapid").await.is_none());
        sqlx::query("UPDATE notification_delivery SET next_attempt_at=clock_timestamp()-interval '1 second' WHERE plugin_id='rapid' AND event_id=$1")
            .bind(first["event_id"].as_i64().unwrap()).execute(&owner).await.unwrap();
    }
    for sequence in 2..=4 {
        let next = claim(&receiver, "rapid").await.unwrap();
        assert_eq!(next["sequence"], sequence);
        assert_eq!(next["attempt"], 1);
        assert!(ack(&receiver, "rapid", &next, "ignored").await);
    }
    assert!(claim(&receiver, "rapid").await.is_none());
    let exhausted: (String, i32, i32, String) = sqlx::query_as("SELECT state,attempts,attempt_limit,last_result FROM notification_delivery WHERE plugin_id='rapid' AND event_id=$1")
        .bind(first["event_id"].as_i64().unwrap()).fetch_one(&owner).await.unwrap();
    assert_eq!(exhausted, ("failed".into(), 3, 3, "retry_exhausted".into()));
    let unchanged: String = sqlx::query_scalar("SELECT state FROM requirement WHERE id=$1")
        .bind(id)
        .fetch_one(&owner)
        .await
        .unwrap();
    assert_eq!(unchanged, "Blocked");
    receiver.close().await;
    owner.close().await;
}

async fn login(owner: &PgPool, plugin: &str) -> PgPool {
    let schema: String = sqlx::query_scalar("SELECT current_schema()")
        .fetch_one(owner)
        .await
        .unwrap();
    let role = format!(
        "x03_{}",
        codexsymphony_server::process::new_identity()
            .unwrap()
            .replace('-', "")
    );
    sqlx::raw_sql(&format!("CREATE ROLE {role} LOGIN PASSWORD 'synthetic-notification-only'; GRANT USAGE ON SCHEMA {schema} TO {role}; GRANT EXECUTE ON FUNCTION notification_claim(text),notification_ack(text,bigint,integer,text) TO {role}"))
        .execute(owner).await.unwrap();
    sqlx::query("INSERT INTO notification_plugin VALUES($1,$2,true)")
        .bind(plugin)
        .bind(&role)
        .execute(owner)
        .await
        .unwrap();
    let options: PgConnectOptions = std::env::var("TEST_DATABASE_URL").unwrap().parse().unwrap();
    PgPoolOptions::new()
        .connect_with(
            options
                .username(&role)
                .password("synthetic-notification-only")
                .options([("search_path", schema)]),
        )
        .await
        .unwrap()
}

async fn claim(pool: &PgPool, plugin: &str) -> Option<Value> {
    sqlx::query_scalar("SELECT notification_claim($1)")
        .bind(plugin)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn ack(pool: &PgPool, plugin: &str, event: &Value, result: &str) -> bool {
    sqlx::query_scalar("SELECT notification_ack($1,$2,$3,$4)")
        .bind(plugin)
        .bind(event["event_id"].as_i64().unwrap())
        .bind(event["attempt"].as_i64().unwrap() as i32)
        .bind(result)
        .fetch_one(pool)
        .await
        .unwrap()
}

fn receive(directory: &Path, event: &Value) -> String {
    // Execute the product dispatcher with its actual bounded subprocess protocol.
    let program = r#"
import hashlib, importlib.util, json, pathlib, sys
source = pathlib.Path(sys.argv[1])
spec = importlib.util.spec_from_file_location('dispatcher', source)
dispatcher = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dispatcher)
receiver = pathlib.Path(sys.argv[2]).resolve()
executable = pathlib.Path(sys.executable).resolve()
cfg = {'argv': [str(executable), str(receiver), sys.argv[3]],
       'implementation': {str(p): hashlib.sha256(p.read_bytes()).hexdigest()
                          for p in [executable, receiver]}}
print(dispatcher.invoke(cfg, json.load(sys.stdin)))
"#;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut process = Command::new("python3")
        .args(["-B", "-c", program])
        .arg(root.join("apps/notifier/dispatch.py"))
        .arg(root.join("apps/server/tests/fixtures/notifications/durable.py"))
        .arg(directory.join("inbox.sqlite3"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    process
        .stdin
        .take()
        .unwrap()
        .write_all(&serde_json::to_vec(event).unwrap())
        .unwrap();
    let output = process.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

#[tokio::test]
async fn scoped_notification_response_loss_revocation_and_retries_preserve_business() {
    let owner = database().await;
    sqlx::raw_sql("INSERT INTO repository(id,version,document) VALUES(42,1,'{}'),(77,1,'{}');
        INSERT INTO plugin_scope(plugin_id,kind,repository_ids,enabled) VALUES('notification:A','repositories','{42}',true),('notification:B','repositories','{77}',true);")
        .execute(&owner).await.unwrap();
    let a = login(&owner, "A").await;
    let b = login(&owner, "B").await;
    sqlx::query("INSERT INTO requirement(version,state,contract,repository_id) VALUES(1,'Draft','{}',42),(1,'Draft','{}',42),(1,'Draft','{}',77)")
        .execute(&owner).await.unwrap();
    for pool in [&a, &b] {
        for sql in [
            "SELECT * FROM requirement",
            "UPDATE requirement SET state='Done'",
            "SELECT * FROM plugin_scope",
            "SELECT * FROM notification_delivery",
        ] {
            assert!(sqlx::query(sql).execute(pool).await.is_err());
        }
    }
    assert!(
        sqlx::query("SELECT notification_claim('B')")
            .execute(&a)
            .await
            .is_err()
    );
    // Multiple invocation identities for one repository retain the same authority.
    for run in ["retained-run", "successor-run"] {
        codexsymphony_server::plugin_scope::admit(&owner, "agent:codex", run, 1, 0)
            .await
            .unwrap();
    }
    let before: Value =
        sqlx::query_scalar("SELECT jsonb_agg(to_jsonb(r) ORDER BY id) FROM requirement r")
            .fetch_one(&owner)
            .await
            .unwrap();
    let directory = std::env::temp_dir().join(format!(
        "gh90-x03-{}",
        codexsymphony_server::process::new_identity().unwrap()
    ));
    fs::create_dir(&directory).unwrap();
    let first = claim(&a, "A").await.unwrap();
    assert_eq!(first["requirement_id"], 1);
    assert_eq!(first["repository_id"], 42);
    assert_eq!(receive(&directory, &first), "unknown");
    // Drop the response entirely: no ack, just as after a dispatcher restart.
    let second = claim(&a, "A").await.unwrap();
    let other = claim(&b, "B").await.unwrap();
    assert_eq!(second["requirement_id"], 2);
    assert_eq!(other["requirement_id"], 3);
    assert_eq!(other["repository_id"], 77);
    assert!(ack(&b, "B", &other, "ignored").await);
    assert!(claim(&a, "A").await.is_none());
    sqlx::query("UPDATE plugin_scope SET enabled=false WHERE plugin_id='notification:A'")
        .execute(&owner)
        .await
        .unwrap();
    sqlx::query("UPDATE notification_delivery SET next_attempt_at=clock_timestamp()-interval '1 second' WHERE plugin_id='A'")
        .execute(&owner).await.unwrap();
    assert!(claim(&a, "A").await.is_none());
    assert!(claim(&b, "B").await.is_none());
    // A real outcome already in flight is retained, even after revocation.
    assert!(ack(&a, "A", &second, "failed").await);
    let replay: bool = sqlx::query_scalar("SELECT notification_replay(2,$1,'A')")
        .bind(second["event_id"].as_i64().unwrap())
        .fetch_one(&owner)
        .await
        .unwrap();
    assert!(!replay);
    sqlx::query("UPDATE plugin_scope SET enabled=true WHERE plugin_id='notification:A'")
        .execute(&owner)
        .await
        .unwrap();
    let retry = claim(&a, "A").await.unwrap();
    assert_eq!(retry["event_id"], first["event_id"]);
    assert_eq!(retry["attempt"], 2);
    assert_eq!(retry["scope_version"], first["scope_version"]);
    assert!(!ack(&a, "A", &first, "accepted").await);
    assert_eq!(receive(&directory, &retry), "accepted");
    assert!(ack(&a, "A", &retry, "accepted").await);
    assert!(!ack(&a, "A", &retry, "accepted").await);
    assert!(claim(&a, "A").await.is_none());
    let inbox = Command::new("python3").args(["-B", "-c", "import sqlite3,sys; c=sqlite3.connect(sys.argv[1]); print(c.execute('SELECT count(*) FROM inbox').fetchone()[0])"])
        .arg(directory.join("inbox.sqlite3")).output().unwrap();
    assert!(inbox.status.success());
    assert_eq!(String::from_utf8(inbox.stdout).unwrap().trim(), "1");
    let after: Value =
        sqlx::query_scalar("SELECT jsonb_agg(to_jsonb(r) ORDER BY id) FROM requirement r")
            .fetch_one(&owner)
            .await
            .unwrap();
    assert_eq!(before, after);
    let attempts: Value = sqlx::query_scalar("SELECT jsonb_agg(to_jsonb(a) ORDER BY plugin_id,event_id,ordinal) FROM notification_attempt a")
        .fetch_one(&owner).await.unwrap();
    assert_eq!(attempts.as_array().unwrap().len(), 4);
    fs::write(directory.join("evidence.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "scope":"synthetic isolated database and real local subprocess; no actual channel delivery",
        "first":first,"retry":retry,"second_requirement":second,"other_repository":other,
        "attempts":attempts,"business_unchanged":before==after,"durable_receiver_rows":1
    })).unwrap()).unwrap();
    println!(
        "GH-90 X03 evidence: {}",
        directory.join("evidence.json").display()
    );
    a.close().await;
    b.close().await;
    owner.close().await;
}
