//! The stdio transport: newline-delimited JSON-RPC with a child process.

use std::collections::{BTreeMap, HashMap};
use std::process::Stdio;
use std::sync::{Arc, Mutex as SyncMutex};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{Mutex, oneshot};
use tokio::task::JoinHandle;

use super::error::{McpError, RpcError};
use super::jsonrpc::{Incoming, reply_to_server};

const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);

type Waiter = oneshot::Sender<Result<Value, RpcError>>;
type Stdin = Arc<Mutex<Option<ChildStdin>>>;

#[derive(Default)]
struct Pending {
    waiters: HashMap<u64, Waiter>,
    closed: bool,
}

pub(crate) struct StdioTransport {
    stdin: Stdin,
    pending: Arc<SyncMutex<Pending>>,
    child: Mutex<Child>,
    tasks: Vec<JoinHandle<()>>,
}

impl StdioTransport {
    pub(crate) fn spawn(
        command: &str,
        args: &[String],
        env: &BTreeMap<String, String>,
    ) -> Result<Self, McpError> {
        let mut program = Command::new(command);
        crate::process::hide_secrets(&mut program);
        let mut child = program
            .args(args)
            .envs(env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| McpError::Transport(format!("cannot start {command}: {error}")))?;
        let stdin: Stdin = Arc::new(Mutex::new(child.stdin.take()));
        let stdout = child.stdout.take().ok_or(McpError::Closed)?;
        let stderr = child.stderr.take().ok_or(McpError::Closed)?;
        let pending = Arc::new(SyncMutex::new(Pending::default()));

        let reader = tokio::spawn(read_stdout(stdout, stdin.clone(), pending.clone()));
        let logger = tokio::spawn({
            let command = command.to_string();
            async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::debug!(%command, %line, "mcp server stderr");
                }
            }
        });
        Ok(Self {
            stdin,
            pending,
            child: Mutex::new(child),
            tasks: vec![reader, logger],
        })
    }

    pub(crate) async fn request(
        &self,
        id: u64,
        message: &Value,
        timeout: Duration,
    ) -> Result<Value, McpError> {
        let receiver = {
            let mut pending = self.pending.lock().unwrap();
            if pending.closed {
                return Err(McpError::Closed);
            }
            let (sender, receiver) = oneshot::channel();
            pending.waiters.insert(id, sender);
            receiver
        };
        if let Err(error) = write_line(&self.stdin, message).await {
            self.pending.lock().unwrap().waiters.remove(&id);
            return Err(error);
        }
        match tokio::time::timeout(timeout, receiver).await {
            Ok(Ok(outcome)) => outcome.map_err(McpError::Rpc),
            Ok(Err(_)) => Err(McpError::Closed),
            Err(_) => {
                self.pending.lock().unwrap().waiters.remove(&id);
                let cancel = json!({
                    "jsonrpc": "2.0",
                    "method": "notifications/cancelled",
                    "params": {"requestId": id, "reason": "timed out"},
                });
                write_line(&self.stdin, &cancel).await.ok();
                Err(McpError::Timeout)
            }
        }
    }

    pub(crate) async fn notify(&self, message: &Value) -> Result<(), McpError> {
        write_line(&self.stdin, message).await
    }

    /// Closes the server's stdin, then kills it if it does not exit soon.
    pub(crate) async fn close(&self) {
        self.stdin.lock().await.take();
        let mut child = self.child.lock().await;
        if tokio::time::timeout(SHUTDOWN_GRACE, child.wait())
            .await
            .is_err()
        {
            child.kill().await.ok();
        }
    }
}

impl Drop for StdioTransport {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

async fn write_line(stdin: &Stdin, message: &Value) -> Result<(), McpError> {
    let mut line = serde_json::to_vec(message).map_err(McpError::transport)?;
    line.push(b'\n');
    let mut stdin = stdin.lock().await;
    let stdin = stdin.as_mut().ok_or(McpError::Closed)?;
    stdin.write_all(&line).await.map_err(|_| McpError::Closed)?;
    stdin.flush().await.map_err(|_| McpError::Closed)
}

async fn read_stdout(
    stdout: tokio::process::ChildStdout,
    stdin: Stdin,
    pending: Arc<SyncMutex<Pending>>,
) {
    let mut lines = BufReader::new(stdout).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        match Incoming::parse(&line) {
            Ok(Incoming::Response { id, outcome }) => {
                let waiter = id
                    .as_u64()
                    .and_then(|id| pending.lock().unwrap().waiters.remove(&id));
                match waiter {
                    Some(waiter) => {
                        waiter.send(outcome).ok();
                    }
                    None => tracing::debug!(%id, "response to no pending request"),
                }
            }
            Ok(Incoming::Request { id, method }) => {
                tracing::debug!(%method, "request from the server");
                write_line(&stdin, &reply_to_server(&id, &method))
                    .await
                    .ok();
            }
            Ok(Incoming::Notification { method }) => {
                tracing::debug!(%method, "notification from the server");
            }
            Err(error) => tracing::debug!(%error, %line, "unreadable line from the server"),
        }
    }
    let mut pending = pending.lock().unwrap();
    pending.closed = true;
    pending.waiters.clear();
}
