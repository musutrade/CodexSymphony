use super::*;
use serde_json::json;
use std::fs;

struct Temporary(PathBuf);
impl Temporary {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "incomplete-recovery-{}",
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
    let id = process::new_identity().unwrap();
    let directory = root.path().join(&id);
    fs::create_dir(&directory).unwrap();
    let request = json!({"resource":{"protocol_version":2,"invocation_id":id,"attempt":1,"resource_id":"fixture","controlled_config_digest":"a".repeat(64),"environment":{"repository_revision":"repository:1@1","contract_digest":"b".repeat(64),"host_profile_ref":"fixture","role":"test"},"extension_id":"fixture","implementation_digest":"c".repeat(64),"deadline_unix_ms":1},"call":null,"invocation_id":id,"stage":"recovery","role":"test","plan_digest":"d".repeat(64),"host_profile":"fixture","resource_root":root.path(),"workspace":null,"required_checks":["environment"]});
    process::durable_write(&directory.join("input.json"), &request).unwrap();
    fs::write(directory.join("not-started.tmp"), "").unwrap();
    fs::write(directory.join("storage-heartbeat.tmp"), "").unwrap();
    let command = Command {
        request_id: "incomplete-test".into(),
        invocation_id: id,
        evidence_sha256: sha256(fs::read(directory.join("input.json")).unwrap()),
        reason: "Original incomplete local probe retained".into(),
    };
    (root, directory, command, host)
}
fn later(host: &Host) -> Host {
    Host {
        machine: host.machine.clone(),
        boot: process::new_identity().unwrap(),
    }
}

#[test]
fn real_boot_boundary_is_required_and_original_unknown_files_are_preserved() {
    let (root, directory, command, host) = fixture();
    let originals = snapshot(&directory).unwrap();
    assert!(!stopped_on_host(&directory, &host).unwrap());
    assert!(!stopped(&directory).unwrap());
    assert!(execute(root.path(), "reconcile-incomplete", &command, &host).is_err());
    let prepared = execute(root.path(), "prepare-incomplete", &command, &host).unwrap();
    assert_eq!(prepared["quiescence_proven"], false);
    assert_eq!(prepared["native_receipt_created"], false);
    assert_eq!(prepared["prior_result"], "unknown");
    assert_eq!(
        prepared,
        execute(root.path(), "prepare-incomplete", &command, &host).unwrap()
    );
    assert!(execute(root.path(), "reconcile-incomplete", &command, &host).is_err());
    assert!(!directory.join("host-incomplete-proof.json").exists());
    let next = later(&host);
    let result = execute(root.path(), "reconcile-incomplete", &command, &next).unwrap();
    assert_eq!(result["quiescence_proven"], true);
    assert_eq!(result["started"], false);
    assert_eq!(result["prior_result"], "unknown");
    assert_eq!(
        result,
        execute(root.path(), "reconcile-incomplete", &command, &next).unwrap()
    );
    assert!(stopped_on_host(&directory, &next).unwrap());
    assert!(stopped_on_host(&directory, &host).is_err());
    assert!(stopped(&directory).is_err());
    assert!(crate::environment_recovery::stopped(&directory).is_err());
    verify(&directory, &later(&next)).unwrap();
    assert!(execute(root.path(), "reconcile-incomplete", &command, &later(&next)).is_err());
    assert_eq!(snapshot(&directory).unwrap(), originals);
    for name in [
        "identity.json",
        "launched",
        "quiescent.json",
        "not-started.json",
        "exit.json",
    ] {
        assert!(!directory.join(name).exists());
    }
}

#[test]
fn changed_inputs_native_files_host_directory_and_decisions_cannot_release_probe() {
    let (root, directory, command, host) = fixture();
    execute(root.path(), "prepare-incomplete", &command, &host).unwrap();
    let next = later(&host);
    let mut conflict = command.clone();
    conflict.reason = "different decision".into();
    assert!(execute(root.path(), "prepare-incomplete", &conflict, &host).is_err());
    assert!(execute(root.path(), "reconcile-incomplete", &conflict, &next).is_err());
    let foreign = Host {
        machine: "f".repeat(32),
        boot: next.boot.clone(),
    };
    assert!(execute(root.path(), "reconcile-incomplete", &command, &foreign).is_err());
    fs::write(directory.join("report.tmp"), "new partial output").unwrap();
    assert!(execute(root.path(), "reconcile-incomplete", &command, &next).is_err());
    fs::remove_file(directory.join("report.tmp")).unwrap();
    execute(root.path(), "reconcile-incomplete", &command, &next).unwrap();
    assert!(verify(&directory, &foreign).is_err());
    fs::write(directory.join("storage-heartbeat.tmp"), "changed").unwrap();
    assert!(stopped_on_host(&directory, &next).is_err());
    fs::write(directory.join("storage-heartbeat.tmp"), "").unwrap();
    let moved = root.path().join("moved");
    fs::rename(&directory, &moved).unwrap();
    assert!(snapshot(&moved).is_err());
}

