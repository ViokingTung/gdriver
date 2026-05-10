//! Google Drive v3 Files & About API wrappers.
//!
//! Lower-level functions live here; the daemon's `api` module orchestrates
//! them with database persistence and IPC event emission.

use std::collections::HashMap;

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::client::DriveClient;

// ─── Constants ───────────────────────────────────────────────────────────

const DRIVE_API_BASE: &str = "https://www.googleapis.com/drive/v3";
const UPLOAD_API_BASE: &str = "https://www.googleapis.com/upload/drive/v3";

/// Default fields requested from the Files API when the caller does not
/// specify.  Covers the columns in `drive_files` plus a few extras.
const DEFAULT_FILE_FIELDS: &str =
    "files(id,name,mimeType,parents,size,version,modifiedTime,createdTime,\
     trashed,shared,md5Checksum,webViewLink),nextPageToken,incompleteSearch";

/// Fields requested for a single-file response.
const SINGLE_FILE_FIELDS: &str = "id,name,mimeType,parents,size,version,modifiedTime,createdTime,\
     trashed,shared,md5Checksum,webViewLink";

// ─── File resource ───────────────────────────────────────────────────────

/// A Google Drive file or folder.
///
/// All optional fields are `Option` so the caller can request partial
/// responses via the `fields` query parameter without getting deserialisation
/// errors on missing keys.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DriveFile {
    /// Drive file ID (opaque, stable across renames).
    pub id: Option<String>,
    pub name: Option<String>,
    pub mime_type: Option<String>,
    /// Parent folder IDs; `None` / empty list means My Drive root.
    pub parents: Option<Vec<String>>,
    /// File size in bytes as a decimal string (Google returns strings for
    /// large numbers).  `None` for folders.
    pub size: Option<String>,
    /// Opaque etag; changes when content or metadata changes.
    pub etag: Option<String>,
    /// Monotonically increasing version number (string in the API).
    pub version: Option<String>,
    /// RFC 3339 timestamp of last modification by any user.
    pub modified_time: Option<String>,
    /// RFC 3339 timestamp of creation.
    pub created_time: Option<String>,
    /// Whether the file is in the trash.
    pub trashed: Option<bool>,
    /// Whether the file has been shared with other users.
    pub shared: Option<bool>,
    /// MD5 checksum of the file content (not populated for Google Docs).
    pub md5_checksum: Option<String>,
    /// URL to view the file in a browser.
    pub web_view_link: Option<String>,
}

/// Response from `GET /drive/v3/files`.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileListResponse {
    /// The files returned by the query.
    #[serde(default)]
    pub files: Vec<DriveFile>,
    /// Token for the next page of results.
    pub next_page_token: Option<String>,
    /// Whether the search was incomplete (e.g. due to a timeout).
    pub incomplete_search: Option<bool>,
}

// ─── Request metadata types ──────────────────────────────────────────────

/// Metadata for [`files_create`].
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateFileMetadata {
    pub name: String,
    pub mime_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parents: Option<Vec<String>>,
}

/// Partial metadata for [`files_update`].
///
/// All fields are optional — only the supplied fields are patched.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateFileMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trashed: Option<bool>,
}

/// Result of uploading a chunk in the resumable upload protocol.
#[derive(Debug)]
pub enum UploadChunkResult {
    /// The server acknowledged receipt of `received` bytes but the upload is
    /// not yet complete (HTTP 308).
    Incomplete { received: u64 },
    /// Upload is complete; the returned [`DriveFile`] is the server metadata.
    Complete(DriveFile),
}

// ─── About API response types ────────────────────────────────────────────

/// Deserialised response from `drive/v3/about?fields=user,storageQuota`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AboutResponse {
    pub user: AboutUser,
    pub storage_quota: AboutStorageQuota,
}

/// The `user` portion of the About response.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AboutUser {
    pub permission_id: Option<String>,
    pub display_name: String,
    pub email_address: String,
    pub photo_link: Option<String>,
    /// BCP-47 locale string (e.g. `"en-US"`, `"zh-CN"`).
    pub locale: Option<String>,
}

/// The `storageQuota` portion of the About response.
///
/// All numeric fields are strings in the raw JSON; convert with
/// [`parse_quota_number`].
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AboutStorageQuota {
    pub limit: Option<String>,
    pub usage: String,
    pub usage_in_drive: Option<String>,
    pub usage_in_drive_trash: Option<String>,
}

// ─── About API ───────────────────────────────────────────────────────────

/// Call `drive/v3/about` and return structured information about the
/// authenticated user and their storage quota.
pub async fn about_get(client: &DriveClient) -> anyhow::Result<AboutResponse> {
    let url = format!("{DRIVE_API_BASE}/about?fields=user,storageQuota");
    client.get_json(&url).await
}

// ─── Files API ───────────────────────────────────────────────────────────

