//! The client's end of a stdio connection: newline-delimited JSON-RPC over an in-memory pipe,
//! with a reader task that hands each response to the request with its ID.
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, ReadHalf, WriteHalf};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

/// How long a stdio server may take to stop once the client closed its end of the pipe.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(10);

/// The requests waiting for their answer, by the JSON text of their ID.
type Waiting = Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>;

/// The client's end of a stdio connection.
pub(super) struct Pipe {
    writer: tokio::sync::Mutex<Option<WriteHalf<DuplexStream>>>,
    waiting: Waiting,
    /// Reads the server's lines and hands each answer to its request.
    reader: JoinHandle<()>,
    /// The task of the server, which ends when the client closes the pipe.
    server: JoinHandle<()>,
}

impl Pipe {
    /// The client's end `stream` of the pipe that the task `server` serves.
    pub(super) fn new(stream: DuplexStream, server: JoinHandle<()>) -> Self {
        let (read, write) = tokio::io::split(stream);
        let waiting = Waiting::default();
        let reader = tokio::spawn(read_answers(read, Arc::clone(&waiting)));
        Self {
            writer: tokio::sync::Mutex::new(Some(write)),
            waiting,
            reader,
            server,
        }
    }

    /// Writes `message` and its newline.
    pub(super) async fn write(&self, message: &Value) {
        let mut line = serde_json::to_vec(message).expect("a message serializes");
        line.push(b'\n');
        let mut writer = self.writer.lock().await;
        let writer = writer.as_mut().expect("the pipe is open");
        writer
            .write_all(&line)
            .await
            .expect("the pipe takes the line");
        writer.flush().await.expect("the pipe flushes");
    }

    /// Writes the request `message` and waits for its answer.
    ///
    /// # Panics
    ///
    /// When the server closes the pipe first.
    pub(super) async fn request(&self, message: &Value) -> Value {
        let key = message["id"].to_string();
        let (sender, receiver) = oneshot::channel();
        self.waiting
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.clone(), sender);
        self.write(message).await;
        receiver
            .await
            .unwrap_or_else(|_| panic!("the stdio server closed the pipe before answering {key}"))
    }

    /// Closes the client's end and waits for the server, then the reader, to stop.
    ///
    /// # Panics
    ///
    /// When either fails, or does not stop within [`CLOSE_TIMEOUT`].
    pub(super) async fn close(self) {
        if let Some(mut writer) = self.writer.lock().await.take() {
            writer.shutdown().await.expect("the pipe closes");
        }
        for (task, what) in [
            (self.server, "the stdio server"),
            (self.reader, "the reader"),
        ] {
            match tokio::time::timeout(CLOSE_TIMEOUT, task).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => panic!("{what} failed: {error}"),
                Err(elapsed) => panic!("{what} did not stop within {CLOSE_TIMEOUT:?} ({elapsed})"),
            }
        }
    }
}

/// Reads the server's lines from `read` until the pipe closes, and hands each response to the
/// request in `waiting` with its ID. Notifications and server requests are ignored.
async fn read_answers(read: ReadHalf<DuplexStream>, waiting: Waiting) {
    let mut lines = BufReader::new(read).lines();
    while let Some(line) = lines.next_line().await.expect("the pipe reads") {
        let message: Value = serde_json::from_str(&line).unwrap_or_else(|error| {
            panic!("the server wrote a line that is not JSON ({error}): {line}")
        });
        let is_response = message.get("result").is_some() || message.get("error").is_some();
        let Some(id) = message.get("id").filter(|_| is_response) else {
            continue;
        };
        let sender = waiting
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&id.to_string());
        if let Some(sender) = sender {
            let _ignored = sender.send(message);
        }
    }
}
