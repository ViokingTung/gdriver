//! Integration tests: upload → download → export pipeline.
//!
//! Tests the full content lifecycle: multipart upload, download, Workspace
//! document export, and resumable upload with chunking.

use gdriver_api::client::DriveClient;
use wiremock::{
    matchers::{header, method, path, query_param},
    Mock, MockServer, ResponseTemplate,
};

fn sample_file_json(id: &str, name: &str, mime: &str) -> String {
    format!(
        r#"{{
            "id": "{id}",
            "name": "{name}",
            "mimeType": "{mime}",
            "parents": ["root"],
            "size": "1024",
            "etag": "\"etag_{id}\"",
            "version": "1",
            "modifiedTime": "2026-05-01T12:00:00.000Z",
            "createdTime": "2026-04-01T12:00:00.000Z",
            "trashed": false,
            "md5Checksum": "abc123",
            "webViewLink": "https://drive.google.com/file/d/{id}"
        }}"#
    )
}

// ── Multipart upload → download → verify ───────────────────────────────────

#[tokio::test]
async fn upload_multipart_then_download_verify_content() {
    let server = MockServer::start().await;
    let client = DriveClient::new("up-down-tok");
    let file_content = b"Hello, Drive! This is test content for upload.";

    // 1. Multipart upload
    Mock::given(method("POST"))
        .and(path("/upload/drive/v3/files"))
        .and(query_param("uploadType", "multipart"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            sample_file_json("uploaded-1", "hello.txt", "text/plain"),
            "application/json",
        ))
        .expect(1)
        .mount(&server)
        .await;

    let fields_enc = "id%2Cname%2CmimeType%2Cparents%2Csize%2Cversion%2CmodifiedTime%2CcreatedTime%2Ctrashed%2Cshared%2Cmd5Checksum%2CwebViewLink";
    let upload_url = format!(
        "{}/upload/drive/v3/files?uploadType=multipart&fields={}",
        server.uri(),
        fields_enc
    );

    // Build multipart body
    let boundary = "gdriver_multipart_boundary";
    let metadata_json = serde_json::json!({
        "name": "hello.txt",
        "mimeType": "text/plain"
    })
    .to_string();

    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(b"Content-Type: application/json; charset=UTF-8\r\n\r\n");
    body.extend_from_slice(metadata_json.as_bytes());
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(b"Content-Type: text/plain\r\n\r\n");
    body.extend_from_slice(file_content);
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());

    let content_type = format!("multipart/related; boundary=\"{boundary}\"");
    let resp = client
        .post_raw(&upload_url, body, &content_type)
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    let file: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(file["id"], "uploaded-1");
    assert_eq!(file["name"], "hello.txt");

    // 2. Download the same file
    Mock::given(method("GET"))
        .and(path("/drive/v3/files/uploaded-1"))
        .and(query_param("alt", "media"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(file_content.to_vec(), "text/plain"))
        .expect(1)
        .mount(&server)
        .await;

    let dl_url = format!("{}/drive/v3/files/uploaded-1?alt=media", server.uri());
    let dl_resp = client.get_raw(&dl_url).await.unwrap();
    assert_eq!(dl_resp.status().as_u16(), 200);
    let downloaded = dl_resp.bytes().await.unwrap();
    assert_eq!(&downloaded[..], &file_content[..]);
}

// ── Workspace document export ──────────────────────────────────────────────

#[tokio::test]
async fn export_google_doc_to_docx() {
    let server = MockServer::start().await;
    let client = DriveClient::new("export-tok");

    let docx_content = b"PK\x03\x04\x14\x00fake-docx-content";

    Mock::given(method("GET"))
        .and(path("/drive/v3/files/gdoc-1/export"))
        .and(query_param(
            "mimeType",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            docx_content.to_vec(),
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        ))
        .expect(1)
        .mount(&server)
        .await;

    let export_url = format!(
        "{}/drive/v3/files/gdoc-1/export?mimeType=application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        server.uri()
    );
    let resp = client.get_raw(&export_url).await.unwrap();
    assert_eq!(resp.status().as_u16(), 200);

    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        content_type.contains("wordprocessingml"),
        "expected docx content type"
    );

    let body = resp.bytes().await.unwrap();
    assert_eq!(&body[..], docx_content);
}

#[tokio::test]
async fn export_google_sheet_to_xlsx() {
    let server = MockServer::start().await;
    let client = DriveClient::new("sheet-export");

    Mock::given(method("GET"))
        .and(path("/drive/v3/files/gsheet-1/export"))
        .and(query_param(
            "mimeType",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            b"PK\x03\x04fake-xlsx".to_vec(),
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        ))
        .expect(1)
        .mount(&server)
        .await;

    let export_url = format!(
        "{}/drive/v3/files/gsheet-1/export?mimeType=application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        server.uri()
    );
    let resp = client.get_raw(&export_url).await.unwrap();
    assert_eq!(resp.status().as_u16(), 200);
}

