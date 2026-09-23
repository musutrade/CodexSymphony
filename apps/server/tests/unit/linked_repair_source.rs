use super::*;
use crate::linked_repair_worker::tests::setup;
struct Local {
    repo: std::path::PathBuf,
    head: String,
}
impl Remote for Local {
    async fn baseline(&mut self, _: &crate::github::Policy) -> Result<String> {
        Ok(self.head.clone())
    }
    async fn fetch(&mut self, _: &crate::github::Policy, path: &Path, sha: &str) -> Result<()> {
        assert!(
            std::process::Command::new("git")
                .arg("-C")
                .arg(path)
                .arg("fetch")
                .arg(&self.repo)
                .arg(sha)
                .status()?
                .success()
        );
        Ok(())
    }
}
fn scope() -> String {
    json!({"schema":"linked-repair/v1","checks":{"test":["source"]}}).to_string()
}
#[tokio::test]
async fn current_target_is_frozen_without_reusing_failed_candidate_and_retries_are_finite() {
    let _serial = crate::linked_repair_worker::tests::SERIAL.lock().await;
    let (pool, root, _, _, id) = setup().await;
    sqlx::query("UPDATE linked_failure SET baseline=NULL")
        .execute(&pool)
        .await
        .unwrap();
    let f: Failure = sqlx::query_as("SELECT * FROM linked_failure WHERE id=$1")
        .bind(&id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let input = json!({"child":{"kind":"code_change","repository_id":1},"review":{"repository_version":1,"repair_scope":scope()}});
    let t = target(&pool, &f, &input).await.unwrap();
    assert_eq!(t.repository, 1);
    let mut bad = input.clone();
    bad["child"]["repository_id"] = json!(2);
    assert!(target(&pool, &f, &bad).await.is_err());
    let repo = root.join("repo");
    std::fs::write(repo.join("source"), "advanced target").unwrap();
    assert!(
        std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["commit", "-am", "target advanced"])
            .status()
            .unwrap()
            .success()
    );
    let head = crate::validation_runner::candidate(&repo).unwrap().sha;
    let mut remote = Local {
        repo,
        head: head.clone(),
    };
    attempt_source(&pool, &mut remote, &root, 0, &f, t)
        .await
        .unwrap();
    let (baseline, evidence): (String, Value) =
        sqlx::query_as("SELECT baseline,evidence FROM linked_failure WHERE id=$1")
            .bind(&id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(baseline, head);
    assert_eq!(evidence, f.evidence);
    assert_ne!(evidence["candidate"]["sha"], baseline);
    sqlx::query("UPDATE linked_failure SET source_attempts=0,next_source_at=0")
        .execute(&pool)
        .await
        .unwrap();
    assert!(source_attempt(&pool, &id, 100).await.unwrap());
    assert!(source_attempt(&pool, &id, 200).await.unwrap());
    assert!(source_attempt(&pool, &id, 400).await.unwrap());
    assert!(!source_attempt(&pool, &id, 600).await.unwrap());
    sqlx::query("UPDATE linked_failure SET state='observed',baseline=NULL,next_source_at=0")
        .execute(&pool)
        .await
        .unwrap();
    let mut client = crate::merge_test_support::client::client(&root);
    let intent: crate::automatic_merge::Intent = serde_json::from_value(
        sqlx::query_scalar::<_, Value>("SELECT intent FROM merge_operation")
            .fetch_one(&pool)
            .await
            .unwrap(),
    )
    .unwrap();
    // Transport failure cannot invent a new target baseline.
    assert!(
        RemoteGit {
            client: &mut client,
            now: 1000
        }
        .baseline(&intent.policy)
        .await
        .is_err()
    );
    tick(&pool, &mut client, &root, 1000).await.unwrap();
    let reason: String = sqlx::query_scalar("SELECT blocker FROM linked_failure")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(reason.contains("scope absent"));
    tick(&pool, &mut client, &root, 1000).await.unwrap();
    pool.close().await;
    std::fs::remove_dir_all(root).unwrap();
}
#[tokio::test]
async fn reviewed_multi_repository_check_mapping_selects_one_repository_at_a_time() {
    let _serial = crate::linked_repair_worker::tests::SERIAL.lock().await;
    let (pool, root, _, _, id) = setup().await;
    let evidence: Value = sqlx::query_scalar("SELECT evidence FROM linked_failure WHERE id=$1")
        .bind(&id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let evidence: ValidationEvidence = serde_json::from_value(evidence).unwrap();
    let repositories: Vec<_> = (1..=2)
        .map(|id| crate::integration::Repository {
            repository_id: id,
            repository_version: 1,
            selection: crate::integration::Selection::Fixed {
                sha: evidence.candidate.sha.clone(),
            },
            repair_scope: scope(),
        })
        .collect();
    let auth = crate::integration::Authorization {
        configuration_sha256: evidence.trusted.config_sha256.clone(),
        repositories,
    };
    let binding = crate::integration::Binding {
        requirement: 1,
        revision: 1,
        authorization: 1,
        input_sha256: "input".into(),
        versions: (1..=2)
            .map(|id| crate::integration::Version {
                repository_id: id,
                repository_version: 1,
                github_repository_id: id + 6,
                candidate: evidence.candidate.clone(),
                artifacts: vec![],
            })
            .collect(),
        trusted: evidence.trusted.clone(),
        required: vec!["test".into()],
    };
    assert_eq!(
        select_integration_target(auth.clone(), &binding, &[], &evidence, &binding.required)
            .unwrap()
            .repository,
        1
    );
    assert_eq!(
        select_integration_target(auth.clone(), &binding, &[1], &evidence, &binding.required)
            .unwrap()
            .repository,
        2
    );
    let input = json!({"child":{"kind":"validation_only"},"review":{"integration":auth}});
    assert_eq!(
        merged_target(2, &input, &evidence, &binding.required)
            .unwrap()
            .repository,
        2
    );
    assert!(merged_target(3, &input, &evidence, &binding.required).is_err());
    let mut failure: Failure = sqlx::query_as("SELECT * FROM linked_failure WHERE id=$1")
        .bind(&id)
        .fetch_one(&pool)
        .await
        .unwrap();
    // A mapping alone is insufficient: an actual quiescent failed integration
    // record must exist before its repository versions can become repair inputs.
    assert!(
        integration_target(&pool, &failure, &input, &evidence, &binding.required)
            .await
            .is_err()
    );
    crate::merge_test_support::reviewed_group(&pool, input.clone()).await;
    sqlx::query("INSERT INTO integration_validation(id,requirement_id,authorization_id,revision,binding,job,launch,state,quiescent) VALUES('failed-combination',1,1,1,$1,'{}','{}','failed',true)")
        .bind(json!(binding)).execute(&pool).await.unwrap();
    failure.merge_key = None;
    failure.integration_id = Some("failed-combination".into());
    assert_eq!(target(&pool, &failure, &input).await.unwrap().repository, 1);
    sqlx::query("UPDATE linked_failure SET repository_id=1,state='merged'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(target(&pool, &failure, &input).await.unwrap().repository, 2);
    let mut none = auth;
    for r in &mut none.repositories {
        r.repair_scope = "not authorized".into();
    }
    assert!(select_integration_target(none, &binding, &[], &evidence, &binding.required).is_err());
    pool.close().await;
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn source_transport_failure_and_pause_race_preserve_the_original_failure() {
    let _serial = crate::linked_repair_worker::tests::SERIAL.lock().await;
    let (pool, root, _, _, id) = setup().await;
    sqlx::query("UPDATE linked_failure SET baseline=NULL")
        .execute(&pool)
        .await
        .unwrap();
    let f: Failure = sqlx::query_as("SELECT * FROM linked_failure WHERE id=$1")
        .bind(&id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let input = json!({"child":{"kind":"code_change","repository_id":1},"review":{"repository_version":1,"repair_scope":scope()}});
    let repo = root.join("repo");
    let head = crate::validation_runner::candidate(&repo).unwrap().sha;
    let mut remote = Local { repo, head };
    sqlx::query("UPDATE requirement SET paused=true")
        .execute(&pool)
        .await
        .unwrap();
    attempt_source(
        &pool,
        &mut remote,
        &root,
        100,
        &f,
        target(&pool, &f, &input).await.unwrap(),
    )
    .await
    .unwrap();
    let baseline: Option<String> = sqlx::query_scalar("SELECT baseline FROM linked_failure")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(baseline.is_none());
    sqlx::query("UPDATE requirement SET paused=false")
        .execute(&pool)
        .await
        .unwrap();
    let mut unrelated = target(&pool, &f, &input).await.unwrap();
    unrelated.affected = "f".repeat(40);
    attempt_source(&pool, &mut remote, &root, 200, &f, unrelated)
        .await
        .unwrap();
    let reason: String = sqlx::query_scalar("SELECT blocker FROM linked_failure")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(reason.contains("does not contain the affected version"));
    sqlx::query("UPDATE linked_failure SET state='observed',source_attempts=0,next_source_at=0")
        .execute(&pool)
        .await
        .unwrap();
    crate::merge_test_support::reviewed_group(&pool, input.clone()).await;
    let mut client = crate::merge_test_support::client::client(&root);
    let intent: crate::automatic_merge::Intent = serde_json::from_value(
        sqlx::query_scalar::<_, Value>("SELECT intent FROM merge_operation")
            .fetch_one(&pool)
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(
        RemoteGit {
            client: &mut client,
            now: 1000
        }
        .fetch(&intent.policy, &root, "invalid")
        .await
        .is_err()
    );
    tick(&pool, &mut client, &root, 1000).await.unwrap();
    let (state, receipts, saved): (String, Value, Value) =
        sqlx::query_as("SELECT state,source_receipts,evidence FROM linked_failure")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(state, "observed");
    assert_eq!(receipts.as_array().unwrap().len(), 1);
    assert_eq!(saved, f.evidence);
    sqlx::query("UPDATE linked_failure SET source_attempts=3,next_source_at=0")
        .execute(&pool)
        .await
        .unwrap();
    attempt_source(
        &pool,
        &mut remote,
        &root,
        2000,
        &f,
        target(&pool, &f, &input).await.unwrap(),
    )
    .await
    .unwrap();
    pool.close().await;
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn github_target_requires_an_authenticated_exact_branch_sha() {
    let _serial = crate::linked_repair_worker::tests::SERIAL.lock().await;
    let (pool, root, _, _, _) = setup().await;
    let intent: crate::automatic_merge::Intent = serde_json::from_value(
        sqlx::query_scalar::<_, Value>("SELECT intent FROM merge_operation")
            .fetch_one(&pool)
            .await
            .unwrap(),
    )
    .unwrap();
    let branch = std::sync::Arc::new(std::sync::Mutex::new(json!({"object":{"sha":intent.head}})));
    let value = branch.clone();
    let router = axum::Router::new().fallback(move |request: axum::extract::Request| {
        let value = value.clone();
        async move {
            assert!(request.headers().contains_key("authorization"));
            axum::Json(match request.uri().path() {
                "/repos/owner/repo/installation" => json!({"id":7,"app_id":42}),
                "/app/installations/7/access_tokens" => json!({"token":"disposable-fixture","expires_at":"2099-01-01T00:00:00Z","permissions":{}}),
                "/repos/owner/repo/git/ref/heads/main" => value.lock().unwrap().clone(),
                other => panic!("unexpected source request {other}"),
            })
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let _ = crate::merge_test_support::client::client(&root);
    let mut client =
        AppClient::new(&url, 42, &std::fs::read(root.join("unit-key.pem")).unwrap()).unwrap();
    let mut remote = RemoteGit {
        client: &mut client,
        now: 1000,
    };
    assert_eq!(remote.baseline(&intent.policy).await.unwrap(), intent.head);
    *branch.lock().unwrap() = json!({"object":{}});
    assert!(
        remote
            .baseline(&intent.policy)
            .await
            .unwrap_err()
            .to_string()
            .contains("SHA absent")
    );
    task.abort();
    pool.close().await;
    std::fs::remove_dir_all(root).unwrap();
}
