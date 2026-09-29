//! Host-only boot-boundary reconciliation. Never synthesizes a supervisor receipt.
use crate::{
    environment_host::{Registry, Result},
    environment_probe::Request,
    execution::Receipt,
    process,
    validation::sha256,
};
use serde::{Deserialize, Serialize};
use std::{
    io::Read,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Command {
    request_id: String,
    invocation_id: String,
    evidence_sha256: String,
    reason: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    directory: PathBuf,
    device: u64,
    inode: u64,
    evidence_sha256: String,
    receipt: Receipt,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Origin {
    command: Command,
    binding: Binding,
    machine: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Proof {
    origin: Origin,
    observed_boot: String,
}
struct Host {
    machine: String,
    boot: String,
}
impl Host {
    fn read() -> Result<Self> {
        let host = Self {
            machine: std::fs::read_to_string("/etc/machine-id")?.trim().into(),
            boot: std::fs::read_to_string("/proc/sys/kernel/random/boot_id")?
                .trim()
                .into(),
        };
        if host.machine.len() != 32 || !valid_boot(&host.boot) {
            return Err("kernel host identity unavailable".into());
        }
        Ok(host)
    }
}

fn valid_boot(value: &str) -> bool {
    if value.len() != 36 {
        return false;
    }
    for (index, byte) in value.bytes().enumerate() {
        if matches!(index, 8 | 13 | 18 | 23) {
            if byte != b'-' {
                return false;
            }
        } else if !byte.is_ascii_hexdigit() {
            return false;
        }
    }
    true
}

pub fn run(args: &[String]) -> Result<()> {
    let action = action(args)?;
    let command = input()?;
    let registry = Registry::load()?;
    let result = execute(&registry.evidence_root, action, &command, &Host::read()?)?;
    serde_json::to_writer(std::io::stdout().lock(), &result)?;
    std::io::Write::write_all(&mut std::io::stdout(), b"\n")?;
    Ok(())
}

fn action(args: &[String]) -> Result<&str> {
    let [action, flag] = args else {
        return Err("usage: environment prepare-recovery|reconcile --stdin-json".into());
    };
    if flag != "--stdin-json" || !matches!(action.as_str(), "prepare-recovery" | "reconcile") {
        return Err("invalid environment recovery operation".into());
    }
    Ok(action)
}

fn input() -> Result<Command> {
    let mut input = Vec::new();
    std::io::stdin().take(8193).read_to_end(&mut input)?;
    if input.len() > 8192 {
        return Err("environment recovery input exceeds limit".into());
    }
    Ok(serde_json::from_slice(&input)?)
}

fn execute(root: &Path, action: &str, command: &Command, host: &Host) -> Result<serde_json::Value> {
    let directory = directory(root, &command.invocation_id)?;
    let _lock = process::InstanceLock::acquire(&directory.join("host-recovery.lock"))?;
    let origin = origin(&directory, command, host)?;
    if action == "prepare-recovery" {
        prepare(&directory, &origin, host)?;
    } else {
        reconcile(&directory, &origin, host)?;
    }
    Ok(
        serde_json::json!({"accepted":true,"started":false,"quiescence_proven":action=="reconcile","prior_result":"unknown","model_calls_added":0}),
    )
}

fn directory(root: &Path, id: &str) -> Result<PathBuf> {
    let directory = process::run_directory(root, id)?;
    if directory.canonicalize()? != directory {
        return Err("environment recovery directory must be canonical".into());
    }
    Ok(directory)
}

fn origin(directory: &Path, command: &Command, host: &Host) -> Result<Origin> {
    crate::contract::validate_request_id(&command.request_id)?;
    if !crate::runtime::text_valid(&command.reason, 4096) {
        return Err("environment recovery reason required".into());
    }
    let binding = binding(directory)?;
    if binding.evidence_sha256 != command.evidence_sha256 {
        return Err("original environment evidence changed".into());
    }
    Ok(Origin {
        command: command.clone(),
        binding,
        machine: host.machine.clone(),
    })
}

fn binding(directory: &Path) -> Result<Binding> {
    if directory.canonicalize()? != directory {
        return Err("environment recovery directory must be canonical".into());
    }
    let metadata = std::fs::symlink_metadata(directory)?;
    let request: Request = read_regular(&directory.join("input.json"))?;
    let receipt: Receipt = read_regular(&directory.join("identity.json"))?;
    check_identity(directory, &request, &receipt)?;
    let digest = sha256(serde_json::to_vec(&(request, &receipt))?);
    Ok(Binding {
        directory: directory.into(),
        device: metadata.dev(),
        inode: metadata.ino(),
        evidence_sha256: digest,
        receipt,
    })
}

fn check_identity(directory: &Path, request: &Request, receipt: &Receipt) -> Result<()> {
    let expected = crate::execution::RunKey {
        run_id: request.invocation_id.clone(),
        request_id: request.invocation_id.clone(),
        incarnation: request.invocation_id.clone(),
    };
    if receipt.key != expected
        || directory.file_name().and_then(std::ffi::OsStr::to_str) != Some(&request.invocation_id)
    {
        return Err("environment invocation identity differs".into());
    }
    if !valid_boot(&receipt.process.boot_id) {
        return Err("original environment launch identity missing".into());
    }
    launch_marker(directory)
}

fn launch_marker(directory: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(directory.join("launched"))?;
    if !metadata.is_file() || metadata.len() != 0 {
        return Err("invalid original environment launch marker".into());
    }
    Ok(())
}

fn read_regular<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.len() > 65536 {
        return Err("invalid recovery evidence file".into());
    }
    Ok(process::read(path)?)
}

fn prepare(directory: &Path, origin: &Origin, host: &Host) -> Result<()> {
    if origin.binding.receipt.process.boot_id != host.boot {
        return Err("origin must be witnessed on the original running host boot".into());
    }
    if directory.join("quiescent.json").exists() {
        return Err("native stop receipt already exists".into());
    }
    let path = directory.join("host-recovery-origin.json");
    if path.exists() {
        let previous: Origin = read_regular(&path)?;
        if previous != *origin {
            return Err("environment recovery request conflict".into());
        }
        return Ok(());
    }
    process::durable_write(&path, origin)?;
    Ok(())
}

fn reconcile(directory: &Path, origin: &Origin, host: &Host) -> Result<()> {
    let previous: Origin = read_regular(&directory.join("host-recovery-origin.json"))?;
    if previous != *origin {
        return Err("environment recovery origin differs".into());
    }
    if origin.binding.receipt.process.boot_id == host.boot {
        return Err("same-boot PID absence cannot prove descendant quiescence".into());
    }
    let proof = Proof {
        origin: origin.clone(),
        observed_boot: host.boot.clone(),
    };
    let path = directory.join("host-recovery-proof.json");
    if path.exists() {
        if read_regular::<Proof>(&path)? != proof {
            return Err("environment recovery proof conflict".into());
        }
        return Ok(());
    }
    process::durable_write(&path, &proof)?;
    Ok(())
}

pub fn stopped(directory: &Path) -> Result<bool> {
    let path = directory.join("host-recovery-proof.json");
    if !path.try_exists()? {
        return Ok(false);
    }
    verify(directory, &Host::read()?)?;
    Ok(true)
}

fn verify(directory: &Path, host: &Host) -> Result<()> {
    let proof: Proof = read_regular(&directory.join("host-recovery-proof.json"))?;
    let origin: Origin = read_regular(&directory.join("host-recovery-origin.json"))?;
    if proof.origin != origin
        || origin.binding != binding(directory)?
        || origin.machine != host.machine
    {
        return Err("host recovery proof no longer matches original evidence".into());
    }
    verify_boot(
        &origin.binding.receipt.process.boot_id,
        &proof.observed_boot,
        &host.boot,
    )
}

fn verify_boot(old: &str, witnessed: &str, current: &str) -> Result<()> {
    if old == current || old == witnessed || !valid_boot(witnessed) {
        return Err("host recovery proof lacks a distinct kernel boot".into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "../tests/unit/environment_recovery.rs"]
mod tests;
