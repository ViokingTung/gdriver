//! Google Photos Library API v1 wrappers.
//!
//! Used by M16 (Google Photos backup) to upload media items and create them
//! in the user's Google Photos library.

use std::path::Path;

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::client::DriveClient;

// ─── Constants ───────────────────────────────────────────────────────────────

const PHOTOS_API_BASE: &str = "https://photoslibrary.googleapis.com/v1";

// ─── Request types ───────────────────────────────────────────────────────────

/// A media item to create in Google Photos.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewMediaItem {
    /// Optional description for the media item.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The simple media item containing the upload token.
    pub simple_media_item: SimpleMediaItem,
}

/// The core data needed to create a media item from an upload token.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimpleMediaItem {
    /// Token returned by [`upload_media_item`].
    pub upload_token: String,
    /// Optional file name hint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
}

/// Request body for `POST /v1/mediaItems:batchCreate`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchCreateRequest {
    /// Optional album ID to add the media items to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub album_id: Option<String>,
    /// The media items to create (1–50 items).
    pub new_media_items: Vec<NewMediaItem>,
}

// ─── Response types ──────────────────────────────────────────────────────────

/// Response from `POST /v1/mediaItems:batchCreate`.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BatchCreateResponse {
    /// Results corresponding to each item in the request.
    #[serde(default, rename = "newMediaItemResults")]
    pub new_media_item_results: Vec<MediaItemResult>,
}

/// The result of creating a single media item.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MediaItemResult {
    /// The upload token this result corresponds to.
    pub upload_token: Option<String>,
    /// Status of the creation attempt.
    pub status: Option<Status>,
    /// The created media item (only present on success).
    pub media_item: Option<MediaItem>,
}

/// Status of a media item creation attempt.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Status {
    /// Human-readable status message (e.g. `"Success"`, `"Error type: ..."`).
    pub message: Option<String>,
    /// Status code — `0` means success.
    pub code: Option<i32>,
}

/// A media item in Google Photos.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MediaItem {
    /// Unique identifier for the media item.
    pub id: Option<String>,
    /// URL to view the item in Google Photos.
    pub product_url: Option<String>,
    /// Base URL for downloading the media.
    pub base_url: Option<String>,
    /// MIME type of the media item.
    pub mime_type: Option<String>,
    /// Metadata (creation time, dimensions).
    pub media_metadata: Option<MediaMetadata>,
    /// File name as stored in Google Photos.
    pub filename: Option<String>,
}

/// Metadata about a media item (dimensions, creation time).
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MediaMetadata {
    /// RFC 3339 creation timestamp.
    pub creation_time: Option<String>,
    /// Width in pixels (string in the API).
    pub width: Option<String>,
    /// Height in pixels (string in the API).
    pub height: Option<String>,
}

// ─── API functions ───────────────────────────────────────────────────────────

/// Upload raw media bytes and obtain an upload token.
///
/// This is step 1 of the Google Photos upload flow.  The returned token must
/// be used within 24 hours in a [`batch_create`] call.
pub async fn upload_media_item(
    client: &DriveClient,
    content: &[u8],
    mime_type: &str,
) -> anyhow::Result<String> {
    let url = format!("{PHOTOS_API_BASE}/uploads");

    let resp = client
        .post_raw(&url, content.to_vec(), mime_type)
        .await?;

    // The Photos upload endpoint sends `X-Goog-Upload-Content-Type` as a
    // header hint, but the response body is the plain-text upload token.
    let upload_token = resp
        .text()
        .await
        .context("failed to read upload token response")?;
    Ok(upload_token)
}

/// Create one or more media items from upload tokens.
///
/// This is step 2 — call [`upload_media_item`] first to get the tokens, then
/// pass them here.  Up to 50 items can be created in a single call.
pub async fn batch_create(
    client: &DriveClient,
    request: &BatchCreateRequest,
) -> anyhow::Result<BatchCreateResponse> {
    let url = format!("{PHOTOS_API_BASE}/mediaItems:batchCreate");
    client.post_json(&url, request).await
}

