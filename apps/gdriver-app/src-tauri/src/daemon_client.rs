//! Async client that connects to `gdriver-daemon` over its IPC socket.
//!
//! # Architecture
//! One background reader task and one writer task are spawned per connection.
//! The writer task pulls serialised JSON lines from a channel and sends them
//! to the daemon.  The reader task parses each incoming line and either
//! - resolves the matching pending request via a `oneshot` channel, or
//! - emits the push notification as a Tauri event (`app_handle.emit`).
//!
//! `DaemonClient` itself is cheaply `Clone`able (all fields are `Arc`s or
//! channel senders) so command handlers can extract a clone from the
//! `DaemonState` lock without holding it for the duration of the call.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{oneshot, Mutex};
use tracing::{debug, error, info, warn};

use gdriver_ipc::{JsonRpcError, JsonRpcId, JsonRpcRequest};

// ─── Types ────────────────────────────────────────────────────────────────────

type ResponseSender = oneshot::Sender<Result<Value, JsonRpcError>>;
type PendingMap = Arc<Mutex<HashMap<i64, ResponseSender>>>;

// ─── DaemonClient ─────────────────────────────────────────────────────────────

/// Handle to a live connection with `gdriver-daemon`.
///
/// Cheaply cloneable — all state lives behind `Arc`.
#[derive(Clone)]
pub struct DaemonClient {
    /// Serialised JSON lines destined for the daemon.
    write_tx: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
    /// In-flight requests waiting for a response, keyed by their numeric id.
    pending: PendingMap,
    /// Monotonically increasing request counter.
    next_id: Arc<AtomicI64>,
}

impl DaemonClient {
    /// Connect to a running daemon, or spawn it first if absent, then connect.
    ///
    /// Retries with exponential back-off for up to ~3 seconds.
    pub async fn connect_or_spawn(app_handle: &AppHandle) -> anyhow::Result<Self> {
        let max_attempts = 6u32;
        for attempt in 0..max_attempts {
            match try_connect(app_handle.clone()).await {
                Ok(client) => {
                    info!("connected to daemon (attempt {attempt})");
                    return Ok(client);
                }
                Err(e) if attempt == 0 => {
                    info!("daemon not reachable ({e}), spawning…");
                    if let Err(launch_err) = spawn_daemon() {
                        warn!("spawn failed: {launch_err}");
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                }
                Err(e) => {
                    let delay_ms = 200u64 * (1 << attempt.min(4));
                    debug!("connect attempt {attempt} failed ({e}), retrying in {delay_ms} ms");
                    tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                }
            }
        }
        anyhow::bail!("could not connect to gdriver-daemon after {max_attempts} attempts")
    }

    /// Send a JSON-RPC request and await the daemon's response.
    ///
    /// Returns `Err` if the call fails at the transport or JSON-RPC level.
    pub async fn call(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> anyhow::Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel::<Result<Value, JsonRpcError>>();

        self.pending.lock().await.insert(id, tx);

        let req = JsonRpcRequest::new(method, params, JsonRpcId::Num(id));
        let mut bytes = serde_json::to_vec(&req)
            .map_err(|e| anyhow::anyhow!("serialise error: {e}"))?;
        bytes.push(b'\n');

        self.write_tx
            .send(bytes)
            .map_err(|_| anyhow::anyhow!("writer task is dead"))?;

        match rx.await {
            Ok(Ok(val)) => Ok(val),
            Ok(Err(e)) => anyhow::bail!("RPC error {}: {}", e.code, e.message),
            Err(_) => anyhow::bail!("request dropped (daemon disconnected)"),
        }
    }
}

// ─── DaemonState (Tauri managed state) ───────────────────────────────────────

/// Tauri-managed wrapper holding the (possibly not-yet-initialised) client.
#[derive(Clone)]
pub struct DaemonState(pub Arc<tokio::sync::RwLock<Option<DaemonClient>>>);

impl DaemonState {
    pub fn new() -> Self {
        Self(Arc::new(tokio::sync::RwLock::new(None)))
    }

    /// Clone the inner client, if connected.  Returns `None` while the
    /// background setup task is still connecting.
    pub async fn client(&self) -> Option<DaemonClient> {
        self.0.read().await.clone()
    }

