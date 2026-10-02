use super::*;
use crate::merge_test_support::fixture;
use crate::merge_test_support::fixture::source;
use crate::validation::Candidate;

const DIGEST: &str = "approved-plan-digest";

#[tokio::test]
async fn obsolete_context_cannot_replace_retained_evaluation() {
    for superseded in [false, true] {
        let (pool, _, _, _) = fixture::fixture().await;
        let candidate = seed(&pool).await;
        let mut ctx = context(&candidate);
        ctx.call.identity.invocation_id = "successor".into();
        let original = json!({"original":"unknown result retained"});
        sqlx::query("UPDATE candidate_validation SET hook_context=$1,hook_evaluation=$2,hook_invalidated=$3,superseded_by=CASE WHEN $3 THEN NULL ELSE 'validation' END WHERE id='successor'")
            .bind(json!(ctx)).bind(&original).bind(!superseded).execute(&pool).await.unwrap();
        let late = Evaluation {
            call: ctx.call.clone(),
            verdict: crate::controlled_contract::Verdict::Pass,
            checks: vec![],
        };
        assert!(record(&pool, &ctx, &late).await.is_err());
        let saved: Value = sqlx::query_scalar(
            "SELECT hook_evaluation FROM candidate_validation WHERE id='successor'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(saved, original);
        pool.close().await;
    }
}

fn context(candidate: &Candidate) -> Context {
    Context {
        call: Call {
            identity: InvocationIdentity {
                protocol_version: crate::extension_contract::PROTOCOL_VERSION,
                requirement_id: 1,
                revision: 1,
                run_id: None,
                resource_id: "validation:successor".into(),
                invocation_id: "fixture-invocation".into(),
                attempt: 1,
                config_id: "reviewed-process-validation".into(),
            },
            controlled_config_digest: "controlled".into(),
            operation: Operation::Validate,
            extension_id: "reviewed-process-validation".into(),
            implementation_digest: "implementation".into(),
            candidate: Some(SourceIdentity {
                commit: candidate.sha.clone(),
                tree: candidate.tree.clone(),
            }),
            environment_digest: "environment".into(),
            policy_digest: "policy".into(),
            deadline_unix_ms: i64::MAX,
            required_checks: vec!["test".into()],
        },
        environment_contract: "contract".into(),
        checkout: "/fixture/checkout".into(),
        directory: "/fixture/directory".into(),
    }
}

fn approval(kind: &str) -> Value {
    json!({"actor":"authenticated_operator","command":{"validation_id":"validation","action":{"kind":kind,"plan_digest":DIGEST}}})
}

/// Mirror extension_revalidation::create_successor, then the protected-stop
/// pause that invalidated the never-started successor in the field.
async fn seed(pool: &PgPool) -> Candidate {
    let (sha, tree): (String, String) = sqlx::query_as(
        "SELECT candidate_sha,candidate_tree FROM candidate_validation WHERE id='validation'",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO candidate_validation(id,requirement_id,revision,source_run_id,candidate_sha,candidate_tree,trusted,required_steps,source_before,source_after,entry_before,entry_after,stage,result,retry_of,hook_required) SELECT 'successor',requirement_id,revision,source_run_id,candidate_sha,candidate_tree,trusted,required_steps,source_before,source_before,entry_before,entry_before,'declaration','pending',id,true FROM candidate_validation WHERE id='validation'")
        .execute(pool).await.unwrap();
    sqlx::query("UPDATE candidate_validation SET superseded_by='successor',hook_invalidated=true WHERE id='validation'")
        .execute(pool).await.unwrap();
    sqlx::query("INSERT INTO recovery_failure(event_key,requirement_id,source_validation_id,phase,facts,fingerprint,decision,reason,resolution,resolution_state,successor_validation) VALUES('approved',1,'validation','local','{}','fixture','blocked','fixture',$1,'running','successor')")
        .bind(approval("revalidate_local_delivery")).execute(pool).await.unwrap();
    for paused in [true, false] {
        sqlx::query("UPDATE execution_control SET paused=$1 WHERE id=1")
            .bind(paused)
            .execute(pool)
            .await
            .unwrap();
    }
    assert_eq!(state(pool, "successor").await, (None, true));
    Candidate {
        sha,
        tree,
        immutable: true,
    }
}

async fn state(pool: &PgPool, id: &str) -> (Option<Value>, bool) {
    sqlx::query_as("SELECT hook_context,hook_invalidated FROM candidate_validation WHERE id=$1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn run(pool: &PgPool, sql: &str) {
    sqlx::raw_sql(sql).execute(pool).await.unwrap();
}

/// A rejected call leaves the saved context and the invalidation flag as found.
async fn rejected(pool: &PgPool, r: &Request<'_>, ctx: &Context, plan: &str, case: &str) {
    let before = state(pool, r.id).await;
    assert!(
        persist_context(pool, r, ctx, plan).await.is_err(),
        "{case} must be rejected"
    );
    assert_eq!(state(pool, r.id).await, before, "{case} changed state");
}

fn request<'a>(
    id: &'a str,
    candidate: &'a Candidate,
    plan: &'a crate::validation_runner::Plan,
    path: &'a std::path::Path,
) -> Request<'a> {
    Request {
        id,
        source_run: "run",
        requirement: 1,
        revision: 1,
        checkout: path,
        directory: path,
        candidate,
        plan,
    }
}

#[tokio::test]
async fn approved_unstarted_successor_freezes_fresh_context_and_rejects_every_other_state() {
    let (root, _, plan) = source::fixture();
    let (pool, _, _, _) = fixture::fixture().await;
    let candidate = seed(&pool).await;
    let ctx = context(&candidate);
    let r = request("successor", &candidate, &plan, &root);

    // (apply, revert): each case is isolated and restores the approved state.
    let cases = [
        (
            "UPDATE candidate_validation SET hook_context='{}' WHERE id='successor'",
            "UPDATE candidate_validation SET hook_context=NULL WHERE id='successor'",
            "existing context",
        ),
        (
            "UPDATE candidate_validation SET hook_evaluation='{}' WHERE id='successor'",
            "UPDATE candidate_validation SET hook_evaluation=NULL WHERE id='successor'",
            "existing evaluation",
        ),
        (
            "UPDATE candidate_validation SET started_at=now() WHERE id='successor'",
            "UPDATE candidate_validation SET started_at=NULL WHERE id='successor'",
            "started",
        ),
        (
            "INSERT INTO validation_step(validation_id,step_id,command,status) VALUES('successor','test','[]','pending')",
            "DELETE FROM validation_step WHERE validation_id='successor'",
            "recorded step",
        ),
        (
            "INSERT INTO project_hook_run(run_id,requirement_id,revision,resource_id,workspace,role,frozen) VALUES('successor',1,1,'successor','/fixture','validation','{}'); INSERT INTO project_hook_invocation(invocation_id,run_id,resource_id,event,hook_name,status,output_dir) VALUES('hook','successor','successor','validate','fixture','intent','/fixture')",
            "DELETE FROM project_hook_invocation WHERE run_id='successor'; DELETE FROM project_hook_run WHERE run_id='successor'",
            "hook invocation",
        ),
        (
            "UPDATE candidate_validation SET superseded_by='validation' WHERE id='successor'",
            "UPDATE candidate_validation SET superseded_by=NULL WHERE id='successor'",
            "superseded successor",
        ),
        (
            "UPDATE candidate_validation SET result='blocked' WHERE id='successor'",
            "UPDATE candidate_validation SET result='pending' WHERE id='successor'",
            "blocked result",
        ),
        (
            "UPDATE candidate_validation SET stage='validation' WHERE id='successor'",
            // The timing trigger stamps started_at on this transition; clear both.
            "UPDATE candidate_validation SET stage='declaration',started_at=NULL WHERE id='successor'",
            "started stage",
        ),
        (
            "UPDATE candidate_validation SET superseded_by=NULL WHERE id='validation'",
            "UPDATE candidate_validation SET superseded_by='successor' WHERE id='validation'",
            "old not superseded",
        ),
        (
            "UPDATE candidate_validation SET candidate_tree='other' WHERE id='validation'",
            "UPDATE candidate_validation SET candidate_tree=source_before WHERE id='validation'",
            "old tree differs",
        ),
        (
            "UPDATE recovery_failure SET resolution_state='complete'",
            "UPDATE recovery_failure SET resolution_state='running'",
            "recovery not running",
        ),
        (
            "UPDATE recovery_failure SET resolution_state='pending'",
            "UPDATE recovery_failure SET resolution_state='running'",
            "recovery pending",
        ),
        (
            "UPDATE recovery_failure SET successor_validation=NULL",
            "UPDATE recovery_failure SET successor_validation='successor'",
            "no successor",
        ),
        (
            "UPDATE recovery_failure SET resolution=jsonb_set(resolution,'{actor}','\"model\"')",
            "UPDATE recovery_failure SET resolution=jsonb_set(resolution,'{actor}','\"authenticated_operator\"')",
            "wrong actor",
        ),
        (
            "UPDATE recovery_failure SET resolution=jsonb_set(resolution,'{command,validation_id}','\"successor\"')",
            "UPDATE recovery_failure SET resolution=jsonb_set(resolution,'{command,validation_id}','\"validation\"')",
            "wrong validation",
        ),
        (
            "UPDATE recovery_failure SET resolution=jsonb_set(resolution,'{command,action,kind}','\"revalidate\"')",
            "UPDATE recovery_failure SET resolution=jsonb_set(resolution,'{command,action,kind}','\"revalidate_local_delivery\"')",
            "wrong kind",
        ),
        (
            "UPDATE recovery_failure SET resolution=jsonb_set(resolution,'{command,action,plan_digest}','\"other\"')",
            "UPDATE recovery_failure SET resolution=jsonb_set(resolution,'{command,action,plan_digest}','\"approved-plan-digest\"')",
            "wrong approved plan",
        ),
        (
            "UPDATE requirement SET cancel_requested=true WHERE id=1",
            "UPDATE requirement SET cancel_requested=false WHERE id=1",
            "cancel requested",
        ),
        (
            "UPDATE requirement SET paused=true WHERE id=1",
            "UPDATE requirement SET paused=false WHERE id=1",
            "requirement paused",
        ),
        (
            "UPDATE execution_control SET paused=true WHERE id=1",
            "UPDATE execution_control SET paused=false WHERE id=1",
            "global paused",
        ),
        (
            "UPDATE storage_guard SET blocked=true WHERE id=1",
            "UPDATE storage_guard SET blocked=false WHERE id=1",
            "storage blocked",
        ),
        (
            "UPDATE repository SET document=jsonb_set(document,'{revoked}','true') WHERE id=1",
            "UPDATE repository SET document=jsonb_set(document,'{revoked}','false') WHERE id=1",
            "repository revoked",
        ),
        (
            "UPDATE agent_run SET quiescent=false WHERE id='run'",
            "UPDATE agent_run SET quiescent=true WHERE id='run'",
            "active run",
        ),
    ];
    for (apply, revert, case) in cases {
        run(&pool, apply).await;
        rejected(&pool, &r, &ctx, DIGEST, case).await;
        run(&pool, revert).await;
        assert_eq!(
            state(&pool, "successor").await,
            (None, true),
            "{case} not restored"
        );
    }
    rejected(
        &pool,
        &r,
        &ctx,
        "other",
        "caller plan differs from approval",
    )
    .await;
    let other = Candidate {
        sha: "other".into(),
        ..candidate.clone()
    };
    let wrong = request("successor", &other, &plan, &root);
    rejected(&pool, &wrong, &ctx, DIGEST, "request candidate differs").await;
    let wrong_run = Request {
        source_run: "other",
        ..request("successor", &candidate, &plan, &root)
    };
    rejected(
        &pool,
        &wrong_run,
        &ctx,
        DIGEST,
        "request source run differs",
    )
    .await;
    let wrong_revision = Request {
        revision: 2,
        ..request("successor", &candidate, &plan, &root)
    };
    rejected(
        &pool,
        &wrong_revision,
        &ctx,
        DIGEST,
        "request revision differs",
    )
    .await;

    // Both approved kinds are accepted; the field route is local delivery.
    sqlx::query("UPDATE recovery_failure SET resolution=$1")
        .bind(approval("revalidate_delivery"))
        .execute(&pool)
        .await
        .unwrap();
    persist_context(&pool, &r, &ctx, DIGEST).await.unwrap();
    assert_eq!(state(&pool, "successor").await, (Some(json!(ctx)), false));
    let required: bool =
        sqlx::query_scalar("SELECT hook_required FROM candidate_validation WHERE id='successor'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(required);
    // The ancestor keeps its supersession and invalidation unchanged.
    assert!(state(&pool, "validation").await.1);
    rejected(&pool, &r, &ctx, DIGEST, "second initialization").await;
    // A pause after freezing invalidates durably; the saved call is never revived.
    run(&pool, "UPDATE requirement SET paused=true WHERE id=1").await;
    assert_eq!(state(&pool, "successor").await, (Some(json!(ctx)), true));
    run(&pool, "UPDATE requirement SET paused=false WHERE id=1").await;
    rejected(&pool, &r, &ctx, DIGEST, "paused after freeze").await;
    let calls: i64 = sqlx::query_scalar("SELECT count(*) FROM model_call")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(calls, 0);
    pool.close().await;
}

#[tokio::test]
async fn ordinary_generation_requires_no_invalidation_and_approval_does_not_leak() {
    let (root, _, plan) = source::fixture();
    let (pool, _, _, _) = fixture::fixture().await;
    let candidate = seed(&pool).await;
    let ctx = context(&candidate);
    // Unrelated first generation of the same candidate, without any recovery.
    run(&pool, "INSERT INTO candidate_validation(id,requirement_id,revision,source_run_id,candidate_sha,candidate_tree,trusted,required_steps,source_before,source_after,entry_before,entry_after,stage,result,retry_of,hook_required,hook_invalidated) SELECT 'fresh',requirement_id,revision,source_run_id,candidate_sha,candidate_tree,trusted,required_steps,source_before,source_before,entry_before,entry_before,'declaration','pending','validation',true,true FROM candidate_validation WHERE id='validation'").await;
    let r = request("fresh", &candidate, &plan, &root);
    rejected(&pool, &r, &ctx, DIGEST, "invalidated ordinary generation").await;
    run(
        &pool,
        "UPDATE candidate_validation SET hook_invalidated=false WHERE id='fresh'",
    )
    .await;
    persist_context(&pool, &r, &ctx, "unrelated").await.unwrap();
    assert_eq!(state(&pool, "fresh").await, (Some(json!(ctx)), false));
    // The approved successor is still untouched by the unrelated generation.
    assert_eq!(state(&pool, "successor").await, (None, true));
    pool.close().await;
}

#[tokio::test]
async fn initialization_serializes_with_control_lock_and_concurrent_pause() {
    let (root, _, plan) = source::fixture();
    let (pool, _, _, _) = fixture::fixture().await;
    let candidate = seed(&pool).await;
    let ctx = context(&candidate);
    let r = request("successor", &candidate, &plan, &root);
    // A held control lock blocks initialization; it fails without writing.
    let held = crate::run_store::lock(&pool).await.unwrap();
    rejected(&pool, &r, &ctx, DIGEST, "control lock held").await;
    held.rollback().await.unwrap();
    assert_eq!(state(&pool, "successor").await, (None, true));
    // Either order is serialized: a pause first rejects initialization; a pause
    // after initialization re-invalidates it. A cleared flag never survives.
    let (initialized, paused) = tokio::join!(
        persist_context(&pool, &r, &ctx, DIGEST),
        crate::run_store::pause(&pool, Some(1))
    );
    paused.unwrap();
    let (saved, invalidated) = state(&pool, "successor").await;
    assert!(invalidated);
    assert_eq!(initialized.is_ok(), saved == Some(json!(ctx)));
    if initialized.is_err() {
        assert_eq!(saved, None);
    }
    pool.close().await;
}