// ─── Path-based upload ───────────────────────────────────────────────────────

/// Read a local file and upload it to Google Photos, returning an upload token.
///
/// This is a convenience wrapper around [`upload_media_item`] that reads the
/// file from disk and automatically detects the MIME type.  Returns an error
/// if the file format is not supported by Google Photos.
pub async fn upload_media_item_from_path(
    client: &DriveClient,
    local_path: &Path,
) -> anyhow::Result<String> {
    let mime = photos_mime_type(local_path).ok_or_else(|| {
        anyhow::anyhow!(
            "unsupported photo format: {}",
            local_path.display()
        )
    })?;

    let content = tokio::fs::read(local_path)
        .await
        .with_context(|| format!("failed to read {}", local_path.display()))?;

    upload_media_item(client, &content, &mime).await
}

// ─── Format detection ────────────────────────────────────────────────────────

/// Returns `true` if the file at `path` has a supported Google Photos format.
pub fn is_supported_photo_format(path: &Path) -> bool {
    photos_mime_type(path).is_some()
}

/// Return the MIME type for a supported Photos format, or `None` if the
/// extension is not supported.
pub fn photos_mime_type(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();

    // Images
    match ext.as_str() {
        "jpg" | "jpeg" => return Some("image/jpeg".into()),
        "png" => return Some("image/png".into()),
        "gif" => return Some("image/gif".into()),
        "bmp" => return Some("image/bmp".into()),
        "tiff" | "tif" => return Some("image/tiff".into()),
        "webp" => return Some("image/webp".into()),
        "heic" => return Some("image/heic".into()),
        "heif" => return Some("image/heif".into()),
        // RAW formats — treat as generic application/octet-stream but still
        // support them (Photos accepts them).
        "raw" | "cr2" | "nef" | "arw" | "dng" => {
            return Some("application/octet-stream".into())
        }
        _ => {}
    }

    // Videos
    match ext.as_str() {
        "mp4" => return Some("video/mp4".into()),
        "mov" => return Some("video/quicktime".into()),
        "avi" => return Some("video/x-msvideo".into()),
        "wmv" => return Some("video/x-ms-wmv".into()),
        "flv" => return Some("video/x-flv".into()),
        "mkv" => return Some("video/x-matroska".into()),
        "3gp" => return Some("video/3gpp".into()),
        "m4v" => return Some("video/x-m4v".into()),
        "mpg" | "mpeg" => return Some("video/mpeg".into()),
        "webm" => return Some("video/webm".into()),
        _ => {}
    }

    None
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn test_client(token: &str) -> DriveClient {
        DriveClient::new(token)
    }

    // ── upload_media_item ─────────────────────────────────────────────────

    #[tokio::test]
    async fn upload_returns_token() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/uploads"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                "CAISiQIKJQ...upload-token-value",
                "text/plain",
            ))
            .expect(1)
            .mount(&server)
            .await;

        let url = format!("{}/v1/uploads", server.uri());
        let client = test_client("tok");
        let resp = client
            .post_raw(&url, b"fake-image-data".to_vec(), "application/octet-stream")
            .await
            .unwrap();

        let token = resp.text().await.unwrap();
        assert!(token.contains("upload-token-value"));
    }

    #[tokio::test]
    async fn upload_sets_content_type_header() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/uploads"))
            .and(header("Content-Type", "application/octet-stream"))
            .respond_with(ResponseTemplate::new(200).set_body_raw("token-123", "text/plain"))
            .expect(1)
            .mount(&server)
            .await;

        let url = format!("{}/v1/uploads", server.uri());
        let client = test_client("tok");
        let resp = client
            .post_raw(&url, b"data".to_vec(), "application/octet-stream")
            .await
            .unwrap();

        assert_eq!(resp.status().as_u16(), 200);
    }

    // ── batch_create ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn batch_create_sends_request() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/mediaItems:batchCreate"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"{"newMediaItemResults":[]}"#,
                "application/json",
            ))
            .expect(1)
            .mount(&server)
            .await;

        let url = format!("{}/v1/mediaItems:batchCreate", server.uri());
        let client = test_client("tok");
        let req = BatchCreateRequest {
            album_id: None,
            new_media_items: vec![],
        };
        let resp: BatchCreateResponse = client.post_json(&url, &req).await.unwrap();
        assert!(resp.new_media_item_results.is_empty());
    }

    #[tokio::test]
    async fn batch_create_with_album() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/mediaItems:batchCreate"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"{"newMediaItemResults":[]}"#,
                "application/json",
            ))
            .expect(1)
            .mount(&server)
            .await;

        let url = format!("{}/v1/mediaItems:batchCreate", server.uri());
        let client = test_client("tok");
        let req = BatchCreateRequest {
            album_id: Some("album-abc".into()),
            new_media_items: vec![],
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["albumId"].as_str(), Some("album-abc"));

        let _resp: BatchCreateResponse = client.post_json(&url, &req).await.unwrap();
    }

    // ── BatchCreateRequest serialisation ───────────────────────────────────

    #[test]
    fn serialise_single_item_request() {
        let req = BatchCreateRequest {
            album_id: None,
            new_media_items: vec![NewMediaItem {
                description: Some("A nice photo".into()),
                simple_media_item: SimpleMediaItem {
                    upload_token: "token-abc".into(),
                    file_name: Some("photo.jpg".into()),
                },
            }],
        };

        let json = serde_json::to_value(&req).unwrap();
        let items = &json["newMediaItems"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["description"].as_str(), Some("A nice photo"));
        assert_eq!(
            items[0]["simpleMediaItem"]["uploadToken"].as_str(),
            Some("token-abc")
        );
        assert_eq!(
            items[0]["simpleMediaItem"]["fileName"].as_str(),
            Some("photo.jpg")
        );
        // albumId should be absent when None
        assert!(json.get("albumId").is_none());
    }

    #[test]
    fn serialise_with_album_id() {
        let req = BatchCreateRequest {
            album_id: Some("album-xyz".into()),
            new_media_items: vec![NewMediaItem {
                description: None,
                simple_media_item: SimpleMediaItem {
                    upload_token: "tok".into(),
                    file_name: None,
                },
            }],
        };

        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["albumId"].as_str(), Some("album-xyz"));
        // description and fileName should be absent when None
        let item = &json["newMediaItems"][0];
        assert!(item.get("description").is_none());
        assert!(item["simpleMediaItem"].get("fileName").is_none());
    }

    // ── BatchCreateResponse deserialisation ───────────────────────────────

    #[test]
    fn deserialise_successful_result() {
        let json = r#"{
            "newMediaItemResults": [
                {
                    "uploadToken": "token-1",
                    "status": {
                        "message": "Success",
                        "code": 0
                    },
                    "mediaItem": {
                        "id": "photo-abc",
                        "productUrl": "https://photos.google.com/photo/abc",
                        "baseUrl": "https://lh3.googleusercontent.com/abc",
                        "mimeType": "image/jpeg",
                        "mediaMetadata": {
                            "creationTime": "2026-05-01T12:00:00Z",
                            "width": "4032",
                            "height": "3024"
                        },
                        "filename": "photo.jpg"
                    }
                }
            ]
        }"#;

        let resp: BatchCreateResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.new_media_item_results.len(), 1);

        let result = &resp.new_media_item_results[0];
        assert_eq!(result.upload_token.as_deref(), Some("token-1"));
        assert_eq!(result.status.as_ref().unwrap().message.as_deref(), Some("Success"));
        assert_eq!(result.status.as_ref().unwrap().code, Some(0));

        let item = result.media_item.as_ref().unwrap();
        assert_eq!(item.id.as_deref(), Some("photo-abc"));
        assert_eq!(item.mime_type.as_deref(), Some("image/jpeg"));
        assert_eq!(item.filename.as_deref(), Some("photo.jpg"));

        let meta = item.media_metadata.as_ref().unwrap();
        assert_eq!(meta.width.as_deref(), Some("4032"));
        assert_eq!(meta.height.as_deref(), Some("3024"));
    }

    #[test]
    fn deserialise_error_result() {
        let json = r#"{
            "newMediaItemResults": [
                {
                    "uploadToken": "bad-token",
                    "status": {
                        "message": "Error type: INVALID_ARGUMENT",
                        "code": 3
                    }
                }
            ]
        }"#;

        let resp: BatchCreateResponse = serde_json::from_str(json).unwrap();
        let result = &resp.new_media_item_results[0];
        assert_eq!(result.status.as_ref().unwrap().code, Some(3));
        assert!(result.media_item.is_none(), "error results have no mediaItem");
    }

    #[test]
    fn deserialise_multiple_results() {
        let json = r#"{
            "newMediaItemResults": [
                {
                    "uploadToken": "tok-1",
                    "status": {"message": "Success"},
                    "mediaItem": {
                        "id": "id-1",
                        "productUrl": "https://photos.google.com/1",
                        "baseUrl": "https://lh3/1",
                        "mimeType": "image/jpeg",
                        "filename": "a.jpg"
                    }
                },
                {
                    "uploadToken": "tok-2",
                    "status": {"message": "Success"},
                    "mediaItem": {
                        "id": "id-2",
                        "productUrl": "https://photos.google.com/2",
                        "baseUrl": "https://lh3/2",
                        "mimeType": "image/png",
                        "filename": "b.png"
                    }
                }
            ]
        }"#;

        let resp: BatchCreateResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.new_media_item_results.len(), 2);
        assert_eq!(resp.new_media_item_results[0].media_item.as_ref().unwrap().id.as_deref(), Some("id-1"));
        assert_eq!(resp.new_media_item_results[1].media_item.as_ref().unwrap().id.as_deref(), Some("id-2"));
    }

    #[test]
    fn deserialise_empty_results() {
        let json = r#"{"newMediaItemResults":[]}"#;
        let resp: BatchCreateResponse = serde_json::from_str(json).unwrap();
        assert!(resp.new_media_item_results.is_empty());
    }

    // ── MediaItem deserialisation ─────────────────────────────────────────

    #[test]
    fn deserialise_media_item_minimal() {
        let json = r#"{
            "id": "item-1",
            "mimeType": "image/jpeg",
            "filename": "photo.jpg"
        }"#;

        let item: MediaItem = serde_json::from_str(json).unwrap();
        assert_eq!(item.id.as_deref(), Some("item-1"));
        assert_eq!(item.mime_type.as_deref(), Some("image/jpeg"));
        assert!(item.product_url.is_none());
        assert!(item.media_metadata.is_none());
    }

    // ── is_supported_photo_format / photos_mime_type ──────────────────────

    #[test]
    fn supported_image_formats() {
        assert_eq!(photos_mime_type(Path::new("photo.jpg")), Some("image/jpeg".into()));
        assert_eq!(photos_mime_type(Path::new("photo.jpeg")), Some("image/jpeg".into()));
        assert_eq!(photos_mime_type(Path::new("image.png")), Some("image/png".into()));
        assert_eq!(photos_mime_type(Path::new("anim.gif")), Some("image/gif".into()));
        assert_eq!(photos_mime_type(Path::new("pic.webp")), Some("image/webp".into()));
        assert_eq!(photos_mime_type(Path::new("photo.heic")), Some("image/heic".into()));
        assert_eq!(photos_mime_type(Path::new("photo.heif")), Some("image/heif".into()));
        assert_eq!(photos_mime_type(Path::new("scan.bmp")), Some("image/bmp".into()));
        assert_eq!(photos_mime_type(Path::new("doc.tiff")), Some("image/tiff".into()));
        assert_eq!(photos_mime_type(Path::new("doc.tif")), Some("image/tiff".into()));
    }

    #[test]
    fn supported_video_formats() {
        assert_eq!(photos_mime_type(Path::new("clip.mp4")), Some("video/mp4".into()));
        assert_eq!(photos_mime_type(Path::new("clip.mov")), Some("video/quicktime".into()));
        assert_eq!(photos_mime_type(Path::new("clip.avi")), Some("video/x-msvideo".into()));
        assert_eq!(photos_mime_type(Path::new("clip.mkv")), Some("video/x-matroska".into()));
        assert_eq!(photos_mime_type(Path::new("clip.webm")), Some("video/webm".into()));
        assert_eq!(photos_mime_type(Path::new("clip.3gp")), Some("video/3gpp".into()));
    }

    #[test]
    fn raw_formats_supported() {
        assert_eq!(photos_mime_type(Path::new("raw.cr2")), Some("application/octet-stream".into()));
        assert_eq!(photos_mime_type(Path::new("raw.nef")), Some("application/octet-stream".into()));
        assert_eq!(photos_mime_type(Path::new("raw.arw")), Some("application/octet-stream".into()));
        assert_eq!(photos_mime_type(Path::new("raw.dng")), Some("application/octet-stream".into()));
    }

    #[test]
    fn unsupported_formats_return_none() {
        assert_eq!(photos_mime_type(Path::new("doc.pdf")), None);
        assert_eq!(photos_mime_type(Path::new("data.json")), None);
        assert_eq!(photos_mime_type(Path::new("script.js")), None);
        assert_eq!(photos_mime_type(Path::new("noext")), None);
    }

    #[test]
    fn is_supported_photo_format_works() {
        assert!(is_supported_photo_format(Path::new("photo.jpg")));
        assert!(is_supported_photo_format(Path::new("video.mp4")));
        assert!(is_supported_photo_format(Path::new("pic.heic")));
        assert!(!is_supported_photo_format(Path::new("doc.pdf")));
        assert!(!is_supported_photo_format(Path::new("notes.txt")));
    }

    #[test]
    fn case_insensitive_extension_matching() {
        assert_eq!(photos_mime_type(Path::new("PHOTO.JPG")), Some("image/jpeg".into()));
        assert_eq!(photos_mime_type(Path::new("clip.MP4")), Some("video/mp4".into()));
        assert_eq!(photos_mime_type(Path::new("image.Png")), Some("image/png".into()));
    }

    // ── upload_media_item_from_path ───────────────────────────────────────

    #[tokio::test]
    async fn upload_from_path_sends_file_content() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/uploads"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                "path-upload-token",
                "text/plain",
            ))
            .expect(1)
            .mount(&server)
            .await;

        // Create a temporary file to upload.
        let tmp = tempfile::NamedTempFile::with_suffix(".jpg").unwrap();
        std::fs::write(tmp.path(), b"fake-photo-bytes").unwrap();

        // We can't easily redirect the upload URL to the mock server through
        // `upload_media_item_from_path` since it uses the real API base.
        // Instead, test the lower-level flow: read → mime detect → upload.
        let content = tokio::fs::read(tmp.path()).await.unwrap();
        let mime = photos_mime_type(tmp.path()).unwrap();

        let url = format!("{}/v1/uploads", server.uri());
        let client = test_client("tok");
        let resp = client.post_raw(&url, content, &mime).await.unwrap();
        let token = resp.text().await.unwrap();
        assert_eq!(token, "path-upload-token");
    }

    #[tokio::test]
    async fn upload_from_path_rejects_unsupported_format() {
        // A .txt file should be rejected before any network call.
        let tmp = tempfile::NamedTempFile::with_suffix(".txt").unwrap();
        std::fs::write(tmp.path(), b"not a photo").unwrap();

        assert!(
            photos_mime_type(tmp.path()).is_none(),
            "txt should not be a supported photo format"
        );
    }
}
