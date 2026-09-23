use super::*;
use crate::linked_repair_worker::tests::setup;
#[tokio::test]
async fn repair_acceptance_retains_original_failure_and_integration_never_becomes_done_from_merge()
{
    let _serial = crate::linked_repair_worker::tests::SERIAL.lock().await;
    let (pool, root, _, config, id) = setup().await;
    let intent: serde_json::Value = sqlx::query_scalar("SELECT intent FROM merge_operation")
        .fetch_one(&pool)
        .await
        .unwrap();
    let intent: Intent = serde_json::from_value(intent).unwrap();
    let evidence = crate::merge_test_support::fixture::evidence(
        &root,
        &root.join("repo"),
        config.validation.as_ref().unwrap(),
    );
    let mut tx = crate::run_store::lock(&pool).await.unwrap();
    assert!(!finish(&mut tx, &intent, &evidence).await.unwrap());
    tx.commit().await.unwrap();
    sqlx::query("UPDATE linked_failure SET state='reserved',repair_delivery=$2 WHERE id=$1")
        .bind(&id)
        .bind(&intent.delivery_key)
        .execute(&pool)
        .await
        .unwrap();
    let original: serde_json::Value = sqlx::query_scalar("SELECT evidence FROM linked_failure")
        .fetch_one(&pool)
        .await
        .unwrap();
    let mut tx = crate::run_store::lock(&pool).await.unwrap();
    assert!(!finish(&mut tx, &intent, &evidence).await.unwrap());
    tx.commit().await.unwrap();
    let (state, saved): (String, serde_json::Value) =
        sqlx::query_as("SELECT state,evidence FROM linked_failure")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(state, "complete");
    assert_eq!(saved, original);
    let mut tx = crate::run_store::lock(&pool).await.unwrap();
    finish_integration(&mut tx, &intent).await.unwrap();
    tx.commit().await.unwrap();
    let state: String = sqlx::query_scalar("SELECT state FROM requirement WHERE id=1")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(state, "Running");
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM group_completion")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    pool.close().await;
    std::fs::remove_dir_all(root).unwrap();
}
