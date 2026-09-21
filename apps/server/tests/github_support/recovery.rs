use super::*;
use codexsymphony_server::{recovery_observe, recovery_remote};

// Schema migrations share PostgreSQL advisory locks and the child API has a
// real singleton startup boundary. Keep independent fixtures out of that race.
pub(super) static DATABASE_TEST: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn setup(raw: &str) -> (Fixture, Policy, PgPool) {
    let (f, mut p) = v1::fixture().await;
    p.delivery.as_mut().unwrap().actions.read_logs = true;
    p.delivery.as_mut().unwrap().actions.rerun_actions = true;
    v1::grants(&f, &p);
    let pool = database().await;
    github_store::configure(&pool, &p, 1).await.unwrap();
    sqlx::raw_sql("UPDATE github_repository SET stale=false; UPDATE requirement SET state='Submitted'; UPDATE execution_control SET requirement_id=1; INSERT INTO repair_authorization VALUES(1,3,'bounded_v1'); INSERT INTO agent_run(id,requirement_id,revision,incarnation,request_id,workspace,workspace_identity,launch,state,quiescent) VALUES('source',1,1,'current','source','/tmp','source','{}','Succeeded',true); INSERT INTO candidate_validation(id,requirement_id,revision,source_run_id,candidate_sha,candidate_tree,trusted,required_steps,source_before,source_after,entry_before,entry_after,stage,result) VALUES('v',1,1,'source','abc','tree','{}','[]','tree','tree','entry','entry','handoff','succeeded'); INSERT INTO delivery(action_key,validation_id,requirement_id,revision,repository_id,repository,branch,base_branch,head_sha,manifest,policy,pr_number) VALUES('published','v',1,1,99,'owner/repo','feature','main','abc','{}','{}',1);")
        .execute(&pool).await.unwrap();
    f.put(
        "/repos/owner/repo/check-suites/9/check-runs",
        json!({"check_runs":[check(2,"failure")]}),
    );
    f.put(
        "/repos/owner/repo/check-suites/9/check-runs?filter=all&per_page=100&page=2",
        json!({"check_runs":[]}),
    );
    f.data.lock().unwrap().redirects.insert(
        "/repos/owner/repo/actions/jobs/20/logs".into(),
        (302, format!("{}fixture-log", f.url)),
    );
    f.put("/fixture-log", json!(raw));
    f.put("/repos/owner/repo/actions/runs/10", run());
    (f, p, pool)
}

