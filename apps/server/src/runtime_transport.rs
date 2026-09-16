//! Bounded newline JSON transport; stderr is never decoded as RPC.
use crate::runtime::MAX_FRAME;
use serde_json::{Value, json};
use std::{collections::VecDeque, io, process::Child};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{ChildStderr, ChildStdin, ChildStdout},
    sync::mpsc,
};

pub enum Record {
    Protocol(Vec<u8>),
    Diagnostic(Vec<u8>),
    Closed,
}
pub struct Transport {
    writer: ChildStdin,
    receiver: mpsc::Receiver<io::Result<Record>>,
    pending: VecDeque<Value>,
    readers: Vec<tokio::task::JoinHandle<()>>,
    next: u64,
}
impl Transport {
    pub fn new(child: &mut Child) -> io::Result<Self> {
        let writer = ChildStdin::from_std(
            child
                .stdin
                .take()
                .ok_or_else(|| io::Error::other("missing stdin"))?,
        )?;
        let stdout = ChildStdout::from_std(
            child
                .stdout
                .take()
                .ok_or_else(|| io::Error::other("missing stdout"))?,
        )?;
        let stderr = ChildStderr::from_std(
            child
                .stderr
                .take()
                .ok_or_else(|| io::Error::other("missing stderr"))?,
        )?;
        let (sender, receiver) = mpsc::channel(32);
        let readers = Vec::from([
            tokio::spawn(protocol(stdout, sender.clone())),
            tokio::spawn(diagnostics(stderr, sender)),
        ]);
        Ok(Self {
            writer,
            receiver,
            pending: VecDeque::new(),
            readers,
            next: 0,
        })
    }
    pub async fn send(&mut self, value: &impl serde::Serialize) -> io::Result<()> {
        let mut bytes = serde_json::to_vec(value)?;
        if bytes.len() > MAX_FRAME {
            return Err(io::Error::other("outgoing frame limit"));
        }
        bytes.push(b'\n');
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            self.writer.write_all(&bytes),
        )
        .await??;
        Ok(())
    }
    pub async fn request(
        &mut self,
        method: &str,
        params: &impl serde::Serialize,
    ) -> io::Result<Value> {
        // Optional schema fields may have non-null defaults in app-server.
        // Omit absent top-level parameters; nested user payloads stay byte-for-
        // byte equivalent JSON, including intentional null values.
        let mut params = serde_json::to_value(params)?;
        omit_top_level_nulls(&mut params);
        self.next += 1;
        let id = json!(format!("platform-{}", self.next));
        self.send(&json!({"id":id,"method":method,"params":params}))
            .await?;
        Ok(id)
    }
    pub async fn receive(&mut self) -> io::Result<Record> {
        self.receiver.recv().await.unwrap_or(Ok(Record::Closed))
    }
    pub fn defer(&mut self, value: Value) -> io::Result<()> {
        if self.pending.len() >= 64 {
            return Err(io::Error::other("pending protocol limit"));
        }
        self.pending.push_back(value);
        Ok(())
    }
    pub fn pending(&mut self) -> Option<Value> {
        self.pending.pop_front()
    }
}

fn omit_top_level_nulls(params: &mut Value) {
    if let Some(fields) = params.as_object_mut() {
        let mut absent = Vec::new();
        for (key, value) in fields.iter() {
            if value.is_null() {
                absent.push(key.clone());
            }
        }
        for key in absent {
            fields.remove(&key);
        }
    }
}
impl Drop for Transport {
    fn drop(&mut self) {
        for reader in &self.readers {
            reader.abort();
        }
    }
}

pub async fn frame(reader: &mut (impl AsyncBufRead + Unpin)) -> io::Result<Option<Vec<u8>>> {
    let mut frame = Vec::new();
    loop {
        let data = reader.fill_buf().await?;
        if data.is_empty() {
            return if frame.is_empty() {
                Ok(None)
            } else {
                Err(io::Error::other("incomplete protocol frame"))
            };
        }
        let end = data.iter().position(|byte| *byte == b'\n');
        let count = end.map_or(data.len(), |at| at + 1);
        if frame.len() + count > MAX_FRAME {
            return Err(io::Error::other("protocol frame limit"));
        }
        frame.extend_from_slice(&data[..count]);
        reader.consume(count);
        if end.is_some() {
            return Ok(Some(frame));
        }
    }
}
pub async fn protocol(
    stdout: impl tokio::io::AsyncRead + Unpin,
    sender: mpsc::Sender<io::Result<Record>>,
) {
    let mut reader = BufReader::new(stdout);
    loop {
        let value = match frame(&mut reader).await {
            Ok(Some(bytes)) => Ok(Record::Protocol(bytes)),
            Ok(None) => {
                let _ = sender.send(Ok(Record::Closed)).await;
                return;
            }
            Err(error) => {
                let _ = sender.send(Err(error)).await;
                return;
            }
        };
        if sender.send(value).await.is_err() {
            return;
        }
    }
}
pub async fn diagnostics(
    mut stderr: impl tokio::io::AsyncRead + Unpin,
    sender: mpsc::Sender<io::Result<Record>>,
) {
    let mut buffer = [0; 4096];
    loop {
        match stderr.read(&mut buffer).await {
            Ok(0) => return,
            Ok(count) => {
                if sender
                    .send(Ok(Record::Diagnostic(buffer[..count].to_vec())))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            Err(error) => {
                let _ = sender.send(Err(error)).await;
                return;
            }
        }
    }
}
