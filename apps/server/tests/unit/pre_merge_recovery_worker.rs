use super::*;
use crate::pre_merge_recovery::{self, tests::fixture};
use serde_json::json;
struct Local {
    root: std::path::PathBuf,
    head: String,
}
impl Remote for Local {
    async fn branch(&mut self, _: &Policy) -> Result<String> {
        Ok(self.head.clone())
    }
    async fn fetch(&mut self, _: &Policy, path: &Path, sha: &str) -> Result<()> {
        assert!(
            std::process::Command::new("git")
                .arg("-C")
                .arg(path)
                .arg("fetch")
                .arg(&self.root)
                .arg(sha)
                .status()?
                .success()
        );
        Ok(())
    }
}
#[tokio::test]
async fn recovery_freezes_current_target_without_rewriting_original_candidate() {
    let _serial = crate::linked_repair_worker::tests::SERIAL.lock().await;
    let (pool, root, broker, command) = fixture().await;
    pre_merge_recovery::decide(&pool, 1, &command)
        .await
        .unwrap();
    let mut remote = Local {
        root: root.join("repo"),
        head: command.base.clone(),
    };
    advance(&pool, &mut remote, &root, 100).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT attempts::bigint FROM pre_merge_recovery")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    sqlx::query("UPDATE requirement SET paused=false")
        .execute(&pool)
        .await
        .unwrap();
    let pending = claim(&pool, 100).await.unwrap().unwrap();
    prepare(&pool, &mut remote, &root, &pending).await.unwrap();
    let (state,failure,baseline):(String,String,String)=sqlx::query_as("SELECT p.state,p.failure_id,f.baseline FROM pre_merge_recovery p JOIN linked_failure f ON f.id=p.failure_id").fetch_one(&pool).await.unwrap();
    assert_eq!(state, "ready");
    assert_eq!(baseline, command.base);
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT evidence->>'kind' FROM linked_failure")
            .fetch_one(&pool)
            .await
            .unwrap(),
        "pre_merge_recovery"
    );
    prepare(&pool, &mut remote, &root, &pending).await.unwrap(); // No duplicate source or reservation.
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM linked_failure")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    let mut tx = crate::run_store::lock(&pool).await.unwrap();
    assert!(
        pre_merge_recovery::admitted(&mut tx, &failure)
            .await
            .unwrap()
    );
    tx.rollback().await.unwrap();
    sqlx::query("UPDATE github_pr SET observation=jsonb_set(observation,'{closed}','false')")
        .execute(&pool)
        .await
        .unwrap();
    let mut tx = crate::run_store::lock(&pool).await.unwrap();
    assert!(
        !pre_merge_recovery::admitted(&mut tx, &failure)
            .await
            .unwrap()
    );
    tx.rollback().await.unwrap();
    sqlx::query("UPDATE github_pr SET observation=jsonb_set(observation,'{closed}','true')")
        .execute(&pool)
        .await
        .unwrap();
    // Separate synthetic validation identity: the original evidence is never reassigned.
    sqlx::query("INSERT INTO candidate_validation SELECT (jsonb_populate_record(NULL::candidate_validation,to_jsonb(v)||jsonb_build_object('id','replacement-validation','candidate_sha',repeat('f',40)))).* FROM candidate_validation v WHERE id='validation'").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO delivery(action_key,validation_id,requirement_id,revision,repository_id,repository,branch,base_branch,head_sha,manifest,policy) SELECT 'replacement','replacement-validation',requirement_id,revision,repository_id,repository,'ai/replacement',base_branch,repeat('f',40),manifest,policy FROM delivery WHERE action_key<>'replacement'").execute(&pool).await.unwrap();
    sqlx::query("UPDATE linked_failure SET repair_delivery='replacement' WHERE id=$1")
        .bind(&failure)
        .execute(&pool)
        .await
        .unwrap();
    let mut tx = crate::run_store::lock(&pool).await.unwrap();
    assert!(
        pre_merge_recovery::delivery_allowed(&mut tx, "replacement")
            .await
            .unwrap()
    );
    pre_merge_recovery::supersede(&mut tx, "replacement")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT superseded_by FROM delivery WHERE action_key<>'replacement'"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        "replacement"
    );
    sqlx::query("UPDATE github_pr SET last_synced_at=0")
        .execute(&pool)
        .await
        .unwrap();
    let mut tx = crate::run_store::lock(&pool).await.unwrap();
    assert!(
        !pre_merge_recovery::delivery_allowed(&mut tx, "replacement")
            .await
            .unwrap()
    );
    tx.rollback().await.unwrap();
    sqlx::query("UPDATE pre_merge_recovery SET state='blocked'")
        .execute(&pool)
        .await
        .unwrap();
    let mut tx = crate::run_store::lock(&pool).await.unwrap();
    assert!(
        !pre_merge_recovery::admitted(&mut tx, &failure)
            .await
            .unwrap()
    );
    tx.rollback().await.unwrap();
    // No external merge or model call is manufactured by preparation.
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM model_call")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    let candidate = sqlx::query_scalar::<_, String>(
        "SELECT head_sha FROM delivery WHERE action_key<>'replacement'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(candidate, command.head);
    drop(broker);
    pool.close().await;
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn changed_target_and_retries_preserve_original_identity() {
    let _serial = crate::linked_repair_worker::tests::SERIAL.lock().await;
    let (pool, root, _, command) = fixture().await;
    pre_merge_recovery::decide(&pool, 1, &command)
        .await
        .unwrap();
    sqlx::query("UPDATE requirement SET paused=false")
        .execute(&pool)
        .await
        .unwrap();
    let mut remote = Local {
        root: root.join("repo"),
        head: "f".repeat(40),
    };
    for now in [100, 130, 160, 190] {
        advance(&pool, &mut remote, &root, now).await.unwrap();
    }
    let (state, attempts): (String, i32) =
        sqlx::query_as("SELECT state,attempts FROM pre_merge_recovery")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((state.as_str(), attempts), ("blocked", 3));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM linked_failure")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    assert!(branch_sha(&json!({})).is_err());
    assert_eq!(
        branch_sha(&json!({"object":{"sha":command.base}})).unwrap(),
        command.base
    );
    pool.close().await;
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn interrupted_preparation_exhausts_durably_and_requires_a_new_decision() {
    let _serial = crate::linked_repair_worker::tests::SERIAL.lock().await;
    let (pool, root, _, mut command) = fixture().await;
    pre_merge_recovery::decide(&pool, 1, &command)
        .await
        .unwrap();
    sqlx::query("UPDATE requirement SET paused=false")
        .execute(&pool)
        .await
        .unwrap();
    // Simulate restart after the durable claim, before a preparation outcome exists.
    for now in [100, 130, 160] {
        assert!(claim(&pool, now).await.unwrap().is_some());
    }
    let mut remote = Local {
        root: root.join("repo"),
        head: command.base.clone(),
    };
    advance(&pool, &mut remote, &root, 189).await.unwrap();
    assert_eq!(
        pre_merge_recovery::view(&pool, 1).await.unwrap()["recoveries"][0]["state"],
        "pending"
    );
    advance(&pool, &mut remote, &root, 190).await.unwrap();
    advance(&pool, &mut remote, &root, 220).await.unwrap();
    let retained = pre_merge_recovery::view(&pool, 1).await.unwrap();
    let row = &retained["recoveries"][0];
    assert_eq!(row["state"], "blocked");
    assert_eq!(row["attempts"], 3);
    assert_eq!(row["receipts"].as_array().unwrap().len(), 1);
    assert!(row["failure_id"].is_null());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM model_call")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    // Re-review preserves the previous decision and its exhausted attempt count.
    command.request_id = "explicit-interrupted-recovery".into();
    command.version = sqlx::query_scalar("UPDATE requirement SET paused=true RETURNING version")
        .fetch_one(&pool)
        .await
        .unwrap();
    pre_merge_recovery::decide(&pool, 1, &command)
        .await
        .unwrap();
    let reopened = pre_merge_recovery::view(&pool, 1).await.unwrap();
    let row = &reopened["recoveries"][0];
    assert_eq!(row["state"], "pending");
    assert_eq!(row["attempts"], 0);
    assert_eq!(row["receipts"][1]["previous_attempts"], 3);
    pool.close().await;
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn native_transport_reads_authenticated_branch_and_fails_closed() {
    let _serial = crate::linked_repair_worker::tests::SERIAL.lock().await;
    let (pool, root, _, command) = fixture().await;
    let policy: Policy = serde_json::from_value(
        sqlx::query_scalar::<_, Value>("SELECT intent->'policy' FROM merge_operation")
            .fetch_one(&pool)
            .await
            .unwrap(),
    )
    .unwrap();
    let expected = command.base.clone();
    let router=axum::Router::new().fallback(move |request:axum::extract::Request| {
        let expected=expected.clone(); async move {
            assert!(request.headers().contains_key("authorization"));
            axum::Json(match request.uri().path() {
                "/repos/owner/repo/installation"=>json!({"id":7,"app_id":1042}),
                "/app/installations/7/access_tokens"=>json!({"token":"disposable-fixture","expires_at":"2099-01-01T00:00:00Z","permissions":{}}),
                "/repos/owner/repo/git/ref/heads/%6D%61%69%6E"=>json!({"object":{"sha":expected}}),
                other=>panic!("unexpected request {other}")
            })
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let _ = crate::merge_test_support::client::client(&root);
    let mut client = AppClient::new(
        &url,
        1042,
        &std::fs::read(root.join("unit-key.pem")).unwrap(),
    )
    .unwrap();
    let mut remote = Broker {
        client: &mut client,
        now: crate::github_service::now(),
    };
    assert_eq!(remote.branch(&policy).await.unwrap(), command.base);
    assert!(remote.fetch(&policy, &root, "invalid").await.is_err());
    tick(&pool, &mut client, &root, 100).await.unwrap();
    task.abort();
    pool.close().await;
    std::fs::remove_dir_all(root).unwrap();
}
