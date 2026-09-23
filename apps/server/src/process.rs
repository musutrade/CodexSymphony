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
    spawn_with_transport(supervisor, directory, launch, None)
}

/// The single-threaded supervisor passes these pipes directly to app-server;
/// its own durable stop proof remains independent of stdout/RPC completion.
pub fn spawn_with_transport(
    supervisor: &Path,
    directory: &Path,
    launch: &Launch,
    config: Option<&str>,
) -> io::Result<Child> {
    if directory.join("hook.json").exists() {
        fs::create_dir_all(directory)?;
    } else {
        fs::create_dir(directory)?;
    }
    durable_write(&directory.join("launch.json"), launch)?;
    durable_write(&directory.join("storage-heartbeat.json"), &launch.key)?;
    if let Some(config) = config {
        fs::create_dir(directory.join("codex-home"))?;
        fs::write(directory.join("codex-home/config.toml"), config)?;
        durable_write(&directory.join("runtime.json"), &launch.key)?;
    }
    let stdio = || {
        if config.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        }
    };
    Command::new(supervisor)
        .arg("--supervise")
        .arg(directory)
        .process_group(0)
        .stdin(stdio())
        .stdout(stdio())
        .stderr(stdio())
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
        if directory.join("hook.json").exists() {
            supervise_hook(directory, &launch)?;
        } else {
            supervise_runtime(directory, &launch)?;
        }
    }
    durable_write(&directory.join("quiescent.json"), &receipt)
}

fn supervise_runtime(directory: &Path, launch: &Launch) -> io::Result<()> {
    // The subreaper adopts even descendants that change process groups.
    let mut command = Command::new(&launch.program);
    command.args(&launch.args).current_dir(&launch.workspace);
    configure_runtime(&mut command, directory, launch)?;
    let _child = command.spawn()?;
    drain(directory)
}

fn supervise_hook(directory: &Path, launch: &Launch) -> io::Result<()> {
    let mut command = Command::new(&launch.program);
    command.args(&launch.args).current_dir(&launch.workspace);
    hook_environment(&mut command, directory);
    command.stdin(Stdio::from(File::open(directory.join("input.json"))?));
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let output = hook_output(directory)?;
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            durable_write(&directory.join("spawn-error.json"), &error.to_string())?;
            return Ok(());
        }
    };
    let capture = capture_hook(&mut child, output)?;
    drain_hook(directory, child.id(), &capture)?;
    if capture.finish()? {
        durable_write(&directory.join("truncated.json"), &true)?;
    }
    Ok(())
}

fn hook_output(directory: &Path) -> io::Result<(u64, File, File)> {
    Ok((
        read(&directory.join("hook-limit.json"))?,
        File::create(directory.join("stdout.json"))?,
        File::create(directory.join("stderr.log"))?,
    ))
}

fn capture_hook(
    child: &mut Child,
    (limit, stdout, stderr): (u64, File, File),
) -> io::Result<crate::storage_output::Capture> {
    let mut capture = crate::storage_output::Capture::default();
    capture.stream(
        child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("hook stdout unavailable"))?,
        stdout,
        limit,
    );
    capture.stream(
        child
            .stderr
            .take()
            .ok_or_else(|| io::Error::other("hook stderr unavailable"))?,
        stderr,
        limit,
    );
    Ok(capture)
}

fn hook_environment(command: &mut Command, directory: &Path) {
    command.env_clear();
    for name in [
        "PATH",
        "LANG",
        "LC_ALL",
        "TZ",
        "NO_COLOR",
        "TEST_DATABASE_URL",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "NO_PROXY",
        "SSL_CERT_FILE",
        "SSL_CERT_DIR",
    ] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    command.env("HOME", directory);
}

fn drain_hook(
    directory: &Path,
    primary: u32,
    capture: &crate::storage_output::Capture,
) -> io::Result<()> {
    let mut primary_status = None;
    loop {
        let mut status = 0;
        let pid = unsafe { waitpid(-1, &mut status, 1) };
        if pid == primary as i32 {
            primary_status = Some(status);
        }
        if pid < 0 {
            let error = io::Error::last_os_error();
            match error.raw_os_error() {
                Some(10) => break, // ECHILD proves every descendant stopped.
                Some(4) => {}      // EINTR
                _ => return Err(error),
            }
        }
        if directory.join("stop.json").exists() || !storage_alive(directory) || capture.stopped() {
            stop_children()?;
        }
        thread::sleep(Duration::from_millis(20));
    }
    durable_write(&directory.join("exit.json"), &primary_status)
}

/// Only deployment tool/network settings cross into Runtime. Unknown names,
/// including custom tracker credentials and loader hooks, are excluded.
pub(crate) fn development_environment(command: &mut Command) {
    const ALLOWED: &[&str] = &[
        "PATH",
        "HOME",
        "LANG",
        "LC_ALL",
        "TZ",
        "NO_COLOR",
        "CARGO_HOME",
        "RUSTUP_HOME",
        "CARGO_TARGET_DIR",
        "TEST_DATABASE_URL",
        "DEV_DATABASE_URL",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "NO_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "no_proxy",
        "SSL_CERT_FILE",
        "SSL_CERT_DIR",
    ];
    command.env_clear();
    for name in ALLOWED {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
}

fn runtime_environment(command: &mut Command, directory: &Path) -> io::Result<()> {
    development_environment(command);
    let home = directory.join("codex-home").canonicalize()?;
    let temporary = home.join("tmp");
    fs::create_dir_all(&temporary)?;
    command.env("CODEX_HOME", home).env("TMPDIR", temporary);
    Ok(())
}

fn await_permission(directory: &Path, launch: &Launch) -> io::Result<bool> {
    loop {
        if !storage_alive(directory) {
            return Ok(false);
        }
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
        if directory.join("stop.json").exists() || !storage_alive(directory) {
            stop_children()?;
        }
        thread::sleep(Duration::from_millis(20));
    }
}

/// Fail closed even when ENOSPC prevents writing stop.json, or the controller
/// cannot reach PostgreSQL. Killing descendants needs no filesystem write.
/// Match the bounded startup handshake window, including durable fsync latency.
fn storage_alive(directory: &Path) -> bool {
    let age = fs::metadata(directory.join("storage-heartbeat.json"))
        .and_then(|m| m.modified())
        .and_then(|at| at.elapsed().map_err(io::Error::other));
    age.is_ok_and(|age| age <= Duration::from_secs(15))
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

fn configure_runtime(command: &mut Command, directory: &Path, launch: &Launch) -> io::Result<()> {
    if directory.join("runtime.json").exists() {
        let key: crate::execution::RunKey = read(&directory.join("runtime.json"))?;
        if key != launch.key {
            return Err(io::Error::other("Runtime identity mismatch"));
        }
        runtime_environment(command, directory)?;
    }
    Ok(())
}
