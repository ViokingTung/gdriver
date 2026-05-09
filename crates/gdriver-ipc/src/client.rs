use std::cell::RefCell;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use serde_json::Value;

use crate::methods::*;
use crate::types::*;

/// Return the path to the daemon IPC socket.
///
/// Mirrors the daemon's `socket_path()` logic.
pub fn socket_path() -> PathBuf {
    dirs::runtime_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("gdriver.sock")
}

/// Synchronous, blocking IPC client for communicating with `gdriver-daemon`.
///
/// Designed for use by file manager extensions (Nautilus, Dolphin, etc.) which
/// run inside the file manager's process and cannot use an async runtime.
///
/// The client opens a Unix Domain Socket, sends newline-delimited JSON-RPC 2.0
/// requests, and reads back the corresponding response.  Push notifications
/// from the daemon are silently discarded (extensions are read-only and do not
/// need real-time events).
pub struct IpcClient {
    reader: RefCell<BufReader<UnixStream>>,
    writer: UnixStream,
    next_id: AtomicI64,
}

impl IpcClient {
    /// Connect to the daemon IPC socket.
    ///
    /// `timeout` controls how long each individual read/write may block before
    /// returning an error.
    pub fn connect(timeout: Duration) -> Result<Self, std::io::Error> {
        let stream = UnixStream::connect(socket_path())?;
        stream.set_read_timeout(Some(timeout))?;
        stream.set_write_timeout(Some(timeout))?;
        let writer = stream.try_clone()?;
        let reader = BufReader::new(stream);
        Ok(Self {
            reader: RefCell::new(reader),
            writer,
            next_id: AtomicI64::new(1),
        })
    }

    /// Connect using the default 5-second timeout.
    pub fn connect_default() -> Result<Self, std::io::Error> {
        Self::connect(Duration::from_secs(5))
    }

    /// Send a JSON-RPC request and wait for the matching response.
    ///
    /// Push notifications received while waiting are silently skipped.
    pub fn call(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let request = JsonRpcRequest::new(method, params, JsonRpcId::Num(id));

        let mut json = serde_json::to_string(&request)
            .map_err(|e| JsonRpcError::internal_error(e.to_string()))?;
        json.push('\n');

        // Write the request.
        {
            let mut writer = &self.writer;
            writer.write_all(json.as_bytes()).map_err(io_err)?;
            writer.flush().map_err(io_err)?;
        }

        // Read responses until we get one with a matching id.
        // Push notifications (no id) are silently discarded.
        loop {
            let mut line = String::new();
            let n = self
                .reader
                .borrow_mut()
                .read_line(&mut line)
                .map_err(io_err)?;
            if n == 0 {
                return Err(JsonRpcError::internal_error("daemon disconnected"));
            }

            let resp: JsonRpcResponse = serde_json::from_str(line.trim())
                .map_err(|e| JsonRpcError::internal_error(e.to_string()))?;

            // Push notifications have no id — skip them.
            if resp.id.is_none() {
                continue;
            }

            if resp.is_success() {
                return Ok(resp.result.unwrap_or(Value::Null));
            } else {
                return Err(resp.error.unwrap_or_else(|| JsonRpcError::internal_error("unknown error")));
            }
        }
    }
}

fn io_err(e: std::io::Error) -> JsonRpcError {
    JsonRpcError::internal_error(e.to_string())
}

// ─── C FFI for file manager extensions ──────────────────────────────────────

/// Opaque handle wrapping [`IpcClient`] for C callers.
pub struct GDriverIpcHandle {
    client: IpcClient,
}

