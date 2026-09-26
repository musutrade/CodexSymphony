//! Serialized blocking adapter for legacy validation. Keeps process/file work
//! off the async executor while passing owned jobs through an explicit channel.
use crate::{
    validation::{Candidate, StepEvidence},
    validation_runner::Plan,
};
use std::{
    path::PathBuf,
    sync::{Mutex, OnceLock, mpsc},
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
pub struct Job {
    pub checkout: PathBuf,
    pub directory: PathBuf,
    pub candidate: Candidate,
    pub plan: Plan,
    pub limit: u64,
}
type Message = (Job, tokio::sync::oneshot::Sender<Result<Vec<StepEvidence>>>);
static SENDER: OnceLock<std::result::Result<mpsc::Sender<Message>, String>> = OnceLock::new();
static RECEIVER: OnceLock<Mutex<mpsc::Receiver<Message>>> = OnceLock::new();

fn start() -> std::result::Result<mpsc::Sender<Message>, String> {
    let (sender, receiver) = mpsc::channel();
    if RECEIVER.set(Mutex::new(receiver)).is_err() {
        return Err("legacy validation worker already initialized".into());
    }
    match std::thread::Builder::new()
        .name("legacy-validation".into())
        .spawn(worker)
    {
        Ok(_) => Ok(sender),
        Err(error) => Err(error.to_string()),
    }
}

fn worker() {
    let Some(receiver) = RECEIVER.get() else {
        return;
    };
    let Ok(receiver) = receiver.lock() else {
        return;
    };
    while let Ok((job, reply)) = receiver.recv() {
        if reply.is_closed() {
            continue;
        }
        let result = crate::validation_runner::execute_limited(
            &job.checkout,
            &job.directory,
            &job.candidate,
            &job.plan,
            job.limit,
        );
        let _ = reply.send(result);
    }
}

pub async fn execute(job: Job) -> Result<Vec<StepEvidence>> {
    dispatch(SENDER.get_or_init(start), job).await
}

async fn dispatch(
    sender: &std::result::Result<mpsc::Sender<Message>, String>,
    job: Job,
) -> Result<Vec<StepEvidence>> {
    let sender = match sender {
        Ok(sender) => sender,
        Err(error) => return Err(error.clone().into()),
    };
    let (reply, response) = tokio::sync::oneshot::channel();
    sender.send((job, reply)).map_err(send_error)?;
    response.await?
}
fn send_error(_: mpsc::SendError<Message>) -> &'static str {
    "legacy validation worker unavailable"
}

#[cfg(test)]
#[path = "../tests/unit/validation_legacy.rs"]
mod tests;
