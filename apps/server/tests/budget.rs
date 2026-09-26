//! GH-18 accounting acceptance with real PostgreSQL transactions; zero model calls.
use codexsymphony_server::{
    budget::{Amount, Purpose, Usage, Waiting},
    budget_store::{self, Admission, CallIntent, Increase},
    execution::{Launch, RunKey},
    preparation::{Failure, Retry},
    preparation_store, run_store,
};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};

fn amount(tokens: i64, turns: i64, model_seconds: i64) -> Amount {
    Amount {
        tokens,
        turns,
        model_seconds,
    }
}
fn usage(input: i64, output: i64, seconds: i64, complete: bool) -> Usage {
    Usage {
        input: Some(input),
        cached: Some(input / 2),
        output: Some(output),
        model_seconds: Some(seconds),
        complete,
    }
}
fn key(run: &str) -> RunKey {
    RunKey {
        run_id: run.into(),
        request_id: format!("start-{run}"),
        incarnation: "boot".into(),
    }
}
fn call(run: &str, turn: &str, tokens: i64) -> CallIntent {
    CallIntent {
        key: key(run),
        turn_id: turn.into(),
        purpose: Purpose::Coding,
        reserve: amount(tokens, 1, 10),
    }
}
async fn connect() -> PgPool {
    PgPoolOptions::new()
        .max_connections(5)
        .connect(&std::env::var("TEST_DATABASE_URL").unwrap())
        .await
        .unwrap()
}
async fn fixture() -> PgPool {
    let pool = connect().await;
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
    sqlx::raw_sql("TRUNCATE requirement,repository,business_request RESTART IDENTITY CASCADE;
      INSERT INTO repository(id,version,document) VALUES(1,1,'{\"revoked\":false}');
      INSERT INTO requirement(version,state,contract,revision) VALUES(1,'Running','{}',1);
      INSERT INTO requirement_revision VALUES(1,1,'{\"repository_version\":1,\"contract\":{\"network_access\":[]}}');
      INSERT INTO requirement_budget(requirement_id,limits) VALUES(1,'{\"tokens\":100,\"turns\":10,\"model_seconds\":100}');
      INSERT INTO budget_authorization(requirement_id,version,request_id,actor,reason,delta,limits) SELECT 1,1,'initial','local-user','review',limits,limits FROM requirement_budget;
      INSERT INTO execution_control(id,requirement_id,incarnation,recovery_complete) VALUES(1,1,'boot',true) ON CONFLICT(id) DO UPDATE SET requirement_id=1,incarnation='boot',recovery_complete=true,paused=false;
      UPDATE storage_guard SET blocked=false,error=NULL,policy_version=NULL;")
        .execute(&pool).await.unwrap();
    new_run(&pool, "first", 1).await;
    pool
}
async fn new_run(pool: &PgPool, id: &str, revision: i64) {
    // Action admission probes storage: the isolated source checkout is read-only.
    // Use the test namespace's writable temporary filesystem as run storage.
    sqlx::query("UPDATE agent_run SET quiescent=true,state='Interrupted'")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO agent_run(id,requirement_id,revision,incarnation,request_id,workspace,workspace_identity,launch,state,model) VALUES($1,1,$2,'boot',$3,$4,'worktree','{}','Running','configured-model')")
        .bind(id).bind(revision).bind(key(id).request_id).bind(std::env::temp_dir().to_string_lossy().as_ref()).execute(pool).await.unwrap();
}
async fn grant(pool: &PgPool) -> Increase {
    let grant = Increase {
        request_id: "user-increase".into(),
        requirement_id: 1,
        expected_version: 1,
        actor: "local-user".into(),
        reason: "explicit extra resources".into(),
        delta: amount(100, 1, 20),
    };
    budget_store::increase(pool, &grant).await.unwrap();
    grant
}

#[test]
fn cumulative_counters_and_waits_preserve_unknowns_and_overflow() {
    let unknown = Usage::default();
    assert!(!unknown.settled());
    assert_eq!(unknown.input, None);
    assert_eq!(unknown.actual(), Some(Amount::default()));
    let cached_only = Usage {
        cached: Some(150),
        ..Usage::default()
    };
    assert_eq!(
        cached_only.exposure(amount(100, 1, 10)),
        Some(amount(150, 1, 10))
    );
    assert!(!cached_only.settled());
    assert_eq!(
        unknown.exposure(amount(100, 1, 30)),
        Some(amount(100, 1, 30))
    );
    let high = usage(70, 40, 20, false);
    let low = usage(20, 10, 5, false);
    assert_eq!(high.merge(&low), high);
    assert_eq!(high.exposure(amount(100, 1, 30)), Some(amount(110, 1, 30)));
    let done = high.merge(&Usage {
        complete: true,
        ..Usage::default()
    });
    assert!(done.settled());
    assert_eq!(done.exposure(amount(150, 1, 30)), Some(amount(110, 1, 20)));
    assert!(!usage(-1, 0, 0, false).valid());
    assert!(
        !Usage {
            cached: Some(50),
            ..low.clone()
        }
        .valid()
    );
    assert!(
        Usage {
            cached: Some(50),
            ..unknown.clone()
        }
        .valid()
    );
    assert!(
        Usage {
            complete: true,
            ..unknown
        }
        .exposure(amount(50, 1, 5))
        .is_some()
    );
    assert_eq!(usage(i64::MAX, 1, 0, true).actual(), None);
    assert_eq!(amount(i64::MAX, 0, 0).checked_add(amount(1, 0, 0)), None);
    assert!(!amount(0, 0, 0).positive());
    assert!(!amount(-1, 0, 0).nonnegative());
    assert!(!amount(1, 2, 1).fits(amount(1, 1, 1)));
    assert!(amount(1, 1, 2).reached(amount(2, 2, 2)));
    assert!(!amount(1, 1, 1).execution_exhausted(amount(2, 1, 2)));
    assert!(amount(2, 1, 1).execution_exhausted(amount(2, 1, 2)));
    assert!(amount(1, 2, 1).execution_exhausted(amount(2, 1, 2)));
    assert!(amount(1, 1, 2).execution_exhausted(amount(2, 1, 2)));
    let wait = Waiting {
        human_seconds: 100,
        paused_seconds: 20,
        ci_seconds: 4,
        network_seconds: 7,
    };
    assert_eq!(wait.merge(&Waiting::default()), wait);
    assert!(
        !Waiting {
            ci_seconds: -1,
            ..wait
        }
        .valid()
    );
    for code in [
        "preparation_dependency_missing",
        "preparation_capability_mismatch",
        "preparation_path_unwritable",
        "network_scope_unavailable",
    ] {
        assert!(Failure::new(code, "raw", "proof").automatic_retry());
    }
    for code in [
        "budget_exhausted",
        "authorization_revoked",
        "runtime_protocol_error",
        "storage_unavailable",
        "unknown",
        "validation_code_failed",
    ] {
        let mut retry = Retry::new("preparation", 0);
        assert!(retry.begin(0, false));
        retry.fail(Failure::new(code, "raw", "proof"), 0);
        assert!(retry.todo);
        assert_eq!(retry.next_attempt_at, None);
    }
}

// The real host command waits in a child process; SQLx must still be able to
// finish the parent's asynchronous transaction rollback and release its lock.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn transactions_reconcile_usage_across_runs_revisions_and_crashes() {
    host_budget_command_preserves_usage_and_stop_intents().await;
    last_reserved_turn_can_finish().await;
    let pool = fixture().await;
    let first = call("first", "one", 60);
    sqlx::query("UPDATE agent_run SET model=NULL WHERE id='first'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        budget_store::reserve(&pool, &first).await.unwrap(),
        Admission::Blocked
    );
    sqlx::query("UPDATE agent_run SET model='configured-model' WHERE id='first'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        budget_store::reserve(&pool, &first).await.unwrap(),
        Admission::Reserved
    );
    assert_eq!(
        budget_store::reserve(&pool, &first).await.unwrap(),
        Admission::Existing
    );
    let mut conflict = first.clone();
    conflict.purpose = Purpose::Summary;
    assert!(budget_store::reserve(&pool, &conflict).await.is_err());
    let mut invalid = first.clone();
    invalid.turn_id.clear();
    assert!(budget_store::reserve(&pool, &invalid).await.is_err());
    invalid.turn_id = "bad-reserve".into();
    invalid.reserve.turns = 0;
    assert!(budget_store::reserve(&pool, &invalid).await.is_err());
    let observed = usage(10, 5, 3, false);
    budget_store::settle(&pool, &first.key, "one", "partial", &observed)
        .await
        .unwrap();
    budget_store::settle(&pool, &first.key, "one", "partial", &observed)
        .await
        .unwrap();
    budget_store::settle(&pool, &first.key, "one", "old", &usage(2, 1, 1, false))
        .await
        .unwrap();
    let balance = budget_store::inspect(&pool, 1).await.unwrap();
    assert_eq!(balance.used, amount(15, 1, 3));
    assert_eq!(balance.exposure, amount(60, 1, 10));
    assert_eq!(balance.unresolved_calls, 1);
    assert!(
        budget_store::settle(&pool, &first.key, "one", "partial", &usage(20, 5, 3, false))
            .await
            .is_err()
    );
    assert!(
        budget_store::settle(&pool, &key("wrong"), "one", "late", &observed)
            .await
            .is_err()
    );
    let mut wrong = first.key.clone();
    wrong.incarnation = "other-boot".into();
    assert!(
        budget_store::settle(&pool, &wrong, "one", "late", &observed)
            .await
            .is_err()
    );
    assert!(
        budget_store::settle(
            &pool,
            &first.key,
            "one",
            "negative",
            &usage(-1, 0, 0, false)
        )
        .await
        .is_err()
    );
    // No process response was saved: a database reconnect must retain exposure.
    pool.close().await;
    let pool = connect().await;
    assert_eq!(
        budget_store::inspect(&pool, 1)
            .await
            .unwrap()
            .exposure
            .tokens,
        60
    );
    assert_eq!(
        budget_store::reserve(&pool, &first).await.unwrap(),
        Admission::Existing
    );
    budget_store::settle(&pool, &first.key, "one", "done", &usage(10, 5, 3, true))
        .await
        .unwrap();
    assert_eq!(
        budget_store::inspect(&pool, 1).await.unwrap().exposure,
        amount(15, 1, 3)
    );
    let left = call("first", "concurrent-a", 60);
    let right = call("first", "concurrent-b", 60);
    let (a, b) = tokio::join!(
        budget_store::reserve(&pool, &left),
        budget_store::reserve(&pool, &right)
    );
    let (a, b) = (a.unwrap(), b.unwrap());
    assert!(matches!(
        (&a, &b),
        (Admission::Reserved, Admission::Blocked) | (Admission::Blocked, Admission::Reserved)
    ));
    let pending = if a == Admission::Reserved {
        left
    } else {
        right
    };
    assert_eq!(
        budget_store::inspect(&pool, 1)
            .await
            .unwrap()
            .exposure
            .tokens,
        75
    );
    assert!(!run_store::actions_allowed(&pool, &first.key).await.unwrap());
    let grant = grant(&pool).await;
    budget_store::increase(&pool, &grant).await.unwrap();
    let mut bad = grant.clone();
    bad.delta.tokens += 1;
    assert!(budget_store::increase(&pool, &bad).await.is_err());
    bad.request_id = "stale".into();
    assert!(budget_store::increase(&pool, &bad).await.is_err());
    bad.expected_version = 2;
    bad.reason.clear();
    assert!(budget_store::increase(&pool, &bad).await.is_err());
    bad.reason = "review".into();
    bad.delta = Amount::default();
    assert!(budget_store::increase(&pool, &bad).await.is_err());
    bad.delta = amount(i64::MAX, 0, 0);
    assert!(budget_store::increase(&pool, &bad).await.is_err());
    // Explicit grants leave the original stopped run and every historical grant intact.
    assert!(!run_store::actions_allowed(&pool, &first.key).await.unwrap());
    let grants: i64 = sqlx::query_scalar("SELECT count(*) FROM budget_authorization")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(grants, 2);
    sqlx::raw_sql("INSERT INTO requirement_revision VALUES(1,2,'{\"repository_version\":1}'); UPDATE requirement SET revision=2").execute(&pool).await.unwrap();
    new_run(&pool, "resumed", 2).await;
    assert_eq!(
        budget_store::inspect(&pool, 1)
            .await
            .unwrap()
            .exposure
            .tokens,
        75
    );
    let resumed = call("resumed", "next", 100);
    assert_eq!(
        budget_store::reserve(&pool, &resumed).await.unwrap(),
        Admission::Reserved
    );
    // Final signal without counters does not release an unknown call.
    budget_store::settle(
        &pool,
        &pending.key,
        &pending.turn_id,
        "unknown-final",
        &Usage {
            complete: true,
            ..Usage::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        budget_store::inspect(&pool, 1)
            .await
            .unwrap()
            .exposure
            .tokens,
        175
    );
    // Late counters from the old incarnation reconcile without granting execution.
    budget_store::settle(
        &pool,
        &pending.key,
        &pending.turn_id,
        "reconciled",
        &usage(20, 10, 4, false),
    )
    .await
    .unwrap();
    assert_eq!(
        budget_store::inspect(&pool, 1)
            .await
            .unwrap()
            .exposure
            .tokens,
        145
    );
    budget_store::settle(
        &pool,
        &resumed.key,
        "next",
        "overrun",
        &usage(160, 10, 5, true),
    )
    .await
    .unwrap();
    let balance = budget_store::inspect(&pool, 1).await.unwrap();
    assert_eq!(balance.used.tokens, 215);
    assert!(balance.exhausted);
    assert!(
        !run_store::actions_allowed(&pool, &resumed.key)
            .await
            .unwrap()
    );
    new_run(&pool, "repair", 2).await;
    let mut repair = call("repair", "fix", 1);
    repair.purpose = Purpose::Repair;
    assert_eq!(
        budget_store::reserve(&pool, &repair).await.unwrap(),
        Admission::Blocked
    );
    let waits = Waiting {
        human_seconds: 8000,
        paused_seconds: 1000,
        ci_seconds: 600,
        network_seconds: 100,
    };
    budget_store::record_waiting(&pool, &repair.key, &waits)
        .await
        .unwrap();
    budget_store::record_waiting(&pool, &repair.key, &Waiting::default())
        .await
        .unwrap();
    assert_eq!(
        budget_store::inspect(&pool, 1)
            .await
            .unwrap()
            .used
            .model_seconds,
        12
    );
    let saved: Value = sqlx::query_scalar("SELECT waiting FROM agent_run WHERE id='repair'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(saved, json!(waits));
    assert!(
        budget_store::record_waiting(
            &pool,
            &repair.key,
            &Waiting {
                human_seconds: -1,
                ..waits
            }
        )
        .await
        .is_err()
    );
    sqlx::query("UPDATE agent_run SET stop_requested=false,created_at=now()-interval '8 hours' WHERE id='repair'").execute(&pool).await.unwrap();
    budget_store::expire_runs(&pool).await.unwrap();
    let blocker: String = sqlx::query_scalar("SELECT blocker FROM agent_run WHERE id='repair'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(blocker.starts_with("runtime_timeout"));
    preparation_history_keeps_quota(&pool).await;
    corrupted_accounting_fails_closed(&pool).await;
    pool.close().await;
}

fn host_budget_command(url: Option<&str>, args: &[&str], input: &[u8]) -> std::process::Output {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut command = Command::new(env!("CARGO_BIN_EXE_codexsymphony-server"));
    command
        .args(["budget"])
        .args(args)
        .env_remove("DATABASE_URL");
    if let Some(url) = url {
        command.env("DATABASE_URL", url);
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).ok();
    child.wait_with_output().unwrap()
}

async fn host_budget_command_preserves_usage_and_stop_intents() {
    let pool = fixture().await;
    let original = call("first", "unresolved", 60);
    assert_eq!(
        budget_store::reserve(&pool, &original).await.unwrap(),
        Admission::Reserved
    );
    budget_store::settle(
        &pool,
        &original.key,
        "unresolved",
        "partial",
        &usage(105, 5, 3, false),
    )
    .await
    .unwrap();
    sqlx::raw_sql("UPDATE requirement SET paused=true; UPDATE execution_control SET paused=true; UPDATE agent_run SET stop_requested=true,quiescent=true,state='Interrupted'").execute(&pool).await.unwrap();
    let before = budget_store::inspect(&pool, 1).await.unwrap();
    assert!(before.exhausted);
    let request = json!({"request_id":"host-budget-increase","requirement_id":1,"expected_version":1,"actor":"local-user","reason":"explicit operator authorization after preserving stopped work","delta":{"tokens":200,"turns":0,"model_seconds":100}});
    let url = std::env::var("TEST_DATABASE_URL").unwrap();
    let args = ["increase", "--stdin-json"];
    for _ in 0..2 {
        let result = host_budget_command(Some(&url), &args, request.to_string().as_bytes());
        assert!(
            result.status.success(),
            "budget command failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let after = budget_store::inspect(&pool, 1).await.unwrap();
    assert_eq!(after.limits, amount(300, 10, 200));
    assert_eq!(after.used, before.used);
    assert_eq!(after.exposure, before.exposure);
    assert_eq!(after.unresolved_calls, 1);
    assert!(!after.exhausted);
    let facts: (i64, i64, i64, bool, bool, bool) = sqlx::query_as("SELECT (SELECT count(*) FROM agent_run),(SELECT count(*) FROM model_call),(SELECT count(*) FROM budget_authorization),(SELECT paused FROM requirement WHERE id=1),(SELECT paused FROM execution_control WHERE id=1),(SELECT stop_requested FROM agent_run WHERE id='first')").fetch_one(&pool).await.unwrap();
    assert_eq!(facts, (1, 1, 2, true, true, true));
    for change in [
        json!({"request_id":"stale-budget-request"}),
        json!({"delta":{"tokens":201,"turns":0,"model_seconds":100}}),
        json!({"extra":"not supported"}),
    ] {
        let mut invalid = request.clone();
        invalid
            .as_object_mut()
            .unwrap()
            .extend(change.as_object().unwrap().clone());
        assert!(
            !host_budget_command(Some(&url), &args, invalid.to_string().as_bytes())
                .status
                .success()
        );
    }
    for input in [b"invalid".as_slice(), b"{}", &[b'x'; 8193]] {
        assert!(
            !host_budget_command(Some(&url), &args, input)
                .status
                .success()
        );
    }
    assert!(
        !host_budget_command(None, &args, request.to_string().as_bytes())
            .status
            .success()
    );
    let unavailable = "postgres://sensitive-user:sensitive-password@127.0.0.1:1/unavailable";
    let rejected = host_budget_command(Some(unavailable), &args, request.to_string().as_bytes());
    assert!(!rejected.status.success());
    assert!(!String::from_utf8_lossy(&rejected.stderr).contains("sensitive"));
    assert!(
        !host_budget_command(Some(&url), &["increase"], b"")
            .status
            .success()
    );
    assert_eq!(
        budget_store::inspect(&pool, 1).await.unwrap().limits,
        after.limits
    );
    pool.close().await;
}

async fn last_reserved_turn_can_finish() {
    let pool = fixture().await;
    sqlx::query(
        "UPDATE requirement_budget SET limits='{\"tokens\":100,\"turns\":1,\"model_seconds\":100}'",
    )
    .execute(&pool)
    .await
    .unwrap();
    let last = call("first", "last", 60);
    assert_eq!(
        budget_store::reserve(&pool, &last).await.unwrap(),
        Admission::Reserved
    );
    for complete in [false, true] {
        budget_store::settle(
            &pool,
            &last.key,
            "last",
            &format!("observed-{complete}"),
            &usage(10, 5, 3, complete),
        )
        .await
        .unwrap();
        assert!(!budget_store::inspect(&pool, 1).await.unwrap().exhausted);
        assert!(run_store::actions_allowed(&pool, &last.key).await.unwrap());
    }
    assert_eq!(
        budget_store::reserve(&pool, &call("first", "extra", 1))
            .await
            .unwrap(),
        Admission::Blocked
    );
    assert!(budget_store::inspect(&pool, 1).await.unwrap().exhausted);
    pool.close().await;
}

async fn preparation_history_keeps_quota(pool: &PgPool) {
    sqlx::raw_sql("UPDATE agent_run SET quiescent=true,state='Interrupted'; UPDATE execution_control SET requirement_id=NULL; UPDATE requirement SET state='Ready'").execute(pool).await.unwrap();
    let launch = Launch {
        key: key("probe"),
        workspace: "/tmp".into(),
        workspace_identity: "proof".into(),
        program: "/bin/true".into(),
        args: vec![],
    };
    assert!(
        preparation_store::begin(pool, &launch, 1, 1, "preparation", 0)
            .await
            .unwrap()
    );
    preparation_store::failed_probe(
        pool,
        &launch,
        Failure::new("preparation_dependency_missing", "raw error", "fixture"),
        0,
    )
    .await
    .unwrap();
    preparation_store::authorize_retry(pool, "probe", 10, "user reconciled probe")
        .await
        .unwrap();
    assert!(
        preparation_store::begin(pool, &launch, 1, 1, "preparation", 10)
            .await
            .unwrap()
    );
    let row: Value = sqlx::query_scalar(
        "SELECT event FROM preparation_history WHERE run_id='probe' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(row["retry"]["attempts"], 2);
    assert_eq!(row["budget"]["used"]["tokens"], 215);
    assert_eq!(row["budget"]["limits"]["tokens"], 200);
    assert!(row["budget"]["exhausted"].as_bool().unwrap());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM preparation_record")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(count, 1, "no records for disabled stages");
}

async fn corrupted_accounting_fails_closed(pool: &PgPool) {
    sqlx::query("UPDATE requirement_budget SET limits='null'")
        .execute(pool)
        .await
        .unwrap();
    assert!(budget_store::inspect(pool, 1).await.is_err());
    sqlx::query("UPDATE requirement_budget SET limits=$1")
        .bind(json!(amount(i64::MAX, 10, 100)))
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("UPDATE model_call SET usage=$1")
        .bind(json!(usage(i64::MAX, 1, 0, true)))
        .execute(pool)
        .await
        .unwrap();
    assert!(budget_store::inspect(pool, 1).await.is_err());
    sqlx::query("UPDATE model_call SET usage=$1,reserved=$2")
        .bind(json!(Usage::default()))
        .bind(json!(amount(i64::MAX, 1, 10)))
        .execute(pool)
        .await
        .unwrap();
    assert!(budget_store::inspect(pool, 1).await.is_err());
}
