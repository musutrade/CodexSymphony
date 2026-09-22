use super::*;
use crate::merge_test_support::client;
use crate::merge_test_support::fixture;

#[tokio::test]
async fn checkout_reuse_checks_head_and_invalid_fetch_never_certifies_source() {
    let (pool, root, intent, _) = fixture::fixture().await;
    let repo = root.join("repo");
    let bundle = root.join("seed.bundle");
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args(["bundle", "create"])
        .arg(&bundle)
        .arg("--all")
        .output()
        .unwrap();
    assert!(output.status.success());
    let broker = GitBroker::initialize(&root.join("workspaces"), &bundle).unwrap();
    let id = "delivery-source";
    let workspace = Workspace {
        key: crate::execution::RunKey {
            run_id: id.into(),
            request_id: id.into(),
            incarnation: "test".into(),
        },
        identity: id.into(),
        requirement: 1,
        revision: 1,
        phase: "validation".into(),
        baseline: intent.head.clone(),
        branch: format!("ai/req-1-{id}"),
        path: broker.path(id).unwrap().to_string_lossy().into_owned(),
    };
    broker.prepare(&workspace, true).unwrap();
    let manifest = broker.preserve(&workspace).unwrap();
    sqlx::query("UPDATE delivery SET manifest=$1")
        .bind(serde_json::json!(manifest))
        .execute(&pool)
        .await
        .unwrap();
    let mut client = client::client(&root);
    let path = checkout(&pool, &mut client, &root, &intent, &intent.head, 100)
        .await
        .unwrap();
    assert_eq!(
        checkout(&pool, &mut client, &root, &intent, &intent.head, 100)
            .await
            .unwrap(),
        path
    );
    // A missing/invalid remote object must remain an error; never use the parent HEAD.
    assert!(
        checkout(&pool, &mut client, &root, &intent, "invalid-object", 100)
            .await
            .is_err()
    );
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(&path)
        .args([
            "-c",
            "user.name=test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "--allow-empty",
            "-m",
            "changed checkout",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        checkout(&pool, &mut client, &root, &intent, &intent.head, 100)
            .await
            .unwrap_err()
            .to_string()
            .contains("saved merge checkout identity differs")
    );
    pool.close().await;
    std::fs::remove_dir_all(root).unwrap();
}
