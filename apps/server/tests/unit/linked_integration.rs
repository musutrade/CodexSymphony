use super::*;
use crate::linked_repair_worker::tests::setup;
use serde_json::json;
#[tokio::test]
async fn updated_combo_gets_new_checkouts_and_full_independent_validation() {
    let _serial = crate::linked_repair_worker::tests::SERIAL.lock().await;
    let (pool, root, broker, config, _) = setup().await;
    let plan = config.validation.unwrap();
    let old = crate::validation_runner::candidate(&root.join("repo")).unwrap();
    let (other_root, other_repo, _) = crate::merge_test_support::fixture::source::fixture();
    std::fs::write(other_repo.join("source"), "second repository").unwrap();
    assert!(
        std::process::Command::new("git")
            .arg("-C")
            .arg(&other_repo)
            .args(["commit", "-am", "second source"])
            .status()
            .unwrap()
            .success()
    );
    let second = crate::validation_runner::candidate(&other_repo).unwrap();
    assert!(
        std::process::Command::new("git")
            .arg("-C")
            .arg(root.join("workspaces/canonical.git"))
            .arg("fetch")
            .arg(&other_repo)
            .arg(&second.sha)
            .status()
            .unwrap()
            .success()
    );
    let mut job = Job {
        invocation: "original".into(),
        binding: crate::integration::Binding {
            requirement: 1,
            revision: 1,
            authorization: 1,
            input_sha256: "input".into(),
            versions: vec![
                crate::integration::Version {
                    repository_id: 1,
                    github_repository_id: 7,
                    repository_version: 1,
                    candidate: old.clone(),
                    artifacts: vec!["original".into()],
                },
                crate::integration::Version {
                    repository_id: 2,
                    github_repository_id: 8,
                    repository_version: 1,
                    candidate: second.clone(),
                    artifacts: vec![],
                },
            ],
            trusted: plan.identity().unwrap(),
            required: vec!["test".into()],
        },
        plan,
        checkouts: vec![],
        output_limit: 1048576,
    };
    std::fs::write(root.join("repo/source"), "repaired source").unwrap();
    assert!(
        std::process::Command::new("git")
            .arg("-C")
            .arg(root.join("repo"))
            .args(["commit", "-am", "repair"])
            .status()
            .unwrap()
            .success()
    );
    let repaired = crate::validation_runner::candidate(&root.join("repo")).unwrap();
    assert!(
        std::process::Command::new("git")
            .arg("-C")
            .arg(root.join("workspaces/canonical.git"))
            .arg("fetch")
            .arg(root.join("repo"))
            .arg(&repaired.sha)
            .status()
            .unwrap()
            .success()
    );
    let repair = Repair {
        failure: "failed-integration".into(),
        repository: 1,
        version: json!({"candidate":repaired}),
        previous: "previous".into(),
        job: json!(job),
        launch: json!({}),
    };
    update_version(&mut job, &repair).unwrap();
    assert_eq!(job.binding.versions[1].candidate, second);
    assert_eq!(job.binding.versions[0].candidate, repaired);
    assert_ne!(job.invocation, "original");
    let mut tx = crate::run_store::lock(&pool).await.unwrap();
    assert!(
        checkout_all(&mut tx, &broker, "boot", &mut job)
            .await
            .unwrap()
    );
    tx.commit().await.unwrap();
    assert_eq!(
        crate::integration_process::versions(&job).unwrap(),
        job.binding.versions
    );
    let previous = Launch {
        key: crate::execution::RunKey {
            run_id: "old".into(),
            request_id: "old".into(),
            incarnation: "old".into(),
        },
        workspace: "old".into(),
        workspace_identity: "old".into(),
        program: "old".into(),
        args: vec![],
    };
    let new_launch = launch(&root, Path::new("/bin/true"), "boot", &job, json!(previous)).unwrap();
    assert_eq!(new_launch.key.run_id, job.invocation);
    assert_eq!(new_launch.workspace, job.checkouts[0].to_string_lossy());
    let directory = root.join(&job.invocation);
    std::fs::create_dir(&directory).unwrap();
    crate::process::durable_write(&directory.join("job.json"), &job).unwrap();
    crate::integration_process::run(&directory).unwrap();
    let outcome = crate::integration_process::reconcile(&directory, &job).unwrap();
    assert!(crate::integration::verify(
        &job.binding,
        &job.binding.versions,
        outcome.evidence.as_ref().unwrap()
    ));
    assert_ne!(outcome.evidence.unwrap().candidate, old);
    pool.close().await;
    std::fs::remove_dir_all(root).unwrap();
    std::fs::remove_dir_all(other_root).unwrap();
}
