//! Real subprocess output, retained content and authorized product tool reads.
use codexsymphony_server::{
    diagnostic_api, diagnostic_capture, diagnostic_service, diagnostic_store, diagnostic_tools,
    diagnostics::*, execution::RunKey, git_broker::GitBroker, process, runtime_store,
    runtime_tools, validation::sha256, validation_runner,
};
use serde_json::{Value, json};
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{
    fs,
    path::{Path, PathBuf},
};
use tower::ServiceExt;
static DATABASE_SCENARIO: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
fn key() -> RunKey {
    RunKey {
        run_id: "source".into(),
        request_id: "request".into(),
        incarnation: "boot".into(),
    }
}
fn temporary() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "diagnostic-fixture-{}",
        process::new_identity().unwrap()
    ));
    std::fs::create_dir(&path).unwrap();
    path
}
async fn fixture(root: &Path) -> PgPool {
    let options: PgConnectOptions = std::env::var("TEST_DATABASE_URL").unwrap().parse().unwrap();
    let admin = PgPoolOptions::new()
        .connect_with(options.clone())
        .await
        .unwrap();
    let schema = format!(
        "runtime_{}",
        process::new_identity().unwrap().replace('-', "")
    );
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect_with(options.options([("search_path", schema)]))
        .await
        .unwrap();
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
    sqlx::raw_sql(r#"TRUNCATE requirement,repository,business_request RESTART IDENTITY CASCADE;
      INSERT INTO repository(id,version,document) VALUES(1,1,'{"revoked":false}');
      INSERT INTO requirement(version,state,contract,revision) VALUES(1,'Running','{}',1);
      INSERT INTO requirement_revision VALUES(1,1,'{"repository_id":1,"repository_version":1,"repository":{"model":"gpt-6-astra"},"contract":{"network_access":[]}}');
      INSERT INTO requirement_budget(requirement_id,limits) VALUES(1,'{"tokens":10000,"turns":10,"model_seconds":1000}');
      INSERT INTO budget_authorization(requirement_id,version,request_id,actor,reason,delta,limits) SELECT 1,1,'initial','local-user','review',limits,limits FROM requirement_budget;
      INSERT INTO execution_control(id,requirement_id,incarnation,recovery_complete) VALUES(1,1,'boot',true) ON CONFLICT(id) DO UPDATE SET requirement_id=1,incarnation='boot',recovery_complete=true,paused=false;
      UPDATE storage_guard SET blocked=false,error=NULL;"#).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO agent_run(id,requirement_id,revision,incarnation,request_id,workspace,workspace_identity,launch,state,model) VALUES('source',1,1,'boot','request',$1,'fixture','{}','Created','gpt-6-astra')").bind(root.to_str().unwrap()).execute(&pool).await.unwrap();
    pool
}
async fn session(pool: &PgPool) {
    runtime_store::open(pool, &key(), 100).await.unwrap();
    runtime_store::thread(pool, &key(), "thread", 100)
        .await
        .unwrap();
    runtime_store::turn(pool, &key(), "turn", "call", 100)
        .await
        .unwrap();
}

fn broker(root: &Path) -> GitBroker {
    std::fs::create_dir_all(root.join("canonical.git")).unwrap();
    std::fs::write(
        root.join("canonical.git/config"),
        "[core]\n\trepositoryformatversion = 0\n\tfilemode = true\n\tbare = true\n",
    )
    .unwrap();
    GitBroker::open(root).unwrap()
}

