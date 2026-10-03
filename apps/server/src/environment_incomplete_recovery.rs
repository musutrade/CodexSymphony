//! Host-only recovery of an incomplete local probe across a witnessed kernel boot.
//! Original files and UNKNOWN outcome are preserved; no native receipt is created.
use crate::{
    environment_host::Result,
    environment_probe::Request,
    environment_recovery::{Command, Host, valid_boot},
    process,
    validation::sha256,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::Read,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    directory: PathBuf,
    device: u64,
    inode: u64,
    input_sha256: String,
    files: BTreeMap<String, String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Origin {
    command: Command,
    snapshot: Snapshot,
    machine: String,
    observed_boot: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Proof {
    origin: Origin,
    observed_boot: String,
}

pub(crate) fn execute(
    root: &Path,
    action: &str,
    command: &Command,
    host: &Host,
) -> Result<serde_json::Value> {
    let directory = directory(root, command)?;
    let _lock = process::InstanceLock::acquire(&directory.join("host-recovery.lock"))?;
    let snapshot = snapshot(&directory)?;
    apply(&directory, action, command, snapshot, host)?;
    Ok(
        serde_json::json!({"accepted":true,"started":false,"quiescence_proven":action=="reconcile-incomplete","prior_result":"unknown","model_calls_added":0,"native_receipt_created":false}),
    )
}

fn directory(root: &Path, command: &Command) -> Result<PathBuf> {
    crate::contract::validate_request_id(&command.request_id)?;
    if !crate::runtime::text_valid(&command.reason, 4096) {
        return Err("incomplete recovery reason required".into());
    }
    let directory = process::run_directory(root, &command.invocation_id)?;
    canonical_directory(&directory)?;
    Ok(directory)
}

fn apply(
    directory: &Path,
    action: &str,
    command: &Command,
    snapshot: Snapshot,
    host: &Host,
) -> Result<()> {
    if snapshot.input_sha256 != command.evidence_sha256 {
        return Err("original incomplete input changed".into());
    }
    if action == "prepare-incomplete" {
        prepare(directory, command, snapshot, host)?;
    } else if action == "reconcile-incomplete" {
        reconcile(directory, command, snapshot, host)?;
    } else {
        return Err("invalid incomplete recovery operation".into());
    }
    Ok(())
}

fn snapshot(directory: &Path) -> Result<Snapshot> {
    canonical_directory(directory)?;
    let input = original_input(directory)?;
    let files = native_files(directory)?;
    let digest = sha256(&input);
    if files.get("input.json") != Some(&digest) {
        return Err("incomplete input changed while reading".into());
    }
    let metadata = std::fs::symlink_metadata(directory)?;
    Ok(Snapshot {
        directory: directory.into(),
        device: metadata.dev(),
        inode: metadata.ino(),
        input_sha256: digest,
        files,
    })
}

fn canonical_directory(directory: &Path) -> Result<()> {
    if directory.canonicalize()? != directory {
        return Err("incomplete recovery directory changed".into());
    }
    Ok(())
}

fn original_input(directory: &Path) -> Result<Vec<u8>> {
    for name in [
        "identity.json",
        "launched",
        "quiescent.json",
        "not-started.json",
        "exit.json",
    ] {
        if has_entry(&directory.join(name))? {
            return Err("native launch or stop evidence requires native recovery".into());
        }
    }
    let input = read_limited(&directory.join("input.json"))?;
    let request: Request = serde_json::from_slice(&input)?;
    validate_request(directory, &request)?;
    Ok(input)
}

fn validate_request(directory: &Path, request: &Request) -> Result<()> {
    let id = directory
        .file_name()
        .ok_or("incomplete directory name missing")?
        .to_str()
        .ok_or("invalid incomplete directory name")?;
    if !valid_boot(id) || request.invocation_id != id || request.resource.invocation_id != id {
        return Err("incomplete invocation identity differs".into());
    }
    Ok(())
}

fn native_files(directory: &Path) -> Result<BTreeMap<String, String>> {
    let mut files = BTreeMap::new();
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_str().ok_or("invalid incomplete file name")?;
        if recovery_control(name) {
            continue;
        }
        if files.len() >= 32 {
            return Err("incomplete evidence file count exceeds bound".into());
        }
        files.insert(name.into(), sha256(read_limited(&entry.path())?));
    }
    Ok(files)
}

fn recovery_control(name: &str) -> bool {
    matches!(
        name,
        "host-recovery.lock"
            | "host-incomplete-origin.json"
            | "host-incomplete-origin.tmp"
            | "host-incomplete-proof.json"
            | "host-incomplete-proof.tmp"
    )
}

fn has_entry(path: &Path) -> Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn read_limited(path: &Path) -> Result<Vec<u8>> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.len() > 65536 {
        return Err("invalid incomplete recovery evidence file".into());
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(65537)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 65536 {
        return Err("incomplete recovery evidence grew while reading".into());
    }
    Ok(bytes)
}

fn read_record<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    Ok(serde_json::from_slice(&read_limited(path)?)?)
}

