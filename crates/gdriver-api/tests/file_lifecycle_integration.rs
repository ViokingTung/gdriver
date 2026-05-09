//! Integration tests: full file lifecycle through the DriveClient + wiremock.
//!
//! Simulates the daemon's API usage pattern:
//!   about_get → files_list → files_create → files_get → files_update → files_delete

use std::sync::Arc;
use std::sync::Mutex;

use gdriver_api::client::{DriveClient, TokenRefresher};
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ── Helpers ────────────────────────────────────────────────────────────────

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
            "shared": false,
            "md5Checksum": "d41d8cd98f00b204e9800998ecf8427e",
            "webViewLink": "https://drive.google.com/file/d/{id}"
        }}"#
    )
}

fn about_json() -> &'static str {
    r#"{
        "user": {
            "permissionId": "12345",
            "displayName": "Test User",
            "emailAddress": "test@gmail.com",
            "photoLink": "https://example.com/photo.jpg",
            "locale": "en-US"
        },
        "storageQuota": {
            "limit": "16106127360",
            "usage": "5368709120",
            "usageInDrive": "4294967296",
            "usageInDriveTrash": "1073741824"
        }
    }"#
}

// ── Complete lifecycle ─────────────────────────────────────────────────────

#[tokio::test]
async fn full_file_lifecycle_about_list_create_get_update_delete() {
    let server = MockServer::start().await;
    let client = DriveClient::new("lifecycle-token");

    // 1. about_get — user info + storage quota
    Mock::given(method("GET"))
        .and(path("/drive/v3/about"))
        .and(query_param("fields", "user,storageQuota"))
        .and(header("Authorization", "Bearer lifecycle-token"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(about_json(), "application/json"))
        .expect(1)
        .mount(&server)
        .await;

    let about_url = format!("{}/drive/v3/about?fields=user,storageQuota", server.uri());
    let about: serde_json::Value = client.get_json(&about_url).await.unwrap();
    assert_eq!(about["user"]["displayName"], "Test User");
    assert_eq!(about["user"]["emailAddress"], "test@gmail.com");

    // 2. files_list — initial file listing
    Mock::given(method("GET"))
        .and(path("/drive/v3/files"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(
                format!(
                    r#"{{"files":[{},{}],"nextPageToken":null}}"#,
                    sample_file_json("f1", "alpha.txt", "text/plain"),
                    sample_file_json("f2", "beta.pdf", "application/pdf"),
                ),
                "application/json",
            ),
        )
        .expect(1)
        .mount(&server)
        .await;

    let list_url = format!("{}/drive/v3/files?fields=files(id,name,mimeType,parents,size,version,modifiedTime,createdTime,trashed,shared,md5Checksum,webViewLink),nextPageToken,incompleteSearch", server.uri());
    let list: serde_json::Value = client.get_json(&list_url).await.unwrap();
    assert_eq!(list["files"].as_array().unwrap().len(), 2);
    assert_eq!(list["files"][0]["name"], "alpha.txt");

    // 3. files_create — create a new folder
    Mock::given(method("POST"))
        .and(path("/drive/v3/files"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(
                sample_file_json("new-folder", "My Folder", "application/vnd.google-apps.folder"),
                "application/json",
            ),
        )
        .expect(1)
        .mount(&server)
        .await;

    let create_url = format!("{}/drive/v3/files?fields=id,name,mimeType,parents,size,version,modifiedTime,createdTime,trashed,shared,md5Checksum,webViewLink", server.uri());
    let body = serde_json::json!({"name": "My Folder", "mimeType": "application/vnd.google-apps.folder"});
    let created: serde_json::Value = client.post_json(&create_url, &body).await.unwrap();
    assert_eq!(created["id"], "new-folder");
    assert_eq!(created["name"], "My Folder");

    // 4. files_get — read back the created file
    Mock::given(method("GET"))
        .and(path("/drive/v3/files/new-folder"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(
                sample_file_json("new-folder", "My Folder", "application/vnd.google-apps.folder"),
                "application/json",
            ),
        )
        .expect(1)
        .mount(&server)
        .await;

    let get_url = format!("{}/drive/v3/files/new-folder?fields=id,name,mimeType,parents,size,version,modifiedTime,createdTime,trashed,shared,md5Checksum,webViewLink", server.uri());
    let got: serde_json::Value = client.get_json(&get_url).await.unwrap();
    assert_eq!(got["id"], "new-folder");

    // 5. files_update — rename the folder
    Mock::given(method("PATCH"))
        .and(path("/drive/v3/files/new-folder"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(
                sample_file_json("new-folder", "Renamed Folder", "application/vnd.google-apps.folder"),
                "application/json",
            ),
        )
        .expect(1)
        .mount(&server)
        .await;

    let update_url = format!("{}/drive/v3/files/new-folder?fields=id,name,mimeType,parents,size,version,modifiedTime,createdTime,trashed,shared,md5Checksum,webViewLink", server.uri());
    let update_body = serde_json::json!({"name": "Renamed Folder"});
    let updated: serde_json::Value = client.patch_json(&update_url, &update_body).await.unwrap();
    assert_eq!(updated["name"], "Renamed Folder");

    // 6. files_delete — trash the folder
    Mock::given(method("DELETE"))
        .and(path("/drive/v3/files/new-folder"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    let delete_url = format!("{}/drive/v3/files/new-folder", server.uri());
    client.delete_empty(&delete_url).await.unwrap();
}

// ── Pagination ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn files_list_pagination_with_multiple_pages() {
    let server = MockServer::start().await;
    let client = DriveClient::new("page-token");

    // Page 1: 2 files + nextPageToken
    Mock::given(method("GET"))
        .and(path("/drive/v3/files"))
        .and(query_param("pageToken", "page1"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(
                format!(
                    r#"{{"files":[{},{}],"nextPageToken":"page2"}}"#,
                    sample_file_json("a", "a.txt", "text/plain"),
                    sample_file_json("b", "b.txt", "text/plain"),
                ),
                "application/json",
            ),
        )
        .expect(1)
        .mount(&server)
        .await;

    // Page 2: 1 file, no nextPageToken
    Mock::given(method("GET"))
        .and(path("/drive/v3/files"))
        .and(query_param("pageToken", "page2"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(
                format!(
                    r#"{{"files":[{}],"nextPageToken":null}}"#,
                    sample_file_json("c", "c.txt", "text/plain"),
                ),
                "application/json",
            ),
        )
        .expect(1)
        .mount(&server)
        .await;

    let default_fields = "files(id,name,mimeType,parents,size,version,modifiedTime,createdTime,trashed,shared,md5Checksum,webViewLink),nextPageToken,incompleteSearch";

    let page1_url = format!("{}/drive/v3/files?pageToken=page1&fields={}", server.uri(), default_fields);
    let page1: serde_json::Value = client.get_json(&page1_url).await.unwrap();
    assert_eq!(page1["files"].as_array().unwrap().len(), 2);
    assert_eq!(page1["nextPageToken"], "page2");

    let page2_url = format!("{}/drive/v3/files?pageToken=page2&fields={}", server.uri(), default_fields);
    let page2: serde_json::Value = client.get_json(&page2_url).await.unwrap();
    assert_eq!(page2["files"].as_array().unwrap().len(), 1);
    assert!(page2["nextPageToken"].is_null());
}

// ── Error scenarios ────────────────────────────────────────────────────────

#[tokio::test]
async fn file_not_found_returns_404() {
    let server = MockServer::start().await;
    let client = DriveClient::new("err-tok");

    Mock::given(method("GET"))
        .and(path("/drive/v3/files/missing-id"))
        .respond_with(ResponseTemplate::new(404).set_body_raw(
            r#"{"error":{"code":404,"message":"File not found"}}"#,
            "application/json",
        ))
        .expect(1)
        .mount(&server)
        .await;

    let url = format!("{}/drive/v3/files/missing-id?fields=id,name", server.uri());
    let result: Result<serde_json::Value, _> = client.get_json(&url).await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("HTTP 404"), "error should mention 404: {err}");
}

#[tokio::test]
async fn create_fails_with_403_forbidden() {
    let server = MockServer::start().await;
    let client = DriveClient::new("forbidden-tok");

    Mock::given(method("POST"))
        .and(path("/drive/v3/files"))
        .respond_with(ResponseTemplate::new(403).set_body_raw(
            r#"{"error":{"code":403,"message":"Insufficient permissions"}}"#,
            "application/json",
        ))
        .expect(1)
        .mount(&server)
        .await;

    let url = format!("{}/drive/v3/files?fields=id,name", server.uri());
    let body = serde_json::json!({"name": "test", "mimeType": "text/plain"});
    let result: Result<serde_json::Value, _> = client.post_json(&url, &body).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("HTTP 403"));
}

// ── Token refresh during lifecycle ─────────────────────────────────────────

struct TestRefresher {
    token: String,
    count: Mutex<u32>,
}

impl TestRefresher {
    fn new(token: &str) -> Self {
        Self { token: token.into(), count: Mutex::new(0) }
    }
}

#[async_trait::async_trait]
impl TokenRefresher for TestRefresher {
    async fn refresh(&self) -> anyhow::Result<String> {
        *self.count.lock().unwrap() += 1;
        Ok(self.token.clone())
    }
}

#[tokio::test]
async fn token_refresh_during_file_get_then_succeeds() {
    let server = MockServer::start().await;
    let refresher = Arc::new(TestRefresher::new("fresh-token"));

    // First attempt: 401
    Mock::given(method("GET"))
        .and(path("/drive/v3/files/protected"))
        .and(header("Authorization", "Bearer expired"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;

    // Second attempt: 200 with fresh token
    Mock::given(method("GET"))
        .and(path("/drive/v3/files/protected"))
        .and(header("Authorization", "Bearer fresh-token"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(
                sample_file_json("protected", "secret.txt", "text/plain"),
                "application/json",
            ),
        )
        .mount(&server)
        .await;

    let client = DriveClient::new("expired").with_refresher(refresher.clone());
    let url = format!("{}/drive/v3/files/protected?fields=id,name", server.uri());
    let file: serde_json::Value = client.get_json(&url).await.unwrap();
    assert_eq!(file["id"], "protected");
    assert_eq!(*refresher.count.lock().unwrap(), 1);
}

#[tokio::test]
async fn token_refresh_retried_on_429_during_list() {
    let server = MockServer::start().await;
    let call_count = std::sync::atomic::AtomicU32::new(0);

    Mock::given(method("GET"))
        .and(path("/drive/v3/files"))
        .respond_with(move |_req: &wiremock::Request| {
            let n = call_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n < 2 {
                ResponseTemplate::new(429)
            } else {
                ResponseTemplate::new(200).set_body_raw(r#"{"files":[],"nextPageToken":null}"#, "application/json")
            }
        })
        .mount(&server)
        .await;

    let client = DriveClient::new("rate-limited");
    let url = format!("{}/drive/v3/files?fields=files(id,name),nextPageToken", server.uri());
    let result: Result<serde_json::Value, _> = client.get_json(&url).await;
    assert!(result.is_ok());
}