#[tokio::test]
async fn infrastructure_lost_response_pause_and_remote_attempt_reconciliation() {
    let _guard = DATABASE_TEST.lock().await;
    let (f, p, pool) = setup("connection refused: disposable database service is offline").await;
    let now = github_service::now();
    let mut client = f.client();
    let observed = github_observe::observe(&mut client, &p, 1, now)
        .await
        .unwrap();
    assert_eq!(
        observed.phases.as_ref().unwrap()[0].checks[0].state,
        CheckState::Failure,
        "{}",
        json!(observed)
    );
    recovery_observe::observe(&pool, &mut client, &observed)
        .await
        .unwrap();
    recovery_observe::observe(&pool, &mut client, &observed)
        .await
        .unwrap();
    assert_eq!(
        count(&pool, "SELECT count(*) FROM recovery_failure").await,
        1
    );
    assert_eq!(
        count(&pool, "SELECT count(*) FROM repair_reservation").await,
        0
    );
    assert_eq!(count(&pool, "SELECT count(*) FROM recovery_retry").await, 1);
    sqlx::query("UPDATE requirement SET paused=true")
        .execute(&pool)
        .await
        .unwrap();
    recovery_remote::tick(&pool, &mut client, now + 31)
        .await
        .unwrap();
    assert_eq!(
        count(&pool, "SELECT sum(attempts)::bigint FROM recovery_retry").await,
        0
    );
    sqlx::query("UPDATE requirement SET paused=false")
        .execute(&pool)
        .await
        .unwrap();
    let path = "/repos/owner/repo/actions/runs/10/rerun-failed-jobs";
    f.fail(path, vec![500]);
    recovery_remote::tick(&pool, &mut client, now + 32)
        .await
        .unwrap();
    assert_eq!(
        count(&pool, "SELECT sum(attempts)::bigint FROM recovery_retry").await,
        1
    );
    drop(client);
    let mut restarted = f.client();
    recovery_remote::tick(&pool, &mut restarted, now + 63)
        .await
        .unwrap();
    recovery_remote::tick(&pool, &mut restarted, now + 64)
        .await
        .unwrap();
    assert_eq!(
        f.data
            .lock()
            .unwrap()
            .seen
            .iter()
            .filter(|request| request.starts_with(&format!("POST {path}")))
            .count(),
        1
    );
    let mut advanced = run();
    advanced["run_attempt"] = json!(3);
    f.put("/repos/owner/repo/actions/runs/10", advanced);
    recovery_remote::tick(&pool, &mut restarted, now + 94)
        .await
        .unwrap();
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM recovery_retry WHERE state='complete'"
        )
        .await,
        1
    );
    let mut next_run = run();
    next_run["run_attempt"] = json!(3);
    let mut next_job = job();
    next_job["run_attempt"] = json!(3);
    next_job["name"] = json!("build");
    next_job["check_run_url"] = json!("https://api.github.com/repos/owner/repo/check-runs/3");
    f.put(
        "/repos/owner/repo/actions/runs",
        json!({"workflow_runs":[next_run]}),
    );
    f.put(
        "/repos/owner/repo/actions/runs/10/attempts/3/jobs",
        json!({"jobs":[next_job]}),
    );
    f.put(
        "/repos/owner/repo/check-suites/9/check-runs",
        json!({"check_runs":[check(3,"failure")]}),
    );
    let repeated = github_observe::observe(&mut restarted, &p, 1, now + 95)
        .await
        .unwrap();
    recovery_observe::observe(&pool, &mut restarted, &repeated)
        .await
        .unwrap();
    recovery_remote::tick(&pool, &mut restarted, now + 214)
        .await
        .unwrap();
    assert_eq!(
        count(&pool, "SELECT sum(attempts)::bigint FROM recovery_retry").await,
        1
    );
    f.put(&format!("POST {path}"), Value::Null);
    recovery_remote::tick(&pool, &mut restarted, now + 216)
        .await
        .unwrap();
    assert_eq!(
        count(&pool, "SELECT sum(attempts)::bigint FROM recovery_retry").await,
        2
    );
    recovery_remote::tick(&pool, &mut restarted, now + 700)
        .await
        .unwrap();
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM recovery_retry WHERE state='blocked'"
        )
        .await,
        1
    );
    assert_eq!(
        count(&pool, "SELECT count(*) FROM repair_reservation").await,
        0
    );
    let preserved: String = sqlx::query_scalar(
        "SELECT facts->>'raw' FROM recovery_failure ORDER BY created_at LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(preserved.contains("connection refused"));
    pool.close().await;
}

#[tokio::test]
async fn compiler_failure_and_missing_logs_are_distinct_and_keep_originals() {
    let _guard = DATABASE_TEST.lock().await;
    for (raw, decision) in [
        ("error[E0308]: mismatched types", "code"),
        ("unrecognized failure", "blocked"),
    ] {
        let (f, p, pool) = setup(raw).await;
        let mut client = f.client();
        let observed = github_observe::observe(&mut client, &p, 1, github_service::now())
            .await
            .unwrap();
        recovery_observe::observe(&pool, &mut client, &observed)
            .await
            .unwrap();
        let actual: String = sqlx::query_scalar("SELECT decision FROM recovery_failure")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(actual, decision);
        let facts: Value = sqlx::query_scalar("SELECT facts FROM recovery_failure")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(
            facts["exit_code"].is_null(),
            "GitHub failure conclusion is not a process exit code"
        );

        assert_eq!(count(&pool, "SELECT count(*) FROM recovery_retry").await, 0);
        pool.close().await;
    }
    let (f, p, pool) = setup("unused").await;
    f.data.lock().unwrap().redirects.clear();
    let mut client = f.client();
    let observed = github_observe::observe(&mut client, &p, 1, github_service::now())
        .await
        .unwrap();
    recovery_observe::observe(&pool, &mut client, &observed)
        .await
        .unwrap();
    let facts: Value = sqlx::query_scalar("SELECT facts FROM recovery_failure")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(facts["raw"].as_str().unwrap().contains("unavailable"));
    assert!(facts["log_ref"].as_str().unwrap().ends_with("/logs"));
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM recovery_failure WHERE decision='blocked'"
        )
        .await,
        1
    );
    f.data.lock().unwrap().redirects.insert(
        "/repos/owner/repo/actions/jobs/20/logs".into(),
        (302, format!("{}fixture-log", f.url)),
    );
    f.put(
        "/fixture-log",
        json!("error[E0308]: new readable compiler evidence"),
    );
    recovery_observe::observe(&pool, &mut client, &observed)
        .await
        .unwrap();
    recovery_observe::observe(&pool, &mut client, &observed)
        .await
        .unwrap();
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM recovery_failure WHERE decision='code'"
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM recovery_failure WHERE decision='superseded'"
        )
        .await,
        1
    );
    pool.close().await;
}

