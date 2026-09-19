//! Descriptor-bound product material access. No links or nested mounts are
//! traversed, and a path replacement cannot redirect deletion into a new Run.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    ffi::CString,
    fs::{self, File},
    io::{self, Read},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileIdentity {
    pub device: u64,
    pub inode: u64,
}
impl FileIdentity {
    fn of(metadata: &fs::Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub path: PathBuf,
    pub identity: FileIdentity,
    pub bytes: u64,
    pub logical_bytes: u64,
    pub sha256: String,
    pub directory: bool,
    pub link: Option<PathBuf>,
}

pub struct Directory(File);
impl Directory {
    pub fn child(&self, path: &Path) -> io::Result<Self> {
        Ok(Self(open(self.0.as_raw_fd(), path, true, true, false)?))
    }
    pub fn create_directory(&self, path: &Path) -> io::Result<Self> {
        if path.components().count() != 1
            || !matches!(
                path.components().next(),
                Some(std::path::Component::Normal(_))
            )
        {
            return Err(io::Error::other("one archive directory component required"));
        }
        unsafe extern "C" {
            fn mkdirat(fd: i32, name: *const std::ffi::c_char, mode: u32) -> i32;
        }
        let name = CString::new(path.as_os_str().as_bytes())?;
        // SAFETY: live parent fd, NUL-terminated basename and fixed mode. The fd
        // pins the archive mount even if its visible mountpoint disappears.
        if unsafe { mkdirat(self.0.as_raw_fd(), name.as_ptr(), 0o700) } != 0 {
            return Err(io::Error::last_os_error());
        }
        self.0.sync_all()?;
        self.child(path)
    }
    pub fn open(path: &Path) -> io::Result<Self> {
        if !path.is_absolute() {
            return Err(io::Error::other("absolute material directory required"));
        }
        Ok(Self(open(-100, path, true, false, false)?))
    }
    pub fn identity(&self) -> io::Result<FileIdentity> {
        Ok(FileIdentity::of(&self.0.metadata()?))
    }
    pub fn matches(&self, identity: &FileIdentity) -> io::Result<()> {
        if self.identity()? != *identity {
            return Err(io::Error::other("material directory identity changed"));
        }
        Ok(())
    }
    fn names(&self, limit: u64) -> io::Result<Vec<PathBuf>> {
        let mut names = fs::read_dir(format!("/proc/self/fd/{}", self.0.as_raw_fd()))?
            .take(limit.saturating_add(1).min(usize::MAX as u64) as usize)
            .map(|entry| entry.map(|entry| PathBuf::from(entry.file_name())))
            .collect::<io::Result<Vec<_>>>()?;
        names.sort();
        Ok(names)
    }
    pub fn inventory(&self, limit: u64) -> io::Result<Vec<Entry>> {
        let mut entries = Vec::new();
        self.visit(Path::new(""), limit, true, &mut entries)?;
        Ok(entries)
    }
    pub fn listing(&self, limit: u64) -> io::Result<Vec<Entry>> {
        let mut entries = Vec::new();
        self.visit(Path::new(""), limit, false, &mut entries)?;
        Ok(entries)
    }
    pub fn usage(&self, limit: u64) -> io::Result<u64> {
        let entries = self.listing(limit)?;
        entries.iter().try_fold(
            self.0.metadata()?.blocks().saturating_mul(512),
            |total, entry| {
                total
                    .checked_add(entry.bytes)
                    .ok_or_else(|| io::Error::other("storage size overflow"))
            },
        )
    }
    fn visit(
        &self,
        prefix: &Path,
        limit: u64,
        hashes: bool,
        entries: &mut Vec<Entry>,
    ) -> io::Result<()> {
        for name in self.names(limit)? {
            if entries.len() as u64 >= limit || prefix.components().count() >= 64 {
                return Err(io::Error::other(
                    "material inventory limit; preserve and reconcile",
                ));
            }
            let (entry, child) = self.inspect_entry(&name, prefix, hashes)?;
            let path = entry.path.clone();
            entries.push(entry);
            if let Some(child) = child {
                child.visit(&path, limit, hashes, entries)?;
            }
        }
        Ok(())
    }
    fn inspect_entry(
        &self,
        name: &Path,
        prefix: &Path,
        hashes: bool,
    ) -> io::Result<(Entry, Option<Directory>)> {
        let direct = PathBuf::from(format!("/proc/self/fd/{}", self.0.as_raw_fd())).join(name);
        let metadata = fs::symlink_metadata(&direct)?;
        if metadata.is_symlink() {
            return Ok((link_entry(&direct, prefix.join(name), &metadata)?, None));
        }
        let mut file = open(self.0.as_raw_fd(), name, false, true, false)?;
        let metadata = file.metadata()?;
        let sha256 = entry_digest(&mut file, &metadata, hashes)?;
        let directory = metadata.is_dir();
        let entry = Entry {
            path: prefix.join(name),
            identity: FileIdentity::of(&metadata),
            bytes: metadata.blocks().saturating_mul(512),
            logical_bytes: metadata.len(),
            sha256,
            directory,
            link: None,
        };
        Ok((entry, directory.then_some(Directory(file))))
    }
    pub fn read(&self, path: &Path) -> io::Result<File> {
        let file = open(self.0.as_raw_fd(), path, false, true, false)?;
        if !file.metadata()?.is_file() {
            return Err(io::Error::other("regular material required"));
        }
        Ok(file)
    }
    pub fn create(&self, path: &Path) -> io::Result<File> {
        open(self.0.as_raw_fd(), path, false, true, true)
    }
    /// Missing entries are valid only after a persisted deletion intent. New or
    /// changed entries always block. The caller holds the consumer transaction.
    pub fn remove(&self, manifest: &[Entry]) -> io::Result<()> {
        let current = self.verify_remaining(manifest)?;
        for entry in current.iter().rev() {
            self.remove_entry(entry, manifest)?;
        }
        self.0.sync_all()
    }
    fn verify_remaining(&self, manifest: &[Entry]) -> io::Result<Vec<Entry>> {
        let current = self.inventory(manifest.len() as u64 + 1)?;
        for entry in &current {
            if !manifest.iter().any(|original| same_entry(original, entry)) {
                return Err(io::Error::other("material changed during deletion"));
            }
        }
        Ok(current)
    }
    fn entry_parent(&self, entry: &Entry, manifest: &[Entry]) -> io::Result<File> {
        let parent = entry.path.parent().unwrap_or(Path::new(""));
        let directory = if parent.as_os_str().is_empty() {
            self.0.try_clone()?
        } else {
            open(self.0.as_raw_fd(), parent, true, true, false)?
        };
        if !parent.as_os_str().is_empty() {
            let expected = manifest
                .iter()
                .find(|entry| entry.path == parent)
                .ok_or_else(|| io::Error::other("parent identity missing"))?;
            Directory(directory.try_clone()?).matches(&expected.identity)?;
        }
        Ok(directory)
    }
    fn remove_entry(&self, entry: &Entry, manifest: &[Entry]) -> io::Result<()> {
        let directory = self.entry_parent(entry, manifest)?;
        let direct = PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd()))
            .join(entry.path.file_name().unwrap());
        if FileIdentity::of(&fs::symlink_metadata(direct)?) != entry.identity {
            return Err(io::Error::other("material entry identity changed"));
        }
        unlink(
            &directory,
            Path::new(entry.path.file_name().unwrap()),
            entry.directory,
        )?;
        Ok(())
    }
}

