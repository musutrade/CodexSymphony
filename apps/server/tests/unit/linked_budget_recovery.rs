use super::*;

async fn fixture() -> (PgPool, Command) {
    let (pool, _, _, _, failure) = crate::linked_repair_worker::tests::setup().await;
    sqlx::query("UPDATE requirement SET state='Submitted',paused=true WHERE id=1")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE linked_failure SET state='blocked',blocker='shared item or parent repair budget exhausted' WHERE id=$1").bind(&failure).execute(&pool).await.unwrap();
    let version: i64 =
        sqlx::query_scalar("SELECT version FROM requirement_budget WHERE requirement_id=1")
            .fetch_one(&pool)
            .await
            .unwrap();
    crate::budget_store::increase(
        &pool,
        &crate::budget_store::Increase {
            request_id: "reviewed-repair-increase".into(),
            requirement_id: 1,
            expected_version: version,
            actor: "operator".into(),
            reason: "explicit original-account increase".into(),
            delta: Amount {
                tokens: 10,
                turns: 0,
                model_seconds: 0,
            },
        },
    )
    .await
    .unwrap();
    let current: i64 = sqlx::query_scalar("SELECT version FROM requirement WHERE id=1")
        .fetch_one(&pool)
        .await
        .unwrap();
    (
        pool,
        Command {
            request_id: "recover-original-repair".into(),
            requirement_id: 1,
            version: current,
            failure_id: failure,
            budget_version: version + 1,
            authorization_request_id: "reviewed-repair-increase".into(),
        },
    )
}

#[tokio::test]
async fn explicit_recheck_preserves_account_evidence_and_is_once_per_grant() {
    let _serial = crate::linked_repair_worker::tests::SERIAL.lock().await;
    let (pool, command) = fixture().await;
    let before: Value = sqlx::query_scalar("SELECT jsonb_build_object('budget',(SELECT to_jsonb(b) FROM requirement_budget b WHERE requirement_id=1),'evidence',(SELECT evidence FROM linked_failure LIMIT 1),'calls',(SELECT count(*) FROM model_call),'reservations',(SELECT count(*) FROM repair_reservation))").fetch_one(&pool).await.unwrap();
    let result = recheck(&pool, &command).await.unwrap();
    assert_eq!(result["started"], false);
    assert_eq!(recheck(&pool, &command).await.unwrap(), result);
    crate::budget_admin::dispatch(
        &["repair-recheck".into()],
        &serde_json::to_vec(&command).unwrap(),
        &pool,
    )
    .await
    .unwrap();
    assert!(
        crate::budget_admin::dispatch(&["repair-recheck".into()], b"{}", &pool)
            .await
            .is_err()
    );
    let after: Value = sqlx::query_scalar("SELECT jsonb_build_object('budget',(SELECT to_jsonb(b) FROM requirement_budget b WHERE requirement_id=1),'evidence',(SELECT evidence FROM linked_failure LIMIT 1),'calls',(SELECT count(*) FROM model_call),'reservations',(SELECT count(*) FROM repair_reservation))").fetch_one(&pool).await.unwrap();
    assert_eq!(before, after);
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT state FROM linked_failure LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap(),
        "observed"
    );
    let mut changed = command.clone();
    changed.budget_version += 1;
    assert!(recheck(&pool, &changed).await.is_err());
    sqlx::query("UPDATE linked_failure SET state='blocked',blocker='shared item or parent repair budget exhausted'").execute(&pool).await.unwrap();
    changed = command.clone();
    changed.request_id = "second-use-of-grant".into();
    assert!(recheck(&pool, &changed).await.is_err());
}

#[tokio::test]
async fn unsafe_or_unbound_rechecks_do_not_reopen_failures() {
    let _serial = crate::linked_repair_worker::tests::SERIAL.lock().await;
    let (pool, command) = fixture().await;
    let mut invalid = command.clone();
    invalid.request_id = "".into();
    assert!(recheck(&pool, &invalid).await.is_err());
    invalid = command.clone();
    invalid.version += 1;
    assert!(recheck(&pool, &invalid).await.is_err());
    invalid = command.clone();
    invalid.budget_version -= 1;
    assert!(recheck(&pool, &invalid).await.is_err());
    invalid = command.clone();
    invalid.authorization_request_id = "unreviewed".into();
    assert!(recheck(&pool, &invalid).await.is_err());
    sqlx::raw_sql("UPDATE requirement SET paused=false; UPDATE execution_control SET paused=false")
        .execute(&pool)
        .await
        .unwrap();
    assert!(recheck(&pool, &command).await.is_err());
    sqlx::raw_sql("UPDATE requirement SET paused=true; UPDATE linked_failure SET blocker='source preparation failed'").execute(&pool).await.unwrap();
    assert!(recheck(&pool, &command).await.is_err());
    sqlx::raw_sql("UPDATE linked_failure SET blocker='shared item or parent repair budget exhausted'; UPDATE budget_authorization SET delta='{}' WHERE request_id='reviewed-repair-increase'").execute(&pool).await.unwrap();
    assert!(recheck(&pool, &command).await.is_err());
    sqlx::query("UPDATE budget_authorization SET delta='{\"tokens\":0,\"turns\":0,\"model_seconds\":0}' WHERE request_id='reviewed-repair-increase'").execute(&pool).await.unwrap();
    assert!(recheck(&pool, &command).await.is_err());
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT state FROM linked_failure LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap(),
        "blocked"
    );
}
