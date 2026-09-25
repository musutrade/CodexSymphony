use codexsymphony_server::{
    github::{Policy, Selector, Source},
    github_credentials::{FileProvider, Provider, separate_check_identity},
};
#[test]
fn trusted_check_publisher_cannot_be_the_delivery_identity() {
    let mut policy = Policy {
        repository_id: 1,
        repository: "owner/repo".into(),
        default_branch: "main".into(),
        version: 1,
        required: vec![Selector {
            name: "trusted".into(),
            source: Source::CheckRun { app_id: 2 },
        }],
        wait_seconds: 60,
        delivery: None,
    };
    assert!(!separate_check_identity(&policy, 2));
    assert!(separate_check_identity(&policy, 3));
    policy.required.clear();
    assert!(separate_check_identity(&policy, 2));
}
#[test]
fn credential_provider_rejects_workspace_relative_key_paths_without_reading_them() {
    let provider = FileProvider {
        api_url: "https://api.github.com/",
        app_id: 1,
        private_key: std::path::Path::new("candidate.pem"),
    };
    assert!(provider.client().is_err());
}

#[path = "support/delivery.rs"]
mod database_fixture;
#[tokio::test]
async fn local_only_deployment_never_loads_or_probes_app_configuration() {
    let pool = database_fixture::database().await;
    sqlx::query("UPDATE repository SET document=jsonb_set(document,'{delivery}','\"local_git\"')")
        .execute(&pool)
        .await
        .unwrap();
    let handle = codexsymphony_server::github_service::start_path(
        &pool,
        std::path::Path::new("/missing-credential-configuration"),
    )
    .await
    .unwrap();
    handle.await.unwrap();
    pool.close().await;
}

#[test]
fn delivery_identity_is_separate_from_both_contract_phases() {
    use serde_json::json;
    let check = |app| {
        json!({"selector":{"name":"trusted","source":{"kind":"check_run","app_id":app}},
        "applicability":"always","trigger":{"kind":"pull_request"},"job":null})
    };
    let mut policy: Policy = serde_json::from_value(json!({
        "repository_id":1,"repository":"owner/repo","default_branch":"main","version":1,
        "required":[],"wait_seconds":60,"delivery":{
            "schema_version":1,
            "pre_merge":{"source":"head","checkout":"head","checks":[check(2)],"wait_seconds":60},
            "post_merge":{"kind":"checks","checks":[check(3)],"wait_seconds":60,"probe_pr":1},
            "actions":{"rerun_actions":false,"rerequest_checks":false,"merge":false,"merge_method":null,"read_logs":false},
            "protection":{},"rules":[]
        }
    })).unwrap();
    assert!(!separate_check_identity(&policy, 2));
    assert!(!separate_check_identity(&policy, 3));
    assert!(separate_check_identity(&policy, 4));
    policy.delivery.as_mut().unwrap().pre_merge.checks.clear();
    assert!(separate_check_identity(&policy, 2));
    assert!(!separate_check_identity(&policy, 3));
}
