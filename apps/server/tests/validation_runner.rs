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
    fs::write(repo.join(".gitignore"), "build/\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "candidate"]);
    let entry = root.join("entry");
    fs::write(&entry,"#!/bin/sh\ncat source\ntest -z \"$GITHUB_TOKEN$GH_ENTERPRISE_TOKEN$CUSTOM_TRACKER_SECRET$DATABASE_URL\" || exit 9\ncase \"$1\" in artifact) mkdir -p build; head -c 2097152 /dev/zero > build/output;; fail) exit 1;; timeout) sleep 10;; flood) yes flood;; mutate) echo changed >> source;; *) exit 0;; esac\n").unwrap();
    fs::set_permissions(&entry, fs::Permissions::from_mode(0o755)).unwrap();
    let plan = Plan {
        entry_sha256: sha256(fs::read(&entry).unwrap()),
        entry,
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
    let mut artifact = plan.clone();
    artifact.steps[0].command.push("artifact".into());
    assert_eq!(
        runner::execute(&repo, &root.join("artifact"), &candidate, &artifact).unwrap()[0].exit_code,
        Some(0)
    );
    assert_eq!(
        fs::metadata(repo.join("build/output")).unwrap().len(),
        2097152
    );
    let mut changed = plan.clone();
    changed.steps[0].command.push("fail".into());
    assert!(runner::execute(&repo, &directory, &candidate, &changed).is_err());
    for (name, exit) in [("fail", Some(1)), ("timeout", None)] {
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
    let mut mutate = plan.clone();
    mutate.steps[0].command.push("mutate".into());
    assert!(runner::execute(&repo, &root.join("mutate"), &candidate, &mutate).is_err());
    assert!(!root.join("mutate/result.json").exists());
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
    bad.entry_sha256 = "wrong".into();
    assert!(bad.identity().is_err());
    fs::write(&plan.entry, "#!/bin/sh\ntrue\n").unwrap();
    assert!(plan.identity().is_err());
}

#[test]
fn validation_does_not_inherit_control_plane_credentials() {
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "fixed_process_and_reconciliation"])
        .env("GITHUB_TOKEN", "synthetic-token")
        .env("GH_ENTERPRISE_TOKEN", "synthetic-enterprise")
        .env("CUSTOM_TRACKER_SECRET", "synthetic-custom")
        .env("DATABASE_URL", "synthetic-control-database")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn deployment_mount_boundary_runs_real_candidate_validation() {
    let (root, repo, mut plan) = fixture();
    let private = root.join("private");
    fs::create_dir(&private).unwrap();
    fs::set_permissions(&private, fs::Permissions::from_mode(0o700)).unwrap();
    let sentinel = private.join("sentinel");
    fs::write(&sentinel, "non-sensitive-control-sentinel").unwrap();
    fs::set_permissions(&sentinel, fs::Permissions::from_mode(0o600)).unwrap();
    let config = root.join("boundary.json");
    fs::write(
        &config,
        serde_json::to_vec(&serde_json::json!({
            "version":1,"role":"validation","workspace_root":root,
            "runtime_home_root":null,"mounts":[{"path":"/usr","writable":false}],
            "private_paths":[private],"environment":{},"program":"/usr/bin/dash"
        }))
        .unwrap(),
    )
    .unwrap();
    fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
    // Keep the deployed executor in the server source snapshot used by coverage.
    // Materialize the exact compiled source outside the candidate mount.
    let executor = root.join("executor.py");
    fs::write(&executor, include_str!("../deployment/executor.py")).unwrap();
    fs::write(
        &plan.entry,
        format!(
            "#!/bin/sh\nexec /usr/bin/python3 -I '{}' '{}' -c 'test ! -r {} && test ! -e {} && cat source'\n",
            executor.display(), config.display(), sentinel.display(), config.display()
        ),
    )
    .unwrap();
    plan.entry_sha256 = sha256(fs::read(&plan.entry).unwrap());
    plan.steps[0].timeout_seconds = 10;
    let candidate = runner::candidate(&repo).unwrap();
    let directory = root.join("isolated-validation");
    let result = runner::execute(&repo, &directory, &candidate, &plan).unwrap();
    assert_eq!(result[0].exit_code, Some(0), "{}", result[0].output);
    assert_eq!(result[0].output, "candidate");
    assert_eq!(
        runner::execute(&repo, &directory, &candidate, &plan).unwrap(),
        result
    );
    fs::remove_file(directory.join("result.json")).unwrap();
    assert!(runner::execute(&repo, &directory, &candidate, &plan).is_err());
    assert!(directory.join("binding.json").exists());
}
