//! Google Drive v3 Changes API wrappers.
//!
//! The Changes API is the foundation for incremental sync — callers save a
//! start-page token and periodically poll for changes since that token.

use serde::Deserialize;

use crate::client::DriveClient;
use crate::files::DriveFile;

// ─── Constants ───────────────────────────────────────────────────────────────

const DRIVE_API_BASE: &str = "https://www.googleapis.com/drive/v3";

/// Fields requested for each change resource.  We only need the file metadata
/// when the file still exists, plus change-level fields (`changeType`,
/// `removed`, `time`).
const CHANGE_FIELDS: &str =
    "changes(kind,changeType,fileId,removed,time,file(id,name,mimeType,parents,\
     size,version,modifiedTime,createdTime,trashed,md5Checksum,webViewLink)),\
     nextPageToken,newStartPageToken";

// ─── Types ───────────────────────────────────────────────────────────────────

/// A single change event in Google Drive.
///
/// When `removed` is `true` the file has been deleted or the user lost access;
/// `file` will be `None` in this case.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    /// Always `"drive#change"`.
    pub kind: Option<String>,
    /// `"file"` for file-level changes, `"drive"` for shared-drive changes.
    #[serde(rename = "type")]
    pub change_type: Option<String>,
    /// The ID of the file that changed.
    pub file_id: Option<String>,
    /// Whether the file was removed or the user lost access.
    pub removed: Option<bool>,
    /// RFC 3339 timestamp of the change.
    pub time: Option<String>,
    /// Full file resource (absent for removed files).
    pub file: Option<DriveFile>,
}

/// Response from `GET /drive/v3/changes`.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChangeListResponse {
    /// The list of changes (may be empty).
    #[serde(default)]
    pub changes: Vec<Change>,
    /// Token for the next page of changes.
    pub next_page_token: Option<String>,
    /// The latest start-page token.  Save this for the next poll cycle.
    pub new_start_page_token: Option<String>,
}

/// Response from `GET /drive/v3/changes/startPageToken`.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StartPageTokenResponse {
    /// The start-page token for the authenticated user.
    pub start_page_token: String,
}

// ─── API functions ───────────────────────────────────────────────────────────

/// Obtain the start-page token for the authenticated user (or a shared drive).
///
/// This token is the starting point for future `changes_list` calls.  Save it
/// after the initial full sync so incremental syncs can begin from here.
pub async fn changes_get_start_page_token(
    client: &DriveClient,
    drive_id: Option<&str>,
) -> anyhow::Result<String> {
    let mut url = format!("{DRIVE_API_BASE}/changes/startPageToken");
    if let Some(id) = drive_id {
        url.push('?');
        url.push_str("driveId=");
        url.push_str(&crate::files::urlencoding(id));
    }

    let resp: StartPageTokenResponse = client.get_json(&url).await?;
    Ok(resp.start_page_token)
}

