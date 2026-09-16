use codexsymphony_server::{
    validation::sha256,
    validation_runner::{Plan, Step},
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
pub fn fixture() -> (PathBuf, PathBuf, Plan) {
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
