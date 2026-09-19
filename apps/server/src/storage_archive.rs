//! A complete, verified package precedes any persisted deletion permission.
use crate::storage_files::{Directory, Entry, FileIdentity, digest};
use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Package {
    pub path: PathBuf,
    pub identity: FileIdentity,
    pub files: Vec<Entry>,
}

pub fn write(
    source: &Directory,
    target: &Directory,
    files: &[Entry],
    entry_limit: u64,
) -> io::Result<()> {
    for (index, entry) in files
        .iter()
        .enumerate()
        .filter(|(_, entry)| !entry.directory && entry.link.is_none())
    {
        check_entry(source, entry, entry_limit)?;
        copy_entry(source, target, entry, index)?;
    }
    if source.inventory(files.len() as u64 + 1)? != files {
        return Err(io::Error::other("source changed during compression"));
    }
    Ok(())
}

fn check_entry(source: &Directory, entry: &Entry, entry_limit: u64) -> io::Result<()> {
    let mut original = source.read(&entry.path)?;
    if original.metadata()?.len() > entry_limit || digest(&mut original)? != entry.sha256 {
        return Err(io::Error::other(
            "source size/identity changed; preserve original",
        ));
    }
    Ok(())
}
fn copy_entry(
    source: &Directory,
    target: &Directory,
    entry: &Entry,
    index: usize,
) -> io::Result<()> {
    let name = format!("{index}.gz");
    match target.read(Path::new(&name)) {
        Ok(file) => verify_file(file, &entry.sha256)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            compress(source.read(&entry.path)?, target.create(Path::new(&name))?)?;
            verify_file(target.read(Path::new(&name))?, &entry.sha256)?;
        }
        Err(error) => return Err(error),
    }
    Ok(())
}

fn compress(input: std::fs::File, output: std::fs::File) -> io::Result<()> {
    let saved = output.try_clone()?;
    let status = Command::new("/usr/bin/gzip")
        .args(["-n", "-c"])
        .env_clear()
        .stdin(input)
        .stdout(output)
        .stderr(Stdio::null())
        .status()?;
    if !status.success() {
        return Err(io::Error::other("compression failed"));
    }
    saved.sync_all()
}

fn verify_file(input: std::fs::File, expected: &str) -> io::Result<()> {
    let mut child = Command::new("/usr/bin/gzip")
        .args(["-d", "-c"])
        .env_clear()
        .stdin(input)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let actual = digest(
        &mut child
            .stdout
            .take()
            .ok_or(io::Error::other("archive pipe unavailable"))?,
    );
    let status = child.wait()?;
    if !status.success() || actual? != expected {
        return Err(io::Error::other("archive file checksum mismatch"));
    }
    Ok(())
}

pub fn write_record(target: &Directory, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    let file = target.create(Path::new("record.gz"))?;
    let saved = file.try_clone()?;
    let mut child = Command::new("/usr/bin/gzip")
        .args(["-n", "-c"])
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(file)
        .stderr(Stdio::null())
        .spawn()?;
    child
        .stdin
        .take()
        .ok_or(io::Error::other("archive stdin unavailable"))?
        .write_all(bytes)?;
    let status = child.wait()?;
    if !status.success() {
        return Err(io::Error::other("record compression failed"));
    }
    saved.sync_all()
}
pub fn verify_record(target: &Directory, bytes: &[u8]) -> io::Result<()> {
    let expected = digest(&mut std::io::Cursor::new(bytes))?;
    verify_file(target.read(Path::new("record.gz"))?, &expected)
}

pub fn verify(package: &Package) -> io::Result<()> {
    let directory = Directory::open(&package.path)?;
    directory.matches(&package.identity)?;
    let expected: Vec<PathBuf> = package
        .files
        .iter()
        .enumerate()
        .filter(|(_, entry)| !entry.directory && entry.link.is_none())
        .map(|(index, _)| PathBuf::from(format!("{index}.gz")))
        .collect();
    let actual = directory.inventory(expected.len() as u64 + 1)?;
    if !same_inventory(&actual, &expected) {
        return Err(io::Error::other("archive file inventory mismatch"));
    }
    for (index, entry) in package
        .files
        .iter()
        .enumerate()
        .filter(|(_, entry)| !entry.directory && entry.link.is_none())
    {
        verify_file(
            directory.read(Path::new(&format!("{index}.gz")))?,
            &entry.sha256,
        )?;
    }
    Ok(())
}

fn same_inventory(actual: &[Entry], expected: &[PathBuf]) -> bool {
    if actual.len() != expected.len() {
        return false;
    }
    for entry in actual {
        if !expected.contains(&entry.path) {
            return false;
        }
    }
    true
}
