#[cfg(windows)]
use std::io::BufRead;
use std::{
    cell::RefCell,
    io::Write,
    sync::atomic::{AtomicI64, Ordering},
    time::Duration,
};
#[cfg(unix)]
use std::{
    io::{BufRead, BufReader},
    os::unix::net::UnixStream,
    path::PathBuf,
};

#[cfg(windows)]
mod windows_impl {
    use std::{
        ffi::OsStr,
        io::{self, BufReader, Read, Write},
        os::windows::{
            ffi::OsStrExt,
            io::{AsRawHandle, FromRawHandle, OwnedHandle},
        },
        time::Duration,
    };

    use windows_sys::Win32::{
        Foundation::INVALID_HANDLE_VALUE,
        Storage::FileSystem::{
            CreateFileW, ReadFile, WriteFile, FILE_GENERIC_READ, FILE_GENERIC_WRITE,
            FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
        },
        System::Pipes::PIPE_READMODE_BYTE,
    };

    pub struct NamedPipeClient {
        reader: BufReader<PipeReader>,
        writer: PipeWriter,
    }

    struct PipeReader {
        handle: OwnedHandle,
    }

    struct PipeWriter {
        handle: OwnedHandle,
    }

    impl Read for PipeReader {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let mut bytes_read = 0;
            let result = unsafe {
                ReadFile(
                    self.handle.as_raw_handle() as _,
                    buf.as_mut_ptr(),
                    buf.len() as u32,
                    &mut bytes_read,
                    std::ptr::null_mut(),
                )
            };
            if result == 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(bytes_read as usize)
            }
        }
    }

    impl Write for PipeWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let mut bytes_written = 0;
            let result = unsafe {
                WriteFile(
                    self.handle.as_raw_handle() as _,
                    buf.as_ptr(),
                    buf.len() as u32,
                    &mut bytes_written,
                    std::ptr::null_mut(),
                )
            };
            if result == 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(bytes_written as usize)
            }
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl NamedPipeClient {
        pub fn connect(pipe_name: &str, _timeout: Duration) -> io::Result<Self> {
            let wide_name: Vec<u16> = OsStr::new(pipe_name)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();

            // Try to open existing pipe first
            let handle = unsafe {
                CreateFileW(
                    wide_name.as_ptr(),
                    FILE_GENERIC_READ | FILE_GENERIC_WRITE,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    std::ptr::null_mut(),
                    OPEN_EXISTING,
                    0,
                    std::ptr::null_mut(),
                )
            };

            if handle == INVALID_HANDLE_VALUE {
                return Err(io::Error::last_os_error());
            }

            let handle = unsafe { OwnedHandle::from_raw_handle(handle as _) };

            // Set pipe to byte mode
            unsafe {
                windows_sys::Win32::System::Pipes::SetNamedPipeHandleState(
                    handle.as_raw_handle() as _,
                    &PIPE_READMODE_BYTE,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                );
            }

            let reader_handle = handle.try_clone()?;
            let writer_handle = handle;

            Ok(Self {
                reader: BufReader::new(PipeReader {
                    handle: reader_handle,
                }),
                writer: PipeWriter {
                    handle: writer_handle,
                },
            })
        }

        pub fn reader(&mut self) -> &mut BufReader<PipeReader> {
            &mut self.reader
        }

        pub fn writer(&mut self) -> &mut PipeWriter {
            &mut self.writer
        }
    }
}

use serde_json::Value;

use crate::{methods::*, types::*};

/// Return the path to the daemon IPC socket.
///
/// Mirrors the daemon's `socket_path()` logic.
#[cfg(unix)]
pub fn socket_path() -> PathBuf {
    dirs::runtime_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("gdriver.sock")
}

/// Return the named pipe path for Windows.
#[cfg(windows)]
pub fn pipe_path() -> String {
    r"\\.\pipe\gdriver".to_string()
}

/// Synchronous, blocking IPC client for communicating with `gdriver-daemon`.
///
/// Designed for use by file manager extensions (Nautilus, Dolphin, etc.) which
/// run inside the file manager's process and cannot use an async runtime.
///
/// The client opens a Unix Domain Socket (Unix) or Named Pipe (Windows),
/// sends newline-delimited JSON-RPC 2.0 requests, and reads back the
/// corresponding response. Push notifications from the daemon are silently
/// discarded (extensions are read-only and do not need real-time events).
#[cfg(unix)]
pub struct IpcClient {
    reader: RefCell<BufReader<UnixStream>>,
    writer: UnixStream,
    next_id: AtomicI64,
}

#[cfg(windows)]
pub struct IpcClient {
    pipe: RefCell<windows_impl::NamedPipeClient>,
    next_id: AtomicI64,
}

impl IpcClient {
    /// Connect to the daemon IPC socket.
    ///
    /// `timeout` controls how long each individual read/write may block before
    /// returning an error.
    #[cfg(unix)]
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

    /// Connect to the daemon named pipe (Windows).
    #[cfg(windows)]
    pub fn connect(timeout: Duration) -> Result<Self, std::io::Error> {
        let pipe = windows_impl::NamedPipeClient::connect(&pipe_path(), timeout)?;
        Ok(Self {
            pipe: RefCell::new(pipe),
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
    #[cfg(unix)]
    pub fn call(&self, method: &str, params: Option<Value>) -> Result<Value, JsonRpcError> {
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
                return Err(resp
                    .error
                    .unwrap_or_else(|| JsonRpcError::internal_error("unknown error")));
            }
        }
    }

    /// Send a JSON-RPC request and wait for the matching response (Windows).
    #[cfg(windows)]
    pub fn call(&self, method: &str, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let request = JsonRpcRequest::new(method, params, JsonRpcId::Num(id));

        let mut json = serde_json::to_string(&request)
            .map_err(|e| JsonRpcError::internal_error(e.to_string()))?;
        json.push('\n');

        // Write the request.
        {
            let mut pipe = self.pipe.borrow_mut();
            let writer = pipe.writer();
            writer.write_all(json.as_bytes()).map_err(io_err)?;
            writer.flush().map_err(io_err)?;
        }

        // Read responses until we get one with a matching id.
        // Push notifications (no id) are silently discarded.
        loop {
            let mut line = String::new();
            let mut pipe = self.pipe.borrow_mut();
            let n = pipe.reader().read_line(&mut line).map_err(io_err)?;
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
                return Err(resp
                    .error
                    .unwrap_or_else(|| JsonRpcError::internal_error("unknown error")));
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
    #[cfg(unix)]
    fn socket_path_is_non_empty() {
        let path = socket_path();
        assert!(!path.as_os_str().is_empty());
        assert!(path.to_string_lossy().contains("gdriver.sock"));
    }

    #[test]
    #[cfg(windows)]
    fn pipe_path_is_non_empty() {
        let path = pipe_path();
        assert!(!path.is_empty());
        assert!(path.contains("gdriver"));
    }

    #[test]
    #[ignore] // Requires daemon to NOT be running — run with `cargo test -- --ignored`
    fn connect_fails_when_daemon_not_running() {
        // Daemon is not running in the test environment — connection should fail.
        let result = IpcClient::connect(Duration::from_millis(100));
        assert!(result.is_err());
    }

    #[test]
    #[cfg(windows)]
    fn windows_connect_fails_when_daemon_not_running() {
        // On Windows, connection should fail when daemon is not running.
        let result = IpcClient::connect(Duration::from_millis(100));
        assert!(result.is_err());
    }
}
