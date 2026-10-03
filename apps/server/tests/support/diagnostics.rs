use codexsymphony_server::{
    diagnostics::Binding,
    validation::sha256,
    validation_runner::{Plan, Step},
};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
};
pub fn binding(call: &str, attempt: u32) -> Binding {
    Binding {
        identity: codexsymphony_server::extension_contract::InvocationIdentity {
            protocol_version: 2,
            requirement_id: 1,
            revision: 1,
            run_id: Some("source".into()),
            resource_id: "workspace".into(),
            invocation_id: call.into(),
            attempt,
            config_id: "configuration".into(),
        },
        phase: "validation".into(),
        candidate: Some(codexsymphony_server::validation::Candidate {
            sha: "commit".into(),
            tree: "tree".into(),
            immutable: true,
        }),
        validation_id: Some(call.into()),
        generation: attempt as i64,
        implementation_digest: "implementation".into(),
        environment_digest: "environment".into(),
        policy_digest: "policy".into(),
    }
}
fn git(root: &Path, args: &[&str]) {
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap()
            .status
            .success()
    );
}
pub fn subprocess(root: &Path) -> (PathBuf, Plan) {
    let repo = root.join("repo");
    fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "--quiet"]);
    git(&repo, &["config", "user.name", "fixture"]);
    git(&repo, &["config", "user.email", "fixture@example.com"]);
    fs::write(repo.join("source"), "candidate").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "candidate"]);
    let entry = root.join("entry");
    fs::write(
        &entry,
        include_str!("../fixtures/diagnostics/controlled-reports.sh"),
    )
    .unwrap();
    fs::set_permissions(&entry, fs::Permissions::from_mode(0o755)).unwrap();
    let plan = Plan {
        entry_sha256: sha256(fs::read(&entry).unwrap()),
        entry,
        steps: vec![
            Step {
                id: "shell".into(),
                command: vec!["/gate-entry".into()],
                timeout_seconds: 10,
                code_failure: true,
            },
            Step {
                id: "python".into(),
                command: vec!["/gate-entry".into(), "--symphony-feedback-v2".into()],
                timeout_seconds: 10,
                code_failure: true,
            },
        ],
    };
    (repo, plan)
}
