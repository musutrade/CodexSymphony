use codexsymphony_server::{
    validation::sha256,
    validation_runner::{self as runner, Plan, Step},
};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
};
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
fn fixture() -> (PathBuf, PathBuf, Plan) {
    let root = std::env::temp_dir().join(format!(
        "validation-runner-{}",
        codexsymphony_server::process::new_identity().unwrap()
    ));
    fs::create_dir(&root).unwrap();
    let repo = root.join("repo");
    fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "--quiet"]);
    git(&repo, &["config", "user.name", "test"]);
    git(&repo, &["config", "user.email", "test@example.com"]);
    fs::write(repo.join("source"), "candidate").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "candidate"]);
    let entry = root.join("entry");
    fs::write(&entry,"#!/bin/sh\ncat /candidate/source\ntest ! -e /home/gem/.codex/auth.json || exit 9\ncase \"$1\" in fail) exit 1;; timeout) sleep 10;; flood) yes flood;; mutate) echo changed >> /candidate/source;; *) exit 0;; esac\n").unwrap();
    fs::set_permissions(&entry, fs::Permissions::from_mode(0o755)).unwrap();
    let sandbox = PathBuf::from("/usr/bin/bwrap");
    let plan = Plan {
        entry_sha256: sha256(fs::read(&entry).unwrap()),
        entry,
        sandbox_sha256: sha256(fs::read(&sandbox).unwrap()),
        sandbox,
        steps: vec![Step {
            id: "test".into(),
            command: vec!["/gate-entry".into()],
            timeout_seconds: 1,
            code_failure: true,
        }],
    };
    (root, repo, plan)
}
#[test]
fn fixed_process_and_reconciliation() {
    let (root, repo, plan) = fixture();
    let candidate = runner::candidate(&repo).unwrap();
    let directory = root.join("success");
    let evidence = runner::execute(&repo, &directory, &candidate, &plan).unwrap();
    assert_eq!(evidence[0].exit_code, Some(0));
    assert_eq!(evidence[0].output, "candidate");
    assert_eq!(
        runner::execute(&repo, &directory, &candidate, &plan).unwrap(),
        evidence
    );
    let mut changed = plan.clone();
    changed.steps[0].command.push("fail".into());
    assert!(runner::execute(&repo, &directory, &candidate, &changed).is_err());
    for (name, exit) in [("fail", Some(1)), ("timeout", None), ("mutate", Some(2))] {
        let mut p = plan.clone();
        p.steps[0].command.push(name.into());
        let result = runner::execute(&repo, &root.join(name), &candidate, &p).unwrap();
        assert_eq!(result[0].exit_code, exit);
        assert_eq!(runner::candidate(&repo).unwrap(), candidate);
    }
    let mut flood = plan.clone();
    flood.steps[0].command.push("flood".into());
    assert!(runner::execute(&repo, &root.join("flood"), &candidate, &flood).is_err());
    assert!(runner::execute(&repo, &root.join("flood"), &candidate, &flood).is_err());
    let mut wrong = candidate.clone();
    wrong.sha = "changed".into();
    assert!(runner::execute(&repo, &root.join("wrong"), &wrong, &plan).is_err());
    fs::write(repo.join("untracked"), "dirty").unwrap();
    assert!(runner::candidate(&repo).is_err());
    assert!(runner::candidate(&root).is_err());
}
#[test]
fn protected_plan_rejects_tampering() {
    let (_root, _repo, plan) = fixture();
    let mut bad = plan.clone();
    bad.steps.clear();
    assert!(bad.identity().is_err());
    let mut bad = plan.clone();
    bad.steps[0].command = vec!["/bin/true".into()];
    assert!(bad.identity().is_err());
    let mut bad = plan.clone();
    bad.steps[0].timeout_seconds = 0;
    assert!(bad.identity().is_err());
    let mut bad = plan.clone();
    bad.sandbox_sha256 = "wrong".into();
    assert!(bad.identity().is_err());
    fs::write(&plan.entry, "#!/bin/sh\ntrue\n").unwrap();
    assert!(plan.identity().is_err());
}