#[path = "support/diagnostics.rs"]
mod producer;
use producer::{binding, subprocess};
fn repair_tool(kind: &str, args: Value) -> Value {
    let mut request = tool(kind, args);
    request["params"]["threadId"] = json!("repair-thread");
    request
}
fn tool(kind: &str, args: Value) -> Value {
    json!({"id":"diagnostic-rpc","method":"item/tool/call","params":{"threadId":"thread","turnId":"turn","callId":"read","tool":kind,"arguments":args}})
}
async fn request(
    pool: &PgPool,
    git: &GitBroker,
    key: &RunKey,
    request: &Value,
    transcript: &mut Vec<Value>,
) -> Value {
    let mut request = request.clone();
    if key.run_id == "repair" {
        request["params"]["threadId"] = json!("repair-thread");
    }
    let response = runtime_tools::handle(pool, git, key, &request)
        .await
        .unwrap();
    transcript.push(json!({"request":request,"response":response}));
    assert_eq!(response["success"], true, "{response}");
    serde_json::from_str(response["contentItems"][0]["text"].as_str().unwrap()).unwrap()
}
async fn api(pool: &PgPool, path: &str) -> (u16, Value) {
    let response = diagnostic_api::routes()
        .with_state(pool.clone())
        .oneshot(
            axum::http::Request::builder()
                .uri(path)
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}
async fn page(pool: &PgPool, source: Option<&str>, after: i64) -> Page {
    let mut tx = pool.begin().await.unwrap();
    let page = diagnostic_store::list_in(&mut tx, 1, source, after)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    page
}
async fn content(pool: &PgPool, id: &str) -> (Artifact, Vec<u8>) {
    let mut tx = pool.begin().await.unwrap();
    let value = diagnostic_store::load_in(&mut tx, 1, None, id)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    value
}
async fn assert_retention_protected(pool: &PgPool, id: &str, deadline: i64) {
    diagnostic_service::expire(pool, deadline).await.unwrap();
    let (bytes_kept, still_live): (bool, bool) = sqlx::query_as(
        "SELECT raw_payload IS NOT NULL,retired_at IS NULL FROM diagnostic_artifact WHERE artifact_id=$1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .unwrap();
    assert!(
        bytes_kept && still_live,
        "active consumer lost its retained diagnostic"
    );
}
async fn reconstructed(pool: &PgPool, git: &GitBroker, id: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut offset = 0;
    let mut transcript = Vec::new();
    loop {
        let chunk: Chunk = serde_json::from_value(
            request(
                pool,
                git,
                &key(),
                &tool(
                    "read_diagnostic",
                    json!({"artifact_id":id,"offset":offset,"limit":8192}),
                ),
                &mut transcript,
            )
            .await,
        )
        .unwrap();
        assert_eq!(chunk.artifact.artifact_id, id);
        assert_eq!(chunk.offset, offset);
        bytes.extend_from_slice(chunk.text.as_bytes());
        offset = chunk.next;
        if chunk.end {
            assert_eq!(chunk.artifact.export_bytes, bytes.len() as u64);
            assert_eq!(chunk.artifact.export_sha256, Some(sha256(&bytes)));
            return bytes;
        }
    }
}

#[tokio::test]
async fn reconnect_concurrent_reads_late_attempts_and_historical_revocation_keep_scope() {
    let _scenario = DATABASE_SCENARIO.lock().await;
    let root = temporary();
    let pool = fixture(&root).await;
    session(&pool).await;
    let git = broker(&root.join("broker"));
    let limits = diagnostic_store::limits(&pool).await.unwrap();
    let old = format!("OLD-FIRST\n{}OLD-LAST\n", "old 中文 failure\n".repeat(2000));
    let newer = format!("NEW-FIRST\n{}NEW-LAST\n", "new failed\n".repeat(4000));
    let first_binding = binding("same-call", 1);
    fs::write(root.join("report.md"), &old).unwrap();
    let first = diagnostic_capture::capture(
        &root,
        "report.md",
        &first_binding,
        limits.file,
        false,
        limits.expires_at,
    )
    .unwrap();
    let first_id = first.artifact.artifact_id.clone();
    diagnostic_store::persist(&pool, "source", vec![first], &limits)
        .await
        .unwrap();
    let mut next_binding = binding("same-call", 2);
    next_binding.candidate.as_mut().unwrap().sha = "new-candidate".into();
    next_binding.candidate.as_mut().unwrap().tree = "new-tree".into();
    fs::write(root.join("report.md"), &newer).unwrap();
    let next = diagnostic_capture::capture(
        &root,
        "report.md",
        &next_binding,
        limits.file,
        false,
        limits.expires_at,
    )
    .unwrap();
    let next_id = next.artifact.artifact_id.clone();
    assert_ne!(first_id, next_id);
    diagnostic_store::persist(&pool, "source", vec![next], &limits)
        .await
        .unwrap();
    // A late producer response for the earlier attempt cannot replace its first
    // captured evidence or acquire the newer candidate's identity.
    let late = diagnostic_capture::capture(
        &root,
        "report.md",
        &first_binding,
        limits.file,
        false,
        limits.expires_at,
    )
    .unwrap();
    diagnostic_store::persist(&pool, "source", vec![late], &limits)
        .await
        .unwrap();
    let options = pool.connect_options();
    pool.close().await;
    fs::remove_file(root.join("report.md")).unwrap();
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect_with((*options).clone())
        .await
        .unwrap();
    let (old_read, next_read) = tokio::join!(
        reconstructed(&pool, &git, &first_id),
        reconstructed(&pool, &git, &next_id)
    );
    assert_eq!(old_read, old.trim_end().as_bytes());
    assert_eq!(next_read, newer.trim_end().as_bytes());
    let (repeated, concurrent) = tokio::join!(
        reconstructed(&pool, &git, &first_id),
        reconstructed(&pool, &git, &first_id)
    );
    assert_eq!(repeated, concurrent);
    assert_eq!(content(&pool, &first_id).await.0.binding, first_binding);
    assert_eq!(content(&pool, &next_id).await.0.binding, next_binding);
    sqlx::query("UPDATE plugin_scope SET enabled=false WHERE plugin_id='agent:codex'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        runtime_tools::handle(
            &pool,
            &git,
            &key(),
            &tool(
                "read_diagnostic",
                json!({"artifact_id":first_id,"offset":0,"limit":8192})
            )
        )
        .await
        .is_err()
    );
    sqlx::raw_sql(
        r#"INSERT INTO repository(id,version,document) VALUES(2,1,'{"revoked":false}');
      INSERT INTO requirement_revision VALUES(1,2,'{"repository_id":2,"repository_version":1}');
      UPDATE requirement SET revision=2,repository_id=2;
      UPDATE repository SET document='{"revoked":true}' WHERE id=1;"#,
    )
    .execute(&pool)
    .await
    .unwrap();
    let (status, listed) = api(&pool, "/api/requirements/1/diagnostics/0").await;
    assert_eq!(status, 200);
    assert_eq!(listed["artifacts"], json!([]));
    assert_eq!(
        api(
            &pool,
            &format!("/api/requirements/1/diagnostic-artifacts/{first_id}/0/8192")
        )
        .await
        .0,
        409
    );
    fs::write(root.join("restart-concurrency-evidence.json"), serde_json::to_vec_pretty(&json!({"kind":"database connection restart and real product tool requests; no paid model","old_artifact":first_id,"new_artifact":next_id,"old_export_sha256":sha256(&old_read),"new_export_sha256":sha256(&next_read),"concurrent_reads":true,"late_attempt_preserved":true,"agent_scope_revocation_denied":true,"historical_repository_revocation_denied":true})).unwrap()).unwrap();
    pool.close().await;
}
#[tokio::test]
async fn real_multilanguage_reports_are_read_through_product_tools_and_repair_scope() {
    let _scenario = DATABASE_SCENARIO.lock().await;
    let root = temporary();
    let pool = fixture(&root).await;
    session(&pool).await;
    let git = broker(&root.join("broker"));
    let (repo, mut plan) = subprocess(&root);
    plan.steps[0].command.push("controlled-assertion".into());
    let candidate = validation_runner::candidate(&repo).unwrap();
    let directory = root.join("output");
    let evidence = validation_runner::execute(&repo, &directory, &candidate, &plan).unwrap();
    assert_eq!(evidence[0].exit_code, Some(2));
    assert_eq!(
        codexsymphony_server::bounded_recovery::native_failure(&evidence[0].output),
        "check_exit"
    );
    assert_eq!(
        codexsymphony_server::extension_feedback::verdict(&evidence[1]),
        codexsymphony_server::controlled_contract::Verdict::Fail
    );
    let trusted = plan.identity().unwrap();
    let failure = codexsymphony_server::validation::ValidationEvidence {
        candidate: candidate.clone(),
        source_before: candidate.tree.clone(),
        source_after: candidate.tree.clone(),
        entry_before: trusted.protected_entry_sha256.clone(),
        entry_after: trusted.protected_entry_sha256.clone(),
        trusted,
        steps: evidence.clone(),
    };
    let original_failure = failure.clone();
    let required = vec!["shell".into(), "python".into()];
    let scope = codexsymphony_server::linked_repair::Scope::parse(
        r#"{"schema":"linked-repair/v1","checks":{"shell":["source"],"python":["attachment-source"]}}"#,
    )
    .unwrap();
    assert!(codexsymphony_server::linked_repair::failed_code(
        &failure, &required
    ));
    assert_eq!(
        scope.paths(&failure, &required).unwrap(),
        vec!["attachment-source", "source"]
    );
    assert_eq!(failure, original_failure);
    assert_eq!(
        codexsymphony_server::validation::verify(
            &failure,
            &failure.candidate,
            &failure.trusted,
            &required
        ),
        Err(codexsymphony_server::validation::ValidationError::ExitFailed)
    );
    let mut b = binding("validation", 1);
    b.candidate = Some(candidate.clone());
    diagnostic_service::plan(&pool, "source", &directory, &b, &plan)
        .await
        .unwrap();
    diagnostic_service::plan(&pool, "source", &directory, &b, &plan)
        .await
        .unwrap();
    let mut transcript = Vec::new();
    let listed: Page = serde_json::from_value(
        request(
            &pool,
            &git,
            &key(),
            &tool("list_diagnostics", json!({"after":0})),
            &mut transcript,
        )
        .await,
    )
    .unwrap();
    assert_eq!(listed.artifacts.len(), 4);
    // Reproduce the actual reader's missing-offset requests. Shape errors must
    // be actionable without exposing artifact existence or weakening scope.
    for artifact_id in [
        listed.artifacts[0].artifact_id.as_str(),
        "unrelated-artifact",
    ] {
        for args in [
            json!({"artifact_id":artifact_id,"limit":8192}),
            json!({"artifact_id":artifact_id,"offset":null,"limit":8192}),
            json!({"artifact_id":artifact_id,"offset":-1,"limit":8192}),
            json!({"artifact_id":artifact_id,"offset":"143000","limit":8192}),
            json!({"artifact_id":artifact_id,"offset":0,"limit":8192,"extra":true}),
        ] {
            let response =
                runtime_tools::handle(&pool, &git, &key(), &tool("read_diagnostic", args))
                    .await
                    .unwrap();
            assert_eq!(response["success"], false);
            let message = response["contentItems"][0]["text"].as_str().unwrap();
            assert!(message.starts_with("Invalid diagnostic arguments."));
            assert!(
                message.contains("offset (nonnegative integer, required even for a tail read)")
            );
            assert!(message.contains("report_blocker"));
            assert!(!message.contains(artifact_id));
        }
    }
    let denied = runtime_tools::handle(
        &pool,
        &git,
        &key(),
        &tool(
            "read_diagnostic",
            json!({"artifact_id":"unrelated-artifact","offset":0,"limit":128}),
        ),
    )
    .await
    .unwrap();
    assert_eq!(denied["success"], false);
    assert_eq!(
        denied["contentItems"][0]["text"],
        "Diagnostic unavailable, unauthorized, expired or invalid byte range; refresh the manifest. No authority or quality conclusion is implied."
    );
    for args in [json!({}), json!({"after":null})] {
        let response = runtime_tools::handle(&pool, &git, &key(), &tool("list_diagnostics", args))
            .await
            .unwrap();
        assert_eq!(response["success"], false);
        assert!(
            response["contentItems"][0]["text"]
                .as_str()
                .unwrap()
                .starts_with("Invalid diagnostic arguments.")
        );
    }
    let invalid_page = runtime_tools::handle(
        &pool,
        &git,
        &key(),
        &tool("list_diagnostics", json!({"after":-1})),
    )
    .await
    .unwrap();
    assert_eq!(invalid_page["success"], false);
    assert_eq!(
        invalid_page["contentItems"][0]["text"],
        denied["contentItems"][0]["text"]
    );
    let mut malformed = tool(
        "read_diagnostic",
        json!({"artifact_id":listed.artifacts[0].artifact_id,"offset":0,"limit":8192}),
    );
    malformed["params"]["callId"] = json!(true);
    assert!(
        runtime_tools::handle(&pool, &git, &key(), &malformed)
            .await
            .is_err()
    );
    malformed["params"]["callId"] = json!("read");
    malformed["params"]["namespace"] = json!("unapproved");
    assert!(
        runtime_tools::handle(&pool, &git, &key(), &malformed)
            .await
            .is_err()
    );
    // Corrupt retained metadata cannot silently become an authorized repair
    // context or expose the underlying parse/SQL error to the client.
    let first_id = &listed.artifacts[0].artifact_id;
    let original_manifest: Value =
        sqlx::query_scalar("SELECT manifest FROM diagnostic_artifact WHERE artifact_id=$1")
            .bind(first_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query("UPDATE diagnostic_artifact SET manifest=jsonb_set(manifest,'{availability}','\"invalid-state\"'::jsonb) WHERE artifact_id=$1").bind(first_id).execute(&pool).await.unwrap();
    let error = runtime_store::input(&pool, &key()).await.unwrap_err();
    assert!(error.to_string().contains("diagnostic context unavailable"));
    assert!(!error.to_string().contains("invalid-state"));
    // Retained-data deserialization failures are not public argument failures.
    for request in [
        tool(
            "read_diagnostic",
            json!({"artifact_id":first_id,"offset":0,"limit":128}),
        ),
        tool("list_diagnostics", json!({"after":0})),
    ] {
        let response = runtime_tools::handle(&pool, &git, &key(), &request)
            .await
            .unwrap();
        assert_eq!(response["success"], false);
        assert_eq!(
            response["contentItems"][0]["text"],
            denied["contentItems"][0]["text"]
        );
    }
    sqlx::query("UPDATE diagnostic_artifact SET manifest=$2 WHERE artifact_id=$1")
        .bind(first_id)
        .bind(original_manifest)
        .execute(&pool)
        .await
        .unwrap();
    let mut digest_records = Vec::new();
    for artifact in &listed.artifacts {
        assert_eq!(artifact.binding.candidate, Some(candidate.clone()));
        assert_eq!(artifact.availability, Availability::Available);
        let mut bytes = Vec::new();
        let mut offset = 0;
        loop {
            let chunk: Chunk = serde_json::from_value(
                request(
                    &pool,
                    &git,
                    &key(),
                    &tool(
                        "read_diagnostic",
                        json!({"artifact_id":artifact.artifact_id,"offset":offset,"limit":8192}),
                    ),
                    &mut transcript,
                )
                .await,
            )
            .unwrap();
            assert_eq!(chunk.offset, offset);
            bytes.extend_from_slice(chunk.text.as_bytes());
            offset = chunk.next;
            if chunk.end {
                break;
            }
        }
        assert_eq!(bytes.len() as u64, artifact.export_bytes);
        assert_eq!(Some(sha256(&bytes)), artifact.export_sha256);
        if artifact.export_bytes > 8192 {
            let text = String::from_utf8_lossy(&bytes);
            assert!(text.contains("FIRST"));
            assert!(text.contains("LAST"));
        }
        let (status, user) = api(
            &pool,
            &format!(
                "/api/requirements/1/diagnostic-artifacts/{}/0/8192",
                artifact.artifact_id
            ),
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(user["artifact"], json!(artifact));
        digest_records.push(json!({"artifact":artifact,"reconstructed_bytes":bytes.len(),"reconstructed_sha256":sha256(&bytes)}));
    }
    assert_eq!(
        api(&pool, "/api/requirements/1/diagnostics/0").await.1,
        json!(listed)
    );
    assert_eq!(
        api(&pool, "/api/requirements/999/diagnostics/0").await.0,
        409
    );
    assert_eq!(
        api(&pool, "/api/requirements/1/diagnostics/-1").await.0,
        409
    );
    assert_eq!(
        api(
            &pool,
            "/api/requirements/1/diagnostic-artifacts/forged/0/8192"
        )
        .await
        .0,
        409
    );
    // Restart/read after producer cleanup still uses retained database content.
    fs::remove_dir_all(&directory).unwrap();
    assert_eq!(page(&pool, Some("source"), 0).await.artifacts.len(), 4);
    sqlx::raw_sql("UPDATE agent_run SET quiescent=true,state='Interrupted'; INSERT INTO agent_run(id,requirement_id,revision,incarnation,request_id,workspace,workspace_identity,launch,state,model) VALUES('repair',1,1,'boot','repair-request','/fixture','fixture','{}','Created','gpt-6-astra')").execute(&pool).await.unwrap();
    let repair = RunKey {
        run_id: "repair".into(),
        request_id: "repair-request".into(),
        incarnation: "boot".into(),
    };
    runtime_store::open(&pool, &repair, 100).await.unwrap();
    runtime_store::thread(&pool, &repair, "repair-thread", 100)
        .await
        .unwrap();
    runtime_store::turn(&pool, &repair, "turn", "call", 100)
        .await
        .unwrap();
    assert!(page(&pool, Some("repair"), 0).await.artifacts.is_empty());
    assert_eq!(
        runtime_tools::handle(
            &pool,
            &git,
            &repair,
            &repair_tool(
                "read_diagnostic",
                json!({"artifact_id":listed.artifacts[0].artifact_id,"offset":0,"limit":8192})
            )
        )
        .await
        .unwrap()["success"],
        false
    );
    sqlx::query("INSERT INTO candidate_validation(id,requirement_id,revision,source_run_id,candidate_sha,candidate_tree,trusted,required_steps,source_before,source_after,entry_before,entry_after,stage,result) VALUES('validation',1,1,'source',$1,$2,'{}','[]','a','a','b','b','done','gate_failed')").bind(&candidate.sha).bind(&candidate.tree).execute(&pool).await.unwrap();
    sqlx::raw_sql("INSERT INTO repair_reservation(requirement_id,ordinal,source_validation_id,repair_run_id,failure,status) VALUES(1,1,'validation','repair','{}','started')").execute(&pool).await.unwrap();
    let context = diagnostic_tools::context(&pool, &repair).await.unwrap();
    assert_eq!(context["manifest"]["artifacts"], json!(listed.artifacts));
    let input = runtime_store::input(&pool, &repair).await.unwrap();
    assert!(input.contains("read_diagnostic"));
    let first = request(
        &pool,
        &git,
        &repair,
        &tool(
            "read_diagnostic",
            json!({"artifact_id":listed.artifacts[0].artifact_id,"offset":0,"limit":8192}),
        ),
        &mut transcript,
    )
    .await;
    assert_eq!(first["artifact"], json!(listed.artifacts[0]));
    // Replayed read must reauthorize after pause/cancel/revocation, not return a cached success.
    for statement in [
        "UPDATE execution_control SET paused=true",
        "UPDATE requirement SET cancel_requested=true",
        "UPDATE repository SET document='{\"revoked\":true}'",
    ] {
        sqlx::query(statement).execute(&pool).await.unwrap();
        let result = runtime_tools::handle(
            &pool,
            &git,
            &repair,
            &repair_tool("list_diagnostics", json!({"after":0})),
        )
        .await;
        assert!(match result {
            Err(_) => true,
            Ok(value) => value["success"] == false,
        });
        let malformed_denied = runtime_tools::handle(
            &pool,
            &git,
            &repair,
            &repair_tool(
                "read_diagnostic",
                json!({"artifact_id":listed.artifacts[0].artifact_id,"limit":8192}),
            ),
        )
        .await;
        if let Ok(value) = malformed_denied {
            assert_eq!(value["success"], false);
            assert_eq!(
                value["contentItems"][0]["text"], denied["contentItems"][0]["text"],
                "lifecycle denial must precede malformed-argument feedback"
            );
        }
        sqlx::raw_sql("UPDATE execution_control SET paused=false;UPDATE requirement SET cancel_requested=false;UPDATE repository SET document='{\"revoked\":false}'").execute(&pool).await.unwrap();
    }
    fs::write(root.join("tool-read-evidence.json"),serde_json::to_vec_pretty(&json!({"kind":"controlled fixture, no paid model","candidate":candidate,"requests_responses":transcript,"digests":digest_records})).unwrap()).unwrap();
    println!(
        "GH-126 tool evidence: {}",
        root.join("tool-read-evidence.json").display()
    );
    pool.close().await;
}

#[tokio::test]
async fn quotas_expiration_identity_corruption_and_pagination_are_truthful() {
    let _scenario = DATABASE_SCENARIO.lock().await;
    let root = temporary();
    let pool = fixture(&root).await;
    session(&pool).await;
    let mut limits = diagnostic_store::limits(&pool).await.unwrap();
    limits.call = 30;
    limits.task = 60;
    limits.count = 40;
    fs::write(root.join("log.txt"), "first line\nlast line\n").unwrap();
    let b = binding("small", 1);
    let capture =
        diagnostic_capture::capture(&root, "log.txt", &b, 10, false, limits.expires_at).unwrap();
    assert_eq!(capture.artifact.availability, Availability::Partial);
    let id = capture.artifact.artifact_id.clone();
    diagnostic_store::persist(&pool, "source", vec![capture], &limits)
        .await
        .unwrap();
    let (manifest, bytes) = content(&pool, &id).await;
    assert_eq!(manifest.original_bytes, Some(21));
    assert_eq!(manifest.retained_bytes, 10);
    assert!(bytes.is_empty());
    fs::write(root.join("log.txt"), "new contradictory evidence\n").unwrap();
    let capture =
        diagnostic_capture::capture(&root, "log.txt", &b, 100, false, limits.expires_at).unwrap();
    diagnostic_store::persist(&pool, "source", vec![capture], &limits)
        .await
        .unwrap();
    assert_eq!(content(&pool, &id).await.0, manifest);
    for field in ["binding", "purpose"] {
        let mut captured =
            diagnostic_capture::capture(&root, "log.txt", &b, 100, false, limits.expires_at)
                .unwrap();
        if field == "binding" {
            captured.artifact.binding.policy_digest = "forged".into();
        } else {
            captured.artifact.purpose = "forged".into();
        }
        assert!(
            diagnostic_store::persist(&pool, "source", vec![captured], &limits)
                .await
                .is_err()
        );
    }
    let newer = binding("small", 2);
    let capture =
        diagnostic_capture::capture(&root, "log.txt", &newer, 100, false, limits.expires_at)
            .unwrap();
    let newer_id = capture.artifact.artifact_id.clone();
    diagnostic_store::persist(&pool, "source", vec![capture], &limits)
        .await
        .unwrap();
    assert_ne!(id, newer_id);
    let list = page(&pool, None, 0).await;
    assert_eq!(list.artifacts[1].availability, Availability::Missing);
    assert!(list.artifacts[1].reason.as_ref().unwrap().contains("quota"));
    for n in 0..9 {
        let missing = diagnostic_capture::missing(
            &binding(&format!("missing-{n}"), 1),
            "absent",
            "not generated",
            limits.expires_at,
        )
        .unwrap();
        diagnostic_store::persist(&pool, "source", vec![missing], &limits)
            .await
            .unwrap();
    }
    let first = page(&pool, None, 0).await;
    assert_eq!(first.artifacts.len(), 8);
    let second = page(&pool, None, first.next.unwrap()).await;
    assert_eq!(second.artifacts.len(), 3);
    assert_eq!(second.next, None);
    let mut tx = pool.begin().await.unwrap();
    assert!(
        diagnostic_store::list_in(&mut tx, 1, None, -1)
            .await
            .is_err()
    );
    assert!(
        diagnostic_store::load_in(&mut tx, 2, None, &id)
            .await
            .is_err()
    );
    tx.rollback().await.unwrap();
    // Committed corruption state survives a rejected content request.
    sqlx::query("UPDATE diagnostic_artifact SET export_payload=$2 WHERE artifact_id=$1")
        .bind(&id)
        .bind(b"tampered".as_slice())
        .execute(&pool)
        .await
        .unwrap();
    let (bad, bytes) = content(&pool, &id).await;
    assert_eq!(bad.availability, Availability::Corrupt);
    assert!(chunk(bad, &bytes, 0, 4).is_err());
    limits.count = 1;
    assert!(
        diagnostic_store::persist(
            &pool,
            "source",
            vec![
                diagnostic_capture::missing(
                    &binding("count", 1),
                    "absent",
                    "missing",
                    limits.expires_at
                )
                .unwrap()
            ],
            &limits
        )
        .await
        .is_err()
    );
    diagnostic_service::expire(&pool, limits.expires_at)
        .await
        .unwrap();
    assert_ne!(
        page(&pool, None, 0).await.artifacts[0].availability,
        Availability::Expired
    );
    sqlx::query("UPDATE agent_run SET quiescent=true,state='Interrupted'")
        .execute(&pool)
        .await
        .unwrap();
    // Expiry must also respect consumers that exist between active processes.
    // These are retained control facts, not a new diagnostic recovery machine.
    sqlx::query("UPDATE agent_run SET user_paused=true WHERE id='source'")
        .execute(&pool)
        .await
        .unwrap();
    assert_retention_protected(&pool, &id, limits.expires_at).await;
    sqlx::query("UPDATE agent_run SET user_paused=false WHERE id='source'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO runtime_resume(source_run,job,status) VALUES('source','{}','prepared')",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert_retention_protected(&pool, &id, limits.expires_at).await;
    sqlx::query("UPDATE runtime_resume SET status='dispatched' WHERE source_run='source'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO candidate_validation(id,requirement_id,revision,source_run_id,candidate_sha,candidate_tree,trusted,required_steps,source_before,source_after,entry_before,entry_after,stage,result) VALUES('expiry-failure',1,1,'source','commit','tree','{}','[]','commit','commit','entry','entry','validation','gate_failed')")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO repair_reservation(requirement_id,ordinal,source_validation_id,failure,status) VALUES(1,1,'expiry-failure','{}','reserved')")
        .execute(&pool).await.unwrap();
    assert_retention_protected(&pool, &id, limits.expires_at).await;
    sqlx::query("UPDATE repair_reservation SET status='failed' WHERE requirement_id=1")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE candidate_validation SET result='succeeded',stage='handoff' WHERE id='expiry-failure'")
        .execute(&pool).await.unwrap();
    assert_retention_protected(&pool, &id, limits.expires_at).await;
    sqlx::query("UPDATE candidate_validation SET stage='done' WHERE id='expiry-failure'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO workspace_operation(run_id,request_id,command,status) VALUES('source','expiry-preserve','{}','pending')")
        .execute(&pool).await.unwrap();
    assert_retention_protected(&pool, &id, limits.expires_at).await;
    sqlx::query("UPDATE workspace_operation SET status='complete',result='{}' WHERE request_id='expiry-preserve'")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO project_hook_run(run_id,requirement_id,revision,resource_id,workspace,role,frozen) VALUES('source',1,1,'fixture','fixture','coding','{}')")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO project_hook_invocation(invocation_id,run_id,resource_id,event,hook_name,status,stop_confirmed,output_dir) VALUES('expiry-unknown-hook','source','fixture','after_run','fixture','unknown',false,'fixture')")
        .execute(&pool).await.unwrap();
    assert_retention_protected(&pool, &id, limits.expires_at).await;
    sqlx::query("UPDATE project_hook_invocation SET stop_confirmed=true WHERE invocation_id='expiry-unknown-hook'")
        .execute(&pool).await.unwrap();
    sqlx::raw_sql("CREATE FUNCTION expiration_disk_fault() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'controlled expiration disk failure' USING ERRCODE='53100'; END $$; CREATE TRIGGER expiration_disk_fault BEFORE UPDATE OF raw_payload ON diagnostic_artifact FOR EACH ROW EXECUTE FUNCTION expiration_disk_fault();").execute(&pool).await.unwrap();
    assert!(
        diagnostic_service::expire(&pool, limits.expires_at)
            .await
            .is_err()
    );
    assert_ne!(
        page(&pool, None, 0).await.artifacts[0].availability,
        Availability::Expired
    );
    sqlx::query("DROP TRIGGER expiration_disk_fault ON diagnostic_artifact")
        .execute(&pool)
        .await
        .unwrap();
    diagnostic_service::expire(&pool, limits.expires_at)
        .await
        .unwrap();
    assert_eq!(
        page(&pool, None, 0).await.artifacts[0].availability,
        Availability::Expired
    );
    assert_eq!(
        api(
            &pool,
            &format!("/api/requirements/1/diagnostic-artifacts/{id}/0/4")
        )
        .await
        .0,
        409
    );
    let bytes:(i64,i64)=sqlx::query_as("SELECT count(*) FILTER(WHERE raw_payload IS NOT NULL),count(*) FILTER(WHERE export_payload IS NOT NULL) FROM diagnostic_artifact").fetch_one(&pool).await.unwrap();
    assert_eq!(bytes, (0, 0));
    pool.close().await;
}

#[tokio::test]
async fn subprocess_failures_limits_and_atomic_capture_failure_preserve_truth() {
    let _scenario = DATABASE_SCENARIO.lock().await;
    let root = temporary();
    let pool = fixture(&root).await;
    let (repo, mut plan) = subprocess(&root);
    let candidate = validation_runner::candidate(&repo).unwrap();
    plan.steps.truncate(1);
    plan.steps[0].timeout_seconds = 1;
    let mut states = Vec::new();
    for mode in ["timeout", "crash", "malformed", "flood"] {
        plan.steps[0].id = "shell".into();
        plan.steps[0].command = vec!["/gate-entry".into(), mode.into()];
        if mode == "malformed" {
            plan.steps[0].id = "python".into();
            plan.steps[0].command = vec![
                "/gate-entry".into(),
                "--symphony-feedback-v2".into(),
                mode.into(),
            ];
        }
        let directory = root.join(mode);
        let result = validation_runner::execute_limited(&repo, &directory, &candidate, &plan, 512);
        if mode == "flood" {
            assert!(result.is_err());
        } else {
            let result = result.unwrap();
            if mode != "malformed" {
                assert_eq!(result[0].exit_code, None);
            } else {
                assert_eq!(
                    codexsymphony_server::extension_feedback::verdict(&result[0]),
                    codexsymphony_server::controlled_contract::Verdict::Unknown
                );
            }
        }
        diagnostic_service::plan(&pool, "source", &directory, &binding(mode, 1), &plan)
            .await
            .unwrap();
    }
    let list = page(&pool, None, 0).await;
    assert_eq!(list.artifacts.len(), 4);
    for artifact in list.artifacts {
        let mode = artifact.binding.identity.invocation_id.clone();
        assert_eq!(
            artifact.availability,
            if mode == "flood" {
                Availability::Partial
            } else {
                Availability::Available
            }
        );
        states.push(json!({"mode":mode,"manifest":artifact}));
    }
    let directory = root.join("not-started");
    diagnostic_service::plan(
        &pool,
        "source",
        &directory,
        &binding("start-failure", 1),
        &plan,
    )
    .await
    .unwrap();
    assert_eq!(
        page(&pool, None, 0)
            .await
            .artifacts
            .last()
            .unwrap()
            .availability,
        Availability::Missing
    );
    // Hard cancellation is a real child-group stop, with retained output; no producer replay.
    let directory = root.join("cancel");
    let flag = std::sync::atomic::AtomicBool::new(false);
    plan.steps[0].command = vec!["/gate-entry".into(), "timeout".into()];
    plan.steps[0].timeout_seconds = 10;
    let result = std::thread::scope(|scope| {
        scope.spawn(|| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while !fs::metadata(directory.join("step-0.log")).is_ok_and(|meta| meta.len() > 0) {
                assert!(
                    std::time::Instant::now() < deadline,
                    "producer did not emit its cancellation evidence"
                );
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            flag.store(true, std::sync::atomic::Ordering::Release);
        });
        validation_runner::execute_cancellable(&repo, &directory, &candidate, &plan, &flag)
    });
    assert!(result.is_err());
    assert!(directory.join("stop.json").is_file());
    diagnostic_service::plan(&pool, "source", &directory, &binding("cancel", 1), &plan)
        .await
        .unwrap();
    assert_eq!(
        page(&pool, None, 0)
            .await
            .artifacts
            .last()
            .unwrap()
            .availability,
        Availability::Partial
    );
    let limits = diagnostic_store::limits(&pool).await.unwrap();
    fs::write(root.join("disk.log"), "retained producer bytes\n").unwrap();
    sqlx::raw_sql("CREATE FUNCTION reject_diagnostic_bytes() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.raw_payload IS NOT NULL THEN RAISE EXCEPTION 'controlled disk full' USING ERRCODE='53100'; END IF; RETURN NEW; END $$;CREATE TRIGGER reject_capture BEFORE INSERT ON diagnostic_artifact FOR EACH ROW EXECUTE FUNCTION reject_diagnostic_bytes()").execute(&pool).await.unwrap();
    let captured = diagnostic_capture::capture(
        &root,
        "disk.log",
        &binding("disk-full", 1),
        1024,
        false,
        limits.expires_at,
    )
    .unwrap();
    assert!(
        diagnostic_store::persist(&pool, "source", vec![captured], &limits)
            .await
            .is_err()
    );
    assert_eq!(
        fs::read_to_string(root.join("disk.log")).unwrap(),
        "retained producer bytes\n"
    );
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM diagnostic_artifact WHERE invocation_id='disk-full'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 0);
    fs::write(
        root.join("failure-state-evidence.json"),
        serde_json::to_vec_pretty(&states).unwrap(),
    )
    .unwrap();
    println!(
        "GH-126 failure capture evidence: {}",
        root.join("failure-state-evidence.json").display()
    );
    pool.close().await;
}

fn deployment(root: &Path) -> codexsymphony_server::storage_store::Deployment {
    use codexsymphony_server::{
        storage_files::Directory,
        storage_lifecycle::{CATEGORIES, Limit, Policy},
        storage_store::{Deployment, Root},
    };
    let paths = ["execution", "cold", "database"].map(|name| {
        let path = root.join(name);
        fs::create_dir(&path).unwrap();
        Root {
            identity: Directory::open(&path).unwrap().identity().unwrap(),
            path,
        }
    });
    let [execution, cold, database_filesystem] = paths;
    Deployment {
        execution,
        cold,
        database_filesystem,
        database_extras: vec![],
        policy: Policy {
            version: "diagnostics-v1".into(),
            reason: "controlled capacity acceptance".into(),
            global_bytes: 4 << 30,
            control_bytes: 256 << 20,
            run_bytes: 32 << 20,
            requirement_bytes: 128 << 20,
            entry_bytes: 1 << 20,
            entry_count: 10000,
            categories: CATEGORIES
                .into_iter()
                .map(|category| {
                    (
                        category,
                        Limit {
                            bytes: 2 << 30,
                            seconds: 86400,
                            reserve_bytes: 1 << 20,
                        },
                    )
                })
                .collect(),
        },
    }
}
#[tokio::test]
async fn installed_storage_policy_and_actual_database_capacity_limit_capture() {
    let _scenario = DATABASE_SCENARIO.lock().await;
    let root = temporary();
    let pool = fixture(&root).await;
    let mut config = deployment(&root);
    codexsymphony_server::storage_store::install(&pool, &config)
        .await
        .unwrap();
    let limits = diagnostic_store::limits(&pool).await.unwrap();
    assert_eq!(limits.file, config.policy.entry_bytes);
    fs::write(root.join("log.md"), "capacity evidence\n").unwrap();
    let capture = diagnostic_capture::capture(
        &root,
        "log.md",
        &binding("capacity", 1),
        limits.file,
        false,
        limits.expires_at,
    )
    .unwrap();
    diagnostic_store::persist(&pool, "source", vec![capture], &limits)
        .await
        .unwrap();
    assert_eq!(
        page(&pool, None, 0).await.artifacts[0].availability,
        Availability::Available
    );
    config.policy.version = "diagnostics-v2".into();
    config.policy.global_bytes = 1 << 20;
    config.policy.control_bytes = 65536;
    codexsymphony_server::storage_store::install(&pool, &config)
        .await
        .unwrap();
    let limits = diagnostic_store::limits(&pool).await.unwrap();
    let capture = diagnostic_capture::capture(
        &root,
        "log.md",
        &binding("capacity", 2),
        limits.file,
        false,
        limits.expires_at,
    )
    .unwrap();
    diagnostic_store::persist(&pool, "source", vec![capture], &limits)
        .await
        .unwrap();
    assert_eq!(
        page(&pool, None, 0).await.artifacts[1].availability,
        Availability::Missing
    );
    pool.close().await;
}

#[tokio::test]
async fn excessive_registration_preserves_bounded_manifest_and_explicit_omission() {
    let _scenario = DATABASE_SCENARIO.lock().await;
    let root = temporary();
    let pool = fixture(&root).await;
    let (_, mut plan) = subprocess(&root);
    let step = plan.steps[0].clone();
    plan.steps = (0..70)
        .map(|i| {
            let mut step = step.clone();
            step.id = format!("step-{i}");
            step
        })
        .collect();
    diagnostic_service::plan(
        &pool,
        "source",
        &root.join("missing"),
        &binding("many", 1),
        &plan,
    )
    .await
    .unwrap();
    let total: i64 = sqlx::query_scalar("SELECT count(*) FROM diagnostic_artifact")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(total, 64);
    let omitted:Value=sqlx::query_scalar("SELECT manifest FROM diagnostic_artifact WHERE manifest->>'purpose'='diagnostic-manifest-limit'").fetch_one(&pool).await.unwrap();
    assert_eq!(omitted["availability"], "missing");
    assert!(omitted["reason"].as_str().unwrap().contains("quota"));
    pool.close().await;
}
