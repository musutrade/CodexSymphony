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
    fs::write(&entry,r#"#!/bin/sh
case "$1" in
 --symphony-feedback-v2) if [ "$2" = malformed ]; then printf '{malformed'; exit 0; fi; exec python3 -c 'import os,json,pathlib; d=pathlib.Path(os.environ["SYMPHONY_DIAGNOSTIC_DIR"]); report="FAIL-FIRST\n"+"测量 failed\n"*6000+"FAIL-LAST\n"; (d/"report.md").write_text(report); (d/"single.json").write_text(json.dumps({"failures":["FIRST"]+["failure"]*10000+["LAST"]})); print(json.dumps({"protocol_version":2,"check_id":"python","verdict":"fail","fault":None,"artifacts":[{"kind":"report","path":"report.md"},{"kind":"report","path":"single.json"}]}))';;
 timeout) printf 'failure before timeout\n'; sleep 10;;
 crash) printf 'failure before crash\n'; kill -9 $$;;
 malformed) printf '{malformed';;
 flood) yes 'failure overflowing capture';;
 *) printf 'FAIL-FIRST\n'; i=0; while [ "$i" -lt 6000 ]; do printf 'shell failure item %s\n' "$i"; i=$((i+1)); done; printf 'FAIL-LAST\n'; exit 2;;
esac
"#).unwrap();
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
