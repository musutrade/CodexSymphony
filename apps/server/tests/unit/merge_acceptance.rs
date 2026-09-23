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

#[tokio::test]
async fn retained_blocked_merge_can_record_one_original_failure() {
    let (pool, root, intent, mut remote) = fixture::fixture().await;
    crate::merge_worker::tick(&pool, &mut remote, 100)
        .await
        .unwrap();
    sqlx::query("UPDATE merge_operation SET state='blocked',merged_sha=$1,blocker='post_merge acceptance needs original-item review: post_merge binding rejected: ExitFailed'")
        .bind(&intent.head)
        .execute(&pool)
        .await
        .unwrap();
    let plan = merge_validation::plan(&pool, &intent).await.unwrap();
    let mut evidence = fixture::evidence(&root, &root.join("repo"), &plan);
    evidence.steps[0].exit_code = Some(1);
    evidence.steps[0].output =
        "FAIL independent source acceptance contract: marker must be ready".into();
    evidence.steps[0].output_sha256 = validation::sha256(&evidence.steps[0].output);
    assert!(crate::linked_repair::failed_code(
        &evidence,
        &["test".into()]
    ));
    for _ in 0..2 {
        crate::linked_failure_store::post_merge(&pool, &intent, &evidence, &["test".into()])
            .await
            .unwrap();
    }
    let (count, blocker): (i64, String) =
        sqlx::query_as("SELECT (SELECT count(*) FROM linked_failure),blocker FROM merge_operation")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 1);
    assert_eq!(blocker, "original-item linked repair pending");
    pool.close().await;
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn blocked_merge_replays_original_failed_invocation() {
    let _serial = crate::linked_repair_worker::tests::SERIAL.lock().await;
    let (pool, root, _, config, _) = crate::linked_repair_worker::tests::setup().await;
    sqlx::query("DELETE FROM linked_failure")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE merge_operation SET state='blocked',acceptance=NULL,blocker='post_merge acceptance needs original-item review: post_merge binding rejected: ExitFailed'")
        .execute(&pool)
        .await
        .unwrap();
    let intent: Intent = serde_json::from_value(
        sqlx::query_scalar::<_, Value>("SELECT intent FROM merge_operation")
            .fetch_one(&pool)
            .await
            .unwrap(),
    )
    .unwrap();
    let mut client = crate::merge_test_support::client::client(&root);
    let checkout =
        merge_validation::checkout(&pool, &mut client, &root, &intent, &intent.head, 100)
            .await
            .unwrap();
    let plan = config.validation.as_ref().unwrap();
    let candidate = crate::validation_runner::candidate(&checkout).unwrap();
    let trusted = plan.identity().unwrap();
    let directory = root
        .join("validations")
        .join(format!("post-merge-{}", intent.action_key()));
    std::fs::create_dir_all(&directory).unwrap();
    let output = "FAIL independent source acceptance contract: marker must be ready\n";
    let step = crate::validation::StepEvidence {
        id: "test".into(),
        command: vec!["/gate-entry".into()],
        exit_code: Some(1),
        output: output.into(),
        output_sha256: validation::sha256(output),
        log_ref: directory.join("step-0.log").to_string_lossy().into(),
        consumer: "handoff".into(),
        code_failure: true,
    };
    std::fs::write(directory.join("step-0.log"), output).unwrap();
    std::fs::write(
        directory.join("binding.json"),
        serde_json::to_vec(&json!({"candidate":candidate,"identity":trusted})).unwrap(),
    )
    .unwrap();
    std::fs::write(
        directory.join("result.json"),
        serde_json::to_vec(&vec![step]).unwrap(),
    )
    .unwrap();
    reconcile_classified_failure(&pool, &mut client, &root, 100)
        .await
        .unwrap();
    let (count, blocker): (i64, String) =
        sqlx::query_as("SELECT (SELECT count(*) FROM linked_failure),blocker FROM merge_operation")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 1);
    assert_eq!(blocker, "original-item linked repair pending");
    reconcile_classified_failure(&pool, &mut client, &root, 100)
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM linked_failure")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    pool.close().await;
    std::fs::remove_dir_all(root).unwrap();
}