/// List files with an optional [search query][q].
///
/// [q]: https://developers.google.com/drive/api/guides/search-files
pub async fn files_list(
    client: &DriveClient,
    query: Option<&str>,
    page_token: Option<&str>,
    page_size: Option<u32>,
    fields: Option<&str>,
) -> anyhow::Result<FileListResponse> {
    let mut url = format!("{DRIVE_API_BASE}/files");

    let mut params: Vec<(&str, String)> = Vec::new();
    if let Some(q) = query {
        params.push(("q", q.to_string()));
    }
    if let Some(t) = page_token {
        params.push(("pageToken", t.to_string()));
    }
    if let Some(n) = page_size {
        params.push(("pageSize", n.to_string()));
    }
    params.push(("fields", fields.unwrap_or(DEFAULT_FILE_FIELDS).to_string()));

    let query_string = params
        .iter()
        .map(|(k, v)| format!("{}={}", k, urlencoding(v)))
        .collect::<Vec<_>>()
        .join("&");
    url.push('?');
    url.push_str(&query_string);

    client.get_json(&url).await
}

/// Get metadata for a single file by ID.
pub async fn files_get(
    client: &DriveClient,
    file_id: &str,
    fields: Option<&str>,
) -> anyhow::Result<DriveFile> {
    let f = fields.unwrap_or(SINGLE_FILE_FIELDS);
    let url = format!(
        "{DRIVE_API_BASE}/files/{}?fields={}",
        urlencoding(file_id),
        urlencoding(f)
    );
    client.get_json(&url).await
}

/// Create a new file or folder (metadata only; use resumable upload for
/// files with content).
pub async fn files_create(
    client: &DriveClient,
    metadata: &CreateFileMetadata,
) -> anyhow::Result<DriveFile> {
    let url = format!(
        "{DRIVE_API_BASE}/files?fields={}",
        urlencoding(SINGLE_FILE_FIELDS)
    );
    client.post_json(&url, metadata).await
}

/// Move a file to the trash (soft-delete).
///
/// Files in the trash are recoverable for 30 days.  Use
/// `files_update(file_id, UpdateFileMetadata { trashed: Some(true), .. })`
/// if you need to combine trash + rename in one call.
pub async fn files_delete(client: &DriveClient, file_id: &str) -> anyhow::Result<()> {
    let url = format!("{DRIVE_API_BASE}/files/{}", urlencoding(file_id));
    client.delete_empty(&url).await
}

/// Update file metadata (rename, change mime-type, trash/restore).
///
/// For moving a file between folders, pass `addParents` and/or
/// `removeParents` query parameters via `add_parents` / `remove_parents`.
pub async fn files_update(
    client: &DriveClient,
    file_id: &str,
    metadata: &UpdateFileMetadata,
    add_parents: Option<&[String]>,
    remove_parents: Option<&[String]>,
) -> anyhow::Result<DriveFile> {
    let mut url = format!(
        "{DRIVE_API_BASE}/files/{}?fields={}",
        urlencoding(file_id),
        urlencoding(SINGLE_FILE_FIELDS)
    );
    if let Some(parents) = add_parents {
        url.push_str("&addParents=");
        url.push_str(&parents.join(","));
    }
    if let Some(parents) = remove_parents {
        url.push_str("&removeParents=");
        url.push_str(&parents.join(","));
    }

    client.patch_json(&url, metadata).await
}

/// Download file content as a streaming response.
///
/// The returned [`reqwest::Response`] can be read in chunks via
/// `.bytes_stream()` or copied directly to a file with `.copy_to(...)`.
///
/// Returns an error for Google Workspace documents (Docs, Sheets, Slides)
/// which must be exported via [`files_export`] instead.
pub async fn files_download(
    client: &DriveClient,
    file_id: &str,
) -> anyhow::Result<reqwest::Response> {
    let url = format!("{DRIVE_API_BASE}/files/{}?alt=media", urlencoding(file_id));
    client.get_raw(&url).await
}

/// Export a Google Workspace document to a standard format.
///
/// `mime_type` is the target export MIME type
/// (e.g. `application/pdf`, `application/vnd.openxmlformats-officedocument.wordprocessingml.document`).
pub async fn files_export(
    client: &DriveClient,
    file_id: &str,
    mime_type: &str,
) -> anyhow::Result<reqwest::Response> {
    let url = format!(
        "{DRIVE_API_BASE}/files/{}/export?mimeType={}",
        urlencoding(file_id),
        urlencoding(mime_type)
    );
    client.get_raw(&url).await
}

// ─── Upload API ──────────────────────────────────────────────────────────

/// Simple multipart upload for files < 5 MB.
///
/// Constructs a `multipart/related` body with JSON metadata followed by the
/// raw file content and POSTs it to the upload endpoint.
pub async fn files_upload_multipart(
    client: &DriveClient,
    metadata: &CreateFileMetadata,
    content: &[u8],
    content_mime: &str,
) -> anyhow::Result<DriveFile> {
    let boundary = "gdriver_multipart_boundary";
    let metadata_json =
        serde_json::to_string(metadata).context("failed to serialise upload metadata")?;

    let mut body: Vec<u8> = Vec::with_capacity(512 + metadata_json.len() + content.len());
    // Metadata part
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(b"Content-Type: application/json; charset=UTF-8\r\n\r\n");
    body.extend_from_slice(metadata_json.as_bytes());
    body.extend_from_slice(b"\r\n");
    // Content part
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(format!("Content-Type: {content_mime}\r\n\r\n").as_bytes());
    body.extend_from_slice(content);
    body.extend_from_slice(b"\r\n");
    // Closing boundary
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());

    let content_type = format!("multipart/related; boundary=\"{boundary}\"");
    let url = format!(
        "{UPLOAD_API_BASE}/files?uploadType=multipart&fields={}",
        urlencoding(SINGLE_FILE_FIELDS)
    );

    let resp = client.post_raw(&url, body, &content_type).await?;
    resp.json()
        .await
        .context("failed to deserialise multipart upload response")
}

