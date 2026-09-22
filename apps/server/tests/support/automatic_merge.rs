use codexsymphony_server::{
    automatic_merge::Intent, github::*, github_contract::*, github_http::Error,
    merge_worker::Remote, validation::ValidationEvidence,
};
use serde_json::{Value, json};
use sqlx::PgPool;
#[path = "delivery.rs"]
mod database;
#[path = "validation_runner.rs"]
pub(crate) mod source;

pub(crate) fn policy(
    plan: &codexsymphony_server::validation_runner::Plan,
    path: &std::path::Path,
) -> Policy {
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
pub(crate) fn observation(intent: &Intent, now: i64) -> Observation {
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
pub(crate) fn evidence(
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
pub(crate) fn intent(policy: Policy, head: String) -> Intent {
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

pub(crate) struct Fake {
    pub(crate) observation: Observation,
    pub(crate) merged: usize,
    pub(crate) lost: bool,
    pub(crate) read_error: Option<u16>,
    pub(crate) pause: Option<PgPool>,
    pub(crate) blockers: Vec<String>,
    pub(crate) final_pr: Option<Value>,
    pub(crate) clock: Option<i64>,
}
impl Remote for Fake {
    fn current_time(&self, fallback: i64) -> i64 {
        self.clock.unwrap_or(fallback)
    }
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
            blockers: self.blockers.clone(),
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
        if let Some(pr) = &self.final_pr {
            return Ok(pr.clone());
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
pub(crate) async fn fixture() -> (PgPool, std::path::PathBuf, Intent, Fake) {
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
        blockers: vec![],
        final_pr: None,
        clock: None,
    };
    (pool, root, i, f)
}
pub(crate) async fn state(pool: &PgPool) -> String {
    sqlx::query_scalar("SELECT state FROM merge_operation ORDER BY created_at DESC LIMIT 1")
        .fetch_one(pool)
        .await
        .unwrap()
}
