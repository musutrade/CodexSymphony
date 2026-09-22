use codexsymphony_server::{
    automatic_merge::{self, Intent},
    delivery_control,
    github::*,
    github_contract::*,
    github_http::Error,
    merge_worker::{self, Remote},
    validation::ValidationEvidence,
};
use serde_json::{Value, json};
use sqlx::PgPool;
#[path = "support/delivery.rs"]
mod database;
#[path = "support/validation_runner.rs"]
mod source;

fn policy(plan: &codexsymphony_server::validation_runner::Plan, path: &std::path::Path) -> Policy {
    let selector = Selector {
        name: "ci".into(),
        source: Source::CheckRun { app_id: 42 },
    };
    Policy {
        repository_id: 7,
        repository: "owner/repo".into(),
        default_branch: "main".into(),
        version: 1,
        required: vec![selector.clone()],
        wait_seconds: 900,
        delivery: Some(DeliveryContract {
            schema_version: 1,
            pre_merge: PreMerge {
                source: PreMergeSource::Head,
                checkout: PreMergeSource::Head,
                checks: vec![RequiredCheck {
                    selector,
                    applicability: "always".into(),
                    trigger: Trigger::External {
                        event: "pull_request".into(),
                        path: "ci.yml".into(),
                        blob_sha: "a".repeat(40),
                    },
                    job: None,
                }],
                wait_seconds: 900,
            },
            post_merge: PostMerge::FixedValidation {
                plan_id: path.to_string_lossy().into_owned(),
                configuration_sha256: plan.identity().unwrap().config_sha256,
                authorization: "disposable test only".into(),
                wait_seconds: 900,
            },
            actions: Actions {
                merge: true,
                merge_method: Some("squash".into()),
                rerun_actions: false,
                rerequest_checks: false,
                read_logs: false,
            },
            protection: json!({"required_status_checks":{"strict":true,"checks":[{"context":"ci","app_id":42}]},"enforce_admins":{"enabled":true}}),
            rules: vec![],
        }),
    }
}
fn observation(intent: &Intent, now: i64) -> Observation {
    let checks = vec![Check {
        selector: intent.policy.required[0].clone(),
        state: CheckState::Success,
        evidence: vec![json!({"id":1})],
        history: None,
    }];
    Observation {
        policy: intent.policy.clone(),
        repository_id: 7,
        number: 12,
        head: intent.head.clone(),
        base: intent.base.clone(),
        head_ref: intent.branch.clone(),
        base_ref: "main".into(),
        test_merge_sha: Some("c".repeat(40)),
        merged_sha: None,
        merge: MergeFact::Unmerged,
        closed: false,
        checks: checks.clone(),
        last_synced_at: now,
        actual_checkout_sha: None,
        phases: Some(vec![PhaseEvidence {
            phase: "pre_merge".into(),
            head_sha: intent.head.clone(),
            base_sha: intent.base.clone(),
            check_sha: Some(intent.head.clone()),
            expected_checkout_sha: Some(intent.head.clone()),
            actual_checkout_sha: None,
            validation: None,
            checks,
            blockers: vec![],
            wait_seconds: 900,
        }]),
    }
}
fn evidence(
    root: &std::path::Path,
    repo: &std::path::Path,
    plan: &codexsymphony_server::validation_runner::Plan,
) -> ValidationEvidence {
    let candidate = codexsymphony_server::validation_runner::candidate(repo).unwrap();
    let trusted = plan.identity().unwrap();
    ValidationEvidence {
        source_before: candidate.tree.clone(),
        source_after: candidate.tree.clone(),
        entry_before: trusted.protected_entry_sha256.clone(),
        entry_after: trusted.protected_entry_sha256.clone(),
        steps: codexsymphony_server::validation_runner::execute(
            repo,
            &root.join("evidence"),
            &candidate,
            plan,
        )
        .unwrap(),
        candidate,
        trusted,
    }
}
fn intent(policy: Policy, head: String) -> Intent {
    Intent {
        delivery_key: "delivery".into(),
        requirement: 1,
        revision: 1,
        authorization: None,
        policy,
        pr: 12,
        checkout_sha: Some(head.clone()),
        head,
        base: "b".repeat(40),
        branch: "ai/req-1".into(),
        validation_id: "validation".into(),
        dependencies: json!([]),
    }
}

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