fn prepare(directory: &Path, command: &Command, snapshot: Snapshot, host: &Host) -> Result<()> {
    let origin = Origin {
        command: command.clone(),
        snapshot,
        machine: host.machine.clone(),
        observed_boot: host.boot.clone(),
    };
    write_once(&directory.join("host-incomplete-origin.json"), &origin)
}

fn reconcile(directory: &Path, command: &Command, snapshot: Snapshot, host: &Host) -> Result<()> {
    let origin: Origin = read_record(&directory.join("host-incomplete-origin.json"))?;
    if origin.command != *command || origin.snapshot != snapshot || origin.machine != host.machine {
        return Err("incomplete recovery origin differs".into());
    }
    verify_boot(&origin.observed_boot, &host.boot)?;
    write_once(
        &directory.join("host-incomplete-proof.json"),
        &Proof {
            origin,
            observed_boot: host.boot.clone(),
        },
    )
}

fn write_once<T: Serialize + serde::de::DeserializeOwned + PartialEq>(
    path: &Path,
    value: &T,
) -> Result<()> {
    if has_entry(path)? {
        let original: T = read_record(path)?;
        if original != *value {
            return Err("incomplete recovery decision conflicts".into());
        }
        return Ok(());
    }
    process::durable_write(path, value)?;
    Ok(())
}

pub(crate) fn stopped(directory: &Path) -> Result<bool> {
    let path = directory.join("host-incomplete-proof.json");
    if !has_entry(&path)? {
        return Ok(false);
    }
    stopped_on_host(directory, &Host::read()?)
}

fn stopped_on_host(directory: &Path, host: &Host) -> Result<bool> {
    let path = directory.join("host-incomplete-proof.json");
    if !has_entry(&path)? {
        return Ok(false);
    }
    verify(directory, host)?;
    Ok(true)
}

fn verify(directory: &Path, host: &Host) -> Result<()> {
    let proof: Proof = read_record(&directory.join("host-incomplete-proof.json"))?;
    verify_origin(directory, &proof.origin, host)?;
    verify_boot(&proof.origin.observed_boot, &proof.observed_boot)?;
    if proof.origin.observed_boot == host.boot || !valid_boot(&host.boot) {
        return Err("incomplete recovery proof lacks a distinct current kernel boot".into());
    }
    Ok(())
}

fn verify_origin(directory: &Path, expected: &Origin, host: &Host) -> Result<()> {
    let origin: Origin = read_record(&directory.join("host-incomplete-origin.json"))?;
    if *expected != origin
        || origin.snapshot != snapshot(directory)?
        || origin.machine != host.machine
    {
        return Err("incomplete recovery proof no longer matches original evidence".into());
    }
    Ok(())
}

fn verify_boot(origin: &str, later: &str) -> Result<()> {
    if !valid_boot(origin) || !valid_boot(later) || origin == later {
        return Err("incomplete recovery requires a distinct actual kernel boot".into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "../tests/unit/environment_incomplete_recovery.rs"]
mod tests;
