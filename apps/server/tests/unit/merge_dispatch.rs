use super::*;
use crate::merge_test_support::client;
use crate::merge_test_support::fixture;

#[tokio::test]
async fn absent_fixed_and_external_policies_never_dispatch_and_invalid_dispatch_fails() {
    let (pool, root, mut intent, _) = fixture::fixture().await;
    let mut client = client::client(&root);
    dispatch(&pool, &mut client, &intent, "merged", 100)
        .await
        .unwrap();
    let mut contract = intent.policy.delivery.take().unwrap();
    dispatch(&pool, &mut client, &intent, "merged", 100)
        .await
        .unwrap();
    let check = contract.pre_merge.checks[0].clone();
    contract.post_merge = PostMerge::Checks {
        checks: vec![check],
        wait_seconds: 900,
        probe_pr: 12,
    };
    intent.policy.delivery = Some(contract);
    dispatch(&pool, &mut client, &intent, "merged", 100)
        .await
        .unwrap();
    if let PostMerge::Checks { checks, .. } =
        &mut intent.policy.delivery.as_mut().unwrap().post_merge
    {
        checks[0].trigger = Trigger::WorkflowDispatch;
    }
    let error = dispatch(&pool, &mut client, &intent, "merged", 100)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("pinned Actions workflow"));
    pool.close().await;
    std::fs::remove_dir_all(root).unwrap();
}
