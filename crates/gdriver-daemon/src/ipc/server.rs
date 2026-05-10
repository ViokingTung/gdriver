use std::sync::Arc;

use gdriver_ipc::{JsonRpcError, JsonRpcRequest, JsonRpcResponse};
use serde::Serialize;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::broadcast,
};
use tracing::{debug, error, info, warn};

use super::handlers::Router;

// ─── Public API ───────────────────────────────────────────────────────────────

/// Sender half of the daemon-to-client push-notification channel.
///
/// Clone this and pass it to any subsystem that needs to emit events to all
/// connected IPC clients (e.g. the sync engine, OAuth handler).
///
/// The channel carries newline-free serialised JSON strings; the server
/// appends the required `\n` before writing to each client socket.
pub type PushSender = broadcast::Sender<String>;

/// IPC server that accepts JSON-RPC connections from the Tauri app and file
/// manager extensions.
pub struct IpcServer {
    push_tx: PushSender,
}

impl IpcServer {
    /// Create a new server.  Call [`push_sender`] before [`run`] if you need
    /// to emit push events from other parts of the daemon.
    pub fn new() -> Self {
        // Capacity 256: each slot holds one serialised push notification.
        // Slow subscribers that lag beyond this limit skip the oldest messages.
        let (push_tx, _) = broadcast::channel(256);
        Self { push_tx }
    }

    /// Return a cloned sender that can be used to push JSON-RPC notifications
    /// to all currently connected clients.
    pub fn push_sender(&self) -> PushSender {
        self.push_tx.clone()
    }

    /// Start the accept loop.  This future runs until cancelled (e.g. via
    /// `tokio::signal::ctrl_c()`).
    pub async fn run(self, router: Arc<Router>) -> anyhow::Result<()> {
        run_platform(self, router).await
    }
}

// ─── Platform-specific accept loops ──────────────────────────────────────────

#[cfg(unix)]
async fn run_platform(server: IpcServer, router: Arc<Router>) -> anyhow::Result<()> {
    use tokio::net::UnixListener;

    let path = unix_socket_path();

    // Remove a stale socket left by a previous crash before binding.
    if path.exists() {
        std::fs::remove_file(&path)?;
    }

    let listener = UnixListener::bind(&path)?;
    info!("IPC server listening on {}", path.display());

    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                let router = Arc::clone(&router);
                let push_rx = server.push_tx.subscribe();
                tokio::spawn(handle_connection(stream, router, push_rx));
            }
            Err(e) => error!("IPC accept error: {e}"),
        }
    }
}

#[cfg(windows)]
async fn run_platform(server: IpcServer, router: Arc<Router>) -> anyhow::Result<()> {
    use tokio::net::windows::named_pipe::{PipeMode, ServerOptions};

    const PIPE_NAME: &str = r"\\.\pipe\gdriver";

    // The first server instance must be created before any client can connect.
    let mut pipe = ServerOptions::new()
        .first_pipe_instance(true)
        .pipe_mode(PipeMode::Byte)
        .create(PIPE_NAME)?;

    info!("IPC server listening on {PIPE_NAME}");

    loop {
        // Block until a client connects.
        pipe.connect().await?;

        // Create the next server instance *before* spawning the handler so
        // that another client can connect while we serve the current one.
        let next = ServerOptions::new()
            .pipe_mode(PipeMode::Byte)
            .create(PIPE_NAME)?;
        let current = std::mem::replace(&mut pipe, next);

        let router = Arc::clone(&router);
        let push_rx = server.push_tx.subscribe();
        tokio::spawn(handle_connection(current, router, push_rx));
    }
}

// ─── Generic connection handler (Unix + Windows share this) ──────────────────

/// Handle one client connection for its entire lifetime.
///
/// The stream is split into independent read and write halves so that push
/// events can be forwarded to the client at any time, even while waiting for
/// the next inbound request.
///
/// # Message framing
/// Messages are newline-delimited JSON (NDJSON): each JSON object occupies
/// exactly one line.  Both requests (daemon ← client) and responses / push
/// notifications (daemon → client) use this framing.
///
/// # Cancel safety
/// `BufReader::read_line` is not fully cancel-safe: if the `push_rx` branch
/// of the `select!` fires while a read is in progress, bytes already pulled
/// into the `BufReader`'s internal buffer (but not yet forming a complete
/// line) can be silently discarded.  For the typical small IPC messages used
/// here this is inconsequential in practice; a future milestone can upgrade
/// to `tokio_util::codec::LinesCodec` if needed.
async fn handle_connection<S>(
    stream: S,
    router: Arc<Router>,
    mut push_rx: broadcast::Receiver<String>,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (reader, mut writer) = tokio::io::split(stream);
    let mut reader = BufReader::new(reader);
    let mut line = String::new();

    debug!("new IPC connection");

    loop {
        line.clear();

        tokio::select! {
            // Read the next newline-terminated request from the client.
            result = reader.read_line(&mut line) => {
                match result {
                    Ok(0) => {
                        // EOF — client closed the connection.
                        debug!("IPC client disconnected");
                        break;
                    }
                    Ok(_) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        match serde_json::from_str::<JsonRpcRequest>(trimmed) {
                            Ok(req) => {
                                if let Some(resp) = router.handle(req).await {
                                    if send_message(&mut writer, &resp).await.is_err() {
                                        break;
                                    }
                                }
                            }
                            Err(e) => {
                                warn!("IPC parse error ({e}): {trimmed:.80}");
                                let resp = JsonRpcResponse::error(
                                    None,
                                    JsonRpcError::parse_error(),
                                );
                                // Best-effort: ignore write errors after a
                                // parse error since the connection is likely
                                // broken anyway.
                                let _ = send_message(&mut writer, &resp).await;
                            }
                        }
                    }
                    Err(e) => {
                        error!("IPC read error: {e}");
                        break;
                    }
                }
            }

            // Forward push notifications from the daemon to this client.
            push_result = push_rx.recv() => {
                match push_result {
                    Ok(msg) => {
                        // `msg` is already serialised JSON without a trailing
                        // newline; we add one here for framing.
                        let framed = msg + "\n";
                        if writer.write_all(framed.as_bytes()).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("IPC push receiver lagged, skipped {n} messages");
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        // Daemon is shutting down.
                        break;
                    }
                }
            }
        }
    }

    debug!("IPC connection closed");
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Serialise `msg` as a single JSON line and write it to `writer`.
async fn send_message<W, T>(writer: &mut W, msg: &T) -> std::io::Result<()>
where
    W: AsyncWriteExt + Unpin,
    T: Serialize,
{
    let mut buf = match serde_json::to_vec(msg) {
        Ok(v) => v,
        Err(e) => {
            error!("IPC serialisation error: {e}");
            return Ok(()); // non-fatal; skip this message
        }
    };
    buf.push(b'\n');
    writer.write_all(&buf).await
}

/// Canonical Unix socket path:
///   `$XDG_RUNTIME_DIR/gdriver.sock`   (Linux, set by the user session)
///   `$TMPDIR/gdriver.sock`             (macOS fallback)
#[cfg(unix)]
fn unix_socket_path() -> std::path::PathBuf {
    dirs::runtime_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("gdriver.sock")
}
