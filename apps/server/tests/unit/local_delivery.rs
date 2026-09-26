use super::*;

#[tokio::test]
async fn direct_adapter_dispatch_requires_current_local_authority() {
    let _serial = crate::linked_repair_worker::tests::SERIAL.lock().await;
    let (pool, root, broker, _, _) = crate::linked_repair_worker::tests::setup().await;
    let job = Job {
        action_key: "unregistered-local-operation".into(),
        validation_id: "unregistered".into(),
        requirement_id: 1,
        revision: 1,
        head_sha: "0".repeat(40),
        manifest: serde_json::Value::Null,
        policy: serde_json::Value::Null,
        local_binding: serde_json::Value::Null,
        local_acceptance_started: false,
        local_acceptance: None,
        state: "unknown".into(),
        attempts: 1,
    };
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM delivery_attempt")
        .fetch_one(&pool)
        .await
        .unwrap();
    let mut adapter = Local {
        pool: &pool,
        broker: &broker,
    };
    let result = adapter.submit(&job).await;
    assert!(matches!(result, Err(error) if error.to_string().contains("authority changed")));
    let control = Admission {
        pool: &pool,
        root: &root,
        attempt: None,
    };
    assert!(control.check(&Operation::Submit, &job).is_ok());
    assert!(control.check(&Operation::Reconcile, &job).is_ok());
    assert!(control.check(&Operation::BeforeDeliver, &job).is_err());
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM delivery_attempt")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(before, after);
    pool.close().await;
}
