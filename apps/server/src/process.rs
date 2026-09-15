//! Linux subreaper supervisor. A durable receipt is written only after ECHILD.
//! Losing the supervisor without that receipt is UNKNOWN, even if its PID is gone.
use crate::execution::{Launch, ProcessIdentity, Receipt};
use serde::{Serialize, de::DeserializeOwned};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::Duration,
};

pub struct InstanceLock {
    _file: File,
}
impl InstanceLock {
    pub fn acquire(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)?;
        file.try_lock().map_err(io::Error::other)?;
        Ok(Self { _file: file })
    }
}

pub fn new_identity() -> io::Result<String> {
    Ok(fs::read_to_string("/proc/sys/kernel/random/uuid")?
        .trim()
        .to_owned())
}

pub fn run_directory(root: &Path, id: &str) -> io::Result<PathBuf> {
    if id.is_empty() || !id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-') {
        return Err(io::Error::other("invalid Run identity"));
    }
    Ok(root.join(id))
}

pub fn read<T: DeserializeOwned>(path: &Path) -> io::Result<T> {
    serde_json::from_slice(&fs::read(path)?).map_err(io::Error::other)
}

pub fn durable_write(path: &Path, value: &impl Serialize) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("missing parent"))?;
    let temporary = path.with_extension("tmp");
    let mut file = File::create(&temporary)?;
    file.write_all(&serde_json::to_vec(value)?)?;
    file.sync_all()?;
    fs::rename(temporary, path)?;
    File::open(parent)?.sync_all()
}

pub fn spawn(supervisor: &Path, directory: &Path, launch: &Launch) -> io::Result<Child> {
    fs::create_dir(directory)?;
    durable_write(&directory.join("launch.json"), launch)?;
    Command::new(supervisor)
        .arg("--supervise")
        .arg(directory)
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
}

pub fn identity(pid: u32) -> io::Result<ProcessIdentity> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat"))?;
    parse_identity(
        pid,
        &stat,
        fs::read_to_string("/proc/sys/kernel/random/boot_id")?,
    )
}

pub fn parse_identity(pid: u32, stat: &str, boot_id: String) -> io::Result<ProcessIdentity> {
    let (_, tail) = stat
        .rsplit_once(')')
        .ok_or_else(|| io::Error::other("invalid proc stat"))?;
    let fields: Vec<&str> = tail.split_whitespace().collect();
    Ok(ProcessIdentity {
        pid,
        group: field(&fields, 2)?,
        start_ticks: field(&fields, 19)?,
        boot_id: boot_id.trim().into(),
    })
}

fn field<T: std::str::FromStr>(fields: &[&str], index: usize) -> io::Result<T> {
    fields
        .get(index)
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| io::Error::other("invalid proc identity"))
}

fn claim_launch(directory: &Path) -> io::Result<()> {
    // One-use launch: even a second helper invocation after clean exit cannot
    // replay start.json underneath an already issued quiescence receipt.
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join("launched"))?
        .sync_all()?;
    File::open(directory)?.sync_all()?;
    Ok(())
}

fn prepare(directory: &Path) -> io::Result<(InstanceLock, Launch, Receipt)> {
    let _lock = InstanceLock::acquire(&directory.join("supervisor.lock"))?;
    claim_launch(directory)?;
    subreaper()?;
    let launch: Launch = read(&directory.join("launch.json"))?;
    let receipt = Receipt {
        key: launch.key.clone(),
        process: identity(std::process::id())?,
    };
    durable_write(&directory.join("identity.json"), &receipt)?;
    Ok((_lock, launch, receipt))
}

pub fn supervise(directory: &Path) -> io::Result<()> {
    let (_lock, launch, receipt) = prepare(directory)?;
    if await_permission(directory, &launch)? {
        // The supervisor is single threaded; every orphan is adopted here,
        // including descendants that call setsid or change process groups.
        let mut command = Command::new(&launch.program);
        command.args(&launch.args).current_dir(&launch.workspace);
        let _child = command.spawn()?;
        drain(directory)?;
    }
    durable_write(&directory.join("quiescent.json"), &receipt)
}

fn await_permission(directory: &Path, launch: &Launch) -> io::Result<bool> {
    loop {
        if directory.join("stop.json").exists() {
            return Ok(false);
        }
        if directory.join("start.json").exists() {
            let key = read::<crate::execution::RunKey>(&directory.join("start.json"))?;
            return Ok(key == launch.key);
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn drain(directory: &Path) -> io::Result<()> {
    loop {
        if reap()? {
            return Ok(());
        }
        if directory.join("stop.json").exists() {
            stop_children()?;
        }
        thread::sleep(Duration::from_millis(20));
    }
}

unsafe extern "C" {
    fn prctl(option: i32, ...) -> i32;
    fn waitpid(pid: i32, status: *mut i32, options: i32) -> i32;
    fn syscall(number: std::ffi::c_long, ...) -> std::ffi::c_long;
}

fn subreaper() -> io::Result<()> {
    // PR_SET_CHILD_SUBREAPER. No signal handlers or post-fork Rust callbacks.
    let result = unsafe {
        prctl(
            36,
            1 as std::ffi::c_ulong,
            0 as std::ffi::c_ulong,
            0 as std::ffi::c_ulong,
            0 as std::ffi::c_ulong,
        )
    };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn reap() -> io::Result<bool> {
    // WNOHANG; reap ALL children, not only the initially spawned command.
    let result = unsafe { waitpid(-1, std::ptr::null_mut(), 1) };
    if result >= 0 {
        return Ok(false);
    }
    let error = io::Error::last_os_error();
    match error.raw_os_error() {
        Some(10) => Ok(true), // ECHILD is the only affirmative stop proof.
        Some(4) => Ok(false), // EINTR
        _ => Err(error),
    }
}

fn stop_children() -> io::Result<()> {
    let pid = std::process::id();
    let children = fs::read_to_string(format!("/proc/self/task/{pid}/children"))?;
    for child in children.split_whitespace() {
        stop_child(child)?;
    }
    Ok(())
}

fn stop_child(pid: &str) -> io::Result<()> {
    let pid: i32 = pid.parse().map_err(io::Error::other)?;
    // Linux pidfd_open / pidfd_send_signal numbers on supported x86_64/aarch64.
    // Only this thread reaps; a listed child cannot be recycled before pidfd_open.
    // The open handle pins the target; no PID/group signal can hit a reused PID.
    let fd = unsafe { syscall(434, pid, 0) } as i32;
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    let result = unsafe { syscall(424, fd.as_raw_fd(), 9, std::ptr::null::<u8>(), 0) };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