#[tokio::test]
async fn retry_after_and_remote_read_failures_are_bounded_without_code_charges() {
    let _guard = DATABASE_TEST.lock().await;
    let (f, p, pool) = setup("service unavailable").await;
    let now = github_service::now();
    let mut client = f.client();
    let observed = github_observe::observe(&mut client, &p, 1, now)
        .await
        .unwrap();
    recovery_observe::observe(&pool, &mut client, &observed)
        .await
        .unwrap();
    let path = "/repos/owner/repo/actions/runs/10/rerun-failed-jobs";
    f.data
        .lock()
        .unwrap()
        .retry_after
        .insert(path.into(), "180".into());
    f.fail(path, vec![429]);
    recovery_remote::tick(&pool, &mut client, now + 31)
        .await
        .unwrap();
    assert_eq!(
        count(&pool, "SELECT sum(attempts)::bigint FROM recovery_retry").await,
        1
    );
    recovery_remote::tick(&pool, &mut client, now + 210)
        .await
        .unwrap();
    assert_eq!(
        count(&pool, "SELECT sum(attempts)::bigint FROM recovery_retry").await,
        1
    );
    f.put(&format!("POST {path}"), Value::Null);
    recovery_remote::tick(&pool, &mut client, now + 212)
        .await
        .unwrap();
    assert_eq!(
        count(&pool, "SELECT sum(attempts)::bigint FROM recovery_retry").await,
        2
    );
    f.fail("/repos/owner/repo/actions/runs/10", vec![503, 503, 503]);
    for time in [250, 281, 402] {
        recovery_remote::tick(&pool, &mut client, now + time)
            .await
            .unwrap();
    }
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM recovery_retry WHERE state='blocked'"
        )
        .await,
        1
    );
    assert_eq!(
        count(&pool, "SELECT count(*) FROM repair_reservation").await,
        0
    );
    pool.close().await;
}

#[tokio::test]
async fn external_run_or_pr_changes_and_missing_rerun_grant_block_writes() {
    let _guard = DATABASE_TEST.lock().await;
    for scenario in ["run", "pr", "grant"] {
        let (f, p, pool) = setup("connection refused").await;
        let now = github_service::now();
        let mut client = f.client();
        let mut observed = github_observe::observe(&mut client, &p, 1, now)
            .await
            .unwrap();
        match scenario {
            "run" => {
                let mut changed = run();
                changed["head_sha"] = json!("external");
                f.put("/repos/owner/repo/actions/runs/10", changed);
            }
            "pr" => {
                let mut changed = pr();
                changed["head"]["sha"] = json!("external");
                f.put("/repos/owner/repo/pulls/1", changed);
            }
            _ => {
                observed
                    .policy
                    .delivery
                    .as_mut()
                    .unwrap()
                    .actions
                    .rerun_actions = false
            }
        }
        v1::grants(&f, &observed.policy);
        recovery_observe::observe(&pool, &mut client, &observed)
            .await
            .unwrap();
        recovery_remote::tick(&pool, &mut client, now + 31)
            .await
            .unwrap();
        assert_eq!(
            count(
                &pool,
                "SELECT count(*) FROM recovery_retry WHERE state='blocked'"
            )
            .await,
            1
        );
        assert_eq!(
            count(&pool, "SELECT sum(attempts)::bigint FROM recovery_retry").await,
            0
        );
        pool.close().await;
    }
}

