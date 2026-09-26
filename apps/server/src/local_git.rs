//! Reviewed, local bare targets. Atomic receipt refs distinguish a lost reply
//! from an operation that never happened, even after the target advances.
use crate::{delivery_extension::Result, validation::sha256};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    pub reference: String,
    pub repository_id: i64,
    pub repository_version: i64,
    pub path: PathBuf,
    pub branch: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    pub target: Target,
    pub device: u64,
    pub inode: u64,
    pub config_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Observation {
    NotSubmitted,
    Delivered,
    Conflict,
}

pub fn installed() -> Result<Vec<Target>> {
    let Some(path) = std::env::var_os("LOCAL_GIT_TARGETS") else {
        return Ok(Vec::new());
    };
    let path = Path::new(&path);
    let metadata = fs::symlink_metadata(path)?;
    if !path.is_absolute() || !metadata.is_file() || metadata.mode() & 0o022 != 0 {
        return Err("local target registry must be an absolute protected regular file".into());
    }
    let targets: Vec<Target> = serde_json::from_slice(&fs::read(path)?)?;
    unique_targets(&targets)?;
    Ok(targets)
}

fn unique_targets(targets: &[Target]) -> Result<()> {
    let mut seen = std::collections::BTreeSet::new();
    for target in targets {
        if !seen.insert(target.reference.clone()) {
            return Err("duplicate local target reference".into());
        }
    }
    Ok(())
}

pub fn resolve(
    targets: &[Target],
    reference: &str,
    repository: i64,
    version: i64,
    branch: &str,
) -> Result<Binding> {
    for target in targets {
        if target.reference == reference {
            if target.repository_id != repository
                || target.repository_version != version
                || target.branch != branch
            {
                return Err("local target differs from reviewed repository".into());
            }
            return bind(target);
        }
    }
    Err("local_git delivery target not installed".into())
}

pub fn bind(target: &Target) -> Result<Binding> {
    let metadata = target_directory(target)?;
    git(
        &target.path,
        &["check-ref-format", &format!("refs/heads/{}", target.branch)],
        b"",
    )?;
    if git(&target.path, &["rev-parse", "--is-bare-repository"], b"")? != "true" {
        return Err(
            "local delivery requires a bare repository; never update a user checkout".into(),
        );
    }
    Ok(Binding {
        target: target.clone(),
        device: metadata.dev(),
        inode: metadata.ino(),
        config_sha256: sha256(fs::read(target.path.join("config"))?),
    })
}

fn target_directory(target: &Target) -> Result<fs::Metadata> {
    valid_target_identity(target)?;
    if !target.path.is_absolute() || fs::canonicalize(&target.path)? != target.path {
        return Err("local target must be a canonical absolute directory".into());
    }
    let metadata = fs::symlink_metadata(&target.path)?;
    if !metadata.is_dir() || metadata.mode() & 0o022 != 0 {
        return Err("local target must be a protected directory".into());
    }
    Ok(metadata)
}

fn valid_target_identity(target: &Target) -> Result<()> {
    if target.repository_id <= 0 || target.repository_version <= 0 || target.reference.is_empty() {
        return Err("invalid local target identity".into());
    }
    Ok(())
}

pub fn check(binding: &Binding) -> Result<()> {
    if bind(&binding.target)? != *binding {
        return Err("local target identity changed".into());
    }
    Ok(())
}

pub fn head(binding: &Binding) -> Result<String> {
    check(binding)?;
    let value = git(
        &binding.target.path,
        &[
            "rev-parse",
            "--verify",
            &format!("refs/heads/{}^{{commit}}", binding.target.branch),
        ],
        b"",
    )?;
    oid(&value)?;
    let reference = format!("refs/heads/{}", binding.target.branch);
    let actual = git(
        &binding.target.path,
        &[
            "for-each-ref",
            "--format=%(refname) %(objectname) %(objecttype) %(symref)",
            &reference,
        ],
        b"",
    )?;
    if actual != format!("{reference} {value} commit") {
        return Err("local target branch must be a direct commit reference".into());
    }
    Ok(value)
}

fn receipt(action: &str) -> String {
    format!("refs/symphony-deliveries/{}", sha256(action))
}

pub fn observe(
    binding: &Binding,
    action: &str,
    expected: &str,
    candidate: &str,
) -> Result<Observation> {
    oid(expected)?;
    oid(candidate)?;
    check(binding)?;
    let refs = git(
        &binding.target.path,
        &[
            "for-each-ref",
            "--format=%(refname) %(objectname) %(symref)",
            &receipt(action),
        ],
        b"",
    )?;
    if refs == format!("{} {candidate}", receipt(action)) {
        return Ok(Observation::Delivered);
    }
    if refs.is_empty() && head(binding)? == expected {
        return Ok(Observation::NotSubmitted);
    }
    Ok(Observation::Conflict)
}

pub fn submit(
    binding: &Binding,
    source: &Path,
    action: &str,
    expected: &str,
    candidate: &str,
) -> Result<Observation> {
    if observe(binding, action, expected, candidate)? != Observation::NotSubmitted {
        return observe(binding, action, expected, candidate);
    }
    let source = fs::canonicalize(source)?;
    let source = source.to_str().ok_or("local source path is not UTF-8")?;
    git(
        &binding.target.path,
        &[
            "-c",
            "protocol.file.allow=always",
            "fetch",
            "--no-tags",
            "--no-write-fetch-head",
            "--",
            source,
            candidate,
        ],
        b"",
    )?;
    git(
        &binding.target.path,
        &["merge-base", "--is-ancestor", expected, candidate],
        b"",
    )?;
    check(binding)?;
    let update = format!(
        "start\nupdate refs/heads/{} {candidate} {expected}\ncreate {} {candidate}\nprepare\ncommit\n",
        binding.target.branch,
        receipt(action)
    );
    // A failed/timeout command has an unknown outcome. The caller retains the
    // intent and observes this same receipt; it never blindly repeats the write.
    git(
        &binding.target.path,
        &["update-ref", "--no-deref", "--stdin"],
        update.as_bytes(),
    )?;
    observe(binding, action, expected, candidate)
}

fn oid(value: &str) -> Result<()> {
    if value.len() != 40 {
        return Err("full commit identity required".into());
    }
    for byte in value.bytes() {
        if !byte.is_ascii_hexdigit() {
            return Err("invalid commit identity".into());
        }
    }
    Ok(())
}

fn git(path: &Path, args: &[&str], input: &[u8]) -> Result<String> {
    let mut child = Command::new("/usr/bin/timeout")
        .args(["--signal=KILL", "10s", "/usr/bin/git"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LANG", "C.UTF-8")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "protocol.allow=never",
            "-c",
            "gc.auto=0",
            "-c",
            "core.fsync=all",
            "-c",
            "maintenance.auto=false",
        ])
        .arg("--git-dir")
        .arg(path)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    child
        .stdin
        .take()
        .ok_or("local Git stdin unavailable")?
        .write_all(input)?;
    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Err("local Git operation failed or timed out; reconcile retained intent".into());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}
