//! File upload: reads a local file and pushes it to Google Drive.
//!
//! Strategy:
//! - < 5 MB: simple multipart upload with MD5 verification.
//! - >= 5 MB: resumable upload in 5 MB chunks.

use std::path::Path;

use gdriver_api::{
    client::DriveClient,
    files::{self, CreateFileMetadata, UploadChunkResult},
};
use sqlx::SqlitePool;
use tracing::{debug, error, info, warn};

use crate::db;

/// Files below this size use simple multipart upload.
const RESUMABLE_THRESHOLD: u64 = 5 * 1024 * 1024; // 5 MB

/// Chunk size for resumable uploads (must be a multiple of 256 KiB).
const CHUNK_SIZE: u64 = 5 * 1024 * 1024; // 5 MB

/// Maximum number of retry attempts before logging a persistent error.
const MAX_RETRIES: i32 = 3;

/// Process a single upload task.
///
/// Returns `Ok(())` whether the upload succeeded or is being retried.  Callers
/// should always advance past the task — errors are recorded on the task row
/// itself so the UI can surface them via the sync-errors list.
pub async fn upload_file(
    db: &SqlitePool,
    client: &DriveClient,
    task: &db::queue::SyncTask,
) -> anyhow::Result<()> {
    let task_id = match task.id {
        Some(id) => id,
        None => {
            warn!("upload task has no id, skipping");
            return Ok(());
        }
    };

    let local_path = match task.local_path.as_deref() {
        Some(p) => p,
        None => {
            warn!(task_id, "upload task has no local_path, marking completed");
            db::queue::update_task_status(db, task_id, "completed", Some("no local_path")).await?;
            return Ok(());
        }
    };

    let path = Path::new(local_path);

    // ── Mark in-progress ──────────────────────────────────────────────────
    db::queue::update_task_status(db, task_id, "in_progress", None).await?;

    // ── Read file and determine metadata ──────────────────────────────────
    let content = match tokio::fs::read(path).await {
        Ok(c) => c,
        Err(e) => {
            return handle_upload_failure(db, task, task_id, &format!("read error: {e}")).await;
        }
    };

    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "Untitled".into());
    let mime = guess_mime(path);

    let md5_before = format!("{:x}", md5::compute(&content));

    let metadata = CreateFileMetadata {
        name: file_name.clone(),
        mime_type: mime.clone(),
        parents: None, // uploaded to My Drive root by default
    };

    // ── Choose upload strategy ────────────────────────────────────────────
    let file_len = content.len() as u64;

    let result = if file_len < RESUMABLE_THRESHOLD {
        do_multipart_upload(client, &metadata, &content, &mime, task_id).await
    } else {
        do_resumable_upload(client, &metadata, &content, file_len, task_id).await
    };

    match result {
        Ok(api_file) => {
            // ── Verify MD5 if provided ─────────────────────────────────
            if let Some(ref drive_md5) = api_file.md5_checksum {
                if !drive_md5.eq_ignore_ascii_case(&md5_before) {
                    warn!(
                        task_id,
                        local_md5 = %md5_before,
                        drive_md5 = %drive_md5,
                        "MD5 mismatch after upload"
                    );
                }
            }

            // ── Persist the Drive file metadata ────────────────────────
            let db_file = crate::sync::initial::map_api_file_to_db(&api_file, &task.account_id);
            if let Err(e) = db::files::upsert_file(db, &db_file).await {
                error!(task_id, error = %e, "failed to persist uploaded file metadata");
            }

            db::queue::update_task_status(db, task_id, "completed", None).await?;

            info!(
                task_id,
                file_id = %db_file.id,
                name = %db_file.name,
                size = file_len,
                "upload completed"
            );
        }
        Err(e) => {
            handle_upload_failure(db, task, task_id, &format!("{e:#}")).await?;
        }
    }

    Ok(())
}

// ─── Upload strategies ───────────────────────────────────────────────────────

async fn do_multipart_upload(
    client: &DriveClient,
    metadata: &CreateFileMetadata,
    content: &[u8],
    content_mime: &str,
    task_id: i64,
) -> anyhow::Result<gdriver_api::files::DriveFile> {
    debug!(task_id, size = content.len(), "multipart upload");
    files::files_upload_multipart(client, metadata, content, content_mime).await
}

