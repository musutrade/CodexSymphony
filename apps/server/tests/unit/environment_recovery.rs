use super::*;
use serde_json::json;
use std::fs;

struct Temporary(PathBuf);
impl Temporary {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "environment-recovery-{}",
            process::new_identity().unwrap()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for Temporary {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn fixture() -> (Temporary, PathBuf, Command, Host) {
    let root = Temporary::new();
    let host = Host::read().unwrap();
    let invocation = process::new_identity().unwrap();
    let directory = root.path().join(&invocation);
    fs::create_dir(&directory).unwrap();
    let request = json!({"resource":{"protocol_version":2,"invocation_id":invocation,"attempt":1,"resource_id":"fixture","controlled_config_digest":"a".repeat(64),"environment":{"repository_revision":"repository:1@1","contract_digest":"b".repeat(64),"host_profile_ref":"fixture","role":"test"},"extension_id":"fixture","implementation_digest":"c".repeat(64),"deadline_unix_ms":1},"call":null,"invocation_id":invocation,"stage":"recovery","role":"test","plan_digest":"d".repeat(64),"host_profile":"fixture","resource_root":root.path(),"workspace":null,"required_checks":["environment"]});
    process::durable_write(&directory.join("input.json"), &request).unwrap();
    let key = crate::execution::RunKey {
        run_id: invocation.clone(),
        request_id: invocation.clone(),
        incarnation: invocation.clone(),
    };
    let receipt = Receipt {
        key,
        process: crate::execution::ProcessIdentity {
            pid: 99999999,
            group: 99999999,
            start_ticks: 1,
            boot_id: host.boot.clone(),
        },
    };
    process::durable_write(&directory.join("identity.json"), &receipt).unwrap();
    fs::write(directory.join("launched"), "").unwrap();
    let command = Command {
        request_id: "recover-test".into(),
        invocation_id: invocation,
        evidence_sha256: binding(&directory).unwrap().evidence_sha256,
        reason: "Controlled missing supervisor receipt".into(),
    };
    (root, directory, command, host)
}

#[test]
fn prepare_does_not_release_same_boot_and_later_boot_preserves_original_unknown() {
    let (root, directory, command, host) = fixture();
    let originals = ["input.json", "identity.json", "launched"]
        .map(|name| (name, fs::read(directory.join(name)).unwrap()));
    assert!(!stopped(&directory).unwrap());
    let prepared = execute(root.path(), "prepare-recovery", &command, &host).unwrap();
    assert_eq!(prepared["quiescence_proven"], false);
    assert_eq!(
        prepared,
        execute(root.path(), "prepare-recovery", &command, &host).unwrap()
    );
    assert!(execute(root.path(), "reconcile", &command, &host).is_err());
    assert!(!directory.join("host-recovery-proof.json").exists());
    let later = Host {
        machine: host.machine.clone(),
        boot: process::new_identity().unwrap(),
    };
    assert!(execute(root.path(), "prepare-recovery", &command, &later).is_err());
    let accepted = execute(root.path(), "reconcile", &command, &later).unwrap();
    assert_eq!(accepted["quiescence_proven"], true);
    assert_eq!(accepted["prior_result"], "unknown");
    assert_eq!(
        accepted,
        execute(root.path(), "reconcile", &command, &later).unwrap()
    );
    verify(&directory, &later).unwrap();
    assert!(verify(&directory, &host).is_err());
    assert!(stopped(&directory).is_err());
    let next = Host {
        machine: host.machine.clone(),
        boot: process::new_identity().unwrap(),
    };
    verify(&directory, &next).unwrap();
    assert!(execute(root.path(), "reconcile", &command, &next).is_err());
    assert!(!directory.join("quiescent.json").exists());
    assert!(!directory.join("exit.json").exists());
    for (name, bytes) in originals {
        assert_eq!(fs::read(directory.join(name)).unwrap(), bytes);
    }
}

#[test]
fn changed_origin_host_directory_and_request_cannot_reuse_a_stop_proof() {
    let (root, directory, command, host) = fixture();
    let mut bad = command.clone();
    bad.evidence_sha256 = "bad".into();
    assert!(execute(root.path(), "prepare-recovery", &bad, &host).is_err());
    bad = command.clone();
    bad.reason = String::new();
    assert!(execute(root.path(), "prepare-recovery", &bad, &host).is_err());
    bad = command.clone();
    bad.request_id = String::new();
    assert!(execute(root.path(), "prepare-recovery", &bad, &host).is_err());
    execute(root.path(), "prepare-recovery", &command, &host).unwrap();
    bad = command.clone();
    bad.reason = "Conflicting decision".into();
    assert!(execute(root.path(), "prepare-recovery", &bad, &host).is_err());
    let foreign = Host {
        machine: "f".repeat(32),
        boot: process::new_identity().unwrap(),
    };
    assert!(execute(root.path(), "reconcile", &command, &foreign).is_err());
    let later = Host {
        machine: host.machine.clone(),
        boot: process::new_identity().unwrap(),
    };
    execute(root.path(), "reconcile", &command, &later).unwrap();
    assert!(verify(&directory, &foreign).is_err());
    let path = directory.join("host-recovery-proof.json");
    let mut proof: Proof = process::read(&path).unwrap();
    proof.observed_boot = "invalid".into();
    process::durable_write(&path, &proof).unwrap();
    assert!(verify(&directory, &later).is_err());
    proof.observed_boot = later.boot.clone();
    proof.origin.binding.inode += 1;
    process::durable_write(&path, &proof).unwrap();
    assert!(verify(&directory, &later).is_err());
    let moved = root.path().join("moved");
    fs::rename(&directory, &moved).unwrap();
    assert!(binding(&moved).is_err());
}

#[test]
fn missing_aliased_oversize_and_substituted_originals_are_rejected() {
    let (root, directory, command, host) = fixture();
    assert!(execute(root.path(), "reconcile", &command, &host).is_err());
    fs::write(directory.join("launched"), "forged").unwrap();
    assert!(binding(&directory).is_err());
    fs::remove_file(directory.join("launched")).unwrap();
    std::os::unix::fs::symlink(directory.join("input.json"), directory.join("launched")).unwrap();
    assert!(binding(&directory).is_err());
    fs::remove_file(directory.join("launched")).unwrap();
    fs::write(directory.join("launched"), "").unwrap();
    let alias = root.path().join("alias");
    std::os::unix::fs::symlink(&directory, &alias).unwrap();
    assert!(binding(&alias).is_err());
    assert!(super::directory(root.path(), "alias").is_err());
    assert!(super::directory(root.path(), "../outside").is_err());
    let input = directory.join("input.json");
    let bytes = fs::read(&input).unwrap();
    fs::write(&input, vec![b'x'; 65537]).unwrap();
    assert!(binding(&directory).is_err());
    fs::write(&input, bytes).unwrap();
    let identity = directory.join("identity.json");
    let receipt: Receipt = process::read(&identity).unwrap();
    let mut changed = receipt.clone();
    changed.key.run_id = "different".into();
    process::durable_write(&identity, &changed).unwrap();
    assert!(binding(&directory).is_err());
    changed = receipt.clone();
    changed.process.boot_id = "invalid".into();
    process::durable_write(&identity, &changed).unwrap();
    assert!(binding(&directory).is_err());
    process::durable_write(&identity, &receipt).unwrap();
    fs::write(directory.join("quiescent.json"), "{}").unwrap();
    assert!(execute(root.path(), "prepare-recovery", &command, &host).is_err());
    fs::remove_file(directory.join("quiescent.json")).unwrap();
    fs::remove_file(&identity).unwrap();
    std::os::unix::fs::symlink(&input, &identity).unwrap();
    assert!(binding(&directory).is_err());
}

#[test]
fn malformed_kernel_boot_ids_are_rejected() {
    assert!(!valid_boot("short"));
    let old = process::new_identity().unwrap();
    assert!(verify_boot(&old, &old, &process::new_identity().unwrap()).is_err());
    assert!(!valid_boot("00000000x0000-0000-0000-000000000000"));
    assert!(!valid_boot("z0000000-0000-0000-0000-000000000000"));
}
