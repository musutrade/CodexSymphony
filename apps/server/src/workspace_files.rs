//! No-follow, durable regular-file storage. No cleanup on errors.
use crate::workspace::FileEntry;
use std::{
    ffi::CString,
    fs::{self, File},
    io::{self, Read, Write},
    os::{
        fd::FromRawFd,
        unix::{ffi::OsStrExt, fs::PermissionsExt},
    },
    path::{Component, Path},
    process::{Command, Stdio},
};

pub(crate) type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub(crate) fn require(ok: bool, message: &str) -> Result<()> {
    if !ok {
        return Err(io::Error::other(message).into());
    }
    Ok(())
}

pub(crate) fn component(value: &str) -> Result<()> {
    require(
        !value.is_empty() && value.bytes().all(identifier_byte),
        "invalid identity",
    )
}

fn identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'-'
}

pub(crate) fn safe(path: &Path) -> Result<()> {
    require(path.is_absolute(), "absolute managed path required")?;
    let mut prefix = std::path::PathBuf::new();
    for part in path.components() {
        require(!matches!(part, Component::ParentDir), "parent traversal")?;
        prefix.push(part);
        require(
            !fs::symlink_metadata(&prefix)?.is_symlink(),
            "symlink rejected",
        )?;
    }
    Ok(())
}

pub fn read(path: &Path) -> Result<Vec<u8>> {
    safe(path)?;
    let mut file = open_regular(path, false)?;
    require(file.metadata()?.is_file(), "regular file required")?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

pub fn write(path: &Path, bytes: &[u8], executable: bool) -> Result<()> {
    let parent = path.parent().ok_or("missing parent")?;
    safe(parent)?;
    write_durable(path, bytes, executable)?;
    require(read(path)? == bytes, "file verification failed")?;
    sync_path(parent)
}

fn write_durable(path: &Path, bytes: &[u8], executable: bool) -> Result<()> {
    let mut file = open_regular(path, true)?;
    file.write_all(bytes)?;
    file.set_permissions(fs::Permissions::from_mode(if executable {
        0o700
    } else {
        0o600
    }))?;
    file.sync_all()?;
    Ok(())
}

/// Linux openat2 resolves the entire path with NO_SYMLINKS, closing the race
/// between checking parent directories and opening the final source file.
fn open_regular(path: &Path, create: bool) -> Result<File> {
    #[repr(C)]
    struct OpenHow {
        flags: u64,
        mode: u64,
        resolve: u64,
    }
    unsafe extern "C" {
        fn syscall(number: std::ffi::c_long, ...) -> std::ffi::c_long;
    }
    let name = CString::new(path.as_os_str().as_bytes())?;
    // O_CLOEXEC | O_NONBLOCK; create adds O_WRONLY | O_CREAT | O_EXCL.
    let (flags, mode) = if create {
        (0x808c1, 0o600)
    } else {
        (0x80800, 0)
    };
    let how = OpenHow {
        flags,
        mode,
        resolve: 0x06,
    };
    // SAFETY: NUL-terminated path and correctly sized Linux open_how remain
    // alive for this call. No borrowed descriptor is transferred.
    let descriptor = unsafe {
        syscall(
            437,
            -100_i32,
            name.as_ptr(),
            &how,
            std::mem::size_of::<OpenHow>(),
        )
    };
    if descriptor < 0 {
        return Err(io::Error::last_os_error().into());
    }
    // SAFETY: openat2 returned a new descriptor, owned only by this File.
    Ok(unsafe { File::from_raw_fd(descriptor as i32) })
}

pub(crate) fn digest(bytes: &[u8]) -> Result<String> {
    let output = run_digest(bytes)?;
    require(output.status.success(), "digest failed")?;
    let text = String::from_utf8(output.stdout)?;
    Ok(text.get(..64).ok_or("invalid digest")?.to_owned())
}

fn run_digest(bytes: &[u8]) -> Result<std::process::Output> {
    let mut child = Command::new("/usr/bin/sha256sum")
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .ok_or("missing digest stdin")?
        .write_all(bytes)?;
    child.wait_with_output().map_err(Into::into)
}

pub(crate) fn sync_path(path: &Path) -> Result<()> {
    File::open(path)?.sync_all().map_err(Into::into)
}

pub(crate) fn digest_file(path: &Path) -> Result<String> {
    digest(&read(path)?)
}

/// Only these root caches are rebuildable. Tracked files under them are
/// rejected by the caller rather than silently discarded.
pub const CACHE_ROOTS: &[&str] = &["target", "node_modules", ".angular"];

pub(crate) fn forbidden(name: &str) -> bool {
    name.starts_with(".env.")
        || matches!(
            name,
            ".git"
                | ".env"
                | ".ssh"
                | ".git-credentials"
                | ".gitconfig"
                | ".netrc"
                | ".npmrc"
                | ".pypirc"
                | "id_rsa"
                | "id_ed25519"
        )
}

pub(crate) fn inventory(root: &Path) -> Result<(Vec<FileEntry>, Vec<String>)> {
    safe(root)?;
    let mut files = Vec::new();
    let mut excluded = Vec::new();
    walk(root, root, &mut files, &mut excluded)?;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    excluded.sort();
    Ok((files, excluded))
}

fn walk(
    root: &Path,
    directory: &Path,
    files: &mut Vec<FileEntry>,
    excluded: &mut Vec<String>,
) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let relative = path.strip_prefix(root)?.to_str().ok_or("non-UTF8 path")?;
        if relative == ".git" {
            continue;
        }
        if CACHE_ROOTS.contains(&relative) {
            // Check the cache itself too: never accept a link as a cache.
            safe(&path)?;
            excluded.push(relative.to_owned());
            continue;
        }
        record(root, &path, files, excluded)?;
    }
    Ok(())
}

fn record(
    root: &Path,
    path: &Path,
    files: &mut Vec<FileEntry>,
    excluded: &mut Vec<String>,
) -> Result<()> {
    safe(path)?;
    require(
        !forbidden(
            path.file_name()
                .and_then(|v| v.to_str())
                .ok_or("invalid path")?,
        ),
        "credential or nested Git path rejected",
    )?;
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() {
        return walk(root, path, files, excluded);
    }
    files.push(FileEntry {
        path: path
            .strip_prefix(root)?
            .to_str()
            .ok_or("invalid path")?
            .to_owned(),
        digest: digest(&read(path)?)?,
        executable: metadata.permissions().mode() & 0o111 != 0,
    });
    Ok(())
}

pub(crate) fn copy_files(source: &Path, target: &Path, files: &[FileEntry]) -> Result<()> {
    for entry in files {
        copy_entry(source, target, entry)?;
    }
    Ok(())
}

fn copy_entry(source: &Path, target: &Path, entry: &FileEntry) -> Result<()> {
    let relative = Path::new(&entry.path);
    require(relative.is_relative(), "invalid manifest path")?;
    let bytes = read(&source.join(relative))?;
    require(digest(&bytes)? == entry.digest, "content digest mismatch")?;
    let destination = target.join(relative);
    let parent = destination.parent().ok_or("missing parent")?;
    fs::create_dir_all(parent)?;
    write(&destination, &bytes, entry.executable)?;
    sync_parents(parent, target)
}

fn sync_parents(directory: &Path, root: &Path) -> Result<()> {
    let mut current = directory;
    loop {
        File::open(current)?.sync_all()?;
        if current == root {
            return Ok(());
        }
        current = current.parent().ok_or("missing storage root")?;
    }
}