fn link_entry(direct: &Path, path: PathBuf, metadata: &fs::Metadata) -> io::Result<Entry> {
    let target = fs::read_link(direct)?;
    Ok(Entry {
        path,
        identity: FileIdentity::of(metadata),
        bytes: metadata.blocks().saturating_mul(512),
        logical_bytes: metadata.len(),
        sha256: format!("{:x}", Sha256::digest(target.as_os_str().as_bytes())),
        directory: false,
        link: Some(target),
    })
}
fn entry_digest(file: &mut File, metadata: &fs::Metadata, hashes: bool) -> io::Result<String> {
    if !metadata.is_dir() && !metadata.is_file() {
        return Err(io::Error::other("non-regular material protected"));
    }
    if metadata.is_dir() || !hashes {
        return Ok(String::new());
    }
    digest(file)
}

pub fn digest(reader: &mut impl Read) -> io::Result<String> {
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn same_entry(original: &Entry, current: &Entry) -> bool {
    original.path == current.path
        && original.identity == current.identity
        && original.directory == current.directory
        && original.sha256 == current.sha256
}

fn open(
    parent: i32,
    path: &Path,
    directory: bool,
    beneath: bool,
    create: bool,
) -> io::Result<File> {
    #[repr(C)]
    struct How {
        flags: u64,
        mode: u64,
        resolve: u64,
    }
    unsafe extern "C" {
        fn syscall(number: std::ffi::c_long, ...) -> std::ffi::c_long;
    }
    let path = CString::new(path.as_os_str().as_bytes())?;
    let how = How {
        flags: 0x80800 | if directory { 0x10000 } else { 0 } | if create { 0xc1 } else { 0 },
        mode: if create { 0o600 } else { 0 },
        resolve: if beneath { 0x0f } else { 0x06 },
    };
    // SAFETY: the name and Linux open_how are live and correctly sized. The new
    // descriptor is transferred exactly once to File.
    let descriptor =
        unsafe { syscall(437, parent, path.as_ptr(), &how, std::mem::size_of::<How>()) };
    if descriptor < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { File::from_raw_fd(descriptor as i32) })
}

fn unlink(parent: &File, path: &Path, directory: bool) -> io::Result<()> {
    unsafe extern "C" {
        fn unlinkat(fd: i32, name: *const std::ffi::c_char, flags: i32) -> i32;
    }
    let name = CString::new(path.as_os_str().as_bytes())?;
    // SAFETY: live directory fd, NUL terminated basename; no owned fd transfers.
    let result = unsafe {
        unlinkat(
            parent.as_raw_fd(),
            name.as_ptr(),
            if directory { 0x200 } else { 0 },
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    parent.sync_all()
}