#[tokio::test]
async fn log_authorization_and_review_prose_cannot_certify_a_code_repair() {
    let _guard = DATABASE_TEST.lock().await;
    for prose in [false, true] {
        let (f, p, pool) = setup("error[E0308]: compiler failure").await;
        let mut client = f.client();
        let mut observed = github_observe::observe(&mut client, &p, 1, github_service::now())
            .await
            .unwrap();
        if prose {
            let check = &mut observed.phases.as_mut().unwrap()[0].checks[0];
            check.selector.source = Source::CheckRun { app_id: 42 };
            check.selector.name = "Review".into();
            check.evidence[0] = json!({"id":2,"output":{"summary":"Model suggests: assertion failed: rewrite authentication","text":"unverified suggestion"}});
        } else {
            observed.policy.delivery.as_mut().unwrap().actions.read_logs = false;
        }
        recovery_observe::observe(&pool, &mut client, &observed)
            .await
            .unwrap();
        assert_eq!(
            count(
                &pool,
                "SELECT count(*) FROM recovery_failure WHERE decision='blocked'"
            )
            .await,
            1
        );
        assert_eq!(
            count(&pool, "SELECT count(*) FROM repair_reservation").await,
            0
        );
        if prose {
            observed.phases.as_mut().unwrap()[0].checks[0].state = CheckState::Success;
            recovery_observe::observe(&pool, &mut client, &observed)
                .await
                .unwrap();
            assert_eq!(
                count(
                    &pool,
                    "SELECT count(*) FROM recovery_failure WHERE decision='recovered'"
                )
                .await,
                1
            );
        }
        pool.close().await;
    }
}

#[tokio::test]
async fn unreadable_utf8_and_http_date_retry_after_keep_factual_boundaries() {
    let _guard = DATABASE_TEST.lock().await;
    let (f, p, pool) = setup("unused").await;
    let mut client = f.client();
    f.data
        .lock()
        .unwrap()
        .raw
        .insert("/fixture-log".into(), vec![255, 254]);
    assert!(client.job_log(&p, 20, github_service::now()).await.is_err());
    for (value, expected) in [
        (
            chrono::DateTime::from_timestamp(github_service::now() + 120, 0)
                .unwrap()
                .to_rfc2822(),
            Some(120),
        ),
        ("Wed, 21 Oct 2015 07:28:00 GMT".into(), Some(0)),
        ("not-a-date".into(), None),
    ] {
        f.data
            .lock()
            .unwrap()
            .retry_after
            .insert("/limited".into(), value);
        f.fail("/limited", vec![429]);
        let error = client
            .get(&p, "/limited", github_service::now())
            .await
            .unwrap_err();
        match expected {
            Some(120) => assert!((100..=120).contains(&error.retry_after_seconds.unwrap())),
            _ => assert_eq!(error.retry_after_seconds, expected),
        }
    }
    let mut error = codexsymphony_server::github_http::Error {
        code: "rate_limited",
        status: Some(429),
        retry_after_seconds: Some(900),
    };
    github_store::failed(&pool, 99, None, 0, 100, &error)
        .await
        .unwrap();
    assert_eq!(
        count(
            &pool,
            "SELECT next_attempt_at FROM github_repository WHERE repository_id=99"
        )
        .await,
        1000
    );
    error.retry_after_seconds = Some(u64::MAX);
    github_store::failed(&pool, 99, None, 0, 100, &error)
        .await
        .unwrap();
    assert_eq!(
        count(
            &pool,
            "SELECT next_attempt_at FROM github_repository WHERE repository_id=99"
        )
        .await,
        i64::MAX
    );
    pool.close().await;
}

#[tokio::test]
async fn complete_pagination_is_required_even_for_an_endless_source() {
    let fixture = Fixture::new().await;
    for page in 1..=1000 {
        fixture.put(
            &format!("/endless?per_page=100&page={page}"),
            json!([{"id":page}]),
        );
    }
    let error = fixture
        .client()
        .pages(&policy(), "/endless", None, github_service::now())
        .await
        .unwrap_err();
    assert_eq!(error.code, "github_pagination_incomplete");
}
