use super::*;
use crate::merge_test_support::fixture;

#[tokio::test]
async fn pause_at_acceptance_start_never_marks_execution_started() {
    let (pool, root, intent, mut remote) = fixture::fixture().await;
    crate::merge_worker::tick(&pool, &mut remote, 100)
        .await
        .unwrap();
    sqlx::raw_sql("UPDATE merge_operation SET state='merged'; UPDATE requirement SET paused=true")
        .execute(&pool)
        .await
        .unwrap();
    assert!(!begin_acceptance(&pool, &intent).await.unwrap());
    let started: bool = sqlx::query_scalar("SELECT acceptance_started FROM merge_operation")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(!started);
    sqlx::query("UPDATE requirement SET paused=false")
        .execute(&pool)
        .await
        .unwrap();
    assert!(begin_acceptance(&pool, &intent).await.unwrap());
    let started: bool = sqlx::query_scalar("SELECT acceptance_started FROM merge_operation")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(started);
    let mut absent = intent.clone();
    absent.policy.delivery = None;
    assert_eq!(post_wait(&absent), 0);
    assert_eq!(post_wait(&intent), 900);
    if let crate::github_contract::PostMerge::FixedValidation { wait_seconds, .. } = &mut absent
        .policy
        .delivery
        .insert(intent.policy.delivery.unwrap())
        .post_merge
    {
        *wait_seconds = u64::MAX;
    }
    assert_eq!(post_wait(&absent), i64::MAX);
    pool.close().await;
    std::fs::remove_dir_all(root).unwrap();
}