/// Connect to the daemon and return an opaque handle.
///
/// Returns a null pointer on failure.  The caller must free the handle with
/// [`gdriver_disconnect`] when done.
///
/// # Safety
/// The returned pointer must be passed to `gdriver_disconnect` exactly once.
#[no_mangle]
pub unsafe extern "C" fn gdriver_connect() -> *mut GDriverIpcHandle {
    match IpcClient::connect_default() {
        Ok(client) => Box::into_raw(Box::new(GDriverIpcHandle { client })),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Free the handle and close the connection.
///
/// # Safety
/// `handle` must have been returned by `gdriver_connect` and must not be used
/// after this call.
#[no_mangle]
pub unsafe extern "C" fn gdriver_disconnect(handle: *mut GDriverIpcHandle) {
    if !handle.is_null() {
        drop(unsafe { Box::from_raw(handle) });
    }
}

/// Query the sync state for a file identified by its local path.
///
/// Returns a heap-allocated JSON string that the caller must free with
/// [`gdriver_free_string`].  Returns a null pointer on IPC failure.
///
/// Example return value:
/// ```json
/// {"state":"synced","file_id":"abc123","name":"report.pdf","is_folder":false,"drive_url":"https://drive.google.com/file/d/abc123/view"}
/// ```
///
/// # Safety
/// `handle` must be a valid pointer from `gdriver_connect`.
/// `path` must be a valid, null-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn gdriver_get_sync_state(
    handle: *mut GDriverIpcHandle,
    path: *const std::ffi::c_char,
) -> *mut std::ffi::c_char {
    let handle = match unsafe { handle.as_ref() } {
        Some(h) => h,
        None => return std::ptr::null_mut(),
    };
    let path = match unsafe { std::ffi::CStr::from_ptr(path) }.to_str() {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };

    let params = serde_json::json!({ "path": path });
    match handle.client.call(FS_GET_SYNC_STATE, Some(params)) {
        Ok(val) => match serde_json::to_string(&val) {
            Ok(s) => std::ffi::CString::new(s).unwrap().into_raw(),
            Err(_) => std::ptr::null_mut(),
        },
        Err(_) => std::ptr::null_mut(),
    }
}

/// Set a file's offline availability.
///
/// Returns `"ok"` on success, or a null pointer on failure.
///
/// # Safety
/// `handle` must be a valid pointer from `gdriver_connect`.
/// `path` must be a valid, null-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn gdriver_set_offline(
    handle: *mut GDriverIpcHandle,
    path: *const std::ffi::c_char,
    enabled: bool,
) -> *mut std::ffi::c_char {
    let handle = match unsafe { handle.as_ref() } {
        Some(h) => h,
        None => return std::ptr::null_mut(),
    };
    let path = match unsafe { std::ffi::CStr::from_ptr(path) }.to_str() {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };

    let params = serde_json::json!({ "path": path, "enabled": enabled });
    match handle.client.call(FS_SET_OFFLINE, Some(params)) {
        Ok(_) => std::ffi::CString::new("ok").unwrap().into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Get the Google Drive share link for a file.
///
/// Returns a heap-allocated URL string that the caller must free with
/// [`gdriver_free_string`].  Returns a null pointer on failure.
///
/// # Safety
/// `handle` must be a valid pointer from `gdriver_connect`.
/// `path` must be a valid, null-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn gdriver_get_share_link(
    handle: *mut GDriverIpcHandle,
    path: *const std::ffi::c_char,
) -> *mut std::ffi::c_char {
    let handle = match unsafe { handle.as_ref() } {
        Some(h) => h,
        None => return std::ptr::null_mut(),
    };
    let path = match unsafe { std::ffi::CStr::from_ptr(path) }.to_str() {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };

    let params = serde_json::json!({ "path": path });
    match handle.client.call(FS_GET_SHARE_LINK, Some(params)) {
        Ok(val) => {
            // The handler returns {"url": "..."} — extract the url field.
            let url = val.get("url").and_then(|v| v.as_str()).unwrap_or("");
            std::ffi::CString::new(url).unwrap().into_raw()
        }
        Err(_) => std::ptr::null_mut(),
    }
}

/// Free a string returned by `gdriver_get_sync_state`, `gdriver_get_share_link`,
/// or `gdriver_set_offline`.
///
/// # Safety
/// `s` must have been returned by one of the above functions and must not be
/// used after this call.
#[no_mangle]
pub unsafe extern "C" fn gdriver_free_string(s: *mut std::ffi::c_char) {
    if !s.is_null() {
        drop(unsafe { std::ffi::CString::from_raw(s) });
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_path_is_non_empty() {
        let path = socket_path();
        assert!(!path.as_os_str().is_empty());
        assert!(path.to_string_lossy().contains("gdriver.sock"));
    }

    #[test]
    #[ignore] // Requires daemon to NOT be running — run with `cargo test -- --ignored`
    fn connect_fails_when_daemon_not_running() {
        // Daemon is not running in the test environment — connection should fail.
        let result = IpcClient::connect(Duration::from_millis(100));
        assert!(result.is_err());
    }
}
