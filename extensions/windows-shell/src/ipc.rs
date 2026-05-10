// ─── IPC Client for gdriver-daemon (Windows Named Pipes) ──────────────────
//
// Implements JSON-RPC 2.0 over Named Pipes, matching the protocol used by
// the Rust `gdriver-ipc` crate and the Python IPC clients for Linux
// extensions.

use std::{
    io::Write,
    time::Duration,
};

use serde_json::Value;
use windows::Win32::{Foundation::*, Storage::FileSystem::*};

/// Named Pipe path for the gdriver daemon on Windows.
const PIPE_PATH: &str = r"\\.\pipe\gdriver";

/// Default timeout for IPC calls.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

/// JSON-RPC 2.0 error response.
#[derive(Debug)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
}

impl std::fmt::Display for JsonRpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "JSON-RPC error {}: {}", self.code, self.message)
    }
}

impl std::error::Error for JsonRpcError {}

/// Synchronous blocking IPC client for communicating with gdriver-daemon.
///
/// Opens a Named Pipe, sends newline-delimited JSON-RPC 2.0 requests,
/// and reads back the corresponding response. Push notifications from the
/// daemon are silently discarded.
pub struct IpcClient {
    pipe: HANDLE,
    buf: Vec<u8>,
    next_id: u64,
}

// HANDLE is safe to send across threads
unsafe impl Send for IpcClient {}

impl IpcClient {
    /// Create a new IPC client connected to the daemon.
    pub fn new() -> Result<Self, JsonRpcError> {
        Self::with_timeout(DEFAULT_TIMEOUT)
    }

    /// Create a new IPC client with a specific timeout.
    pub fn with_timeout(timeout: Duration) -> Result<Self, JsonRpcError> {
        let pipe = unsafe {
            CreateFileA(
                PIPE_PATH,
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )
        };

        let pipe = match pipe {
            Ok(h) => h,
            Err(e) => {
                return Err(JsonRpcError {
                    code: -1,
                    message: format!("failed to connect to daemon pipe: {}", e),
                });
            }
        };

        // Set pipe timeout
        let timeout_ms = timeout.as_millis() as u32;
        unsafe {
            let _ = SetNamedPipeHandleState(pipe, Some(&timeout_ms), None, None);
        }

        Ok(Self {
            pipe,
            buf: Vec::new(),
            next_id: 1,
        })
    }

    /// Send a JSON-RPC request and wait for the matching response.
    pub fn call(&mut self, method: &str, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let req_id = self.next_id;
        self.next_id += 1;

        let mut request = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "id": req_id,
        });

        if let Some(p) = params {
            request["params"] = p;
        }

        let data = serde_json::to_string(&request).map_err(|e| JsonRpcError {
            code: -1,
            message: format!("failed to serialize request: {}", e),
        })?;

        // Send the request
        let mut bytes_written = 0u32;
        let request_bytes = format!("{}\n", data).into_bytes();
        unsafe {
            WriteFile(
                self.pipe,
                Some(&request_bytes),
                Some(&mut bytes_written),
                None,
            )
            .map_err(|e| JsonRpcError {
                code: -1,
                message: format!("failed to write to pipe: {}", e),
            })?;
        }

        // Read the response
        loop {
            let line = self.readline()?;
            let line_str = String::from_utf8_lossy(&line);

            let resp: Value = serde_json::from_str(&line_str).map_err(|e| JsonRpcError {
                code: -1,
                message: format!("invalid JSON: {}", e),
            })?;

            // Push notifications have no id — skip them
            if resp.get("id").is_none() {
                continue;
            }

            if let Some(error) = resp.get("error") {
                return Err(JsonRpcError {
                    code: error.get("code").and_then(|c| c.as_i64()).unwrap_or(-1) as i32,
                    message: error
                        .get("message")
                        .and_then(|m| m.as_str())
                        .unwrap_or("unknown error")
                        .to_string(),
                });
            }

            return Ok(resp.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    /// Read a newline-delimited line from the pipe buffer.
    fn readline(&mut self) -> Result<Vec<u8>, JsonRpcError> {
        loop {
            // Check if we already have a complete line in the buffer
            if let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
                let line = self.buf[..pos].to_vec();
                self.buf = self.buf[pos + 1..].to_vec();
                return Ok(line);
            }

            // Read more data from the pipe
            let mut chunk = [0u8; 4096];
            let mut bytes_read = 0u32;
            unsafe {
                ReadFile(self.pipe, Some(&mut chunk), Some(&mut bytes_read), None).map_err(
                    |e| JsonRpcError {
                        code: -1,
                        message: format!("failed to read from pipe: {}", e),
                    },
                )?;
            }

            if bytes_read == 0 {
                return Err(JsonRpcError {
                    code: -1,
                    message: "daemon disconnected".to_string(),
                });
            }

            self.buf.extend_from_slice(&chunk[..bytes_read as usize]);
        }
    }
}

impl Drop for IpcClient {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.pipe);
        }
    }
}

// ─── Convenience wrappers ──────────────────────────────────────────────────

/// Query the sync state for a file.
pub fn get_sync_state(path: &str) -> Option<Value> {
    let mut client = IpcClient::new().ok()?;
    client
        .call("fs.getSyncState", Some(serde_json::json!({"path": path})))
        .ok()
}

/// Set a file's offline availability.
pub fn set_offline(path: &str, enabled: bool) -> bool {
    let mut client = match IpcClient::new() {
        Ok(c) => c,
        Err(_) => return false,
    };
    client
        .call(
            "fs.setOffline",
            Some(serde_json::json!({"path": path, "enabled": enabled})),
        )
        .is_ok()
}

/// Get the Google Drive share link for a file.
pub fn get_share_link(path: &str) -> Option<String> {
    let mut client = IpcClient::new().ok()?;
    let result = client
        .call("fs.getShareLink", Some(serde_json::json!({"path": path})))
        .ok()?;
    result.get("url")?.as_str().map(String::from)
}
