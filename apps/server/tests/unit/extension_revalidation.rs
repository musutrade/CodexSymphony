use super::*;
use crate::merge_test_support::fixture;
use crate::merge_test_support::fixture::source;

#[tokio::test]
async fn restart_reuses_successor_and_rechecks_plan_and_authority() {
    let (root, _, mut plan) = source::fixture();
    assert!(
        check_plan(
            &json!({"command":{"action":{"plan_digest":"different"}}}),
            &plan
        )
        .is_err()
    );
    let digest = plan.identity().unwrap().config_sha256;
    assert!(check_plan(&json!({"command":{"action":{"plan_digest":digest}}}), &plan).is_ok());
    plan.entry = root.join("absent-entry");
    assert!(check_plan(&json!({}), &plan).is_err());
    let (pool, _, _, _) = fixture::fixture().await;
    // Synthetic interrupted intent; no process completion is asserted here.
    sqlx::query("INSERT INTO recovery_failure(event_key,requirement_id,source_validation_id,phase,facts,fingerprint,decision,reason,resolution_state,successor_validation) VALUES('restart',1,'validation','local','{}','fixture','blocked','fixture','running','validation')").execute(&pool).await.unwrap();
    assert_eq!(
        claim(&pool, "restart", "validation", &plan).await.unwrap(),
        "validation"
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM candidate_validation")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    sqlx::query("UPDATE requirement SET paused=true")
        .execute(&pool)
        .await
        .unwrap();
    assert!(claim(&pool, "restart", "validation", &plan).await.is_err());
    pool.close().await;
}
