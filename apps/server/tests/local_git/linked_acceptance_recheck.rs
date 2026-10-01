//! Acceptance correction for a validation-only item whose delivery came from an
//! exact original-item linked repair. Reuses the parent failed-validator fixture.
use super::{Product, failed_validator, git, product_tick, recheck_cli};
use codexsymphony_server::local_acceptance_recheck::{Command, request};
use serde_json::Value;

const FAILURE: &str = "integration:fixture-failed";
const DOCUMENT: &str = r#"{"schema":"linked-repair-input/v1","paths":["value"]}"#;
/// The binding `local_repair::record_failure` writes when it merges the reserved
/// failure on failed acceptance: candidate object, failed evidence, delivery key.
const RECORDED: &str = "UPDATE linked_failure f SET final_version=jsonb_build_object('candidate',d.local_acceptance#>'{evidence,candidate}','failed_evidence',d.local_acceptance->'evidence','local_delivery',d.action_key) FROM delivery d WHERE d.action_key=f.repair_delivery AND f.id='integration:fixture-failed'";

/// Turn the fixture into the R3 shape: a validation-only item whose delivered
/// candidate was produced by the succeeded repair Run (validation_store marks it
/// succeeded once the repair candidate validates) of a merged integration failure.
async fn bind_repair(p: &Product, command: &Command) {
    sqlx::query("UPDATE group_execution_item SET input=jsonb_set(input,'{child,kind}','\"validation_only\"') WHERE requirement_id=1")
        .execute(&p.pool).await.unwrap();
    sqlx::query("INSERT INTO integration_validation(id,requirement_id,authorization_id,revision,binding,job,launch,state,quiescent) SELECT 'fixture-failed',1,authorization_id,1,'{}','{}','{}','failed',true FROM group_execution_item WHERE requirement_id=1")
        .execute(&p.pool).await.unwrap();
    sqlx::query("INSERT INTO linked_failure(id,requirement_id,revision,integration_id,evidence,required_steps,state,repository_id,document,repair_delivery) VALUES($1,1,1,'fixture-failed','{}','[]','merged',1,$2::jsonb,$3)")
        .bind(FAILURE).bind(DOCUMENT).bind(&command.delivery_key)
        .execute(&p.pool).await.unwrap();
    exec(p, RECORDED).await;
    sqlx::query(
        "INSERT INTO linked_run_input(run_id,failure_id,document) VALUES('source',$1,$2::jsonb)",
    )
    .bind(FAILURE)
    .bind(DOCUMENT)
    .execute(&p.pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO repair_reservation(requirement_id,ordinal,linked_failure_id,repair_run_id,failure,status,resources,resources_transferred) VALUES(1,1,$1,'source','{}','succeeded','{\"tokens\":1,\"turns\":1,\"model_seconds\":1}',true)")
        .bind(FAILURE)
        .execute(&p.pool).await.unwrap();
    sqlx::query("INSERT INTO agent_run(id,requirement_id,revision,incarnation,request_id,workspace,workspace_identity,launch,state,quiescent,phase) VALUES('other-run',1,1,'boot','other-run','/nonexistent','other-run','{}','Succeeded',true,'validation')")
        .execute(&p.pool).await.unwrap();
}

async fn exec(p: &Product, sql: &str) {
    sqlx::raw_sql(sql).execute(&p.pool).await.unwrap();
}

/// Apply one binding mutation, require rejection without registration, then restore.
async fn rejected(p: &Product, command: &Command, label: &str, change: &str, restore: &str) {
    exec(p, change).await;
    assert!(
        request(&p.pool, command).await.is_err(),
        "accepted correction with {label}"
    );
    exec(p, restore).await;
}

async fn snapshot(p: &Product) -> Value {
    sqlx::query_scalar("SELECT jsonb_build_object('calls',(SELECT count(*) FROM model_call),'attempts',(SELECT count(*) FROM delivery_attempt),'rechecks',(SELECT count(*) FROM local_acceptance_recheck),'budgets',(SELECT jsonb_agg(to_jsonb(b) ORDER BY requirement_id) FROM requirement_budget b),'groups',(SELECT jsonb_agg(to_jsonb(b)) FROM group_budget b),'reservations',(SELECT jsonb_agg(to_jsonb(x) ORDER BY requirement_id,ordinal) FROM repair_reservation x),'failures',(SELECT jsonb_agg(to_jsonb(f) ORDER BY id) FROM linked_failure f),'delivery',(SELECT jsonb_agg(to_jsonb(d)) FROM delivery d),'validations',(SELECT jsonb_agg(to_jsonb(v) ORDER BY id) FROM candidate_validation v))")
        .fetch_one(&p.pool).await.unwrap()
}

#[tokio::test]
async fn validation_only_repair_delivery_admits_correction_without_spending() {
    let (p, command) = failed_validator().await;
    bind_repair(&p, &command).await;
    let before = snapshot(&p).await;
    let output = recheck_cli(
        &p,
        &["acceptance-recheck", "--stdin-json"],
        &serde_json::to_vec(&command).unwrap(),
    )
    .await;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let ack: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(ack["accepted"], true);
    assert_eq!(ack["started"], false);
    assert_eq!(ack["model_calls_added"], 0);
    // Idempotent replay of the same request.
    assert_eq!(request(&p.pool, &command).await.unwrap(), ack);
    let after = snapshot(&p).await;
    assert_eq!(after["calls"], before["calls"]);
    assert_eq!(after["attempts"], before["attempts"]);
    assert_eq!(after["rechecks"], 1);
    assert_eq!(after["budgets"], before["budgets"]);
    assert_eq!(after["groups"], before["groups"]);
    assert_eq!(after["reservations"], before["reservations"]);
    let failures: Vec<(String, String)> =
        sqlx::query_as("SELECT id,state FROM linked_failure ORDER BY id")
            .fetch_all(&p.pool)
            .await
            .unwrap();
    let post_local = format!("post-local:{}", command.delivery_key);
    assert_eq!(
        failures,
        vec![
            (FAILURE.to_string(), "merged".to_string()),
            (post_local, "cancelled".to_string()),
        ]
    );
    let previous: Value =
        sqlx::query_scalar("SELECT previous_failure FROM local_acceptance_recheck")
            .fetch_one(&p.pool)
            .await
            .unwrap();
    assert_eq!(previous["local_delivery"], command.delivery_key.as_str());
    p.pool.close().await;
}

#[tokio::test]
async fn validation_only_correction_requires_the_exact_succeeded_repair_binding() {
    let (p, command) = failed_validator().await;
    // Without any repair binding the original kind restriction still applies.
    exec(&p, "UPDATE group_execution_item SET input=jsonb_set(input,'{child,kind}','\"validation_only\"') WHERE requirement_id=1").await;
    assert!(request(&p.pool, &command).await.is_err());
    exec(&p, "UPDATE group_execution_item SET input=jsonb_set(input,'{child,kind}','\"code_change\"') WHERE requirement_id=1").await;
    bind_repair(&p, &command).await;
    let before = snapshot(&p).await;
    let cases: [(&str, &str, &str); 17] = [
        (
            "final version for another delivery",
            "UPDATE linked_failure SET final_version=jsonb_set(final_version,'{local_delivery}','\"other-delivery\"') WHERE id='integration:fixture-failed'",
            RECORDED,
        ),
        (
            "final version for another candidate",
            "UPDATE linked_failure SET final_version=jsonb_set(final_version,'{candidate,sha}','\"0000000000000000000000000000000000000000\"') WHERE id='integration:fixture-failed'",
            RECORDED,
        ),
        (
            "final version without binding",
            "UPDATE linked_failure SET final_version=NULL WHERE id='integration:fixture-failed'",
            RECORDED,
        ),
        (
            "no repair reservation",
            "UPDATE repair_reservation SET linked_failure_id=NULL,source_validation_id='validation' WHERE requirement_id=1",
            "UPDATE repair_reservation SET source_validation_id=NULL,linked_failure_id='integration:fixture-failed' WHERE requirement_id=1",
        ),
        (
            "unbound delivery",
            "UPDATE linked_failure SET repair_delivery=NULL WHERE id='integration:fixture-failed'",
            "UPDATE linked_failure SET repair_delivery=(SELECT action_key FROM delivery WHERE validation_id='validation') WHERE id='integration:fixture-failed'",
        ),
        (
            "other repository",
            "UPDATE linked_failure SET repository_id=2 WHERE id='integration:fixture-failed'",
            "UPDATE linked_failure SET repository_id=1 WHERE id='integration:fixture-failed'",
        ),
        (
            "other revision",
            "UPDATE linked_failure SET revision=2 WHERE id='integration:fixture-failed'",
            "UPDATE linked_failure SET revision=1 WHERE id='integration:fixture-failed'",
        ),
        (
            "other failure requirement",
            "UPDATE linked_failure SET requirement_id=2 WHERE id='integration:fixture-failed'",
            "UPDATE linked_failure SET requirement_id=1 WHERE id='integration:fixture-failed'",
        ),
        (
            "other source run",
            "UPDATE linked_run_input SET run_id='other-run' WHERE failure_id='integration:fixture-failed'; UPDATE repair_reservation SET repair_run_id='other-run' WHERE requirement_id=1",
            "UPDATE repair_reservation SET repair_run_id='source' WHERE requirement_id=1; UPDATE linked_run_input SET run_id='source' WHERE failure_id='integration:fixture-failed'",
        ),
        (
            "reservation for another run",
            "UPDATE repair_reservation SET repair_run_id='other-run' WHERE requirement_id=1",
            "UPDATE repair_reservation SET repair_run_id='source' WHERE requirement_id=1",
        ),
        (
            "changed run document",
            "UPDATE linked_run_input SET document=document||'{\"paths\":[\"other\"]}' WHERE failure_id='integration:fixture-failed'",
            "UPDATE linked_run_input SET document=(SELECT document FROM linked_failure WHERE id='integration:fixture-failed') WHERE failure_id='integration:fixture-failed'",
        ),
        (
            "reserved failure",
            "UPDATE linked_failure SET state='reserved' WHERE id='integration:fixture-failed'",
            "UPDATE linked_failure SET state='merged' WHERE id='integration:fixture-failed'",
        ),
        (
            "completed failure",
            "UPDATE linked_failure SET state='complete' WHERE id='integration:fixture-failed'",
            "UPDATE linked_failure SET state='merged' WHERE id='integration:fixture-failed'",
        ),
        (
            "pending reservation",
            "UPDATE repair_reservation SET status='reserved' WHERE requirement_id=1",
            "UPDATE repair_reservation SET status='succeeded' WHERE requirement_id=1",
        ),
        (
            "unvalidated reservation",
            "UPDATE repair_reservation SET status='started' WHERE requirement_id=1",
            "UPDATE repair_reservation SET status='succeeded' WHERE requirement_id=1",
        ),
        (
            "failed reservation",
            "UPDATE repair_reservation SET status='failed' WHERE requirement_id=1",
            "UPDATE repair_reservation SET status='succeeded' WHERE requirement_id=1",
        ),
        (
            "reservation of another requirement",
            "UPDATE repair_reservation SET requirement_id=2 WHERE requirement_id=1",
            "UPDATE repair_reservation SET requirement_id=1 WHERE requirement_id=2",
        ),
    ];
    for (label, change, restore) in cases {
        rejected(&p, &command, label, change, restore).await;
    }
    assert_eq!(snapshot(&p).await, before);
    p.pool.close().await;
}

#[tokio::test]
async fn validation_only_repair_binding_keeps_proof_target_pause_and_owner_guards() {
    let (p, command) = failed_validator().await;
    bind_repair(&p, &command).await;
    let before = snapshot(&p).await;
    rejected(
        &p,
        &command,
        "running control",
        "UPDATE execution_control SET paused=false",
        "UPDATE execution_control SET paused=true",
    )
    .await;
    rejected(
        &p,
        &command,
        "invalidated proof",
        "UPDATE candidate_validation SET hook_invalidated=true WHERE id='validation'",
        "UPDATE candidate_validation SET hook_invalidated=false WHERE id='validation'",
    )
    .await;
    rejected(
        &p,
        &command,
        "revoked repository",
        "UPDATE repository SET revoked_through_version=version WHERE id=1",
        "UPDATE repository SET revoked_through_version=0 WHERE id=1",
    )
    .await;
    rejected(
        &p,
        &command,
        "repair already reserved for the post-local failure",
        "INSERT INTO repair_reservation(requirement_id,ordinal,linked_failure_id,failure,status) SELECT 1,2,id,'{}','succeeded' FROM linked_failure WHERE local_delivery IS NOT NULL",
        "DELETE FROM repair_reservation WHERE requirement_id=1 AND ordinal=2",
    )
    .await;
    rejected(
        &p,
        &command,
        "failed candidate validation",
        "UPDATE candidate_validation SET result='gate_failed' WHERE id='validation'",
        "UPDATE candidate_validation SET result='succeeded' WHERE id='validation'",
    )
    .await;
    git(
        &p.binding.target.path,
        &[
            "update-ref",
            "refs/heads/main",
            &p.manifest.workspace.baseline,
            &p.manifest.head,
        ],
    );
    assert!(request(&p.pool, &command).await.is_err());
    git(
        &p.binding.target.path,
        &[
            "update-ref",
            "refs/heads/main",
            &p.manifest.head,
            &p.manifest.workspace.baseline,
        ],
    );
    let mut bad = command.clone();
    bad.plan = p.plan.clone();
    assert!(request(&p.pool, &bad).await.is_err());
    bad = command.clone();
    bad.plan.steps[0].code_failure = false;
    assert!(request(&p.pool, &bad).await.is_err());
    bad = command.clone();
    bad.version += 1;
    assert!(request(&p.pool, &bad).await.is_err());
    assert_eq!(snapshot(&p).await, before);
    // All guards restored: the exact binding is admitted once.
    assert_eq!(request(&p.pool, &command).await.unwrap()["accepted"], true);
    p.pool.close().await;
}

/// A corrected pass of a repaired validation-only delivery must return the item
/// to integration revalidation; it must not complete the item directly.
#[tokio::test]
async fn validation_only_corrected_pass_returns_to_integration_revalidation() {
    let (p, command) = failed_validator().await;
    bind_repair(&p, &command).await;
    request(&p.pool, &command).await.unwrap();
    exec(&p, "UPDATE execution_control SET paused=false").await;
    product_tick(&p).await;
    let (state, accepted, released): (String, Value, bool) = sqlx::query_as("SELECT r.state,d.local_acceptance,d.released FROM delivery d JOIN requirement r ON r.id=d.requirement_id WHERE d.validation_id='validation'")
        .fetch_one(&p.pool).await.unwrap();
    assert_eq!(accepted["passed"], true);
    assert!(released);
    assert_eq!(state, "Running");
    let failure: String = sqlx::query_scalar("SELECT state FROM linked_failure WHERE id=$1")
        .bind(FAILURE)
        .fetch_one(&p.pool)
        .await
        .unwrap();
    assert_eq!(failure, "merged");
    let version: Value = sqlx::query_scalar("SELECT final_version FROM linked_failure WHERE id=$1")
        .bind(FAILURE)
        .fetch_one(&p.pool)
        .await
        .unwrap();
    // The merged binding is extended, never replaced: failed evidence survives.
    assert_eq!(version["local_delivery"], command.delivery_key.as_str());
    assert_eq!(version["candidate"], accepted["evidence"]["candidate"]);
    assert_eq!(version["evidence"], accepted["evidence"]);
    assert!(version["failed_evidence"].is_object());
    let counts: (i64, i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM model_call),(SELECT count(*) FROM delivery_attempt),(SELECT count(*) FROM group_completion WHERE requirement_id=1)")
        .fetch_one(&p.pool).await.unwrap();
    assert_eq!(counts, (0, 1, 0));
    assert_eq!(
        codexsymphony_server::local_git::head(&p.binding).unwrap(),
        p.manifest.head
    );
    p.pool.close().await;
}

/// After admission, a merged failure whose recorded delivery or candidate no
/// longer matches the corrected pass must not reopen integration revalidation.
async fn corrected_pass_with_binding(change: &str) {
    let (p, command) = failed_validator().await;
    bind_repair(&p, &command).await;
    request(&p.pool, &command).await.unwrap();
    exec(&p, change).await;
    let changed: Value = sqlx::query_scalar("SELECT final_version FROM linked_failure WHERE id=$1")
        .bind(FAILURE)
        .fetch_one(&p.pool)
        .await
        .unwrap();
    exec(&p, "UPDATE execution_control SET paused=false").await;
    product_tick(&p).await;
    let state: String = sqlx::query_scalar("SELECT state FROM requirement WHERE id=1")
        .fetch_one(&p.pool)
        .await
        .unwrap();
    assert_ne!(
        state, "Running",
        "unbound merged failure reopened integration: {change}"
    );
    let (failure, version): (String, Value) =
        sqlx::query_as("SELECT state,final_version FROM linked_failure WHERE id=$1")
            .bind(FAILURE)
            .fetch_one(&p.pool)
            .await
            .unwrap();
    assert_eq!(failure, "merged");
    assert_eq!(version, changed);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM model_call")
            .fetch_one(&p.pool)
            .await
            .unwrap(),
        0
    );
    p.pool.close().await;
}