// ── Resumable upload full flow ─────────────────────────────────────────────

#[tokio::test]
async fn resumable_upload_full_flow_start_chunks_query() {
    let server = MockServer::start().await;
    let client = DriveClient::new("resume-tok");

    let upload_uri = format!("{}/resume/session-abc", server.uri());

    // 1. Start resumable session
    Mock::given(method("POST"))
        .and(path("/upload/drive/v3/files"))
        .and(query_param("uploadType", "resumable"))
        .respond_with(ResponseTemplate::new(200).append_header("Location", &upload_uri))
        .expect(1)
        .mount(&server)
        .await;

    let start_url = format!(
        "{}/upload/drive/v3/files?uploadType=resumable",
        server.uri()
    );
    let metadata = serde_json::json!({
        "name": "large-file.bin",
        "mimeType": "application/octet-stream"
    });
    let metadata_json = serde_json::to_vec(&metadata).unwrap();
    let start_resp = client
        .post_raw(&start_url, metadata_json, "application/json; charset=UTF-8")
        .await
        .unwrap();
    assert_eq!(start_resp.status().as_u16(), 200);
    let location = start_resp
        .headers()
        .get("Location")
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(location, upload_uri);

    // 2. Upload first chunk (100 bytes of 300 total) → 308 Incomplete
    Mock::given(method("PUT"))
        .and(path("/resume/session-abc"))
        .and(header("Content-Range", "bytes 0-99/300"))
        .respond_with(ResponseTemplate::new(308).append_header("Range", "bytes=0-99"))
        .expect(1)
        .mount(&server)
        .await;

    let mut headers = std::collections::HashMap::new();
    headers.insert("Content-Range".into(), "bytes 0-99/300".into());
    let chunk1 = vec![0u8; 100];
    let resp1 = client
        .put_raw_no_redirect(&upload_uri, chunk1, &headers)
        .await
        .unwrap();
    assert_eq!(resp1.status().as_u16(), 308);

    // 3. Query progress
    Mock::given(method("PUT"))
        .and(path("/resume/session-abc"))
        .and(header("Content-Range", "bytes */300"))
        .respond_with(ResponseTemplate::new(308).append_header("Range", "bytes=0-99"))
        .expect(1)
        .mount(&server)
        .await;

    let mut q_headers = std::collections::HashMap::new();
    q_headers.insert("Content-Range".into(), "bytes */300".into());
    let q_resp = client
        .put_raw_no_redirect(&upload_uri, Vec::new(), &q_headers)
        .await
        .unwrap();
    assert_eq!(q_resp.status().as_u16(), 308);
    let range = q_resp.headers().get("Range").unwrap().to_str().unwrap();
    assert!(range.contains("bytes=0-99"));

    // 4. Upload final chunks → 200 Complete
    Mock::given(method("PUT"))
        .and(path("/resume/session-abc"))
        .and(header("Content-Range", "bytes 100-299/300"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            sample_file_json("large-1", "large-file.bin", "application/octet-stream"),
            "application/json",
        ))
        .expect(1)
        .mount(&server)
        .await;

    let mut final_headers = std::collections::HashMap::new();
    final_headers.insert("Content-Range".into(), "bytes 100-299/300".into());
    let chunk_final = vec![1u8; 200];
    let final_resp = client
        .put_raw_no_redirect(&upload_uri, chunk_final, &final_headers)
        .await
        .unwrap();
    assert_eq!(final_resp.status().as_u16(), 200);
    let completed: serde_json::Value = final_resp.json().await.unwrap();
    assert_eq!(completed["id"], "large-1");
    assert_eq!(completed["name"], "large-file.bin");
}

// ── Error recovery ─────────────────────────────────────────────────────────

#[tokio::test]
async fn upload_retries_on_server_error_503() {
    let server = MockServer::start().await;
    let client = DriveClient::new("retry-tok");
    let call_count = std::sync::atomic::AtomicU32::new(0);

    Mock::given(method("POST"))
        .and(path("/drive/v3/files"))
        .respond_with(move |_req: &wiremock::Request| {
            let n = call_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n < 2 {
                ResponseTemplate::new(503)
            } else {
                ResponseTemplate::new(200).set_body_raw(
                    sample_file_json("after-retry", "retried.txt", "text/plain"),
                    "application/json",
                )
            }
        })
        .mount(&server)
        .await;

    let url = format!("{}/drive/v3/files?fields=id,name", server.uri());
    let body = serde_json::json!({"name": "retried.txt", "mimeType": "text/plain"});
    let result: serde_json::Value = client.post_json(&url, &body).await.unwrap();
    assert_eq!(result["id"], "after-retry");
}
