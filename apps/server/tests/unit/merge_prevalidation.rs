use super::*;
use crate::merge_test_support::fixture;

#[tokio::test]
async fn authorization_is_rechecked_before_marking_test_merge_started() {
    let (pool, root, intent, mut remote) = fixture::fixture().await;
    crate::merge_worker::tick(&pool, &mut remote, 100)
        .await
        .unwrap();
    assert!(validation_allowed(&pool, &intent).await.unwrap());
    sqlx::query("UPDATE requirement SET paused=true")
        .execute(&pool)
        .await
        .unwrap();
    assert!(!validation_allowed(&pool, &intent).await.unwrap());
    assert!(!begin_validation(&pool, &intent).await.unwrap());
    let started: bool = sqlx::query_scalar("SELECT pre_validation_started FROM merge_operation")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(!started);
    sqlx::query("UPDATE requirement SET paused=false")
        .execute(&pool)
        .await
        .unwrap();
    assert!(begin_validation(&pool, &intent).await.unwrap());
    let started: bool = sqlx::query_scalar("SELECT pre_validation_started FROM merge_operation")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(started);
    pool.close().await;
    std::fs::remove_dir_all(root).unwrap();
}
