//! Integration tests: JSON-RPC 2.0 protocol over Unix domain sockets.
//!
//! Starts a mock daemon that speaks JSON-RPC, then connects with an IpcClient
//! and verifies request/response round-trips, notification handling, and error
//! cases — simulating "Daemon 启动 + IPC 连通性测试".

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use gdriver_ipc::types::*;
use serde_json::{json, Value};

static SOCK_SEQ: AtomicU64 = AtomicU64::new(0);

fn unique_sock_path(label: &str) -> std::path::PathBuf {
    let n = SOCK_SEQ.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!("gdriver-{}-{}-{}.sock", label, std::process::id(), n))
}

/// Start a mock daemon on a temp socket that handles one JSON-RPC request.
///
/// Returns the socket path so the client can connect.
fn start_mock_daemon(
    handler: impl Fn(&JsonRpcRequest) -> JsonRpcResponse + Send + 'static,
) -> (std::path::PathBuf, Arc<AtomicBool>) {
    let done = Arc::new(AtomicBool::new(false));
    let done_clone = done.clone();

    let tmp = unique_sock_path("daemon");
    // Remove stale socket if it exists
    let _ = std::fs::remove_file(&tmp);

    let listener = UnixListener::bind(&tmp).expect("failed to bind test socket");

    std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept failed");
        serve_one(stream, &handler);
        done_clone.store(true, Ordering::SeqCst);
    });

    (tmp, done)
}

fn serve_one(stream: UnixStream, handler: &impl Fn(&JsonRpcRequest) -> JsonRpcResponse) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut writer = stream;

    let mut line = String::new();
    reader.read_line(&mut line).expect("failed to read request");
    let request: JsonRpcRequest = serde_json::from_str(line.trim()).expect("invalid request");
    let response = handler(&request);
    let mut json = serde_json::to_string(&response).expect("response serialize failed");
    json.push('\n');
    writer.write_all(json.as_bytes()).expect("write failed");
    writer.flush().expect("flush failed");
}

// ── Basic round-trip ───────────────────────────────────────────────────────

#[test]
fn round_trip_ping_pong() {
    let (sock_path, done) = start_mock_daemon(|req| {
        assert_eq!(req.method, "ping");
        JsonRpcResponse::success(req.id.clone(), json!("pong"))
    });

    let stream = UnixStream::connect(&sock_path).expect("connect failed");
    stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    stream.set_write_timeout(Some(Duration::from_secs(2))).unwrap();
    let mut writer = stream.try_clone().unwrap();
    let mut reader = BufReader::new(stream);

    // Send request
    let request = JsonRpcRequest::new("ping", None, JsonRpcId::Num(1));
    let mut req_json = serde_json::to_string(&request).unwrap();
    req_json.push('\n');
    writer.write_all(req_json.as_bytes()).unwrap();
    writer.flush().unwrap();

    // Read response
    let mut resp_line = String::new();
    reader.read_line(&mut resp_line).unwrap();
    let response: JsonRpcResponse = serde_json::from_str(resp_line.trim()).unwrap();
    assert!(response.is_success());
    assert_eq!(response.result, Some(Value::String("pong".into())));
    assert_eq!(response.id, Some(JsonRpcId::Num(1)));

    // Wait for server to finish
    while !done.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(10));
    }
    let _ = std::fs::remove_file(&sock_path);
}

#[test]
fn round_trip_sync_state_query() {
    let (sock_path, done) = start_mock_daemon(|req| {
        assert_eq!(req.method, "fs.get_sync_state");
        let params = req.params.as_ref().unwrap();
        assert_eq!(params["path"], "/home/user/GoogleDrive/doc.txt");
        JsonRpcResponse::success(
            req.id.clone(),
            json!({
                "state": "synced",
                "file_id": "abc123",
                "name": "doc.txt",
                "is_folder": false,
                "drive_url": "https://drive.google.com/file/d/abc123"
            }),
        )
    });

    let stream = UnixStream::connect(&sock_path).expect("connect failed");
    stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    stream.set_write_timeout(Some(Duration::from_secs(2))).unwrap();
    let mut writer = stream.try_clone().unwrap();
    let mut reader = BufReader::new(stream);

    let request = JsonRpcRequest::new(
        "fs.get_sync_state",
        Some(json!({"path": "/home/user/GoogleDrive/doc.txt"})),
        JsonRpcId::Str("req-uuid".into()),
    );
    let mut req_json = serde_json::to_string(&request).unwrap();
    req_json.push('\n');
    writer.write_all(req_json.as_bytes()).unwrap();
    writer.flush().unwrap();

    let mut resp_line = String::new();
    reader.read_line(&mut resp_line).unwrap();
    let response: JsonRpcResponse = serde_json::from_str(resp_line.trim()).unwrap();
    assert!(response.is_success());

    let result = response.result.unwrap();
    assert_eq!(result["state"], "synced");
    assert_eq!(result["file_id"], "abc123");

    while !done.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(10));
    }
    let _ = std::fs::remove_file(&sock_path);
}

