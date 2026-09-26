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
    let error = execute(
        &pool,
        &intent,
        path.clone(),
        root.join("missing-required-check"),
        plan(&pool, &intent).await.unwrap(),
        vec!["missing-required-check".into()],
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("validation failed"));
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

#[tokio::test]
async fn scope_revocation_stops_merge_validation_permission_without_removing_intent() {
    let (pool, _root, intent, _) = fixture::fixture().await;
    assert!(execution_allowed(&pool, &intent).await.unwrap());
    sqlx::query("UPDATE plugin_scope SET enabled=false WHERE plugin_id='validation:native'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(!execution_allowed(&pool, &intent).await.unwrap());
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM requirement WHERE id=$1)")
        .bind(intent.requirement)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(exists);
}