    /// Wait up to `timeout` for the daemon connection to become available.
    pub async fn wait_for_client(
        &self,
        timeout: std::time::Duration,
    ) -> anyhow::Result<DaemonClient> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if let Some(c) = self.client().await {
                return Ok(c);
            }
            if tokio::time::Instant::now() >= deadline {
                anyhow::bail!("timed out waiting for daemon connection");
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }
}

// ─── Connection helpers ───────────────────────────────────────────────────────

/// Try to connect to the daemon socket once, returning a ready `DaemonClient`.
async fn try_connect(app_handle: AppHandle) -> anyhow::Result<DaemonClient> {
    #[cfg(unix)]
    {
        use tokio::net::UnixStream;
        let stream = UnixStream::connect(socket_path()).await?;
        let (reader, writer) = tokio::io::split(stream);
        Ok(DaemonClient::from_io(reader, writer, app_handle))
    }

    #[cfg(windows)]
    {
        use tokio::net::windows::named_pipe::ClientOptions;
        let pipe = ClientOptions::new().open(r"\\.\pipe\gdriver")?;
        let (reader, writer) = tokio::io::split(pipe);
        Ok(DaemonClient::from_io(reader, writer, app_handle))
    }
}

impl DaemonClient {
    /// Construct a `DaemonClient` from split async I/O halves and start the
    /// background reader / writer tasks.
    fn from_io<R, W>(reader: R, writer: W, app_handle: AppHandle) -> Self
    where
        R: tokio::io::AsyncRead + Unpin + Send + 'static,
        W: tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        let (write_tx, write_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let next_id = Arc::new(AtomicI64::new(1));

        tokio::spawn(writer_task(writer, write_rx));
        tokio::spawn(reader_task(reader, Arc::clone(&pending), app_handle));

        Self { write_tx, pending, next_id }
    }
}

// ─── Background tasks ─────────────────────────────────────────────────────────

/// Drains the write channel and forwards each byte slice to the daemon.
async fn writer_task<W>(
    mut writer: W,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
) where
    W: tokio::io::AsyncWrite + Unpin,
{
    while let Some(bytes) = rx.recv().await {
        if writer.write_all(&bytes).await.is_err() {
            break;
        }
    }
    debug!("IPC writer task exited");
}

/// Reads newline-delimited JSON from the daemon and dispatches each message:
/// - JSON-RPC response (has numeric `id`, no `method`) → resolves pending call
/// - JSON-RPC notification (has `method`, no `id`) → emits Tauri event
async fn reader_task<R>(
    reader: R,
    pending: PendingMap,
    app_handle: AppHandle,
) where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut reader = BufReader::new(reader);
    let mut line = String::new();

    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => {
                warn!("daemon closed the connection");
                break;
            }
            Ok(_) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                handle_message(trimmed, &pending, &app_handle).await;
            }
            Err(e) => {
                error!("IPC read error: {e}");
                break;
            }
        }
    }

    // Fail all in-flight requests so callers don't hang.
    let mut map = pending.lock().await;
    for (_, tx) in map.drain() {
        let _ = tx.send(Err(JsonRpcError::internal_error("daemon disconnected")));
    }

    debug!("IPC reader task exited");
}

/// Dispatch one JSON line to either the pending-response map or Tauri events.
async fn handle_message(line: &str, pending: &PendingMap, app_handle: &AppHandle) {
    let Ok(val) = serde_json::from_str::<Value>(line) else {
        warn!("IPC parse error: {line:.80}");
        return;
    };

    if val.get("method").is_some() {
        // Push notification from daemon → Tauri event
        let method = val["method"].as_str().unwrap_or("daemon:unknown");
        let params = val.get("params").cloned().unwrap_or(Value::Null);
        debug!("← push event: {method}");
        // Emit to all windows — use window-level emit for reliability.
        for (label, window) in app_handle.webview_windows() {
            if let Err(e) = window.emit(method, params.clone()) {
                warn!("emit failed for {method} on window {label}: {e}");
            }
        }
    } else {
        // Response to a pending request
        let Some(id) = val.get("id").and_then(Value::as_i64) else {
            warn!("IPC message with no usable id: {line:.80}");
            return;
        };

        let result = if let Some(result) = val.get("result") {
            Ok(result.clone())
        } else if let Some(err_val) = val.get("error") {
            match serde_json::from_value::<JsonRpcError>(err_val.clone()) {
                Ok(e) => Err(e),
                Err(_) => Err(JsonRpcError::internal_error("malformed error object")),
            }
        } else {
            Ok(Value::Null)
        };

        if let Some(tx) = pending.lock().await.remove(&id) {
            let _ = tx.send(result);
        } else {
            warn!("response for unknown request id {id}");
        }
    }
}

// ─── Daemon process management ────────────────────────────────────────────────

/// Spawn `gdriver-daemon` as an independent process.
///
/// In development the daemon binary is a sibling of the app binary in
/// `target/debug/`.  In production it will be bundled as a Tauri sidecar
/// (configured in M17/M18).
fn spawn_daemon() -> anyhow::Result<()> {
    let binary = daemon_binary_path();
    info!("spawning daemon: {}", binary.display());

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            std::process::Command::new(&binary)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .pre_exec(|| {
                    libc::setsid();
                    Ok(())
                })
                .spawn()
                .map_err(|e| anyhow::anyhow!("failed to spawn {:?}: {e}", binary))?;
        }
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new(&binary)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .creation_flags(0x00000200) // CREATE_NEW_PROCESS_GROUP
            .spawn()
            .map_err(|e| anyhow::anyhow!("failed to spawn {:?}: {e}", binary))?;
    }

    Ok(())
}

fn daemon_binary_path() -> std::path::PathBuf {
    // Primary: sibling of the current executable (works in dev and in bundles)
    if let Ok(exe) = std::env::current_exe() {
        let candidate = exe
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .join("gdriver-daemon");
        if candidate.exists() {
            return candidate;
        }
    }
    // Fallback: rely on PATH
    std::path::PathBuf::from("gdriver-daemon")
}

// ─── Platform socket path ─────────────────────────────────────────────────────

/// Must mirror the path used by `gdriver-daemon/src/ipc/server.rs`.
#[cfg(unix)]
fn socket_path() -> std::path::PathBuf {
    dirs::runtime_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("gdriver.sock")
}