/// List changes since `page_token`.
///
/// `fields` can be used to request a partial response; defaults to
/// [`CHANGE_FIELDS`] which includes full file metadata for surviving files.
///
/// Returns the next page of changes plus the latest `new_start_page_token`
/// which should replace the stored token for the next poll.
pub async fn changes_list(
    client: &DriveClient,
    page_token: &str,
    page_size: Option<u32>,
    fields: Option<&str>,
    drive_id: Option<&str>,
    include_removed: Option<bool>,
) -> anyhow::Result<ChangeListResponse> {
    let mut url = format!("{DRIVE_API_BASE}/changes");

    let mut params: Vec<(&str, String)> = Vec::new();
    params.push(("pageToken", page_token.to_string()));
    if let Some(n) = page_size {
        params.push(("pageSize", n.to_string()));
    }
    params.push(("fields", fields.unwrap_or(CHANGE_FIELDS).to_string()));
    if let Some(id) = drive_id {
        params.push(("driveId", id.to_string()));
    }
    if let Some(removed) = include_removed {
        params.push(("includeRemoved", removed.to_string()));
    }

    let query_string = params
        .iter()
        .map(|(k, v)| format!("{}={}", k, crate::files::urlencoding(v)))
        .collect::<Vec<_>>()
        .join("&");
    url.push('?');
    url.push_str(&query_string);

    client.get_json(&url).await
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn test_client(token: &str) -> DriveClient {
        DriveClient::new(token)
    }

    // ── changes_get_start_page_token ──────────────────────────────────────

    #[tokio::test]
    async fn start_page_token_returns_value() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/drive/v3/changes/startPageToken"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"{"startPageToken":"987654"}"#,
                "application/json",
            ))
            .expect(1)
            .mount(&server)
            .await;

        let url = format!("{}/drive/v3/changes/startPageToken", server.uri());
        let client = test_client("tok");
        let resp: StartPageTokenResponse = client.get_json(&url).await.unwrap();
        assert_eq!(resp.start_page_token, "987654");
    }

    #[tokio::test]
    async fn start_page_token_with_drive_id() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/drive/v3/changes/startPageToken"))
            .and(query_param("driveId", "shared-drive-1"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"{"startPageToken":"42"}"#,
                "application/json",
            ))
            .expect(1)
            .mount(&server)
            .await;

        let url = format!(
            "{}/drive/v3/changes/startPageToken?driveId=shared-drive-1",
            server.uri()
        );
        let client = test_client("tok");
        let resp: StartPageTokenResponse = client.get_json(&url).await.unwrap();
        assert_eq!(resp.start_page_token, "42");
    }

    // ── changes_list ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn changes_list_sends_required_params() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/drive/v3/changes"))
            .and(query_param("pageToken", "start-tok"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"{"changes":[],"newStartPageToken":"next-tok"}"#,
                "application/json",
            ))
            .expect(1)
            .mount(&server)
            .await;

        let url = format!(
            "{}/drive/v3/changes?pageToken=start-tok&fields={}",
            server.uri(),
            crate::files::urlencoding(CHANGE_FIELDS)
        );
        let client = test_client("tok");
        let resp: ChangeListResponse = client.get_json(&url).await.unwrap();
        assert!(resp.changes.is_empty());
        assert_eq!(resp.new_start_page_token.as_deref(), Some("next-tok"));
    }

    #[tokio::test]
    async fn changes_list_with_optional_params() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/drive/v3/changes"))
            .and(query_param("pageToken", "tok"))
            .and(query_param("pageSize", "50"))
            .and(query_param("includeRemoved", "true"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"{"changes":[],"newStartPageToken":"fresh"}"#,
                "application/json",
            ))
            .expect(1)
            .mount(&server)
            .await;

        let url = format!(
            "{}/drive/v3/changes?pageToken=tok&pageSize=50&includeRemoved=true&fields={}",
            server.uri(),
            crate::files::urlencoding(CHANGE_FIELDS)
        );
        let client = test_client("tok");
        let resp: ChangeListResponse = client.get_json(&url).await.unwrap();
        assert_eq!(resp.new_start_page_token.as_deref(), Some("fresh"));
    }

    // ── Change deserialisation ────────────────────────────────────────────

    #[test]
    fn deserialise_file_change() {
        let json = r#"{
            "kind": "drive#change",
            "type": "file",
            "fileId": "abc123",
            "removed": false,
            "time": "2026-05-01T12:00:00.000Z",
            "file": {
                "id": "abc123",
                "name": "report.pdf",
                "mimeType": "application/pdf",
                "parents": ["root"],
                "size": "1024",
                "etag": "\"etag1\"",
                "version": "5",
                "modifiedTime": "2026-05-01T12:00:00.000Z",
                "createdTime": "2026-04-01T12:00:00.000Z",
                "trashed": false,
                "md5Checksum": "abc123def456",
                "webViewLink": "https://drive.google.com/file/d/abc123"
            }
        }"#;

        let change: Change = serde_json::from_str(json).unwrap();
        assert_eq!(change.kind.as_deref(), Some("drive#change"));
        assert_eq!(change.change_type.as_deref(), Some("file"));
        assert_eq!(change.file_id.as_deref(), Some("abc123"));
        assert_eq!(change.removed, Some(false));
        assert_eq!(change.time.as_deref(), Some("2026-05-01T12:00:00.000Z"));

        let file = change.file.unwrap();
        assert_eq!(file.name.as_deref(), Some("report.pdf"));
        assert_eq!(file.size.as_deref(), Some("1024"));
    }

    #[test]
    fn deserialise_removed_file_change() {
        let json = r#"{
            "kind": "drive#change",
            "type": "file",
            "fileId": "deleted-456",
            "removed": true,
            "time": "2026-05-02T08:00:00.000Z"
        }"#;

        let change: Change = serde_json::from_str(json).unwrap();
        assert_eq!(change.file_id.as_deref(), Some("deleted-456"));
        assert_eq!(change.removed, Some(true));
        assert!(change.file.is_none(), "removed files have no file resource");
    }

    #[test]
    fn deserialise_change_without_optional_fields() {
        let json = r#"{
            "type": "file",
            "fileId": "minimal-789"
        }"#;

        let change: Change = serde_json::from_str(json).unwrap();
        assert_eq!(change.file_id.as_deref(), Some("minimal-789"));
        assert!(change.kind.is_none());
        assert!(change.removed.is_none());
        assert!(change.time.is_none());
        assert!(change.file.is_none());
    }

    #[test]
    fn deserialise_change_with_folder() {
        let json = r#"{
            "kind": "drive#change",
            "type": "file",
            "fileId": "folder-1",
            "removed": false,
            "time": "2026-05-03T10:00:00.000Z",
            "file": {
                "id": "folder-1",
                "name": "New Folder",
                "mimeType": "application/vnd.google-apps.folder",
                "parents": ["shared-folder"],
                "size": null,
                "etag": "\"folder-etag\"",
                "version": "3",
                "modifiedTime": "2026-05-03T10:00:00.000Z",
                "createdTime": "2026-05-01T00:00:00.000Z",
                "trashed": false,
                "shared": true,
                "webViewLink": "https://drive.google.com/drive/folders/folder-1"
            }
        }"#;

        let change: Change = serde_json::from_str(json).unwrap();
        let file = change.file.unwrap();
        assert_eq!(file.mime_type.as_deref(), Some("application/vnd.google-apps.folder"));
        assert_eq!(file.size, None, "folders have no size");
        assert_eq!(file.shared, Some(true));
    }

    // ── ChangeListResponse deserialisation ────────────────────────────────

    #[test]
    fn deserialise_empty_changes_list() {
        let json = r#"{
            "changes": [],
            "newStartPageToken": "tok-latest"
        }"#;

        let resp: ChangeListResponse = serde_json::from_str(json).unwrap();
        assert!(resp.changes.is_empty());
        assert_eq!(resp.new_start_page_token.as_deref(), Some("tok-latest"));
        assert!(resp.next_page_token.is_none());
    }

    #[test]
    fn deserialise_changes_list_with_pagination() {
        let json = r#"{
            "changes": [
                {
                    "kind": "drive#change",
                    "type": "file",
                    "fileId": "f1",
                    "removed": false,
                    "time": "2026-05-01T12:00:00.000Z",
                    "file": {
                        "id": "f1",
                        "name": "doc.txt",
                        "mimeType": "text/plain",
                        "size": "100"
                    }
                },
                {
                    "kind": "drive#change",
                    "type": "file",
                    "fileId": "f2",
                    "removed": true,
                    "time": "2026-05-01T13:00:00.000Z"
                }
            ],
            "nextPageToken": "page-2",
            "newStartPageToken": "latest-tok"
        }"#;

        let resp: ChangeListResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.changes.len(), 2);
        assert_eq!(resp.changes[0].file_id.as_deref(), Some("f1"));
        assert!(resp.changes[0].file.is_some());
        assert_eq!(resp.changes[1].file_id.as_deref(), Some("f2"));
        assert_eq!(resp.changes[1].removed, Some(true));
        assert!(resp.changes[1].file.is_none());
        assert_eq!(resp.next_page_token.as_deref(), Some("page-2"));
        assert_eq!(resp.new_start_page_token.as_deref(), Some("latest-tok"));
    }

    // ── StartPageTokenResponse deserialisation ────────────────────────────

    #[test]
    fn deserialise_start_page_token() {
        let json = r#"{"startPageToken":"12345678"}"#;
        let resp: StartPageTokenResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.start_page_token, "12345678");
    }
}