// ── Error responses ────────────────────────────────────────────────────────

#[test]
fn method_not_found_error() {
    let (sock_path, done) = start_mock_daemon(|req| {
        let err = JsonRpcError::method_not_found(&req.method);
        JsonRpcResponse::error(req.id.clone(), err)
    });

    let stream = UnixStream::connect(&sock_path).expect("connect failed");
    stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    stream.set_write_timeout(Some(Duration::from_secs(2))).unwrap();
    let mut writer = stream.try_clone().unwrap();
    let mut reader = BufReader::new(stream);

    let request = JsonRpcRequest::new("unknown.method", None, JsonRpcId::Num(42));
    let mut req_json = serde_json::to_string(&request).unwrap();
    req_json.push('\n');
    writer.write_all(req_json.as_bytes()).unwrap();
    writer.flush().unwrap();

    let mut resp_line = String::new();
    reader.read_line(&mut resp_line).unwrap();
    let response: JsonRpcResponse = serde_json::from_str(resp_line.trim()).unwrap();
    assert!(!response.is_success());
    let err = response.error.unwrap();
    assert_eq!(err.code, -32601); // Method not found
    assert_eq!(err.message, "Method not found");
    assert_eq!(err.data, Some(serde_json::Value::String("unknown.method".into())));

    while !done.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(10));
    }
    let _ = std::fs::remove_file(&sock_path);
}

#[test]
fn internal_error_response() {
    let (sock_path, done) = start_mock_daemon(|req| {
        let err = JsonRpcError::internal_error("something went wrong");
        JsonRpcResponse::error(req.id.clone(), err)
    });

    let stream = UnixStream::connect(&sock_path).expect("connect failed");
    stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    stream.set_write_timeout(Some(Duration::from_secs(2))).unwrap();
    let mut writer = stream.try_clone().unwrap();
    let mut reader = BufReader::new(stream);

    let request = JsonRpcRequest::new("fs.delete", Some(json!({"path": "/x"})), JsonRpcId::Num(7));
    let mut req_json = serde_json::to_string(&request).unwrap();
    req_json.push('\n');
    writer.write_all(req_json.as_bytes()).unwrap();
    writer.flush().unwrap();

    let mut resp_line = String::new();
    reader.read_line(&mut resp_line).unwrap();
    let response: JsonRpcResponse = serde_json::from_str(resp_line.trim()).unwrap();
    assert!(!response.is_success());
    let err = response.error.unwrap();
    assert_eq!(err.code, -32603);
    assert!(err.message.contains("something went wrong"));

    while !done.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(10));
    }
    let _ = std::fs::remove_file(&sock_path);
}

// ── Notification (no id) ───────────────────────────────────────────────────

#[test]
fn push_notification_has_no_id() {
    // A notification (push event) is just a JSON-RPC request without an id.
    let notif = JsonRpcRequest::notification(
        "event:sync_status_changed",
        Some(json!({"status": "syncing", "ts": 1700000000000i64})),
    );
    assert!(notif.is_notification());
    assert!(notif.id.is_none());

    let json = serde_json::to_string(&notif).unwrap();
    let parsed: JsonRpcRequest = serde_json::from_str(&json).unwrap();
    assert!(parsed.is_notification());

    // Verify the notification does not contain an "id" key in the raw JSON.
    assert!(!json.contains("\"id\""));
}

// ── Connection failure without daemon ──────────────────────────────────────

#[test]
fn connect_to_nonexistent_socket_fails() {
    let nonexistent = unique_sock_path("nonexistent");
    let _ = std::fs::remove_file(&nonexistent);

    let result = std::os::unix::net::UnixStream::connect(&nonexistent);
    assert!(result.is_err(), "should fail when no daemon is listening");
}

// ── Multiple sequential requests on same connection ────────────────────────

