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
    let failure: Failure = sqlx::query_as("SELECT * FROM linked_failure WHERE id=$1")
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
    let mut none = auth;
    for r in &mut none.repositories {
        r.repair_scope = "not authorized".into();
    }
    assert!(select_integration_target(none, &binding, &[], &evidence, &binding.required).is_err());
    pool.close().await;
    std::fs::remove_dir_all(root).unwrap();
}