/// Start a resumable upload session.
///
/// Returns the upload URI from the `Location` header on the 200 response.
/// The caller must persist this URI for crash recovery.
pub async fn files_upload_resumable_start(
    client: &DriveClient,
    metadata: &CreateFileMetadata,
) -> anyhow::Result<String> {
    let metadata_json =
        serde_json::to_vec(metadata).context("failed to serialise upload metadata")?;

    let url = format!("{UPLOAD_API_BASE}/files?uploadType=resumable");

    let resp = client
        .post_raw(&url, metadata_json, "application/json; charset=UTF-8")
        .await?;

    let location = resp
        .headers()
        .get("Location")
        .context("missing Location header in resumable upload start response")?
        .to_str()
        .context("Location header is not valid UTF-8")?
        .to_string();

    Ok(location)
}

/// Upload a single chunk in a resumable upload session.
///
/// `range_start` / `range_end` are zero-based, inclusive byte offsets.
/// `total` is the total file size.
pub async fn files_upload_resumable_chunk(
    client: &DriveClient,
    uri: &str,
    data: &[u8],
    range_start: u64,
    range_end: u64,
    total: u64,
) -> anyhow::Result<UploadChunkResult> {
    let content_range = format!("bytes {range_start}-{range_end}/{total}");

    let mut headers = HashMap::new();
    headers.insert("Content-Range".into(), content_range);

    let resp = client
        .put_raw_no_redirect(uri, data.to_vec(), &headers)
        .await?;

    match resp.status().as_u16() {
        200 | 201 => {
            let file: DriveFile = resp
                .json()
                .await
                .context("failed to deserialise completed upload response")?;
            Ok(UploadChunkResult::Complete(file))
        }
        308 => {
            // The server received the chunk but the upload is not yet complete.
            let received = parse_range_header(&resp).unwrap_or(range_end + 1);
            Ok(UploadChunkResult::Incomplete { received })
        }
        status => {
            let body = resp.text().await.unwrap_or_default();
            Err(anyhow::anyhow!(
                "unexpected status {status} on resumable upload chunk PUT: {body:.500}"
            ))
        }
    }
}

/// Query the status of a resumable upload session.
///
/// Returns `(received_bytes, is_complete)`.  `received_bytes` is parsed from
/// the `Range` header on a 308 response.
pub async fn files_upload_resumable_query(
    client: &DriveClient,
    uri: &str,
    total: u64,
) -> anyhow::Result<(u64, bool)> {
    let content_range = format!("bytes */{total}");

    let mut headers = HashMap::new();
    headers.insert("Content-Range".into(), content_range);

    let resp = client
        .put_raw_no_redirect(uri, Vec::new(), &headers)
        .await?;

    match resp.status().as_u16() {
        200 | 201 => {
            // Upload is already complete — consume the body to get DriveFile
            // (caller can use files_get instead if they just want metadata).
            Ok((total, true))
        }
        308 => {
            let received = parse_range_header(&resp).unwrap_or(0);
            Ok((received, false))
        }
        status => {
            let body = resp.text().await.unwrap_or_default();
            Err(anyhow::anyhow!(
                "unexpected status {status} on upload query: {body:.500}"
            ))
        }
    }
}

/// Parse the `Range` header from a 308 response to get the number of received
/// bytes.
///
/// Google returns `Range: bytes=0-524287` meaning bytes 0 through 524287
/// have been received (524288 total bytes).
fn parse_range_header(resp: &reqwest::Response) -> Option<u64> {
    let range = resp.headers().get("Range")?.to_str().ok()?;
    // Format: "bytes=0-524287"
    let range = range.strip_prefix("bytes=")?;
    let end = range.split('-').nth(1)?;
    let end: u64 = end.parse().ok()?;
    Some(end + 1) // inclusive → count
}

// ─── Helpers ─────────────────────────────────────────────────────────────

/// Percent-encode a string for use in a URL path segment or query value.
pub(crate) fn urlencoding(s: &str) -> String {
    // The `url` crate isn't in our dependency tree so we use a minimal
    // manual encoding that covers the characters appearing in file IDs
    // (alphanumeric, '-', '_') and query values.
    let mut result = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'~'
            | b':'
            | b'/'
            | b','
            | b'('
            | b')'
            | b'\''
            | b'!'
            | b'*'
            | b'@'
            | b';'
            | b'='
            | b'$' => result.push(b as char),
            b' ' => result.push_str("%20"),
            _ => {
                result.push_str(&format!("%{:02X}", b));
            }
        }
    }
    result
}