#[test]
fn pipelining_two_requests_on_same_socket() {
    // This test validates that the protocol can handle pipelined requests.
    // We test the JSON-RPC ID matching logic by verifying that a server can
    // handle two distinct requests on the same connection.

    let tmp = unique_sock_path("pipe");
    let _ = std::fs::remove_file(&tmp);
    let listener = UnixListener::bind(&tmp).expect("bind");

    let handle = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut writer = stream;

        // Request 1: ping with id=1
        let mut line1 = String::new();
        reader.read_line(&mut line1).unwrap();
        let req1: JsonRpcRequest = serde_json::from_str(line1.trim()).unwrap();
        assert_eq!(req1.id, Some(JsonRpcId::Num(1)));
        let resp1 = JsonRpcResponse::success(Some(JsonRpcId::Num(1)), json!("pong1"));
        let mut json1 = serde_json::to_string(&resp1).unwrap();
        json1.push('\n');
        writer.write_all(json1.as_bytes()).unwrap();
        writer.flush().unwrap();

        // Request 2: ping with id=2
        let mut line2 = String::new();
        reader.read_line(&mut line2).unwrap();
        let req2: JsonRpcRequest = serde_json::from_str(line2.trim()).unwrap();
        assert_eq!(req2.id, Some(JsonRpcId::Num(2)));
        let resp2 = JsonRpcResponse::success(Some(JsonRpcId::Num(2)), json!("pong2"));
        let mut json2 = serde_json::to_string(&resp2).unwrap();
        json2.push('\n');
        writer.write_all(json2.as_bytes()).unwrap();
        writer.flush().unwrap();
    });

    let stream = UnixStream::connect(&tmp).expect("connect");
    stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    stream.set_write_timeout(Some(Duration::from_secs(2))).unwrap();
    let mut writer = stream.try_clone().unwrap();
    let mut reader = BufReader::new(stream);

    // Send request 1
    let req1 = JsonRpcRequest::new("ping", None, JsonRpcId::Num(1));
    let mut j1 = serde_json::to_string(&req1).unwrap();
    j1.push('\n');
    writer.write_all(j1.as_bytes()).unwrap();
    writer.flush().unwrap();

    // Read response 1
    let mut r1 = String::new();
    reader.read_line(&mut r1).unwrap();
    let resp1: JsonRpcResponse = serde_json::from_str(r1.trim()).unwrap();
    assert_eq!(resp1.result, Some(Value::String("pong1".into())));

    // Send request 2
    let req2 = JsonRpcRequest::new("ping", None, JsonRpcId::Num(2));
    let mut j2 = serde_json::to_string(&req2).unwrap();
    j2.push('\n');
    writer.write_all(j2.as_bytes()).unwrap();
    writer.flush().unwrap();

    // Read response 2
    let mut r2 = String::new();
    reader.read_line(&mut r2).unwrap();
    let resp2: JsonRpcResponse = serde_json::from_str(r2.trim()).unwrap();
    assert_eq!(resp2.result, Some(Value::String("pong2".into())));

    handle.join().unwrap();
    let _ = std::fs::remove_file(&tmp);
}

// ── Large payload round-trip ───────────────────────────────────────────────

#[test]
fn large_payload_round_trip() {
    let (sock_path, done) = start_mock_daemon(|req| {
        assert_eq!(req.method, "fs.list_files");
        // Echo back the params as the result so we can verify size
        JsonRpcResponse::success(req.id.clone(), req.params.clone().unwrap_or(Value::Null))
    });

    let stream = UnixStream::connect(&sock_path).expect("connect failed");
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    stream.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut writer = stream.try_clone().unwrap();
    let mut reader = BufReader::new(stream);

    // Build a large JSON payload (~200 files)
    let mut files = Vec::new();
    for i in 0..200 {
        files.push(json!({
            "id": format!("file-{}", i),
            "name": format!("document-{}.pdf", i),
            "mimeType": "application/pdf",
            "path": format!("/home/user/GoogleDrive/document-{}.pdf", i),
            "state": "synced",
            "size": 1024000
        }));
    }

    let request = JsonRpcRequest::new(
        "fs.list_files",
        Some(json!({"files": files, "total": 200})),
        JsonRpcId::Num(0),
    );
    let mut req_json = serde_json::to_string(&request).unwrap();
    req_json.push('\n');
    writer.write_all(req_json.as_bytes()).unwrap();
    writer.flush().unwrap();

    let mut resp_line = String::new();
    reader.read_line(&mut resp_line).unwrap();
    let response: JsonRpcResponse = serde_json::from_str(resp_line.trim()).unwrap();
    assert!(response.is_success());
    let result = response.result.unwrap();
    assert_eq!(result["total"], 200);
    assert_eq!(result["files"].as_array().unwrap().len(), 200);

    while !done.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(10));
    }
    let _ = std::fs::remove_file(&sock_path);
}
