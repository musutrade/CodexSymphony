//! Scoped cache directories. Contents never stand in for validation evidence.
use crate::{
    environment::Cache,
    environment_host::{Profile, Registry, Result},
};
use std::{fs, os::unix::fs::PermissionsExt, path::Path};

pub fn check(registry: &Registry, profile: &Profile, cache: Option<&Cache>) -> Result<()> {
    let Some(cache) = cache else {
        return Ok(());
    };
    let directory = directory(profile, cache)?;
    verify_directory(&directory, cache)?;
    if let Some(seed) = &cache.seed {
        verify_seed(&registry.evidence_root.join("seeds").join(seed), seed)?;
    }
    Ok(())
}

fn directory(profile: &Profile, cache: &Cache) -> Result<std::path::PathBuf> {
    let base = profile.resource_root.join("cache");
    if !base.exists() {
        fs::create_dir(&base)?;
    }
    if fs::symlink_metadata(&base)?.file_type().is_symlink() {
        return Err("cache base cannot be a symlink".into());
    }
    let directory = base.join(&cache.identity);
    if !directory.exists() {
        fs::create_dir_all(&directory)?;
        fs::set_permissions(
            &directory,
            fs::Permissions::from_mode(if cache.writable { 0o700 } else { 0o500 }),
        )?;
    }
    Ok(directory)
}

fn verify_directory(directory: &Path, cache: &Cache) -> Result<()> {
    let metadata = fs::symlink_metadata(directory)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("cache scope is not an owned directory".into());
    }
    let actual_write = metadata.permissions().mode() & 0o222 != 0;
    if actual_write != cache.writable {
        return Err("cache write permission drift".into());
    }
    let actual = bytes(directory, !cache.writable)?;
    if actual > cache.capacity_bytes {
        return Err(format!(
            "cache capacity exceeded: expected <= {}, actual {actual}",
            cache.capacity_bytes
        )
        .into());
    }
    Ok(())
}

fn verify_seed(path: &Path, seed: &str) -> Result<()> {
    // Only an operator installs a seed. Agent cache preparation never writes it.
    bytes(path, true)?;
    let manifest = fs::read(path.join("manifest.json"))?;
    if crate::validation::sha256(&manifest) != seed {
        return Err("cache seed identity drift".into());
    }
    let expected: std::collections::BTreeMap<String, String> = serde_json::from_slice(&manifest)?;
    let mut actual = std::collections::BTreeMap::new();
    seed_files(path, path, &mut actual)?;
    if actual != expected {
        return Err("cache seed contents differ from manifest".into());
    }
    Ok(())
}

fn seed_files(
    root: &Path,
    path: &Path,
    files: &mut std::collections::BTreeMap<String, String>,
) -> Result<()> {
    for entry in fs::read_dir(path)? {
        let path = entry?.path();
        if path.is_dir() {
            seed_files(root, &path, files)?;
        } else if path != root.join("manifest.json") {
            let (name, digest) = seed_file(root, &path)?;
            files.insert(name, digest);
        }
    }
    Ok(())
}

fn seed_file(root: &Path, path: &Path) -> Result<(String, String)> {
    let name = path
        .strip_prefix(root)?
        .to_str()
        .ok_or("non-UTF8 seed path")?
        .into();
    Ok((name, crate::validation::sha256(fs::read(path)?)))
}

fn bytes(path: &Path, readonly: bool) -> Result<u64> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Err("cache/seed symlinks are not permitted".into());
    }
    if readonly && metadata.permissions().mode() & 0o222 != 0 {
        return Err("cache seed/read-only cache permits writes".into());
    }
    if metadata.is_file() {
        return Ok(metadata.len());
    }
    if !metadata.is_dir() {
        return Err("cache contains a special file".into());
    }
    directory_bytes(path, readonly)
}

fn directory_bytes(path: &Path, readonly: bool) -> Result<u64> {
    let mut total = 0_u64;
    for entry in fs::read_dir(path)? {
        total = total
            .checked_add(bytes(&entry?.path(), readonly)?)
            .ok_or("cache size overflow")?;
    }
    Ok(total)
}