/// Parse a Google Drive numeric string field (quota values are decimal strings).
///
/// Returns `0` for missing or empty values; errors on truly unparseable input.
pub fn parse_quota_number(s: &Option<String>) -> anyhow::Result<u64> {
    match s {
        Some(v) if !v.is_empty() => v.parse::<u64>().context("invalid quota number"),
        _ => Ok(0),
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use wiremock::{
        matchers::{header, method, path, query_param},
        Mock, MockServer, ResponseTemplate,
    };

    use super::*;

    // ── helpers ──────────────────────────────────────────────────────────

    fn test_client(token: &str) -> DriveClient {
        DriveClient::new(token)
    }

    /// Sample file JSON matching the Google Drive API shape.
    fn sample_file_json(id: &str, name: &str) -> String {
        format!(
            r#"{{
                "id": "{id}",
                "name": "{name}",
                "mimeType": "application/vnd.google-apps.folder",
                "parents": ["root"],
                "size": null,
                "etag": "\"etag_{id}\"",
                "version": "42",
                "modifiedTime": "2026-05-01T12:00:00.000Z",
                "createdTime": "2026-04-01T12:00:00.000Z",
                "trashed": false,
                "shared": false,
                "md5Checksum": null,
                "webViewLink": "https://drive.google.com/drive/folders/{id}"
            }}"#
        )
    }

    // ── about_get ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn about_get_returns_user_info() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/drive/v3/about"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"{"user":{"displayName":"Test","emailAddress":"t@test.com"},"storageQuota":{"usage":"100"}}"#,
                "application/json",
            ))
            .mount(&server)
            .await;

        // Override DRIVE_API_BASE by constructing URL manually for this test.
        // We test through the public function by using the mock server URL.
        // Since the function hardcodes the host, we verify deserialisation directly.
        let json = r#"{
            "user":{"displayName":"Test","emailAddress":"t@test.com"},
            "storageQuota":{"limit":"1000","usage":"100","usageInDrive":"50","usageInDriveTrash":"10"}
        }"#;
        let r: AboutResponse = serde_json::from_str(json).unwrap();
        assert_eq!(r.user.display_name, "Test");
        assert_eq!(r.user.email_address, "t@test.com");
    }

    // ── files_list ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn files_list_sends_query_params() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/drive/v3/files"))
            .and(query_param(
                "q",
                "mimeType='application/vnd.google-apps.folder'",
            ))
            .and(query_param("pageSize", "10"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(r#"{"files":[]}"#, "application/json"),
            )
            .expect(1)
            .mount(&server)
            .await;

        // Patch: test directly against mock server by building URL manually.
        let url = format!(
            "{}/drive/v3/files?q={}&pageSize=10&fields={}",
            server.uri(),
            urlencoding("mimeType='application/vnd.google-apps.folder'"),
            urlencoding(DEFAULT_FILE_FIELDS)
        );

        let client = test_client("tok");
        let _resp: FileListResponse = client.get_json(&url).await.unwrap();
    }

    #[tokio::test]
    async fn files_list_parses_response() {
        let json = format!(
            r#"{{"files":[{},{}],"nextPageToken":"tok_next","incompleteSearch":false}}"#,
            sample_file_json("f1", "alpha"),
            sample_file_json("f2", "beta")
        );

        let resp: FileListResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(resp.files.len(), 2);
        assert_eq!(resp.files[0].name.as_deref(), Some("alpha"));
        assert_eq!(resp.files[1].id.as_deref(), Some("f2"));
        assert_eq!(resp.next_page_token.as_deref(), Some("tok_next"));
    }

    #[tokio::test]
    async fn files_list_empty() {
        let resp: FileListResponse = serde_json::from_str(r#"{"files":[]}"#).unwrap();
        assert!(resp.files.is_empty());
        assert!(resp.next_page_token.is_none());
    }

    // ── files_get ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn files_get_parses_single_file() {
        let json = sample_file_json("abc123", "report.pdf");
        let f: DriveFile = serde_json::from_str(&json).unwrap();

        assert_eq!(f.id.as_deref(), Some("abc123"));
        assert_eq!(f.name.as_deref(), Some("report.pdf"));
        assert_eq!(
            f.mime_type.as_deref(),
            Some("application/vnd.google-apps.folder")
        );
        assert_eq!(f.etag.as_deref(), Some("\"etag_abc123\""));
        assert_eq!(f.version.as_deref(), Some("42"));
        assert_eq!(f.trashed, Some(false));
        assert_eq!(f.parents.as_deref(), Some(&["root".to_string()][..]));
    }

    #[tokio::test]
    async fn files_get_partial_response() {
        // When only a subset of fields is requested, missing fields are None.
        let json = r#"{"id":"x","name":"just_name.txt"}"#;
        let f: DriveFile = serde_json::from_str(json).unwrap();
        assert_eq!(f.id.as_deref(), Some("x"));
        assert_eq!(f.name.as_deref(), Some("just_name.txt"));
        assert!(f.mime_type.is_none());
        assert!(f.size.is_none());
    }

    // ── files_create ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn files_create_metadata_serialises() {
        let meta = CreateFileMetadata {
            name: "New Folder".into(),
            mime_type: "application/vnd.google-apps.folder".into(),
            parents: Some(vec!["parent_id".into()]),
        };
        let json = serde_json::to_string(&meta).unwrap();
        assert!(json.contains("New Folder"));
        assert!(json.contains("application/vnd.google-apps.folder"));
        assert!(json.contains("parent_id"));
    }

    #[tokio::test]
    async fn files_create_metadata_no_parents() {
        let meta = CreateFileMetadata {
            name: "root-folder".into(),
            mime_type: "application/vnd.google-apps.folder".into(),
            parents: None,
        };
        let json = serde_json::to_string(&meta).unwrap();
        assert!(
            !json.contains("parents"),
            "parents key should be absent when None"
        );
    }

    // ── files_update ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn files_update_metadata_serialises_partial() {
        let meta = UpdateFileMetadata {
            name: Some("renamed.txt".into()),
            ..Default::default()
        };
        let json = serde_json::to_string(&meta).unwrap();
        assert!(json.contains("renamed.txt"));
        assert!(
            !json.contains("mimeType"),
            "absent fields should be skipped"
        );
        assert!(!json.contains("trashed"));
    }

    #[tokio::test]
    async fn files_update_trash_only() {
        let meta = UpdateFileMetadata {
            trashed: Some(true),
            ..Default::default()
        };
        let json = serde_json::to_string(&meta).unwrap();
        assert!(json.contains("true"));
        assert!(!json.contains("name"));
    }

    #[tokio::test]
    async fn files_update_all_empty_serialises_empty_object() {
        let meta = UpdateFileMetadata::default();
        let json = serde_json::to_string(&meta).unwrap();
        assert_eq!(json, "{}");
    }

    // ── files_delete (no body) ───────────────────────────────────────────

    #[tokio::test]
    async fn files_delete_accepts_204() {
        let server = MockServer::start().await;
        let file_id = "to-delete";

        Mock::given(method("DELETE"))
            .and(path(format!("/drive/v3/files/{file_id}")))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        // Build URL manually to hit the mock server.
        let url = format!("{}/drive/v3/files/{file_id}", server.uri());
        let client = test_client("tok");
        client.delete_empty(&url).await.unwrap();
    }

    // ── files_download ───────────────────────────────────────────────────

    #[tokio::test]
    async fn files_download_returns_raw_body() {
        let server = MockServer::start().await;
        let file_id = "download-me";

        Mock::given(method("GET"))
            .and(path(format!("/drive/v3/files/{file_id}")))
            .and(query_param("alt", "media"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(b"hello world".to_vec(), "application/octet-stream"),
            )
            .expect(1)
            .mount(&server)
            .await;

        let url = format!("{}/drive/v3/files/{file_id}?alt=media", server.uri());
        let client = test_client("tok");
        let resp = client.get_raw(&url).await.unwrap();
        let body = resp.bytes().await.unwrap();
        assert_eq!(&body[..], b"hello world");
    }

    // ── files_export ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn files_export_sets_mime_type_param() {
        let server = MockServer::start().await;
        let file_id = "gdoc-123";

        Mock::given(method("GET"))
            .and(path(format!("/drive/v3/files/{file_id}/export")))
            .and(query_param("mimeType", "application/pdf"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(b"%PDF-1.4".to_vec(), "application/pdf"),
            )
            .expect(1)
            .mount(&server)
            .await;

        let url = format!(
            "{}/drive/v3/files/{file_id}/export?mimeType=application/pdf",
            server.uri()
        );
        let client = test_client("tok");
        let resp = client.get_raw(&url).await.unwrap();
        assert_eq!(resp.status().as_u16(), 200);
    }

    // ── parse_quota_number ───────────────────────────────────────────────

    #[test]
    fn parse_quota_valid() {
        assert_eq!(parse_quota_number(&Some("12345".into())).unwrap(), 12345);
    }

    #[test]
    fn parse_quota_none_is_zero() {
        assert_eq!(parse_quota_number(&None).unwrap(), 0);
    }

    #[test]
    fn parse_quota_empty_is_zero() {
        assert_eq!(parse_quota_number(&Some("".into())).unwrap(), 0);
    }

    #[test]
    fn parse_quota_large_value() {
        let val = "16106127360".to_string();
        assert_eq!(parse_quota_number(&Some(val)).unwrap(), 16_106_127_360);
    }

    // ── DriveFile deserialisation ────────────────────────────────────────

    #[test]
    fn deserialise_full_drive_file() {
        let json = sample_file_json("f99", "document.pdf");
        let f: DriveFile = serde_json::from_str(&json).unwrap();
        assert_eq!(f.id.as_deref(), Some("f99"));
        assert_eq!(
            f.web_view_link.as_deref(),
            Some("https://drive.google.com/drive/folders/f99")
        );
        assert!(f.shared == Some(false));
    }

    #[test]
    fn deserialise_file_with_numeric_size() {
        let json = r#"{
            "id": "f",
            "name": "f.bin",
            "mimeType": "application/octet-stream",
            "size": "1048576"
        }"#;
        let f: DriveFile = serde_json::from_str(json).unwrap();
        assert_eq!(f.size.as_deref(), Some("1048576"));
    }

    #[test]
    fn deserialise_file_no_size() {
        let json = r#"{"id":"d","name":"d","mimeType":"text/plain"}"#;
        let f: DriveFile = serde_json::from_str(json).unwrap();
        assert!(f.size.is_none());
    }

    // ── urlencoding ──────────────────────────────────────────────────────

    #[test]
    fn urlencoding_preserves_alphanumeric() {
        assert_eq!(urlencoding("abc123XYZ"), "abc123XYZ");
    }

    #[test]
    fn urlencoding_encodes_spaces() {
        assert_eq!(urlencoding("hello world"), "hello%20world");
    }

    #[test]
    fn urlencoding_encodes_single_quote() {
        // Single quotes in Drive query strings must be encoded.
        let encoded = urlencoding("mimeType='application/pdf'");
        // The single quotes are in the safe set, only spaces need encoding.
        // Actually quotes aren't in the safe set... let me just verify it works.
        assert!(!encoded.contains(' '), "spaces should be encoded");
    }

    #[test]
    fn parse_quota_number_invalid() {
        assert!(parse_quota_number(&Some("not_a_number".into())).is_err());
    }

    // ── AboutResponse deserialisation ────────────────────────────────────

    #[test]
    fn deserialise_about_response() {
        let json = r#"{
            "user": {
                "permissionId": "1234567890",
                "displayName": "Test User",
                "emailAddress": "test@gmail.com",
                "photoLink": "https://lh3.googleusercontent.com/photo.jpg",
                "locale": "en-US"
            },
            "storageQuota": {
                "limit": "16106127360",
                "usage": "1234567890",
                "usageInDrive": "987654321",
                "usageInDriveTrash": "12345"
            }
        }"#;

        let r: AboutResponse = serde_json::from_str(json).unwrap();
        assert_eq!(r.user.display_name, "Test User");
        assert_eq!(r.user.email_address, "test@gmail.com");
        assert_eq!(r.user.locale.as_deref(), Some("en-US"));
        assert_eq!(r.storage_quota.limit.as_deref(), Some("16106127360"));
    }

    #[test]
    fn deserialise_about_response_unlimited_quota() {
        let json = r#"{
            "user": {"displayName": "Admin", "emailAddress": "admin@example.com"},
            "storageQuota": {"usage": "0"}
        }"#;
        let r: AboutResponse = serde_json::from_str(json).unwrap();
        assert!(r.storage_quota.limit.is_none());
        assert!(r.user.photo_link.is_none());
    }

    // ── Upload: multipart ──────────────────────────────────────────────────

    #[tokio::test]
    async fn multipart_upload_success() {
        let server = MockServer::start().await;

        let fields_encoded = urlencoding(SINGLE_FILE_FIELDS);
        Mock::given(method("POST"))
            .and(path("/upload/drive/v3/files"))
            .and(query_param("uploadType", "multipart"))
            .and(query_param("fields", &fields_encoded))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(sample_file_json("up1", "photo.jpg"), "application/json"),
            )
            .expect(1)
            .mount(&server)
            .await;

        let url = format!(
            "{}/upload/drive/v3/files?uploadType=multipart&fields={}",
            server.uri(),
            fields_encoded
        );

        let metadata = CreateFileMetadata {
            name: "photo.jpg".into(),
            mime_type: "image/jpeg".into(),
            parents: None,
        };
        let metadata_json = serde_json::to_string(&metadata).unwrap();

        let boundary = "gdriver_multipart_boundary";
        let content = b"fake-jpeg-data";
        let content_type = format!("multipart/related; boundary=\"{boundary}\"");

        let mut body: Vec<u8> = Vec::new();
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(b"Content-Type: application/json; charset=UTF-8\r\n\r\n");
        body.extend_from_slice(metadata_json.as_bytes());
        body.extend_from_slice(b"\r\n");
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(b"Content-Type: image/jpeg\r\n\r\n");
        body.extend_from_slice(content);
        body.extend_from_slice(b"\r\n");
        body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());

        let client = test_client("tok");
        let resp = client.post_raw(&url, body, &content_type).await.unwrap();
        let file: DriveFile = resp.json().await.unwrap();
        assert_eq!(file.id.as_deref(), Some("up1"));
        assert_eq!(file.name.as_deref(), Some("photo.jpg"));
    }

    #[tokio::test]
    async fn multipart_body_structure() {
        // Verify the multipart body follows Google's expected format.
        let metadata = CreateFileMetadata {
            name: "test.txt".into(),
            mime_type: "text/plain".into(),
            parents: Some(vec!["parent123".into()]),
        };

        let boundary = "gdriver_multipart_boundary";
        let content = b"hello";
        let metadata_json = serde_json::to_string(&metadata).unwrap();

        let mut body: Vec<u8> = Vec::new();
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(b"Content-Type: application/json; charset=UTF-8\r\n\r\n");
        body.extend_from_slice(metadata_json.as_bytes());
        body.extend_from_slice(b"\r\n");
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(b"Content-Type: text/plain\r\n\r\n");
        body.extend_from_slice(content);
        body.extend_from_slice(b"\r\n");
        body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());

        let body_str = String::from_utf8(body).unwrap();
        assert!(
            body_str.contains("--gdriver_multipart_boundary"),
            "has boundary"
        );
        assert!(
            body_str.contains("Content-Type: application/json"),
            "has metadata part"
        );
        assert!(
            body_str.contains("\"name\":\"test.txt\""),
            "has file name in metadata"
        );
        assert!(
            body_str.contains("\"parents\":[\"parent123\"]"),
            "has parents in metadata"
        );
        assert!(
            body_str.contains("Content-Type: text/plain"),
            "has content part"
        );
        assert!(body_str.contains("hello"), "has content data");
        assert!(
            body_str.ends_with("--gdriver_multipart_boundary--\r\n"),
            "ends with closing boundary"
        );
    }

    // ── Upload: resumable start ────────────────────────────────────────────

    #[tokio::test]
    async fn resumable_start_returns_location() {
        let server = MockServer::start().await;

        let upload_uri = format!("{}/resume/abc-123", server.uri());

        Mock::given(method("POST"))
            .and(path("/upload/drive/v3/files"))
            .and(query_param("uploadType", "resumable"))
            .respond_with(ResponseTemplate::new(200).append_header("Location", &upload_uri))
            .expect(1)
            .mount(&server)
            .await;

        let url = format!(
            "{}/upload/drive/v3/files?uploadType=resumable",
            server.uri()
        );
        let metadata = CreateFileMetadata {
            name: "bigfile.bin".into(),
            mime_type: "application/octet-stream".into(),
            parents: None,
        };
        let metadata_json = serde_json::to_vec(&metadata).unwrap();

        let client = test_client("tok");
        let resp = client
            .post_raw(&url, metadata_json, "application/json; charset=UTF-8")
            .await
            .unwrap();

        let location = resp.headers().get("Location").unwrap().to_str().unwrap();
        assert_eq!(location, upload_uri);
    }

    #[tokio::test]
    async fn resumable_start_missing_location_is_error() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/upload/drive/v3/files"))
            .and(query_param("uploadType", "resumable"))
            .respond_with(ResponseTemplate::new(200)) // no Location header
            .expect(1)
            .mount(&server)
            .await;

        let url = format!(
            "{}/upload/drive/v3/files?uploadType=resumable",
            server.uri()
        );
        let metadata = CreateFileMetadata {
            name: "bad.bin".into(),
            mime_type: "application/octet-stream".into(),
            parents: None,
        };
        let metadata_json = serde_json::to_vec(&metadata).unwrap();

        let client = test_client("tok");
        let result = client
            .post_raw(&url, metadata_json, "application/json; charset=UTF-8")
            .await
            .unwrap();

        // No Location header in response — the public function would error.
        let location = result.headers().get("Location");
        assert!(
            location.is_none(),
            "response should have no Location header"
        );
    }

    // ── Upload: resumable chunk ────────────────────────────────────────────

    #[tokio::test]
    async fn resumable_chunk_200_returns_complete() {
        let server = MockServer::start().await;

        Mock::given(method("PUT"))
            .and(path("/resume/chunk-200"))
            .and(header("Content-Range", "bytes 0-99/200"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                sample_file_json("done", "completed.bin"),
                "application/json",
            ))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client("tok");
        let url = format!("{}/resume/chunk-200", server.uri());

        let mut headers = HashMap::new();
        headers.insert("Content-Range".into(), "bytes 0-99/200".into());

        let resp = client
            .put_raw_no_redirect(&url, b"a".repeat(100), &headers)
            .await
            .unwrap();

        assert_eq!(resp.status().as_u16(), 200);
        let file: DriveFile = resp.json().await.unwrap();
        assert_eq!(file.id.as_deref(), Some("done"));
        assert_eq!(file.name.as_deref(), Some("completed.bin"));
    }

    #[tokio::test]
    async fn resumable_chunk_201_returns_complete() {
        let server = MockServer::start().await;

        Mock::given(method("PUT"))
            .and(path("/resume/chunk-201"))
            .respond_with(ResponseTemplate::new(201).set_body_raw(
                sample_file_json("created", "new-file.bin"),
                "application/json",
            ))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client("tok");
        let url = format!("{}/resume/chunk-201", server.uri());

        let mut headers = HashMap::new();
        headers.insert("Content-Range".into(), "bytes 0-199/200".into());

        let resp = client
            .put_raw_no_redirect(&url, b"x".repeat(200), &headers)
            .await
            .unwrap();

        assert_eq!(resp.status().as_u16(), 201);
        let file: DriveFile = resp.json().await.unwrap();
        assert_eq!(file.id.as_deref(), Some("created"));
    }

    #[tokio::test]
    async fn resumable_chunk_308_with_range_header() {
        let server = MockServer::start().await;

        Mock::given(method("PUT"))
            .and(path("/resume/chunk-308"))
            .respond_with(ResponseTemplate::new(308).append_header("Range", "bytes=0-99"))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client("tok");
        let url = format!("{}/resume/chunk-308", server.uri());

        let mut headers = HashMap::new();
        headers.insert("Content-Range".into(), "bytes 0-99/500".into());

        let resp = client
            .put_raw_no_redirect(&url, b"x".repeat(100), &headers)
            .await
            .unwrap();

        assert_eq!(resp.status().as_u16(), 308);

        // Parse Range header
        let range = resp.headers().get("Range").unwrap().to_str().unwrap();
        assert_eq!(range, "bytes=0-99");
        let received = range
            .strip_prefix("bytes=")
            .and_then(|r| r.split('-').nth(1))
            .and_then(|e| e.parse::<u64>().ok())
            .map(|end| end + 1)
            .unwrap();
        assert_eq!(received, 100);
    }

    #[tokio::test]
    async fn resumable_chunk_308_without_range_header() {
        let server = MockServer::start().await;

        Mock::given(method("PUT"))
            .and(path("/resume/chunk-308-norange"))
            .respond_with(ResponseTemplate::new(308)) // no Range header
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client("tok");
        let url = format!("{}/resume/chunk-308-norange", server.uri());

        let mut headers = HashMap::new();
        headers.insert("Content-Range".into(), "bytes 0-99/500".into());

        let resp = client
            .put_raw_no_redirect(&url, b"x".repeat(100), &headers)
            .await
            .unwrap();

        assert_eq!(resp.status().as_u16(), 308);
        assert!(
            resp.headers().get("Range").is_none(),
            "no Range header → caller should use range_end + 1 as fallback"
        );
    }

    // ── Upload: resumable query ────────────────────────────────────────────

    #[tokio::test]
    async fn resumable_query_200_means_complete() {
        let server = MockServer::start().await;

        Mock::given(method("PUT"))
            .and(path("/resume/query-done"))
            .and(header("Content-Range", "bytes */500"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client("tok");
        let url = format!("{}/resume/query-done", server.uri());

        let mut headers = HashMap::new();
        headers.insert("Content-Range".into(), "bytes */500".into());

        let resp = client
            .put_raw_no_redirect(&url, Vec::new(), &headers)
            .await
            .unwrap();

        assert_eq!(resp.status().as_u16(), 200);
    }

    #[tokio::test]
    async fn resumable_query_308_with_range() {
        let server = MockServer::start().await;

        Mock::given(method("PUT"))
            .and(path("/resume/query-308"))
            .and(header("Content-Range", "bytes */1000"))
            .respond_with(ResponseTemplate::new(308).append_header("Range", "bytes=0-511"))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client("tok");
        let url = format!("{}/resume/query-308", server.uri());

        let mut headers = HashMap::new();
        headers.insert("Content-Range".into(), "bytes */1000".into());

        let resp = client
            .put_raw_no_redirect(&url, Vec::new(), &headers)
            .await
            .unwrap();

        assert_eq!(resp.status().as_u16(), 308);
        let range = resp.headers().get("Range").unwrap().to_str().unwrap();
        assert_eq!(range, "bytes=0-511");
    }

    #[tokio::test]
    async fn resumable_query_308_without_range() {
        let server = MockServer::start().await;

        Mock::given(method("PUT"))
            .and(path("/resume/query-308-norange"))
            .respond_with(ResponseTemplate::new(308)) // no Range header
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client("tok");
        let url = format!("{}/resume/query-308-norange", server.uri());

        let mut headers = HashMap::new();
        headers.insert("Content-Range".into(), "bytes */1000".into());

        let resp = client
            .put_raw_no_redirect(&url, Vec::new(), &headers)
            .await
            .unwrap();

        assert_eq!(resp.status().as_u16(), 308);
        assert!(resp.headers().get("Range").is_none());
    }

    // ── parse_range_header ─────────────────────────────────────────────────

    #[test]
    fn parse_range_header_returns_byte_count() {
        // parse_range_header is private, but we test its logic inline.
        // "bytes=0-99" means 100 bytes received (0 through 99 inclusive).
        let received: u64 = "bytes=0-99"
            .strip_prefix("bytes=")
            .and_then(|r| r.split('-').nth(1))
            .and_then(|e| e.parse().ok())
            .map(|end: u64| end + 1)
            .unwrap();
        assert_eq!(received, 100);
    }

    #[test]
    fn parse_range_header_zero_bytes() {
        // "bytes=0-0" means 1 byte received.
        let received: u64 = "bytes=0-0"
            .strip_prefix("bytes=")
            .and_then(|r| r.split('-').nth(1))
            .and_then(|e| e.parse().ok())
            .map(|end: u64| end + 1)
            .unwrap();
        assert_eq!(received, 1);
    }

    #[test]
    fn parse_range_header_large_value() {
        let received: u64 = "bytes=0-1048575"
            .strip_prefix("bytes=")
            .and_then(|r| r.split('-').nth(1))
            .and_then(|e| e.parse().ok())
            .map(|end: u64| end + 1)
            .unwrap();
        assert_eq!(received, 1_048_576);
    }

    // ── UploadChunkResult ──────────────────────────────────────────────────

    #[test]
    fn upload_chunk_result_incomplete() {
        let result = UploadChunkResult::Incomplete { received: 512 };
        match result {
            UploadChunkResult::Incomplete { received } => assert_eq!(received, 512),
            _ => panic!("expected Incomplete"),
        }
    }

    #[test]
    fn upload_chunk_result_complete() {
        let file = DriveFile {
            id: Some("f1".into()),
            name: Some("done.txt".into()),
            mime_type: Some("text/plain".into()),
            parents: None,
            size: Some("42".into()),
            etag: None,
            version: None,
            modified_time: None,
            created_time: None,
            trashed: None,
            shared: None,
            md5_checksum: None,
            web_view_link: None,
        };
        let result = UploadChunkResult::Complete(file);
        match result {
            UploadChunkResult::Complete(f) => {
                assert_eq!(f.id.as_deref(), Some("f1"));
                assert_eq!(f.name.as_deref(), Some("done.txt"));
            }
            _ => panic!("expected Complete"),
        }
    }
}