struct Fake {
    observation: Observation,
    merged: usize,
    lost: bool,
    read_error: Option<u16>,
    pause: Option<PgPool>,
}
impl Remote for Fake {
    async fn observe(&mut self, _: &Intent) -> Result<Observation, Error> {
        if let Some(status) = self.read_error {
            return Err(Error {
                code: "read_denied",
                status: Some(status),
                retry_after_seconds: Some(120),
            });
        }
        Ok(self.observation.clone())
    }
    async fn preflight(&mut self, intent: &Intent) -> Result<Capability, Error> {
        Ok(Capability {
            policy: intent.policy.clone(),
            checked_at: self.observation.last_synced_at,
            blockers: vec![],
            permissions: json!({}),
            configuration: json!({}),
        })
    }
    async fn pr(&mut self, intent: &Intent) -> Result<Value, Error> {
        if let Some(pool) = self.pause.take() {
            sqlx::query("UPDATE requirement SET paused=true WHERE id=1")
                .execute(&pool)
                .await
                .unwrap();
        }
        Ok(
            json!({"head":{"sha":intent.head},"base":{"sha":intent.base},"mergeable":true,"mergeable_state":"clean","state":"open","draft":false}),
        )
    }
    async fn merge(&mut self, _: &Intent) -> Result<Value, Error> {
        self.merged += 1;
        if self.lost {
            return Err(Error {
                code: "lost_response",
                status: None,
                retry_after_seconds: None,
            });
        }
        Ok(json!({"merged":true,"sha":"d".repeat(40)}))
    }
}
async fn fixture() -> (PgPool, std::path::PathBuf, Intent, Fake) {
    let pool = database::database().await;
    let (root, repo, plan) = source::fixture();
    let e = evidence(&root, &repo, &plan);
    let path = root.join("plan.json");
    std::fs::write(&path, serde_json::to_vec(&plan).unwrap()).unwrap();
    let mut i = intent(policy(&plan, &path), e.candidate.sha.clone());
    let key: String = sqlx::query_scalar("SELECT action_key FROM delivery")
        .fetch_one(&pool)
        .await
        .unwrap();
    i.delivery_key = key;
    sqlx::query("UPDATE delivery SET head_sha=$1,pr_number=12")
        .bind(&i.head)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE requirement SET state='Submitted'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE delivery_action SET state='confirmed'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE github_repository SET policy=$1,capability=jsonb_build_object('policy',$1::jsonb,'blockers','[]'::jsonb)").bind(json!(i.policy)).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO github_pr(repository_id,number,requirement_id) VALUES(7,12,1)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO requirement_budget(requirement_id,limits) VALUES(1,'{\"tokens\":100,\"turns\":10,\"model_seconds\":100}')").execute(&pool).await.unwrap();
    sqlx::query("UPDATE candidate_validation SET candidate_sha=$1,candidate_tree=$2,trusted=$3,required_steps='[\"test\"]',source_before=$2,source_after=$2,entry_before=$4,entry_after=$4")
        .bind(&i.head).bind(&e.candidate.tree).bind(json!(e.trusted)).bind(&e.entry_before).execute(&pool).await.unwrap();
    let step = &e.steps[0];
    sqlx::query("UPDATE validation_step SET command=$1,exit_code=0,output=$2,output_sha256=$3,log_ref=$4,code_failure=true")
        .bind(json!(step.command)).bind(&step.output).bind(&step.output_sha256).bind(&step.log_ref).execute(&pool).await.unwrap();
    let f = Fake {
        observation: observation(&i, 100),
        merged: 0,
        lost: false,
        read_error: None,
        pause: None,
    };
    (pool, root, i, f)
}
async fn state(pool: &PgPool) -> String {
    sqlx::query_scalar("SELECT state FROM merge_operation ORDER BY created_at DESC LIMIT 1")
        .fetch_one(pool)
        .await
        .unwrap()
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