async fn do_resumable_upload(
    client: &DriveClient,
    metadata: &CreateFileMetadata,
    content: &[u8],
    file_len: u64,
    task_id: i64,
) -> anyhow::Result<gdriver_api::files::DriveFile> {
    debug!(task_id, size = file_len, "resumable upload starting");

    let uri = files::files_upload_resumable_start(client, metadata).await?;
    debug!(task_id, %uri, "resumable session started");

    let mut offset: u64 = 0;
    let mut result = None;

    while offset < file_len {
        let end = std::cmp::min(offset + CHUNK_SIZE, file_len) - 1;
        let chunk = &content[offset as usize..=end as usize];

        debug!(task_id, offset, end, "uploading chunk");

        match files::files_upload_resumable_chunk(client, &uri, chunk, offset, end, file_len).await
        {
            Ok(UploadChunkResult::Incomplete { received }) => {
                debug!(task_id, ?received, "chunk accepted");
                offset = received;
            }
            Ok(UploadChunkResult::Complete(file)) => {
                debug!(task_id, "final chunk accepted");
                result = Some(file);
                break;
            }
            Err(e) => {
                // Query current status so we can resume from where we left off.
                match files::files_upload_resumable_query(client, &uri, file_len).await {
                    Ok((received, _is_complete)) => {
                        warn!(task_id, received, error = %e, "chunk failed; resuming from last received byte");
                        offset = received;
                    }
                    Err(_) => {
                        return Err(e);
                    }
                }
            }
        }
    }

    result.ok_or_else(|| anyhow::anyhow!("resumable upload loop exited without completion"))
}

// ─── Failure handling ────────────────────────────────────────────────────────

async fn handle_upload_failure(
    db: &SqlitePool,
    task: &db::queue::SyncTask,
    task_id: i64,
    error_msg: &str,
) -> anyhow::Result<()> {
    let new_retry = task.retry_count + 1;

    if new_retry > MAX_RETRIES {
        warn!(task_id, retries = new_retry, %error_msg, "upload permanently failed");

        db::queue::update_task_status(db, task_id, "failed", Some(error_msg)).await?;

        // Record in the persistent error log so the UI can surface it.
        let now = chrono::Utc::now().timestamp_millis();
        let sync_error = db::sync_errors::SyncError {
            id: None,
            account_id: Some(task.account_id.clone()),
            file_id: task.file_id.clone(),
            file_name: task
                .local_path
                .as_deref()
                .and_then(|p| Path::new(p).file_name())
                .map(|n| n.to_string_lossy().to_string()),
            error_code: "UPLOAD_FAILED".into(),
            error_msg: error_msg.to_string(),
            is_resolved: false,
            created_at: now,
        };
        if let Err(e) = db::sync_errors::insert_error(db, &sync_error).await {
            error!(task_id, error = %e, "failed to record sync error");
        }
    } else {
        debug!(task_id, retries = new_retry, %error_msg, "upload failed, re-queuing for retry");
        db::queue::update_task_retry(db, task_id, "pending", new_retry, Some(error_msg)).await?;
    }

    Ok(())
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Guess a MIME type from the file extension.  Falls back to
/// `application/octet-stream` when no match is found.
fn guess_mime(path: &Path) -> String {
    match path.extension().and_then(|e| e.to_str()) {
        Some("txt") => "text/plain",
        Some("html" | "htm") => "text/html",
        Some("css") => "text/css",
        Some("js") => "text/javascript",
        Some("json") => "application/json",
        Some("xml") => "application/xml",
        Some("pdf") => "application/pdf",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("svg") => "image/svg+xml",
        Some("mp3") => "audio/mpeg",
        Some("mp4") => "video/mp4",
        Some("zip") => "application/zip",
        Some("gz" | "tar") => "application/gzip",
        Some("doc") => "application/msword",
        Some("docx") => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        Some("xls") => "application/vnd.ms-excel",
        Some("xlsx") => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        _ => "application/octet-stream",
    }
    .to_string()
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guess_mime_known_extensions() {
        assert_eq!(guess_mime(Path::new("file.txt")), "text/plain");
        assert_eq!(guess_mime(Path::new("image.png")), "image/png");
        assert_eq!(guess_mime(Path::new("doc.pdf")), "application/pdf");
        assert_eq!(guess_mime(Path::new("photo.jpeg")), "image/jpeg");
        assert_eq!(guess_mime(Path::new("photo.jpg")), "image/jpeg");
        assert_eq!(guess_mime(Path::new("data.json")), "application/json");
    }

    #[test]
    fn guess_mime_unknown_extension() {
        assert_eq!(
            guess_mime(Path::new("file.xyz")),
            "application/octet-stream"
        );
    }

    #[test]
    fn guess_mime_no_extension() {
        assert_eq!(
            guess_mime(Path::new("Makefile")),
            "application/octet-stream"
        );
    }
}
