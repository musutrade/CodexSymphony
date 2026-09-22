//! Runtime persistence, questions and full client against a deterministic stdio fixture.
use codexsymphony_server::{
    budget::Amount,
    execution::{Launch, RunKey},
    git_broker::GitBroker,
    process, runtime, runtime_client, runtime_questions as questions, runtime_store as store,
    runtime_tools,
};
use serde_json::{Value, json};
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

// PostgreSQL advisory locks are database-wide, even across fixture schemas.
// Keep independent Runtime scenarios apart; each scenario retains its own races.
static DATABASE_SCENARIO: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn key() -> RunKey {
    RunKey {
        run_id: "runtime-test".into(),
        request_id: "request".into(),
        incarnation: "boot".into(),
    }
}
fn temporary() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "runtime-fixture-{}",
        process::new_identity().unwrap()
    ));
    std::fs::create_dir(&path).unwrap();
    path
}
async fn fixture(root: &Path) -> PgPool {
    let options: PgConnectOptions = std::env::var("TEST_DATABASE_URL").unwrap().parse().unwrap();
    let admin = PgPoolOptions::new()
        .connect_with(options.clone())
        .await
        .unwrap();
    let schema = format!(
        "runtime_{}",
        process::new_identity().unwrap().replace('-', "")
    );
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect_with(options.options([("search_path", schema)]))
        .await
        .unwrap();
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
    sqlx::raw_sql(r#"TRUNCATE requirement,repository,business_request RESTART IDENTITY CASCADE;
      INSERT INTO repository(id,version,document) VALUES(1,1,'{"revoked":false}');
      INSERT INTO requirement(version,state,contract,revision) VALUES(1,'Running','{}',1);
      INSERT INTO requirement_revision VALUES(1,1,'{"repository_version":1,"repository":{"model":"gpt-6-astra"},"contract":{"network_access":[]}}');
      INSERT INTO requirement_budget(requirement_id,limits) VALUES(1,'{"tokens":10000,"turns":10,"model_seconds":1000}');
      INSERT INTO budget_authorization(requirement_id,version,request_id,actor,reason,delta,limits) SELECT 1,1,'initial','local-user','review',limits,limits FROM requirement_budget;
      INSERT INTO execution_control(id,requirement_id,incarnation,recovery_complete) VALUES(1,1,'boot',true) ON CONFLICT(id) DO UPDATE SET requirement_id=1,incarnation='boot',recovery_complete=true,paused=false;
      UPDATE storage_guard SET blocked=false,error=NULL;"#).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO agent_run(id,requirement_id,revision,incarnation,request_id,workspace,workspace_identity,launch,state,model) VALUES('runtime-test',1,1,'boot','request',$1,'fixture','{}','Created','gpt-6-astra')").bind(root.to_str().unwrap()).execute(&pool).await.unwrap();
    pool
}
async fn session(pool: &PgPool) {
    store::open(pool, &key(), 100).await.unwrap();
    store::thread(pool, &key(), "thread", 100).await.unwrap();
    store::turn(pool, &key(), "turn", "call", 100)
        .await
        .unwrap();
}
fn tool(id: i64, kind: &str, args: Value) -> Value {
    json!({"id":id,"method":"item/tool/call","params":{"threadId":"thread","turnId":"turn","callId":format!("call-{id}"),"tool":kind,"arguments":args}})
}
fn question() -> Value {
    json!({"id":"question-rpc","method":"item/tool/requestUserInput","params":{"threadId":"thread","turnId":"turn","itemId":"item","isBlocking":true,"questions":[{"id":"choice","question":"Which option?"}]}})
}
fn broker(root: &Path) -> GitBroker {
    std::fs::create_dir_all(root.join("canonical.git")).unwrap();
    std::fs::write(
        root.join("canonical.git/config"),
        "[core]\n\trepositoryformatversion = 0\n\tfilemode = true\n\tbare = true\n",
    )
    .unwrap();
    GitBroker::open(root).unwrap()
}
#[tokio::test]
async fn tool_dispatch_errors_are_sanitized_durable_and_replayable() {
    let _scenario = DATABASE_SCENARIO.lock().await;
    let root = temporary();
    let pool = fixture(&root).await;
    session(&pool).await;
    let git = broker(&root.join("broker"));
    let expected = runtime::reply(
        false,
        "Tool rejected: invalid arguments, authorization, candidate or unresolved operation.",
    );
    for request in [
        tool(
            41,
            "unknown-private-tool",
            json!({"secret":"fixture-secret"}),
        ),
        tool(42, "create_local_commit", json!({"message":""})),
    ] {
        let result = runtime_tools::handle(&pool, &git, &key(), &request)
            .await
            .unwrap();
        assert_eq!(result, expected);
        assert!(!result.to_string().contains("fixture-secret"));
        let saved: Value =
            sqlx::query_scalar("SELECT result FROM runtime_request WHERE run_id=$1 AND rpc_id=$2")
                .bind(&key().run_id)
                .bind(&request["id"])
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(saved, result);
        assert_eq!(
            runtime_tools::handle(&pool, &git, &key(), &request)
                .await
                .unwrap(),
            result
        );
    }
    pool.close().await;
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn persistence_and_full_client_acceptance() {
    let _scenario = DATABASE_SCENARIO.lock().await;
    let root = temporary();
    let pool = fixture(&root).await;
    session(&pool).await;
    let git = broker(&root.join("broker"));
    let q = questions::ask(&pool, &key(), &question(), 100)
        .await
        .unwrap();
    assert_eq!(
        questions::ask(&pool, &key(), &question(), 101)
            .await
            .unwrap()
            .id,
        q.id
    );
    let app = auth_client::router(
        pool.clone(),
        codexsymphony_server::security::RequestPolicy::new(
            "127.0.0.1:3081".parse().unwrap(),
            "https://localhost:4200".into(),
        )
        .unwrap(),
    );
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .header("host", "127.0.0.1:3081")
                .header("origin", "https://localhost:4200")
                .header("x-codexsymphony-csrf", "1")
                .uri("/api/requirements/1/questions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&body).contains(&q.id));
    for (id, body, status) in [
        (q.id.as_str(), json!({"unexpected":true}), 422),
        (q.id.as_str(), json!({"version":0,"answer":{}}), 409),
        ("absent", json!({"version":1,"answer":{}}), 404),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .header("host", "127.0.0.1:3081")
                    .header("origin", "https://localhost:4200")
                    .header("x-codexsymphony-csrf", "1")
                    .method("POST")
                    .uri(format!("/api/questions/{id}/answer"))
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), status);
    }
    assert!(
        questions::ask(&pool, &key(), &json!({"id":"bad","params":{}}), 101)
            .await
            .is_err()
    );
    let mut malformed = question();
    malformed["params"]["questions"][0]["question"] = json!(12);
    assert!(
        questions::ask(&pool, &key(), &malformed, 101)
            .await
            .is_err()
    );
    let mut forged = key();
    forged.request_id = "forged".into();
    assert!(
        questions::ask(&pool, &forged, &question(), 101)
            .await
            .is_err()
    );
    let answer = questions::Answer {
        version: q.version,
        answer: json!({"answers":{"choice":{"answers":["first"]}}}),
    };
    assert!(
        questions::answer(
            &pool,
            &q.id,
            &questions::Answer {
                version: 0,
                answer: answer.answer.clone()
            },
            101
        )
        .await
        .is_err()
    );
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .header("host", "127.0.0.1:3081")
                .header("origin", "https://localhost:4200")
                .header("x-codexsymphony-csrf", "1")
                .method("POST")
                .uri(format!("/api/questions/{}/answer", q.id))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"version":answer.version,"answer":answer.answer}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert!(questions::answer(&pool, &q.id, &answer, 103).await.is_err());
    sqlx::query("UPDATE execution_control SET paused=true")
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        questions::live_answers(&pool, &key(), 103)
            .await
            .unwrap()
            .is_empty()
    );
    let mut child = std::process::Command::new("/bin/cat")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut transport =
        codexsymphony_server::runtime_transport::Transport::new(&mut child).unwrap();
    assert!(
        runtime_client::deliver_answer(&pool, &key(), &mut transport, &q)
            .await
            .is_err()
    );
    child.kill().unwrap();
    child.wait().unwrap();
    drop(transport);
    sqlx::query("UPDATE execution_control SET paused=false")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        questions::live_answers(&pool, &key(), 103)
            .await
            .unwrap()
            .len(),
        1
    );
    questions::delivered(&pool, &q.id).await.unwrap();
    assert!(
        questions::live_answers(&pool, &key(), 104)
            .await
            .unwrap()
            .is_empty()
    );
    let blocked = tool(
        2,
        "report_blocker",
        json!({"reason":"fixture","requires_permission":true}),
    );
    let result = runtime_tools::handle(&pool, &git, &key(), &blocked)
        .await
        .unwrap();
    assert_eq!(result["success"], true);
    assert_eq!(
        runtime_tools::handle(&pool, &git, &key(), &blocked)
            .await
            .unwrap(),
        result
    );
    assert!(!store::can_continue(&pool, &key(), 105).await.unwrap());
    let mut changed = blocked.clone();
    changed["params"]["arguments"]["reason"] = json!("different");
    assert!(
        runtime_tools::handle(&pool, &git, &key(), &changed)
            .await
            .is_err()
    );
    assert!(
        runtime_tools::handle(
            &pool,
            &git,
            &key(),
            &tool(
                3,
                "report_completion",
                json!({"candidate_sha":"abc","summary":"late"})
            )
        )
        .await
        .is_err()
    );
    store::evidence(&pool, &key(), "stderr", &vec![b'x'; runtime::MAX_FRAME])
        .await
        .unwrap();
    let truncated: bool = sqlx::query_scalar(
        "SELECT truncated FROM runtime_evidence WHERE run_id='runtime-test' AND channel='stderr'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(truncated);
    pool.close().await;
    let response = app
        .oneshot(
            Request::builder()
                .header("host", "127.0.0.1:3081")
                .header("origin", "https://localhost:4200")
                .header("x-codexsymphony-csrf", "1")
                .uri("/api/requirements/1/questions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 503);

    let pool = fixture(&root).await;
    session(&pool).await;
    let q = questions::ask(&pool, &key(), &question(), 100)
        .await
        .unwrap();
    questions::expire(&pool, 7300).await.unwrap();
    let saved = questions::answer(&pool, &q.id, &answer, 86400)
        .await
        .unwrap();
    assert_eq!(saved.resume_state, "pending");
    assert!(
        questions::live_answers(&pool, &key(), 86400)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(!store::can_continue(&pool, &key(), 86400).await.unwrap());
    resume_answer(&pool, &root, &q.id).await;
    pool.close().await;
    ending_race(&root, &git).await;
    full_client(&root, &git).await;
    automatic_answer_recovery(&root, false, false).await;
    failed_protocol_sessions(&git).await;
    std::fs::remove_dir_all(root).unwrap();
}
async fn resume_answer(pool: &PgPool, root: &Path, question: &str) {
    let launch = Launch {
        key: RunKey {
            run_id: "resumed".into(),
            request_id: "resume-request".into(),
            incarnation: "boot".into(),
        },
        workspace: root.to_str().unwrap().into(),
        workspace_identity: "new-worktree".into(),
        program: "/bin/true".into(),
        args: vec![],
    };
    assert!(
        !questions::reserve_resume(pool, "runtime-test", &launch)
            .await
            .unwrap()
    );
    sqlx::raw_sql("UPDATE agent_run SET state='Interrupted',quiescent=true; INSERT INTO workspace_snapshot VALUES('runtime-test','{}',false);").execute(pool).await.unwrap();
    assert!(
        !questions::reserve_resume(pool, "runtime-test", &launch)
            .await
            .unwrap()
    );
    sqlx::query("INSERT INTO preparation_record(run_id,requirement_id,revision,launch,retry,ready,checked_at) VALUES('resumed',1,1,$1,'{}',true,extract(epoch FROM now())::bigint)")
        .bind(json!(launch)).execute(pool).await.unwrap();
    sqlx::query("UPDATE execution_control SET paused=true")
        .execute(pool)
        .await
        .unwrap();
    assert!(
        !questions::reserve_resume(pool, "runtime-test", &launch)
            .await
            .unwrap()
    );
    sqlx::query("UPDATE execution_control SET paused=false")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("UPDATE requirement_budget SET exhausted=true")
        .execute(pool)
        .await
        .unwrap();
    assert!(
        !questions::reserve_resume(pool, "runtime-test", &launch)
            .await
            .unwrap()
    );
    sqlx::query("UPDATE requirement_budget SET exhausted=false")
        .execute(pool)
        .await
        .unwrap();
    assert!(
        questions::reserve_resume(pool, "runtime-test", &launch)
            .await
            .unwrap()
    );
    assert!(
        !questions::reserve_resume(pool, "runtime-test", &launch)
            .await
            .unwrap()
    );
    let input = store::input(pool, &launch.key).await.unwrap();
    assert!(input.contains(question));
    assert!(input.contains("first"));
    // A later pause/resume must retain business answers without rewriting the
    // historical link to the first Run that consumed them.
    sqlx::query("UPDATE runtime_question SET resumed_run=run_id WHERE id=$1")
        .bind(question)
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(input, store::input(pool, &launch.key).await.unwrap());
    sqlx::query("UPDATE runtime_question SET resume_state='invalid' WHERE id=$1")
        .bind(question)
        .execute(pool)
        .await
        .unwrap();
    assert!(
        !store::input(pool, &launch.key)
            .await
            .unwrap()
            .contains(question)
    );
    sqlx::query("UPDATE runtime_question SET resume_state='linked' WHERE id=$1")
        .bind(question)
        .execute(pool)
        .await
        .unwrap();
    let state: String = sqlx::query_scalar("SELECT resume_state FROM runtime_question WHERE id=$1")
        .bind(question)
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(state, "linked");
}
async fn ending_race(root: &Path, git: &GitBroker) {
    let pool = fixture(root).await;
    session(&pool).await;
    sqlx::query("INSERT INTO run_workspace VALUES('runtime-test','{}','candidate',NULL)")
        .execute(&pool)
        .await
        .unwrap();
    let a = tool(
        10,
        "report_completion",
        json!({"candidate_sha":"candidate","summary":"finished"}),
    );
    let b = tool(
        11,
        "report_blocker",
        json!({"reason":"race","requires_permission":false}),
    );
    let identity = key();
    let (a, b) = tokio::join!(
        runtime_tools::handle(&pool, git, &identity, &a),
        runtime_tools::handle(&pool, git, &identity, &b)
    );
    let successes = [a, b]
        .into_iter()
        .filter(|r| r.as_ref().is_ok_and(|v| v["success"] == true))
        .count();
    assert_eq!(successes, 1);
    assert!(!store::can_continue(&pool, &key(), 200).await.unwrap());
    store::finalize(&pool).await.unwrap();
    let state: String = sqlx::query_scalar("SELECT state FROM agent_run WHERE id='runtime-test'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_ne!(state, "Succeeded");
    pool.close().await;
}

async fn full_client(root: &Path, git: &GitBroker) {
    full_client_scenario(root, git, client_code(), 1).await;
    let question_root = temporary();
    let code = client_code().replace("  send({'id':77,'method':'item/tool/call'", "  if int(marker.read_text())==1:\n   send({'id':'q','method':'item/tool/requestUserInput','params':{'threadId':'thread','turnId':'turn','itemId':'q','isBlocking':True,'questions':[{'id':'choice','question':'Choose?'}]}})\n   continue\n  send({'id':77,'method':'item/tool/call'")
        .replace(" elif r.get('id')==77:", " elif r.get('id')=='q':\n  send({'method':'item/agentMessage/delta','params':{'threadId':'thread','turnId':'turn','delta':'accepted'}})\n  send({'method':'turn/completed','params':{'threadId':'thread','turn':{'id':'turn','status':'completed'}}})\n elif r.get('id')==77:");
    full_client_scenario(&question_root, git, &code, 2).await;
    std::fs::remove_dir_all(question_root).unwrap();
}
async fn full_client_scenario(root: &Path, git: &GitBroker, code: &str, turns: usize) {
    let pool = fixture(root).await;
    if turns == 1 {
        // A real DB write longer than the idle receive poll must not swallow
        // the already consumed ending RPC. This is not a model timeout.
        sqlx::raw_sql(r#"CREATE FUNCTION slow_ending_evidence() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.channel='stdout' AND convert_from(NEW.payload,'UTF8') LIKE '%"tool": "report_blocker"%' THEN PERFORM pg_sleep(0.15); END IF; RETURN NEW; END $$; CREATE TRIGGER slow_ending_evidence BEFORE INSERT ON runtime_evidence_chunk FOR EACH ROW EXECUTE FUNCTION slow_ending_evidence();"#)
            .execute(&pool).await.unwrap();
    }
    let launch = Launch {
        key: key(),
        workspace: root.to_str().unwrap().into(),
        workspace_identity: "fixture".into(),
        program: "/usr/bin/python3".into(),
        args: vec!["-u".into(), "-c".into(), code.into()],
    };
    sqlx::query("UPDATE agent_run SET launch=$1 WHERE id='runtime-test'")
        .bind(sqlx::types::Json(&launch))
        .execute(&pool)
        .await
        .unwrap();
    let settings = runtime_client::Settings {
        startup_seconds: 5,
        response_seconds: 5,
        stall_seconds: 5,
        reservation: Amount {
            tokens: 100,
            turns: 1,
            model_seconds: 30,
        },
        codex_config: String::new(),
    };
    let answering = if turns == 2 {
        let pool = pool.clone();
        Some(tokio::spawn(async move {
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    if let Some(question) = questions::list(&pool, 1).await.unwrap().first() {
                        tokio::time::sleep(Duration::from_millis(250)).await;
                        questions::answer(
                            &pool,
                            &question.id,
                            &questions::Answer {
                                version: question.version,
                                answer: json!({"answers":{"choice":{"answers":["confirmed"]}}}),
                            },
                            runtime_client::now(),
                        )
                        .await
                        .unwrap();
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
        }))
    } else {
        None
    };
    sqlx::query("INSERT INTO run_workspace(run_id,identity) VALUES('runtime-test','{}')")
        .execute(&pool)
        .await
        .unwrap();
    let result = tokio::time::timeout(
        Duration::from_secs(15),
        codexsymphony_server::runtime_service::tick(
            &pool,
            root,
            Path::new(env!("CARGO_BIN_EXE_codexsymphony-server")),
            git,
            "boot",
            &codexsymphony_server::runtime_service::Config {
                validation: None,
                settings,
                preparation_adapter: "/bin/true".into(),
                preparation: json!({"launcher":["/bin/true"]}),
            },
        ),
    )
    .await
    .unwrap();
    result.unwrap();
    if let Some(task) = answering {
        task.await.unwrap();
    }
    assert_eq!(
        std::fs::read_to_string(root.join("turn-count")).unwrap(),
        turns.to_string()
    );
    let reply: Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("tool-reply")).unwrap()).unwrap();
    assert_eq!(reply["result"]["success"], true);
    let ending: String =
        sqlx::query_scalar("SELECT end_kind FROM runtime_session WHERE run_id='runtime-test'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(ending, "blocker");
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if root.join("runtime-test/quiescent.json").exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    pool.close().await;
}

#[tokio::test]
async fn frames_and_validation_reject_invalid_input() {
    use codexsymphony_server::runtime_transport::frame;
    let mut input = tokio::io::BufReader::new(&b"{\"id\":1}\n"[..]);
    assert!(frame(&mut input).await.unwrap().is_some());
    assert!(frame(&mut input).await.unwrap().is_none());
    assert!(
        frame(&mut tokio::io::BufReader::new(&b"partial"[..]))
            .await
            .is_err()
    );
    let oversized = vec![b'x'; runtime::MAX_FRAME + 1];
    assert!(
        frame(&mut tokio::io::BufReader::new(oversized.as_slice()))
            .await
            .is_err()
    );
    assert!(runtime::expired(28800, 0, None));
    assert!(runtime::expired(7300, 0, Some(100)));
    assert!(!runtime::expired(7299, 0, Some(100)));
    assert!(!runtime::rpc_id_valid(&json!(null)));
    assert!(!runtime::text_valid("\0", 100));
    assert!(!runtime::validate_answers(
        &question(),
        &json!({"answers":{}})
    ));
    assert_eq!(
        runtime_tools::deny("item/commandExecution/requestApproval")["decision"],
        "decline"
    );
    assert_eq!(runtime_tools::deny("unknown")["error"]["code"], -32601);
}

fn client_code() -> &'static str {
    r#"import sys,json,os
from pathlib import Path
def send(v):print(json.dumps(v),flush=True)
for line in sys.stdin:
 r=json.loads(line);m=r.get('method');p=r.get('params',{})
 if m=='initialize':
  print('separate stderr diagnostic',file=sys.stderr,flush=True)
  send({'method':'fixture/queued','params':{'threadId':'unrelated'}})
  send({'id':r['id'],'result':{'userAgent':'fixture/0.154.0 (test)'}})
 elif m=='thread/start':send({'id':r['id'],'result':{'cwd':os.getcwd(),'thread':{'id':'thread','cwd':os.getcwd()}}})
 elif m=='turn/start':
  Path('received-input').write_text(json.dumps(p['input']))
  marker=Path('turn-count');marker.write_text(str(int(marker.read_text())+1) if marker.exists() else '1')
  send({'id':r['id'],'result':{'turn':{'id':'turn'}}})
  send({'method':'turn/completed','params':{'threadId':'thread','turn':{'id':'obsolete','status':'completed'}}})
  send({'id':'permissions','method':'item/permissions/requestApproval','params':{}})
  send({'id':'unknown','method':'unsupported','params':{}})
  send({'method':'thread/tokenUsage/updated','params':{'threadId':'thread','turnId':'turn','tokenUsage':{'total':{'inputTokens':20,'cachedInputTokens':5,'outputTokens':10}}}})
  send({'id':77,'method':'item/tool/call','params':{'threadId':'thread','turnId':'turn','callId':'call','tool':'report_blocker','arguments':{'reason':'fixture stop','requires_permission':False}}})
 elif r.get('id')==77:
  Path('tool-reply').write_text(json.dumps(r));send({'method':'turn/completed','params':{'threadId':'thread','turn':{'id':'turn','status':'completed'}}})
 elif m=='turn/interrupt':send({'id':r['id'],'result':{}})
"#
}

fn local_git(path: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("/usr/bin/git")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@localhost")
        .env("GIT_COMMITTER_NAME", "fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@localhost")
        .arg("-C")
        .arg(path)
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
async fn automatic_answer_recovery(root: &Path, storage_recovery: bool, guard_cleared: bool) {
    use codexsymphony_server::{runtime_resume, runtime_service, workspace::Workspace};
    let pool = fixture(root).await;
    session(&pool).await;
    if !storage_recovery {
        let question = questions::ask(&pool, &key(), &question(), 100)
            .await
            .unwrap();
        questions::expire(&pool, 86400).await.unwrap();
        questions::answer(
            &pool,
            &question.id,
            &questions::Answer {
                version: 1,
                answer: json!({"answers":{"choice":{"answers":["saved overnight"]}}}),
            },
            86401,
        )
        .await
        .unwrap();
    }
    let seed = root.join("seed");
    std::fs::create_dir(&seed).unwrap();
    local_git(&seed, &["init", "--template=", "-b", "main"]);
    std::fs::write(seed.join("source.txt"), "baseline\n").unwrap();
    local_git(&seed, &["add", "."]);
    local_git(&seed, &["commit", "-m", "baseline"]);
    let baseline = local_git(&seed, &["rev-parse", "HEAD"]);
    let bundle = root.join("seed.bundle");
    local_git(
        &seed,
        &["bundle", "create", bundle.to_str().unwrap(), "--all"],
    );
    let broker = GitBroker::initialize(&root.join("workspaces"), &bundle).unwrap();
    let source = Workspace {
        key: key(),
        identity: "fixture".into(),
        requirement: 1,
        revision: 1,
        phase: "execution".into(),
        baseline,
        branch: "ai/req-1-runtime-test".into(),
        path: broker
            .path("runtime-test")
            .unwrap()
            .to_string_lossy()
            .into_owned(),
    };
    broker.prepare(&source, true).unwrap();
    std::fs::write(
        Path::new(&source.path).join("source.txt"),
        "paid unfinished work\n",
    )
    .unwrap();
    if guard_cleared {
        broker
            .commit(&source, "paid commit before declaration")
            .unwrap();
    }
    let manifest = broker.preserve(&source).unwrap();
    if !storage_recovery || guard_cleared {
        sqlx::query("INSERT INTO workspace_snapshot VALUES('runtime-test',$1,$2)")
            .bind(json!(manifest))
            .bind(guard_cleared)
            .execute(&pool)
            .await
            .unwrap();
    }
    sqlx::query("UPDATE agent_run SET quiescent=true,state='Interrupted'")
        .execute(&pool)
        .await
        .unwrap();
    if storage_recovery {
        use codexsymphony_server::{operator_control, storage_service, storage_store};
        let deployment: storage_store::Deployment =
            serde_json::from_slice(&std::fs::read(storage_config(root)).unwrap()).unwrap();
        storage_store::install(&pool, &deployment).await.unwrap();
        sqlx::query("UPDATE agent_run SET stop_requested=true")
            .execute(&pool)
            .await
            .unwrap();
        storage_service::block(&pool, "deterministic storage interruption")
            .await
            .unwrap();
        if guard_cleared {
            // Previous deployment cleared the guard before it could record intent.
            assert!(
                codexsymphony_server::storage::recover(&pool, &deployment.execution.path)
                    .await
                    .unwrap()
            );
        }
        let version: i64 = sqlx::query_scalar("SELECT version FROM requirement WHERE id=1")
            .fetch_one(&pool)
            .await
            .unwrap();
        let command = operator_control::Command {
            version,
            request_id: "storage-resume".into(),
            action: operator_control::Action::StorageRecheck,
        };
        let first = operator_control::execute(&pool, 1, &command).await.unwrap();
        assert_eq!(
            first,
            operator_control::execute(&pool, 1, &command).await.unwrap()
        );
        let intent: (bool, bool) = sqlx::query_as(
            "SELECT storage_resume_requested,user_paused FROM agent_run WHERE id='runtime-test'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(intent, (true, false));
        if !guard_cleared {
            // Intent survives without a snapshot; it cannot launch unfinished preservation.
            assert!(
                runtime_resume::next(&pool, &broker, "boot", &["/usr/bin/python3".into()])
                    .await
                    .unwrap()
                    .is_none()
            );
            sqlx::query("INSERT INTO workspace_snapshot VALUES('runtime-test',$1,false)")
                .bind(json!(manifest))
                .execute(&pool)
                .await
                .unwrap();
        }
    }
    let launcher = vec![
        "/usr/bin/python3".into(),
        "-u".into(),
        "-c".into(),
        completion_code()
            .replace("'id':'thread'", "'id':'resumed-thread'")
            .replace("'threadId':'thread'", "'threadId':'resumed-thread'"),
    ];
    sqlx::query("UPDATE execution_control SET paused=true")
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        runtime_resume::next(&pool, &broker, "boot", &launcher)
            .await
            .unwrap()
            .is_none()
    );
    sqlx::query("UPDATE execution_control SET paused=false")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE requirement_budget SET exhausted=true")
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        runtime_resume::next(&pool, &broker, "boot", &launcher)
            .await
            .unwrap()
            .is_none()
    );
    sqlx::query("UPDATE requirement_budget SET exhausted=false")
        .execute(&pool)
        .await
        .unwrap();
    let mut job = runtime_resume::next(&pool, &broker, "boot", &launcher)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(job.launch.workspace, source.path);
    assert_eq!(
        std::fs::read_to_string(Path::new(&job.launch.workspace).join("source.txt")).unwrap(),
        "paid unfinished work\n"
    );
    assert_eq!(
        runtime_resume::next(&pool, &broker, "boot", &launcher)
            .await
            .unwrap()
            .unwrap()
            .launch
            .key
            .run_id,
        job.launch.key.run_id
    );
    sqlx::query("UPDATE runtime_resume SET status='restoring'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        runtime_resume::next(&pool, &broker, "boot", &launcher)
            .await
            .unwrap()
            .is_none()
    );
    sqlx::query("UPDATE runtime_resume SET status='prepared'")
        .execute(&pool)
        .await
        .unwrap();
    let adapter = root.join("resume-adapter.py");
    // The test adapter supplies deterministic preflight observations; production
    // requires the operator's fixed real command/exec adapter at this boundary.
    std::fs::write(&adapter, r#"import json,sys
p=json.load(sys.stdin)
print(json.dumps({'deployment_identity':'fixture','execution_identity':'sandbox','network':{'configuration_identity':'fixture','reachable':True},'failures':[],'sample':{'cwd':p['workspace']}}))
"#).unwrap();
    let mut config = runtime_service::Config {
        validation: None,
        settings: runtime_client::Settings {
            startup_seconds: 5,
            response_seconds: 5,
            stall_seconds: 5,
            reservation: Amount {
                tokens: 100,
                turns: 1,
                model_seconds: 30,
            },
            codex_config: String::new(),
        },
        preparation_adapter: adapter,
        preparation: json!({"launcher":launcher,"deployment_identity":"fixture"}),
    };
    let valid_launcher = config.preparation["launcher"].clone();
    config.preparation["launcher"] = json!([]);
    assert!(config.launcher().is_err());
    config.preparation["launcher"] = valid_launcher;
    assert!(
        runtime_resume::prepare(
            &pool,
            root,
            &broker,
            &job,
            &config.preparation_adapter,
            config.preparation.clone()
        )
        .await
        .unwrap()
    );
    assert!(
        runtime_resume::prepare(
            &pool,
            root,
            &broker,
            &job,
            &config.preparation_adapter,
            config.preparation.clone()
        )
        .await
        .unwrap()
    );
    sqlx::query("UPDATE preparation_record SET checked_at=checked_at-61 WHERE run_id=$1")
        .bind(&job.launch.key.run_id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        runtime_resume::prepare(
            &pool,
            root,
            &broker,
            &job,
            &config.preparation_adapter,
            config.preparation.clone()
        )
        .await
        .unwrap()
    );
    let attempts: i32 = sqlx::query_scalar(
        "SELECT (retry->>'attempts')::integer FROM preparation_record WHERE run_id=$1",
    )
    .bind(&job.launch.key.run_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(attempts, 2);
    let incarnation = if storage_recovery {
        let old = job.clone();
        if !guard_cleared {
            // A restart can interrupt restoration before preparation is recorded.
            // The source snapshot and partial destination must both survive.
            sqlx::query("UPDATE runtime_resume SET status='restoring'")
                .execute(&pool)
                .await
                .unwrap();
        }
        codexsymphony_server::run_store::begin_incarnation(&pool, "restarted")
            .await
            .unwrap();
        assert!(
            codexsymphony_server::run_store::finish_recovery(&pool, "restarted")
                .await
                .unwrap()
        );
        job = runtime_resume::next(&pool, &broker, "restarted", &launcher)
            .await
            .unwrap()
            .unwrap();
        assert_ne!(job.launch.key.run_id, old.launch.key.run_id);
        assert_eq!(
            std::fs::read_to_string(Path::new(&old.workspace.path).join("source.txt")).unwrap(),
            "paid unfinished work\n"
        );
        let archived: Value =
            sqlx::query_scalar("SELECT job FROM runtime_resume_history WHERE run_id=$1")
                .bind(&old.launch.key.run_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(archived, json!(old));
        assert!(
            !sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM agent_run WHERE id=$1)")
                .bind(&old.launch.key.run_id)
                .fetch_one(&pool)
                .await
                .unwrap()
        );
        "restarted"
    } else {
        "boot"
    };
    tokio::time::timeout(
        Duration::from_secs(20),
        runtime_service::tick(
            &pool,
            root,
            Path::new(env!("CARGO_BIN_EXE_codexsymphony-server")),
            &broker,
            incarnation,
            &config,
        ),
    )
    .await
    .unwrap()
    .unwrap();
    if !storage_recovery {
        let saved = questions::list(&pool, 1).await.unwrap().remove(0);
        assert_eq!(saved.resume_state, "linked");
        assert_eq!(
            saved.resumed_run.as_deref(),
            Some(job.launch.key.run_id.as_str())
        );
        assert!(
            std::fs::read_to_string(Path::new(&job.launch.workspace).join("received-input"))
                .unwrap()
                .contains("saved overnight")
        );
    }
    assert_eq!(
        std::fs::read_to_string(Path::new(&job.launch.workspace).join("turn-count")).unwrap(),
        "1"
    );
    assert!(
        runtime_resume::next(&pool, &broker, incarnation, &config.launcher().unwrap())
            .await
            .unwrap()
            .is_none()
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        while !root
            .join(&job.launch.key.run_id)
            .join("quiescent.json")
            .exists()
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let ending: String = sqlx::query_scalar("SELECT end_kind FROM runtime_session WHERE run_id=$1")
        .bind(&job.launch.key.run_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(ending, "completion");
    if !storage_recovery {
        configured_controller(&pool, root, &config).await;
    }
    pool.close().await;
}

#[tokio::test]
async fn transport_bounds_and_stream_lifetimes() {
    use codexsymphony_server::runtime_transport::{Record, Transport};
    use std::process::{Command, Stdio};
    for (input, output, error) in [
        (false, false, false),
        (true, false, false),
        (true, true, false),
    ] {
        let mut child = Command::new("/bin/true")
            .stdin(if input { Stdio::piped() } else { Stdio::null() })
            .stdout(if output {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stderr(if error { Stdio::piped() } else { Stdio::null() })
            .spawn()
            .unwrap();
        assert!(Transport::new(&mut child).is_err());
        child.wait().unwrap();
    }
    let mut child = Command::new("/usr/bin/python3").args(["-u","-c","import sys,json; print('diagnostic',file=sys.stderr); r=json.loads(sys.stdin.readline()); assert 'optional' not in r['params']; assert r['params']['nested']=={'keep':None}; print(json.dumps({'id':r['id'],'result':{}}))"])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let mut transport = Transport::new(&mut child).unwrap();
    assert!(
        transport
            .send(&"x".repeat(runtime::MAX_FRAME + 1))
            .await
            .is_err()
    );
    let id = transport
        .request("fixture", &json!({"optional":null,"nested":{"keep":null}}))
        .await
        .unwrap();
    for n in 0..64 {
        transport.defer(json!(n)).unwrap();
    }
    assert!(transport.defer(json!(65)).is_err());
    for n in 0..64 {
        assert_eq!(transport.pending(), Some(json!(n)));
    }
    assert!(transport.pending().is_none());
    let mut protocol = false;
    let mut diagnostic = false;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match transport.receive().await.unwrap() {
                Record::Protocol(bytes) => {
                    assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap()["id"], id);
                    protocol = true;
                }
                Record::Diagnostic(bytes) => {
                    assert!(String::from_utf8(bytes).unwrap().contains("diagnostic"));
                    diagnostic = true;
                }
                Record::Closed => break,
            }
        }
    })
    .await
    .unwrap();
    child.wait().unwrap();
    assert!(protocol);
    assert!(diagnostic);
    assert!(transport.send(&json!({})).await.is_err());
}

fn completion_code() -> String {
    client_code().replace("send({'id':77,'method':'item/tool/call','params':{'threadId':'thread','turnId':'turn','callId':'call','tool':'report_blocker','arguments':{'reason':'fixture stop','requires_permission':False}}})", "send({'id':78,'method':'item/tool/call','params':{'threadId':'thread','turnId':'turn','callId':'commit','tool':'create_local_commit','arguments':{'message':'preserved work'}}})")
        .replace(" elif r.get('id')==77:", " elif r.get('id')==78:\n  assert r['result']['success'],r\n  sha=json.loads(r['result']['contentItems'][0]['text'])['sha']\n  send({'id':77,'method':'item/tool/call','params':{'threadId':'thread','turnId':'turn','callId':'completion','tool':'report_completion','arguments':{'candidate_sha':sha,'summary':'fixture completion'}}})\n elif r.get('id')==77:")
}

fn storage_config(root: &Path) -> std::path::PathBuf {
    use codexsymphony_server::{
        storage_files::Directory,
        storage_lifecycle::{CATEGORIES, Limit, Policy},
        storage_store::Root,
    };
    let cold = root.with_extension("cold");
    std::fs::create_dir(&cold).unwrap();
    let owned = |path: &Path| Root {
        path: std::fs::canonicalize(path).unwrap(),
        identity: Directory::open(path).unwrap().identity().unwrap(),
    };
    let policy = Policy {
        version: "runtime-storage-test".into(),
        reason: "synthetic controller filesystem with disposable remote PostgreSQL".into(),
        global_bytes: 16 << 30,
        control_bytes: 256 << 20,
        run_bytes: 1 << 30,
        requirement_bytes: 4 << 30,
        entry_bytes: 1 << 20,
        entry_count: 100000,
        categories: CATEGORIES
            .into_iter()
            .map(|category| {
                (
                    category,
                    Limit {
                        bytes: 8 << 30,
                        seconds: 86400,
                        reserve_bytes: 16 << 20,
                    },
                )
            })
            .collect(),
    };
    let deployment = codexsymphony_server::storage_store::Deployment {
        policy,
        execution: owned(root),
        cold: owned(&cold),
        database_filesystem: owned(root),
        database_extras: vec![],
    };
    let path = root.join("storage-config.json");
    std::fs::write(&path, serde_json::to_vec(&deployment).unwrap()).unwrap();
    path
}

async fn configured_controller(
    pool: &PgPool,
    root: &Path,
    config: &codexsymphony_server::runtime_service::Config,
) {
    let schema: String = sqlx::query_scalar("SELECT current_schema()")
        .fetch_one(pool)
        .await
        .unwrap();
    let storage_path = storage_config(root);
    let config_path = root.join("runtime-config.json");
    std::fs::write(&config_path, json!({"settings":config.settings,"preparation_adapter":config.preparation_adapter,"preparation":config.preparation}).to_string()).unwrap();
    let url = format!(
        "{}{}options=-csearch_path%3D{}",
        std::env::var("TEST_DATABASE_URL").unwrap(),
        if std::env::var("TEST_DATABASE_URL").unwrap().contains('?') {
            "&"
        } else {
            "?"
        },
        schema
    );
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let worker_log = root.join("runtime-worker.log");
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_codexsymphony-server"))
        .env("DATABASE_URL", url)
        .env("RUNTIME_CONFIG", config_path)
        .env("STORAGE_CONFIG", &storage_path)
        .env("EXECUTION_DIRECTORY", root)
        .env("BIND_ADDRESS", address.to_string())
        .env("WEB_ORIGIN", "https://localhost:4200")
        .env("AUTH_CONFIG", server_auth::config(root))
        .env_remove("GITHUB_APP_CONFIG")
        .stdout(std::fs::File::create(&worker_log).unwrap())
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .unwrap();
    let ready = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                panic!("controller exited: {status}");
            }
            let value: String = sqlx::query_scalar("SELECT incarnation FROM execution_control")
                .fetch_one(pool)
                .await
                .unwrap();
            if value != "boot"
                && let Ok(mut stream) = std::net::TcpStream::connect(address)
            {
                use std::io::{Read, Write};
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                write!(
                    stream,
                    "GET /api/health HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
                )
                .unwrap();
                let mut response = String::new();
                stream.read_to_string(&mut response).unwrap();
                assert!(response.starts_with("HTTP/1.1 200"));
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    // A missing archive mount must close admission without killing the scanner.
    let cold = root.with_extension("cold");
    let displaced = root.with_extension("displaced-cold");
    std::fs::rename(&cold, &displaced).unwrap();
    sqlx::query("NOTIFY storage_phase_ended")
        .execute(pool)
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let error: Option<String> = sqlx::query_scalar("SELECT error FROM storage_guard")
                .fetch_one(pool)
                .await
                .unwrap();
            if error.as_deref().is_some_and(|error| {
                error.starts_with(
                    "storage scan failed; preserve registered originals and reconcile:",
                )
            }) {
                break;
            }
            assert!(child.try_wait().unwrap().is_none());
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    std::fs::rename(&displaced, &cold).unwrap();
    // A transient unavailable execution table must not kill the control plane.
    sqlx::query("ALTER TABLE runtime_session RENAME TO unavailable_runtime_session")
        .execute(pool)
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if std::fs::read_to_string(&worker_log)
                .unwrap()
                .contains("Runtime execution or answer recovery requires reconciliation")
            {
                break;
            }
            assert!(child.try_wait().unwrap().is_none());
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert!(child.try_wait().unwrap().is_none());
    sqlx::query("ALTER TABLE unavailable_runtime_session RENAME TO runtime_session")
        .execute(pool)
        .await
        .unwrap();
    std::process::Command::new("/bin/kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .unwrap();
    let status = child.wait().unwrap();
    ready.unwrap();
    assert!(status.success());
    let bad = std::process::Command::new(env!("CARGO_BIN_EXE_codexsymphony-server"))
        .env(
            "DATABASE_URL",
            format!(
                "{}{}options=-csearch_path%3D{}",
                std::env::var("TEST_DATABASE_URL").unwrap(),
                if std::env::var("TEST_DATABASE_URL").unwrap().contains('?') {
                    "&"
                } else {
                    "?"
                },
                schema
            ),
        )
        .env("RUNTIME_CONFIG", root.join("missing-runtime-config"))
        .env("STORAGE_CONFIG", &storage_path)
        .env("EXECUTION_DIRECTORY", root)
        .env("BIND_ADDRESS", "127.0.0.1:0")
        .env("WEB_ORIGIN", "https://localhost:4200")
        .env("AUTH_CONFIG", server_auth::config(root))
        .env_remove("GITHUB_APP_CONFIG")
        .output()
        .unwrap();
    assert!(!bad.status.success());
}

async fn failed_protocol_sessions(git: &GitBroker) {
    let premature = client_code().replace("send({'id':77,'method':'item/tool/call','params':{'threadId':'thread','turnId':'turn','callId':'call','tool':'report_blocker','arguments':{'reason':'fixture stop','requires_permission':False}}})", "send({'id':'q','method':'item/tool/requestUserInput','params':{'threadId':'thread','turnId':'turn','itemId':'q','isBlocking':True,'questions':[{'id':'choice','question':'Still answerable?'}]}})\n  send({'method':'turn/completed','params':{'threadId':'thread','turn':{'id':'turn','status':'interrupted'}}})");
    for code in [
        client_code().replace(
            "'result':{'userAgent':'fixture/0.154.0 (test)'}",
            "'error':{'code':-1,'message':'fixture rejection'}",
        ),
        client_code().replace(
            "'result':{'turn':{'id':'turn'}}",
            "'error':{'code':-1,'message':'fixture turn rejection'}",
        ),
        "import sys; sys.stdin.readline()".into(),
        premature.clone(),
        premature.replace("'status':'interrupted'", "'status':'completed'"),
    ] {
        let root = temporary();
        let pool = fixture(&root).await;
        let launch = Launch {
            key: key(),
            workspace: root.to_string_lossy().into_owned(),
            workspace_identity: "fixture".into(),
            program: "/usr/bin/python3".into(),
            args: vec!["-u".into(), "-c".into(), code],
        };
        sqlx::query("UPDATE agent_run SET launch=$1 WHERE id='runtime-test'")
            .bind(json!(launch))
            .execute(&pool)
            .await
            .unwrap();
        let settings = runtime_client::Settings {
            startup_seconds: 5,
            response_seconds: 5,
            stall_seconds: 5,
            reservation: Amount {
                tokens: 100,
                turns: 1,
                model_seconds: 30,
            },
            codex_config: String::new(),
        };
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            runtime_client::execute(
                &pool,
                &root,
                Path::new(env!("CARGO_BIN_EXE_codexsymphony-server")),
                git,
                &launch,
                &settings,
            ),
        )
        .await
        .unwrap();
        assert!(result.is_err());
        tokio::time::timeout(Duration::from_secs(5), async {
            while !root.join("runtime-test/quiescent.json").exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        pool.close().await;
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn invalid_answers_and_denials() {
    assert!(!runtime::validate_answers(&json!({}), &json!({})));
    assert!(!runtime::validate_answers(&question(), &json!({})));
    assert!(!runtime::validate_answers(
        &json!({"params":{"questions":[{}]}}),
        &json!({"answers":{"x":{"answers":["y"]}}})
    ));
    for method in [
        "item/commandExecution/requestApproval",
        "item/fileChange/requestApproval",
        "item/permissions/requestApproval",
        "execCommandApproval",
        "applyPatchApproval",
        "unknown",
    ] {
        assert!(runtime_tools::deny(method).is_object());
    }
}
struct FailedRead;
impl tokio::io::AsyncRead for FailedRead {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
        _: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Err(std::io::Error::other("synthetic read failure")))
    }
}
#[tokio::test]
async fn stream_errors_and_consumer_cancellation() {
    use codexsymphony_server::runtime_transport::{diagnostics, protocol};
    let (sender, mut receiver) = tokio::sync::mpsc::channel(4);
    protocol(&b"incomplete"[..], sender).await;
    assert!(receiver.recv().await.unwrap().is_err());
    let (sender, mut receiver) = tokio::sync::mpsc::channel(4);
    diagnostics(FailedRead, sender).await;
    assert!(receiver.recv().await.unwrap().is_err());
    let (sender, receiver) = tokio::sync::mpsc::channel(4);
    drop(receiver);
    protocol(&b"{}\n"[..], sender).await;
    let (sender, receiver) = tokio::sync::mpsc::channel(4);
    drop(receiver);
    diagnostics(&b"diagnostic"[..], sender).await;
}

#[tokio::test]
async fn storage_recheck_restores_paid_work_once_without_a_question_or_pause() {
    let _scenario = DATABASE_SCENARIO.lock().await;
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();
    let root = temporary();
    automatic_answer_recovery(&root, true, false).await;
    std::fs::remove_dir_all(root.with_extension("cold")).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn storage_recheck_recovers_committed_work_without_completion_declaration() {
    let _scenario = DATABASE_SCENARIO.lock().await;
    let root = temporary();
    automatic_answer_recovery(&root, true, true).await;
    std::fs::remove_dir_all(root.with_extension("cold")).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn pinned_codex_full_client_survives_idle_provider_response() {
    let _scenario = DATABASE_SCENARIO.lock().await;
    let root = temporary();
    let pool = fixture(&root).await;
    let git = broker(&root.join("broker"));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let app = axum::Router::new().route("/responses", axum::routing::post(|| async {
        tokio::time::sleep(Duration::from_secs(3)).await;
        let events = [
            json!({"type":"response.created","response":{"id":"idle-fixture"}}),
            json!({"type":"response.output_item.done","item":{"type":"function_call","call_id":"idle-call","name":"report_blocker","arguments":"{\"reason\":\"idle fixture complete\",\"requires_permission\":false}"}}),
            json!({"type":"response.completed","response":{"id":"idle-fixture","usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}}),
        ];
        ([("content-type", "text/event-stream")], events.iter().map(|v| format!("data: {v}\n\n")).collect::<String>())
    }));
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let launch = Launch {
        key: key(),
        workspace: root.to_str().unwrap().into(),
        workspace_identity: "fixture".into(),
        program: std::env::var("CODEX_BINARY").unwrap_or_else(|_| {
            String::from_utf8(
                std::process::Command::new("which")
                    .arg("codex")
                    .output()
                    .unwrap()
                    .stdout,
            )
            .unwrap()
            .trim()
            .into()
        }),
        args: vec!["app-server".into()],
    };
    sqlx::query("UPDATE agent_run SET launch=$1 WHERE id='runtime-test'")
        .bind(json!(launch))
        .execute(&pool)
        .await
        .unwrap();
    let settings = runtime_client::Settings {
        startup_seconds: 30,
        response_seconds: 30,
        stall_seconds: 30,
        reservation: Amount {
            tokens: 100,
            turns: 1,
            model_seconds: 30,
        },
        codex_config: format!(
            r#"model = "gpt-6-astra"
model_provider = "idle_fixture"
approval_policy = "never"
sandbox_mode = "danger-full-access"
[features]
apps = false
plugins = false
remote_plugin = false
goals = false
[model_providers.idle_fixture]
name = "Local idle fixture"
base_url = "http://127.0.0.1:{port}"
wire_api = "responses"
requires_openai_auth = false
supports_websockets = false
request_max_retries = 0
stream_max_retries = 0
"#
        ),
    };
    let result = tokio::time::timeout(
        Duration::from_secs(45),
        runtime_client::execute(
            &pool,
            &root,
            Path::new(env!("CARGO_BIN_EXE_codexsymphony-server")),
            &git,
            &launch,
            &settings,
        ),
    )
    .await
    .unwrap();
    server.abort();
    result.unwrap();
    let ending: String =
        sqlx::query_scalar("SELECT end_kind FROM runtime_session WHERE run_id='runtime-test'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(ending, "blocker");
    pool.close().await;
}

#[path = "support/server_auth.rs"]
mod server_auth;

#[path = "support/auth.rs"]
mod auth_client;
