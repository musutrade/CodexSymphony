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

#[test]
fn cancellation_stops_validation_and_preserves_incomplete_invocation() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let (root, repo, mut plan) = fixture();
    plan.steps[0].command.push("timeout".into());
    plan.steps[0].timeout_seconds = 30;
    let candidate = runner::candidate(&repo).unwrap();
    let directory = root.join("cancelled");
    let flag = Arc::new(AtomicBool::new(false));
    let signal = flag.clone();
    let trigger = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(150));
        signal.store(true, Ordering::Release);
    });
    let started = std::time::Instant::now();
    let error =
        runner::execute_cancellable(&repo, &directory, &candidate, &plan, &flag).unwrap_err();
    trigger.join().unwrap();
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    assert!(
        error
            .to_string()
            .contains("stopped by current control intent")
    );
    assert!(directory.join("binding.json").exists());
    assert!(directory.join("step-0.log").exists());
    assert!(!directory.join("result.json").exists());
    assert!(runner::execute(&repo, &directory, &candidate, &plan).is_err());
    fs::remove_dir_all(root).unwrap();
}

fn hook_call(
    candidate: &codexsymphony_server::validation::Candidate,
    plan: &Plan,
) -> codexsymphony_server::controlled_contract::Call {
    use codexsymphony_server::{
        controlled_contract::{Call, Operation, SourceIdentity},
        extension_contract::InvocationIdentity,
    };
    Call {
        identity: InvocationIdentity {
            protocol_version: 1,
            requirement_id: 120,
            revision: 1,
            run_id: Some("coding".into()),
            resource_id: "project".into(),
            invocation_id: "validate".into(),
            attempt: 1,
            config_id: sha256("approved configuration"),
        },
        controlled_config_digest: sha256("reviewed registry"),
        operation: Operation::Validate,
        extension_id: "project-checks".into(),
        implementation_digest: plan.entry_sha256.clone(),
        candidate: Some(SourceIdentity {
            commit: candidate.sha.clone(),
            tree: candidate.tree.clone(),
        }),
        environment_digest: sha256("actual environment"),
        policy_digest: sha256("policy and baseline"),
        deadline_unix_ms: (codexsymphony_server::runtime_client::now() + 30) * 1000,
        required_checks: vec!["test".into()],
    }
}

#[test]
fn two_reviewed_hooks_produce_consumable_p9_evidence_without_harness_gate() {
    use codexsymphony_server::{controlled_contract::Verdict, validation_hook};
    let (root, repo, mut plan) = fixture();
    let candidate = runner::candidate(&repo).unwrap();
    for (name, script) in [
        ("shell", "#!/bin/sh\ntest -f source && cat source\n"),
        (
            "python",
            "#!/usr/bin/python3\nfrom pathlib import Path\nassert Path('source').read_text() == 'candidate'\nprint('project test passed')\n",
        ),
    ] {
        fs::write(&plan.entry, script).unwrap();
        plan.entry_sha256 = sha256(script);
        let call = hook_call(&candidate, &plan);
        let directory = root.join(name);
        let (report, steps) =
            validation_hook::execute(&repo, &directory, &candidate, &plan, &call).unwrap();
        assert_eq!(report.verdict, Verdict::Pass);
        report
            .check_pass(&call, codexsymphony_server::runtime_client::now() * 1000)
            .unwrap();
        assert_eq!(steps.len(), 1);
        assert_eq!(
            report.checks[0].evidence[0].sha256,
            sha256(fs::read(&report.checks[0].evidence[0].artifact_id).unwrap())
        );
        assert_eq!(
            validation_hook::execute(&repo, &directory, &candidate, &plan, &call)
                .unwrap()
                .0,
            report
        );
        let alias = root.join(format!("{name}-alias"));
        std::os::unix::fs::symlink(&plan.entry, &alias).unwrap();
        let mut aliased = plan.clone();
        aliased.entry = alias;
        assert!(
            validation_hook::execute(
                &repo,
                &root.join(format!("{name}-aliased")),
                &candidate,
                &aliased,
                &call
            )
            .is_err()
        );
        let mut stale = call.clone();
        stale.identity.revision += 1;
        assert!(validation_hook::execute(&repo, &directory, &candidate, &plan, &stale).is_err());
        stale = call.clone();
        stale.deadline_unix_ms = 1;
        assert!(validation_hook::execute(&repo, &directory, &candidate, &plan, &stale).is_err());
        stale = call.clone();
        stale.required_checks.push("skipped-test".into());
        assert!(
            validation_hook::execute(&repo, &root.join("missing"), &candidate, &plan, &stale)
                .is_err()
        );
        fs::write(directory.join("evaluation.json"), "{}").unwrap();
        assert!(validation_hook::execute(&repo, &directory, &candidate, &plan, &call).is_err());
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn retained_results_cannot_replace_approved_commands_or_omit_checks() {
    let (root, repo, plan) = fixture();
    let candidate = runner::candidate(&repo).unwrap();
    let directory = root.join("retained");
    let original = runner::execute(&repo, &directory, &candidate, &plan).unwrap();
    let mut bad = original.clone();
    bad[0].command = vec!["skipped-test".into()];
    for changed in [bad, Vec::new()] {
        fs::write(
            directory.join("result.json"),
            serde_json::to_vec(&changed).unwrap(),
        )
        .unwrap();
        assert!(runner::execute(&repo, &directory, &candidate, &plan).is_err());
    }
    let mut duplicate = plan.clone();
    duplicate.steps.push(duplicate.steps[0].clone());
    assert!(duplicate.identity().is_err());
    let mut embedded = plan.clone();
    embedded.entry = repo.join("unreviewed-hook");
    fs::copy(&plan.entry, &embedded.entry).unwrap();
    assert!(runner::execute(&repo, &root.join("embedded"), &candidate, &embedded).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn p9_nonzero_and_unknown_steps_never_become_pass() {
    use codexsymphony_server::{controlled_contract::Verdict, validation_hook};
    let (root, repo, plan) = fixture();
    let candidate = runner::candidate(&repo).unwrap();
    for (mode, expected) in [("fail", Verdict::Fail), ("timeout", Verdict::Unknown)] {
        let mut selected = plan.clone();
        selected.steps[0].command.push(mode.into());
        let call = hook_call(&candidate, &selected);
        let (evaluation, _) =
            validation_hook::execute(&repo, &root.join(mode), &candidate, &selected, &call)
                .unwrap();
        assert_eq!(evaluation.verdict, expected);
        assert!(
            evaluation
                .check_pass(&call, codexsymphony_server::runtime_client::now() * 1000)
                .is_err()
        );
        assert!(root.join(mode).join("step-0.log").is_file());
        assert!(root.join(mode).join("evaluation.json").is_file());
    }
    fs::remove_dir_all(root).unwrap();
}
