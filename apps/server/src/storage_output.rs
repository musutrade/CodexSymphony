//! Bound platform-owned raw streams at ingress, before bytes reach disk.
use std::{
    fs::File,
    io::{self, Read, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
};

pub struct Capture {
    stopped: Arc<AtomicBool>,
    readers: Vec<JoinHandle<io::Result<()>>>,
}
impl Default for Capture {
    fn default() -> Self {
        Self {
            stopped: Arc::new(AtomicBool::new(false)),
            readers: Vec::new(),
        }
    }
}
impl Capture {
    pub fn stream(&mut self, input: impl Read + Send + 'static, file: File, limit: u64) {
        self.reader(input, Arc::new(Mutex::new((file, limit))));
    }
    pub fn combined(
        &mut self,
        stdout: impl Read + Send + 'static,
        stderr: impl Read + Send + 'static,
        file: File,
        limit: u64,
    ) {
        let output = Arc::new(Mutex::new((file, limit)));
        self.reader(stdout, output.clone());
        self.reader(stderr, output);
    }
    fn reader(&mut self, mut input: impl Read + Send + 'static, output: Arc<Mutex<(File, u64)>>) {
        let stopped = self.stopped.clone();
        self.readers.push(std::thread::spawn(move || {
            let result = copy(&mut input, &output, &stopped);
            if result.is_err() {
                stopped.store(true, Ordering::SeqCst);
            }
            result
        }));
    }
    pub fn stopped(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }
    /// The owner stops/reaps the process group before joining its stream readers.
    pub fn finish(self) -> io::Result<bool> {
        for reader in self.readers {
            reader
                .join()
                .map_err(|_| io::Error::other("output reader failed"))??;
        }
        Ok(self.stopped.load(Ordering::SeqCst))
    }
}
fn copy(
    input: &mut impl Read,
    output: &Mutex<(File, u64)>,
    stopped: &AtomicBool,
) -> io::Result<()> {
    let mut buffer = [0u8; 8192];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        let mut output = match output.lock() {
            Ok(output) => output,
            Err(_) => return Err(io::Error::other("output writer failed")),
        };
        let take = (count as u64).min(output.1) as usize;
        output.0.write_all(&buffer[..take])?;
        output.1 -= take as u64;
        if take < count {
            stopped.store(true, Ordering::SeqCst);
        }
    }
    match output.lock() {
        Ok(output) => output.0.sync_all(),
        Err(_) => Err(io::Error::other("output writer failed")),
    }
}
