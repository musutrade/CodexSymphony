use super::*;

pub(crate) async fn fixture() -> (
    PgPool,
    std::path::PathBuf,
    crate::git_broker::GitBroker,
    Decision,
) {
    let (pool, root, broker, _, failure) = crate::linked_repair_worker::tests::setup().await;
    let (merge, head): (String, String) =
        sqlx::query_as("SELECT action_key,intent->>'head' FROM merge_operation")
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query("DELETE FROM linked_failure WHERE id=$1")
        .bind(failure)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE merge_operation SET state='blocked',merged_sha=NULL,acceptance=NULL,merge_started=false,pre_validation_started=false,intent=jsonb_set(intent,'{base}',to_jsonb($1::text)),blocker='test-merge SHA unavailable'").bind(&head).execute(&pool).await.unwrap();
    crate::merge_test_support::reviewed_group(&pool,json!({"child":{"kind":"code_change","repository_id":1},"review":{"repository_version":1,"repair_scope":json!({"schema":"linked-repair/v1","checks":{"test":["source"]}}).to_string()}})).await;
    sqlx::query("UPDATE requirement SET state='Submitted',paused=true")
        .execute(&pool)
        .await
        .unwrap();
    let (version, revision): (i64, i64) =
        sqlx::query_as("SELECT version,revision FROM requirement WHERE id=1")
            .fetch_one(&pool)
            .await
            .unwrap();
    let document: Value = sqlx::query_scalar("SELECT document FROM repository WHERE id=1")
        .fetch_one(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE requirement_revision SET document=$1")
        .bind(json!({"repository_id":1,"repository_version":1,"repository":document}))
        .execute(&pool)
        .await
        .unwrap();
    let intent: crate::automatic_merge::Intent = serde_json::from_value(
        sqlx::query_scalar::<_, Value>("SELECT intent FROM merge_operation")
            .fetch_one(&pool)
            .await
            .unwrap(),
    )
    .unwrap();
    let mut observation =
        crate::merge_test_support::fixture::observation(&intent, crate::github_service::now());
    observation.closed = true;
    sqlx::query("UPDATE github_pr SET stale=false,last_synced_at=extract(epoch FROM now())::bigint,observation=$1").bind(json!(observation)).execute(&pool).await.unwrap();
    let decision = Decision {
        request_id: "pre-merge-fixture".into(),
        version,
        revision,
        merge_key: merge,
        head: head.clone(),
        base: head,
        paths: vec!["source".into()],
        reason: "Recover retained candidate after explicitly closing old PR".into(),
    };
    (pool, root, broker, decision)
}

#[test]
fn bounded_decision_and_original_scope() {
    let mut c = Decision {
        request_id: "request".into(),
        version: 1,
        revision: 1,
        merge_key: "key".into(),
        head: "a".repeat(40),
        base: "b".repeat(40),
        paths: vec!["source".into()],
        reason: "reviewed conflict".into(),
    };
    assert!(validate(&c).is_ok());
    c.head = "g".repeat(40);
    assert!(validate(&c).is_err());
    c.head = "a".repeat(39);
    assert!(!oid(&c.head));
    c.head = "a".repeat(40);
    c.version = 0;
    assert!(validate(&c).is_err());
    c.version = 1;
    c.reason.clear();
    assert!(validate(&c).is_err());
    c.reason = "reviewed".into();
    c.paths.clear();
    assert!(validate(&c).is_err());
    c.paths = vec!["source".into()];
    c.request_id = "".into();
    assert!(validate(&c).is_err());
    let scope = json!({"schema":"linked-repair/v1","checks":{"test":["source"]}}).to_string();
    assert!(validate_paths(&scope, &["source".into()]).is_ok());
    assert!(validate_paths(&scope, &["source".into(), "source".into()]).is_err());
    assert!(validate_paths(&scope, &["other".into()]).is_err());
    assert!(validate_paths("prose", &["source".into()]).is_err());
    assert!(
        decode_error(serde_json::from_str::<Value>("bad").unwrap_err())
            .to_string()
            .contains("retained")
    );
}

#[tokio::test]
async fn exact_closed_pr_request_is_idempotent_and_preserves_account() {
    let _serial = crate::linked_repair_worker::tests::SERIAL.lock().await;
    let (pool, root, _, command) = fixture().await;
    let mut tx = run_store::lock(&pool).await.unwrap();
    assert!(admitted(&mut tx, "ordinary-linked-failure").await.unwrap());
    assert!(delivery_allowed(&mut tx, "unrelated").await.unwrap());
    tx.rollback().await.unwrap();
    let before: Value =
        sqlx::query_scalar("SELECT to_jsonb(b) FROM requirement_budget b WHERE requirement_id=1")
            .fetch_one(&pool)
            .await
            .unwrap();
    let mut bad = command.clone();
    bad.paths = vec!["credentials".into()];
    assert!(decide(&pool, 1, &bad).await.is_err());
    sqlx::query("UPDATE github_pr SET observation=jsonb_set(observation,'{closed}','false')")
        .execute(&pool)
        .await
        .unwrap();
    assert!(decide(&pool, 1, &command).await.is_err());
    sqlx::query("UPDATE github_pr SET observation=jsonb_set(observation,'{closed}','true')")
        .execute(&pool)
        .await
        .unwrap();
    let result = decide(&pool, 1, &command).await.unwrap();
    assert_eq!(result["started"], false);
    assert_eq!(decide(&pool, 1, &command).await.unwrap(), result);
    bad = command.clone();
    bad.reason = "changed".into();
    assert!(decide(&pool, 1, &bad).await.is_err());
    assert_eq!(
        view(&pool, 1).await.unwrap()["recoveries"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let after: Value =
        sqlx::query_scalar("SELECT to_jsonb(b) FROM requirement_budget b WHERE requirement_id=1")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(before, after);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM requirement")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    pool.close().await;
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn http_recovery_rejects_invalid_stale_and_unavailable_requests() {
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    let _serial = crate::linked_repair_worker::tests::SERIAL.lock().await;
    let (pool, root, _, command) = fixture().await;
    let router = crate::pre_merge_recovery_api::routes().with_state(pool.clone());
    let post = |body: String| {
        Request::builder()
            .method("POST")
            .uri("/api/requirements/1/pre-merge-recovery")
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap()
    };
    let get = || {
        Request::builder()
            .uri("/api/requirements/1/pre-merge-recovery")
            .body(Body::empty())
            .unwrap()
    };
    assert_eq!(router.clone().oneshot(get()).await.unwrap().status(), 200);
    assert_eq!(
        router
            .clone()
            .oneshot(post("{}".into()))
            .await
            .unwrap()
            .status(),
        422
    );
    let mut stale = command.clone();
    stale.version += 1;
    assert_eq!(
        router
            .clone()
            .oneshot(post(json!(stale).to_string()))
            .await
            .unwrap()
            .status(),
        409
    );
    assert_eq!(
        router
            .clone()
            .oneshot(post(json!(command).to_string()))
            .await
            .unwrap()
            .status(),
        200
    );
    pool.close().await;
    assert_eq!(router.clone().oneshot(get()).await.unwrap().status(), 503);
    assert_eq!(
        router
            .oneshot(post(json!(command).to_string()))
            .await
            .unwrap()
            .status(),
        503
    );
    std::fs::remove_dir_all(root).unwrap();
}