#[test]
fn native_or_aliased_or_oversized_evidence_and_invalid_inputs_are_rejected() {
    let (root, directory, command, host) = fixture();
    for name in [
        "identity.json",
        "launched",
        "quiescent.json",
        "not-started.json",
        "exit.json",
    ] {
        fs::write(directory.join(name), "").unwrap();
        assert!(execute(root.path(), "prepare-incomplete", &command, &host).is_err());
        fs::remove_file(directory.join(name)).unwrap();
    }
    let alias = root.path().join("alias");
    std::os::unix::fs::symlink(&directory, &alias).unwrap();
    let mut changed = command.clone();
    changed.invocation_id = "alias".into();
    assert!(execute(root.path(), "prepare-incomplete", &changed, &host).is_err());
    changed.invocation_id = "../escape".into();
    assert!(execute(root.path(), "prepare-incomplete", &changed, &host).is_err());
    changed = command.clone();
    changed.evidence_sha256 = "bad".into();
    assert!(execute(root.path(), "prepare-incomplete", &changed, &host).is_err());
    changed = command.clone();
    changed.reason.clear();
    assert!(execute(root.path(), "prepare-incomplete", &changed, &host).is_err());
    changed = command.clone();
    changed.request_id.clear();
    assert!(execute(root.path(), "prepare-incomplete", &changed, &host).is_err());
    assert!(execute(root.path(), "invalid-action", &command, &host).is_err());
    let input = directory.join("input.json");
    let bytes = fs::read(&input).unwrap();
    fs::write(&input, "broken JSON").unwrap();
    assert!(snapshot(&directory).is_err());
    fs::write(&input, vec![b'x'; 65537]).unwrap();
    assert!(snapshot(&directory).is_err());
    fs::write(&input, &bytes).unwrap();
    std::os::unix::fs::symlink(&input, directory.join("aliased-native.json")).unwrap();
    assert!(snapshot(&directory).is_err());
    fs::remove_file(directory.join("aliased-native.json")).unwrap();
    fs::create_dir(directory.join("unexpected-directory")).unwrap();
    assert!(snapshot(&directory).is_err());
    fs::remove_dir(directory.join("unexpected-directory")).unwrap();
    let mut request: Request = serde_json::from_slice(&bytes).unwrap();
    assert!(validate_request(Path::new("/"), &request).is_err());
    use std::os::unix::ffi::OsStringExt;
    let invalid_name = std::ffi::OsString::from_vec(vec![0xff]);
    assert!(validate_request(&root.path().join(&invalid_name), &request).is_err());
    fs::write(directory.join(&invalid_name), "").unwrap();
    assert!(native_files(&directory).is_err());
    fs::remove_file(directory.join(&invalid_name)).unwrap();
    request.resource.invocation_id = process::new_identity().unwrap();
    process::durable_write(&input, &request).unwrap();
    assert!(snapshot(&directory).is_err());
    fs::write(&input, bytes).unwrap();
    for index in 0..32 {
        fs::write(directory.join(format!("extra-{index}.json")), "").unwrap();
    }
    assert!(native_files(&directory).is_err());
    assert!(has_entry(&directory.join("x".repeat(300))).is_err());
}

#[test]
fn damaged_origin_or_proof_never_grants_quiescence() {
    let (root, directory, command, host) = fixture();
    execute(root.path(), "prepare-incomplete", &command, &host).unwrap();
    let next = later(&host);
    execute(root.path(), "reconcile-incomplete", &command, &next).unwrap();
    let path = directory.join("host-incomplete-proof.json");
    let original: Proof = read_record(&path).unwrap();
    fs::write(&path, "broken JSON").unwrap();
    assert!(stopped_on_host(&directory, &next).is_err());
    fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(directory.join("input.json"), &path).unwrap();
    assert!(stopped_on_host(&directory, &next).is_err());
    fs::remove_file(&path).unwrap();
    let mut proof = original.clone();
    proof.observed_boot = "invalid".into();
    process::durable_write(&path, &proof).unwrap();
    assert!(stopped_on_host(&directory, &next).is_err());
    proof = original.clone();
    proof.origin.snapshot.inode += 1;
    process::durable_write(&path, &proof).unwrap();
    assert!(stopped_on_host(&directory, &next).is_err());
    process::durable_write(&path, &original).unwrap();
    let origin_path = directory.join("host-incomplete-origin.json");
    let mut origin: Origin = read_record(&origin_path).unwrap();
    origin.snapshot.inode += 1;
    process::durable_write(&origin_path, &origin).unwrap();
    assert!(stopped_on_host(&directory, &next).is_err());
    assert!(verify_boot("invalid", &next.boot).is_err());
    assert!(verify_boot(&host.boot, &host.boot).is_err());
}