#[tokio::test]
async fn corrected_pass_ignores_merged_failure_of_another_delivery() {
    corrected_pass_with_binding("UPDATE linked_failure SET final_version=jsonb_set(final_version,'{local_delivery}','\"other-delivery\"') WHERE id='integration:fixture-failed'").await;
}

#[tokio::test]
async fn corrected_pass_ignores_merged_failure_of_another_candidate() {
    corrected_pass_with_binding("UPDATE linked_failure SET final_version=jsonb_set(final_version,'{candidate,tree}','\"0000000000000000000000000000000000000000\"') WHERE id='integration:fixture-failed'").await;
}

#[tokio::test]
async fn corrected_pass_ignores_merged_failure_of_another_revision() {
    corrected_pass_with_binding(
        "UPDATE linked_failure SET revision=2 WHERE id='integration:fixture-failed'",
    )
    .await;
}

/// Real Runtime order: linked integration runs before local delivery in every
/// tick. A cancelled post-local failure must not let the merged repair failure
/// re-run integration until the corrected local acceptance has released the
/// delivery; the fixture's empty integration job would fail such an early rerun.
/// After the release, a real Job/Launch/binding schedules and passes the new
/// mixed integration on the exact repaired candidate.
#[tokio::test]
async fn corrected_repair_reruns_integration_only_after_local_acceptance_release() {
    use codexsymphony_server::{
        execution::{Launch, RunKey},
        integration::{Binding, Version},
        integration_process::Job,
        runtime_service,
        validation::{Candidate, sha256},
    };
    use std::path::Path;
    let (p, command) = failed_validator().await;
    bind_repair(&p, &command).await;
    request(&p.pool, &command).await.unwrap();
    exec(&p, "UPDATE execution_control SET paused=false").await;
    let config = super::worker_config(&p);
    let supervisor = Path::new(env!("CARGO_BIN_EXE_codexsymphony-server"));
    runtime_service::tick(
        &p.pool,
        p.root.path(),
        supervisor,
        &p.broker,
        "boot",
        &config,
    )
    .await
    .expect("integration rerun preceded corrected local acceptance");
    let (accepted, released, job, revalidation, integrations): (Value, bool, Value, Option<String>, i64) = sqlx::query_as("SELECT d.local_acceptance,d.released,d.local_acceptance_job,(SELECT revalidation_id FROM linked_failure WHERE id=$1),(SELECT count(*) FROM integration_validation) FROM delivery d WHERE d.validation_id='validation'")
        .bind(FAILURE).fetch_one(&p.pool).await.unwrap();
    assert_eq!(accepted["passed"], true);
    assert!(released);
    assert!(
        job["invocation"]
            .as_str()
            .unwrap()
            .starts_with("local-acceptance-recheck-")
    );
    assert_eq!((revalidation, integrations), (None, 1));
    // Replace the poisoned fixture with the original mixed integration identity.
    let (authorization, input, failed): (i64, Value, Value) = sqlx::query_as("SELECT i.authorization_id,i.input,f.final_version FROM group_execution_item i JOIN linked_failure f ON f.requirement_id=i.requirement_id WHERE i.requirement_id=1 AND f.id=$1")
        .bind(FAILURE).fetch_one(&p.pool).await.unwrap();
    let base = &p.manifest.workspace.baseline;
    let source = p.root.path().join("source");
    let tree = format!("{base}^{{tree}}");
    let fixed = Candidate {
        sha: base.clone(),
        tree: git(&source, &["rev-parse", tree.as_str()]),
        immutable: true,
    };
    let original = Job {
        invocation: "fixture-failed".into(),
        binding: Binding {
            requirement: 1,
            revision: 1,
            authorization,
            input_sha256: sha256(serde_json::to_vec(&input).unwrap()),
            versions: vec![
                Version {
                    repository_id: 1,
                    github_repository_id: 0,
                    repository_version: 1,
                    candidate: serde_json::from_value(failed["candidate"].clone()).unwrap(),
                    artifacts: vec![],
                },
                Version {
                    repository_id: 2,
                    github_repository_id: 456,
                    repository_version: 1,
                    candidate: fixed,
                    artifacts: vec![],
                },
            ],
            trusted: command.plan.identity().unwrap(),
            required: vec!["test".into()],
        },
        plan: command.plan.clone(),
        checkouts: vec![],
        output_limit: 1048576,
    };
    let launch = Launch {
        key: RunKey {
            run_id: "fixture-failed".into(),
            request_id: "fixture-failed".into(),
            incarnation: "old".into(),
        },
        workspace: "/nonexistent".into(),
        workspace_identity: "fixture-failed".into(),
        program: "/bin/true".into(),
        args: vec![],
    };
    sqlx::query("UPDATE integration_validation SET binding=$2,job=$3,launch=$4 WHERE id=$1")
        .bind("fixture-failed")
        .bind(serde_json::json!(original.binding))
        .bind(serde_json::json!(original))
        .bind(serde_json::json!(launch))
        .execute(&p.pool)
        .await
        .unwrap();
    for _ in 0..200 {
        runtime_service::tick(
            &p.pool,
            p.root.path(),
            supervisor,
            &p.broker,
            "boot",
            &config,
        )
        .await
        .unwrap();
        if sqlx::query_scalar::<_, String>("SELECT state FROM requirement WHERE id=1")
            .fetch_one(&p.pool)
            .await
            .unwrap()
            == "Done"
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    let (id, state, binding): (String, String, Value) = sqlx::query_as(
        "SELECT id,state,binding FROM integration_validation WHERE id<>'fixture-failed'",
    )
    .fetch_one(&p.pool)
    .await
    .unwrap();
    assert_eq!(state, "passed");
    assert_eq!(binding["versions"][0]["candidate"]["sha"], p.manifest.head);
    assert_eq!(
        binding["versions"][0]["artifacts"],
        serde_json::json!([format!("linked-failure:{FAILURE}")])
    );
    assert_eq!(binding["versions"][1]["candidate"]["sha"], base.as_str());
    assert_eq!(binding["versions"][1]["github_repository_id"], 456);
    let (failure, revalidation, blocker, fact, requirement, calls): (String, Option<String>, String, Value, String, i64) = sqlx::query_as("SELECT f.state,f.revalidation_id,(SELECT blocker FROM integration_validation WHERE id='fixture-failed'),(SELECT fact FROM group_completion WHERE requirement_id=1),(SELECT state FROM requirement WHERE id=1),(SELECT count(*) FROM model_call) FROM linked_failure f WHERE f.id=$1")
        .bind(FAILURE).fetch_one(&p.pool).await.unwrap();
    assert_eq!(failure, "complete");
    assert_eq!(revalidation.as_deref(), Some(id.as_str()));
    assert_eq!(
        blocker,
        format!("updated exact-version validation scheduled: {id}")
    );
    assert_eq!(fact["validation_id"], id.as_str());
    assert_eq!(fact["source"], "platform-integration-validation/v1");
    assert_eq!((requirement.as_str(), calls), ("Done", 0));
    p.pool.close().await;
}
