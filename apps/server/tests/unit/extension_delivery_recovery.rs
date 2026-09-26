use super::*;
use crate::merge_test_support::fixture;
use serde_json::json;

fn digest(policy: &Policy) -> String {
    crate::validation::sha256(serde_json::to_vec(policy).unwrap())
}

#[tokio::test]
async fn only_explicit_successful_scoped_revalidation_replaces_post_merge_plan() {
    let (pool, _root, mut intent, _) = fixture::fixture().await;
    let original = crate::merge_validation::plan(&pool, &intent).await.unwrap();
    let mut next = original.clone();
    next.steps[0].command.push("reviewed-output".into());
    let action = Action::RevalidateDelivery {
        plan_digest: next.identity().unwrap().config_sha256,
        resume_condition: "same candidate produces complete evidence".into(),
        policy_digest: digest(&intent.policy),
    };
    let mut tx = pool.begin().await.unwrap();
    sqlx::query(
        "UPDATE requirement_revision SET document=document||'{\"repository_id\":1}'::jsonb",
    )
    .execute(&mut *tx)
    .await
    .unwrap();
    authorize(&mut tx, 1, 1, &action).await.unwrap();
    assert!(authorize(&mut tx, 2, 1, &action).await.is_err());
    tx.rollback().await.unwrap();
    assert!(replacement(&pool, &intent).await.unwrap().is_none());
    sqlx::query("UPDATE candidate_validation SET approved_plan=$1,trusted=$2 WHERE id=$3")
        .bind(json!(next))
        .bind(json!(next.identity().unwrap()))
        .bind(&intent.validation_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO recovery_failure(event_key,requirement_id,source_validation_id,phase,facts,fingerprint,decision,reason,resolution,resolution_state,successor_validation) VALUES('scoped',1,'validation','local','{}','fixture','recovered','fixture',$1,'complete','validation')")
        .bind(json!({"command":{"action":action}})).execute(&pool).await.unwrap();
    assert_eq!(
        replacement(&pool, &intent)
            .await
            .unwrap()
            .unwrap()
            .identity()
            .unwrap(),
        next.identity().unwrap()
    );
    assert_eq!(
        crate::merge_validation::plan(&pool, &intent)
            .await
            .unwrap()
            .identity()
            .unwrap(),
        next.identity().unwrap()
    );
    intent.revision = 2;
    assert!(replacement(&pool, &intent).await.unwrap().is_none());
    intent.revision = 1;
    intent.head = "e".repeat(40);
    assert!(replacement(&pool, &intent).await.unwrap().is_none());
    intent.head =
        sqlx::query_scalar("SELECT candidate_sha FROM candidate_validation WHERE id='validation'")
            .fetch_one(&pool)
            .await
            .unwrap();
    intent.policy.version += 1;
    assert!(replacement(&pool, &intent).await.is_err());
    intent.policy.version -= 1;
    sqlx::query("UPDATE recovery_failure SET resolution=jsonb_set(resolution,'{command,action,plan_digest}','\"different\"') WHERE event_key='scoped'").execute(&pool).await.unwrap();
    assert!(replacement(&pool, &intent).await.is_err());
    sqlx::query("UPDATE recovery_failure SET resolution_state='failed' WHERE event_key='scoped'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(replacement(&pool, &intent).await.unwrap().is_none());
    assert_eq!(
        crate::merge_validation::plan(&pool, &intent)
            .await
            .unwrap()
            .identity()
            .unwrap(),
        original.identity().unwrap()
    );
    let mut policy = intent.policy.clone();
    assert!(check_policy(&policy, "wrong").is_err());
    policy.delivery = None;
    assert!(check_policy(&policy, &digest(&policy)).is_err());
    policy = intent.policy;
    policy.delivery.as_mut().unwrap().post_merge = PostMerge::Checks {
        checks: vec![],
        probe_pr: 1,
        wait_seconds: 30,
    };
    assert!(check_policy(&policy, &digest(&policy)).is_err());
    pool.close().await;
}
