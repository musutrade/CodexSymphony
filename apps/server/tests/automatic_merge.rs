use codexsymphony_server::{automatic_merge, delivery_control, github::*, merge_worker};
use serde_json::json;
#[path = "support/automatic_merge.rs"]
mod fixture;
use fixture::*;

#[test]
fn identities_checks_and_protection_are_not_interchangeable() {
    let (root, repo, plan) = source::fixture();
    let e = evidence(&root, &repo, &plan);
    let mut i = intent(policy(&plan, &root.join("plan")), e.candidate.sha.clone());
    let o = observation(&i, 100);
    let required = vec!["test".into()];
    assert!(automatic_merge::admit(&i, &o, &e, &required, 100));
    for method in ["squash", "rebase", "merge"] {
        i.policy.delivery.as_mut().unwrap().actions.merge_method = Some(method.into());
        let mut o = observation(&i, 100);
        o.merge = MergeFact::Merged;
        o.closed = true;
        o.merged_sha = Some("d".repeat(40));
        o.base = "e".repeat(40);
        assert_eq!(automatic_merge::confirmed(&i, &o), Some("d".repeat(40)));
        assert!(!automatic_merge::admit(&i, &o, &e, &required, 100));
    }
    i.policy = o.policy.clone();
    for changed in [
        "head", "base", "policy", "pr", "branch", "checks", "checkout", "stale", "unknown",
    ] {
        let mut next = o.clone();
        match changed {
            "head" => next.head = "f".repeat(40),
            "base" => next.base = "f".repeat(40),
            "policy" => next.policy.version += 1,
            "pr" => next.number += 1,
            "branch" => next.head_ref = "other".into(),
            "checks" => next.phases.as_mut().unwrap()[0].checks[0].state = CheckState::Failure,
            "checkout" => {
                next.phases.as_mut().unwrap()[0].expected_checkout_sha = Some("c".repeat(40))
            }
            "stale" => next.last_synced_at = 0,
            "unknown" => next.merge = MergeFact::Unknown,
            _ => unreachable!(),
        }
        assert!(
            !automatic_merge::admit(&i, &next, &e, &required, 100),
            "{changed}"
        );
    }
    let key = i.action_key();
    i.base = "f".repeat(40);
    assert_ne!(key, i.action_key());
    i.policy.delivery.as_mut().unwrap().protection["required_status_checks"]["strict"] =
        json!(false);
    assert!(!automatic_merge::protected(&i.policy));
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn durable_unknown_reconciles_without_repeating_merge() {
    let (pool, root, _, mut remote) = fixture().await;
    remote.lost = true;
    merge_worker::tick(&pool, &mut remote, 100).await.unwrap();
    assert_eq!(state(&pool).await, "prepared");
    merge_worker::tick(&pool, &mut remote, 100).await.unwrap();
    assert_eq!(state(&pool).await, "unknown");
    assert_eq!(remote.merged, 1);
    for status in [429, 404, 403] {
        remote.read_error = Some(status);
        merge_worker::tick(&pool, &mut remote, 1000).await.unwrap();
        assert_eq!(state(&pool).await, "unknown");
    }
    remote.read_error = None;
    remote.observation.last_synced_at = 2000;
    merge_worker::tick(&pool, &mut remote, 2000).await.unwrap();
    assert_eq!(remote.merged, 1);
    codexsymphony_server::run_store::begin_incarnation(&pool, "restarted")
        .await
        .unwrap();
    remote.observation.merge = MergeFact::Merged;
    remote.observation.closed = true;
    remote.observation.merged_sha = Some("d".repeat(40));
    remote.observation.last_synced_at = 3000;
    merge_worker::tick(&pool, &mut remote, 3000).await.unwrap();
    assert_eq!(state(&pool).await, "merged");
    assert_eq!(remote.merged, 1);
    delivery_control::settle(&pool).await.unwrap();
    let owner: Option<i64> = sqlx::query_scalar("SELECT requirement_id FROM execution_control")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(owner, Some(1));
    let sha: String = sqlx::query_scalar("SELECT merged_sha FROM merge_operation")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(sha, "d".repeat(40));
    std::fs::remove_dir_all(root).unwrap();
    pool.close().await;
}
#[tokio::test]
async fn pause_and_base_race_prevent_dispatch() {
    let (pool, root, _, mut remote) = fixture().await;
    merge_worker::tick(&pool, &mut remote, 100).await.unwrap();
    remote.pause = Some(pool.clone());
    merge_worker::tick(&pool, &mut remote, 100).await.unwrap();
    assert_eq!(remote.merged, 0);
    assert_eq!(state(&pool).await, "prepared");
    sqlx::query("UPDATE requirement SET paused=false")
        .execute(&pool)
        .await
        .unwrap();
    remote.observation.base = "f".repeat(40);
    merge_worker::tick(&pool, &mut remote, 100).await.unwrap();
    assert_eq!(remote.merged, 0);
    assert_eq!(state(&pool).await, "invalidated");
    std::fs::remove_dir_all(root).unwrap();
    pool.close().await;
}

#[tokio::test]
async fn prepared_intent_rechecks_authorization_recovery_and_write_ownership() {
    for change in [
        "UPDATE requirement SET cancel_requested=true",
        "UPDATE execution_control SET recovery_complete=false",
        "UPDATE execution_control SET paused=true",
        "UPDATE repository SET version=version+1",
        "UPDATE github_repository SET stale=true",
        "UPDATE delivery_action SET state='unknown'",
        "UPDATE storage_guard SET blocked=true",
        "UPDATE candidate_validation SET result='blocked'",
    ] {
        let (pool, root, _, mut remote) = fixture().await;
        merge_worker::tick(&pool, &mut remote, 100).await.unwrap();
        assert_eq!(state(&pool).await, "prepared");
        sqlx::raw_sql(change).execute(&pool).await.unwrap();
        merge_worker::tick(&pool, &mut remote, 100).await.unwrap();
        assert_eq!(remote.merged, 0, "{change}");
        let started: bool = sqlx::query_scalar("SELECT merge_started FROM merge_operation")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(!started, "{change}");
        std::fs::remove_dir_all(root).unwrap();
        pool.close().await;
    }
}

#[tokio::test]
async fn cancellation_cannot_release_incomplete_test_merge_validation() {
    let (pool, root, _, mut remote) = fixture().await;
    merge_worker::tick(&pool, &mut remote, 100).await.unwrap();
    sqlx::raw_sql("UPDATE merge_operation SET pre_validation_started=true; UPDATE requirement SET cancel_requested=true; UPDATE delivery SET released=true;")
        .execute(&pool).await.unwrap();
    delivery_control::settle(&pool).await.unwrap();
    let (owner, complete): (Option<i64>, bool) = sqlx::query_as(
        "SELECT c.requirement_id,r.cleanup_complete FROM execution_control c JOIN requirement r ON r.id=1"
    ).fetch_one(&pool).await.unwrap();
    assert_eq!(owner, Some(1));
    assert!(!complete);
    std::fs::remove_dir_all(root).unwrap();
    pool.close().await;
}

#[tokio::test]
async fn merge_finishes_final_reserved_turn_but_never_exceeds_budget() {
    for (mutation, admitted) in [
        ("SELECT 1", true),
        ("UPDATE requirement_budget SET exhausted=true", false),
        (
            "UPDATE group_budget SET reserved='{\"tokens\":91,\"turns\":0,\"model_seconds\":90}'",
            false,
        ),
        (
            "UPDATE group_budget SET used='{\"tokens\":10,\"turns\":3,\"model_seconds\":10}'",
            false,
        ),
    ] {
        let (pool, root, _, mut remote) = fixture().await;
        sqlx::raw_sql(r#"
INSERT INTO imported_draft(id,version,document,source,source_sha256) VALUES('draft-budget',1,'{}','{}','fixture');
INSERT INTO imported_draft_revision(draft_id,version,document,source,source_sha256) VALUES('draft-budget',1,'{}','{}','fixture');
INSERT INTO group_review VALUES('draft-budget',1,1,'{}');
INSERT INTO group_review_revision(draft_id,version,draft_revision,document) VALUES('draft-budget',1,1,'{}');
INSERT INTO group_authorization(draft_id,review_version,request_id,input,snapshot) VALUES('draft-budget',1,'budget-fixture','{}','{}');
INSERT INTO group_queue(draft_id,authorization_id,state) VALUES('draft-budget',1,'waiting_scheduler');
INSERT INTO group_execution_item(draft_id,child_id,authorization_id,requirement_id,input) VALUES('draft-budget','child',1,1,'{"parent_revision":1,"child":{"order":1,"repository_id":1,"depends_on":[]}}');
INSERT INTO group_budget(draft_id,item_id,limits,used,reserved) SELECT 'draft-budget',item,'{"tokens":100,"turns":2,"model_seconds":100}','{"tokens":10,"turns":2,"model_seconds":10}','{"tokens":90,"turns":0,"model_seconds":90}' FROM unnest(ARRAY['','child']) item;
"#).execute(&pool).await.unwrap();
        sqlx::query(mutation).execute(&pool).await.unwrap();
        merge_worker::tick(&pool, &mut remote, 100).await.unwrap();
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM merge_operation")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, i64::from(admitted), "{mutation}");
        if admitted {
            assert_eq!(state(&pool).await, "prepared");
            merge_worker::tick(&pool, &mut remote, 100).await.unwrap();
            assert_eq!(remote.merged, 1);
        } else {
            assert_eq!(remote.merged, 0);
        }
        pool.close().await;
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[tokio::test]
async fn final_capability_plan_and_pr_changes_never_send_merge() {
    for case in ["capability", "plan", "pr", "evidence"] {
        let (pool, root, _, mut remote) = fixture().await;
        merge_worker::tick(&pool, &mut remote, 100).await.unwrap();
        match case {
            "capability" => remote.blockers.push("protection changed".into()),
            "plan" => {
                // Source acceptance has an extra successful step, but the separately
                // authorized post-merge plan cannot execute that step.
                sqlx::query("INSERT INTO validation_step(validation_id,step_id,command,exit_code,output,output_sha256,log_ref,consumer,code_failure,status) SELECT validation_id,'new-acceptance',command,exit_code,output,output_sha256,log_ref,consumer,code_failure,status FROM validation_step")
                    .execute(&pool).await.unwrap();
                sqlx::query(
                    "UPDATE candidate_validation SET required_steps='[\"new-acceptance\"]'",
                )
                .execute(&pool)
                .await
                .unwrap();
            }
            "pr" => remote.final_pr = Some(json!({"mergeable":false})),
            "evidence" => {
                sqlx::query("UPDATE validation_step SET exit_code=1")
                    .execute(&pool)
                    .await
                    .unwrap();
            }
            _ => unreachable!(),
        }
        merge_worker::tick(&pool, &mut remote, 100).await.unwrap();
        assert_eq!(remote.merged, 0, "{case}");
        let expected = match case {
            "capability" | "plan" => "blocked",
            "pr" => "prepared",
            _ => "invalidated",
        };
        assert_eq!(state(&pool).await, expected, "{case}");
        if case == "plan" {
            let blocker: String = sqlx::query_scalar("SELECT blocker FROM merge_operation")
                .fetch_one(&pool)
                .await
                .unwrap();
            assert!(
                blocker.contains("plan does not cover authorized AC steps"),
                "{blocker}"
            );
        }
        std::fs::remove_dir_all(root).unwrap();
        pool.close().await;
    }
}

#[tokio::test]
async fn repository_scope_and_missing_phase_evidence_never_prepare_a_merge() {
    let (pool, root, _, mut remote) = fixture().await;
    for phases in [None, Some(Vec::new())] {
        remote.observation.phases = phases;
        merge_worker::tick(&pool, &mut remote, 100).await.unwrap();
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM merge_operation")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
        assert_eq!(remote.merged, 0);
    }
    sqlx::query("UPDATE plugin_scope SET enabled=false WHERE plugin_id='delivery:github'")
        .execute(&pool)
        .await
        .unwrap();
    let error = merge_worker::tick(&pool, &mut remote, 100)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("scope unavailable"));
    assert_eq!(remote.merged, 0);
    let retained: String =
        sqlx::query_scalar("SELECT result FROM candidate_validation WHERE id='validation'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(retained, "succeeded");
    std::fs::remove_dir_all(root).unwrap();
    pool.close().await;
}
