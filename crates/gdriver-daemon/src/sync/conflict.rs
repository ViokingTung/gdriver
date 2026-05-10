//! Conflict detection and resolution.
//!
//! A conflict occurs when both the local file and the remote Drive file have
//! been modified since the last sync.  Resolution keeps both versions:
//! the local copy is renamed with a "conflict copy" suffix and uploaded
//! separately, while the remote version is downloaded to the original path.

use std::path::Path;

use gdriver_api::{
    client::DriveClient,
    files::{self, CreateFileMetadata, UploadChunkResult},
};
use sqlx::SqlitePool;
use tracing::{debug, error, info, warn};

use crate::{db, ipc::PushSender};

/// Files below this size use simple multipart upload.
const RESUMABLE_THRESHOLD: u64 = 5 * 1024 * 1024; // 5 MB

/// Chunk size for resumable uploads (must be a multiple of 256 KiB).
const CHUNK_SIZE: u64 = 5 * 1024 * 1024; // 5 MB

// ─── Detection ─────────────────────────────────────────────────────────────────

/// Check whether both the local file and the remote Drive file have changed
/// since the last sync.
///
/// * `current_local_mtime_ms` — the local file's mtime right now.
/// * `db_record` — what we have stored in `drive_files` from the last sync.
/// * `current_remote_etag` — the etag returned by the Drive API right now.
pub fn detect_conflict(
    current_local_mtime_ms: i64,
    db_record: &db::files::DriveFile,
    current_remote_etag: &str,
) -> bool {
    let local_changed = db_record
        .local_mtime
        .is_some_and(|stored| current_local_mtime_ms > stored);

    let remote_changed = db_record
        .etag
        .as_deref()
        .is_some_and(|stored| stored != current_remote_etag);

    local_changed && remote_changed
}

// ─── Orchestrator ─────────────────────────────────────────────────────────────

/// Check for a conflict before uploading.  Returns `Ok(Some(()))` when a
/// conflict was detected and resolved successfully, `Ok(None)` when there is no
/// conflict (caller should proceed with the normal upload), and `Err(...)` on
/// failure (caller should fall back to the normal upload).
pub async fn check_and_resolve(
    db: &SqlitePool,
    client: &DriveClient,
    push_tx: &PushSender,
    task: &db::queue::SyncTask,
    file_id: &str,
    local_path: &str,
) -> anyhow::Result<Option<()>> {
    // ── Look up the cached DB record ───────────────────────────────────────
    let db_record = match db::files::get_file_by_id(db, file_id, &task.account_id).await? {
        Some(r) => r,
        None => {
            debug!(
                task_id = task.id,
                file_id, "no cached record; skipping conflict check"
            );
            return Ok(None);
        }
    };

    // ── Get the current local mtime ────────────────────────────────────────
    let current_local_mtime = match tokio::fs::metadata(local_path).await {
        Ok(meta) => match meta.modified() {
            Ok(mtime) => match mtime.duration_since(std::time::UNIX_EPOCH) {
                Ok(dur) => dur.as_millis() as i64,
                Err(_) => return Ok(None),
            },
            Err(_) => return Ok(None),
        },
        Err(_) => return Ok(None),
    };

    // ── Fetch the current remote metadata ──────────────────────────────────
    let remote_file = match files::files_get(
        client,
        file_id,
        Some("id,name,mimeType,size,etag,version,modifiedTime,trashed"),
    )
    .await
    {
        Ok(f) => f,
        Err(e) => {
            warn!(task_id = task.id, file_id, error = %e, "failed to fetch remote metadata for conflict check");
            return Ok(None);
        }
    };

    let current_remote_etag = match remote_file.etag.as_deref() {
        Some(e) => e,
        None => return Ok(None),
    };

    // ── Detect conflict ────────────────────────────────────────────────────
    if !detect_conflict(current_local_mtime, &db_record, current_remote_etag) {
        debug!(task_id = task.id, file_id, "no conflict detected");
        return Ok(None);
    }

    info!(
        task_id = task.id,
        file_id,
        local_path,
        local_mtime = current_local_mtime,
        db_mtime = db_record.local_mtime,
        remote_etag = current_remote_etag,
        db_etag = ?db_record.etag,
        "conflict detected"
    );

    // ── Resolve ────────────────────────────────────────────────────────────
    resolve_conflict(
        db,
        client,
        push_tx,
        task,
        &db_record,
        local_path,
        &remote_file,
    )
    .await?;

    Ok(Some(()))
}

// ─── Resolution ───────────────────────────────────────────────────────────────

async fn resolve_conflict(
    db: &SqlitePool,
    client: &DriveClient,
    push_tx: &PushSender,
    task: &db::queue::SyncTask,
    db_record: &db::files::DriveFile,
    local_path: &str,
    remote_file: &files::DriveFile,
) -> anyhow::Result<()> {
    let task_id = task.id.unwrap_or(0);
    let path = Path::new(local_path);

    // ── Build conflict copy name ───────────────────────────────────────────
    let now = chrono::Local::now();
    let ts = now.format("%Y-%m-%d %H:%M:%S");

    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy())
        .unwrap_or_else(|| std::borrow::Cow::Borrowed("Untitled"));
    let ext = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();

    let conflict_name = format!("{stem} (conflict copy {ts}){ext}");
    let conflict_dir = path.parent().unwrap_or_else(|| Path::new("."));
    let conflict_path = conflict_dir.join(&conflict_name);

    // ── Rename local file → conflict copy ──────────────────────────────────
    debug!(
        task_id,
        ?local_path,
        ?conflict_path,
        "renaming local to conflict copy"
    );
    tokio::fs::rename(local_path, &conflict_path).await?;

    // ── Upload conflict copy to Drive ──────────────────────────────────────
    let conflict_content = tokio::fs::read(&conflict_path).await?;
    let conflict_len = conflict_content.len() as u64;
    let conflict_mime = guess_mime(&conflict_path);

    let metadata = CreateFileMetadata {
        name: conflict_name.clone(),
        mime_type: conflict_mime.clone(),
        parents: db_record.parent_id.clone().map(|p| vec![p]),
    };

    let upload_result = if conflict_len < RESUMABLE_THRESHOLD {
        files::files_upload_multipart(client, &metadata, &conflict_content, &conflict_mime).await
    } else {
        upload_resumable(client, &metadata, &conflict_content, conflict_len, task_id).await
    };

    match upload_result {
        Ok(api_file) => {
            let db_conflict = crate::sync::initial::map_api_file_to_db(&api_file, &task.account_id);
            if let Err(e) = db::files::upsert_file(db, &db_conflict).await {
                error!(task_id, error = %e, "failed to persist conflict copy metadata");
            }
            info!(task_id, conflict_name, "conflict copy uploaded");
        }
        Err(e) => {
            error!(task_id, error = %e, "failed to upload conflict copy");
            // Even if upload failed, continue to download the remote version
            // so the user doesn't lose data.  The conflict copy remains on disk.
        }
    }

    // ── Download remote version to original path ───────────────────────────
    let tmp_path = format!("{}.gdriver-tmp", local_path);
    let download_resp = files::files_download(client, &db_record.id).await?;

    let mut file = tokio::fs::File::create(&tmp_path).await?;
    let mut stream = download_resp.bytes_stream();
    while let Some(chunk) = futures_util::StreamExt::next(&mut stream).await {
        let chunk = chunk?;
        tokio::io::AsyncWriteExt::write_all(&mut file, &chunk).await?;
    }
    tokio::io::AsyncWriteExt::flush(&mut file).await?;

    // Atomic rename into place
    tokio::fs::rename(&tmp_path, local_path).await?;

    // Update DB record for the downloaded remote version
    let mut updated_record = db_record.clone();
    updated_record.sync_state = "cached".into();
    updated_record.local_path = Some(local_path.to_string());

    // Update etag and version from the remote file
    if let Some(ref etag) = remote_file.etag {
        updated_record.etag = Some(etag.clone());
    }
    if let Some(ref ver) = remote_file.version {
        if let Ok(v) = ver.parse::<i64>() {
            updated_record.version = Some(v);
        }
    }
    // Update size from the downloaded content
    if let Ok(meta) = tokio::fs::metadata(local_path).await {
        updated_record.size = Some(meta.len() as i64);
        if let Ok(mtime) = meta.modified() {
            if let Ok(dur) = mtime.duration_since(std::time::UNIX_EPOCH) {
                updated_record.local_mtime = Some(dur.as_millis() as i64);
            }
        }
    }

    db::files::upsert_file(db, &updated_record).await?;

    // ── Mark the original task as completed ────────────────────────────────
    db::queue::update_task_status(db, task_id, "completed", Some("conflict resolved")).await?;

    // ── Push notification ──────────────────────────────────────────────────
    let notif = gdriver_ipc::Notification {
        id: 0, // Daemon sets a placeholder; the frontend ignores the id.
        account_id: Some(task.account_id.clone()),
        is_read: false,
        created_at: chrono::Utc::now().timestamp_millis(),
        kind: gdriver_ipc::NotificationKind::Conflict {
            file_id: Some(db_record.id.clone()),
            file_name: db_record.name.clone(),
            conflict_copy_name: conflict_name.clone(),
        },
    };

    let event = gdriver_ipc::PushEvent::NotificationNew(notif);
    match event.to_notification() {
        Ok(n) => {
            let json = match serde_json::to_string(&n) {
                Ok(s) => s,
                Err(e) => {
                    error!(task_id, error = %e, "failed to serialise conflict notification");
                    return Ok(());
                }
            };
            if let Err(e) = push_tx.send(json) {
                debug!(task_id, "conflict notification push dropped: {e}");
            }
        }
        Err(e) => {
            error!(task_id, error = %e, "failed to build conflict notification");
        }
    }

    info!(task_id, local_path, conflict_name, "conflict resolved");
    Ok(())
}

// ─── Resumable upload helper ──────────────────────────────────────────────────

async fn upload_resumable(
    client: &DriveClient,
    metadata: &CreateFileMetadata,
    content: &[u8],
    file_len: u64,
    task_id: i64,
) -> anyhow::Result<files::DriveFile> {
    debug!(task_id, size = file_len, "resumable upload (conflict copy)");

    let uri = files::files_upload_resumable_start(client, metadata).await?;

    let mut offset: u64 = 0;
    let mut result = None;

    while offset < file_len {
        let end = std::cmp::min(offset + CHUNK_SIZE, file_len) - 1;
        let chunk = &content[offset as usize..=end as usize];

        match files::files_upload_resumable_chunk(client, &uri, chunk, offset, end, file_len).await
        {
            Ok(UploadChunkResult::Incomplete { received }) => {
                offset = received;
            }
            Ok(UploadChunkResult::Complete(file)) => {
                result = Some(*file);
                break;
            }
            Err(e) => match files::files_upload_resumable_query(client, &uri, file_len).await {
                Ok((received, _)) => {
                    warn!(task_id, received, error = %e, "chunk failed; resuming");
                    offset = received;
                }
                Err(_) => return Err(e),
            },
        }
    }

    result.ok_or_else(|| anyhow::anyhow!("resumable upload loop exited without completion"))
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

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

    fn make_db_record(local_mtime: Option<i64>, etag: Option<&str>) -> db::files::DriveFile {
        db::files::DriveFile {
            id: "file-1".into(),
            account_id: "acct-1".into(),
            name: "test.txt".into(),
            mime_type: "text/plain".into(),
            parent_id: None,
            size: Some(100),
            etag: etag.map(String::from),
            version: Some(1),
            modified_time: Some(1_700_000_000_000),
            is_trashed: false,
            is_shared: false,
            local_path: Some("/tmp/test.txt".into()),
            sync_state: "synced".into(),
            local_mtime,
        }
    }

    #[test]
    fn no_conflict_when_local_unchanged() {
        // Local file not modified since last sync.
        let db = make_db_record(Some(1_700_000_000_000), Some("\"etag-1\""));
        assert!(!detect_conflict(1_700_000_000_000, &db, "\"etag-2\""));
    }

    #[test]
    fn no_conflict_when_remote_unchanged() {
        // Remote etag hasn't changed since last sync.
        let db = make_db_record(Some(1_700_000_000_000), Some("\"etag-1\""));
        assert!(!detect_conflict(1_700_000_001_000, &db, "\"etag-1\""));
    }

    #[test]
    fn conflict_when_both_changed() {
        let db = make_db_record(Some(1_700_000_000_000), Some("\"etag-1\""));
        assert!(detect_conflict(1_700_000_001_000, &db, "\"etag-2\""));
    }

    #[test]
    fn no_conflict_when_no_local_mtime() {
        // File was never synced locally (e.g. first upload).
        let db = make_db_record(None, Some("\"etag-1\""));
        assert!(!detect_conflict(1_700_000_001_000, &db, "\"etag-2\""));
    }

    #[test]
    fn no_conflict_when_no_cached_etag() {
        // No cached etag (file metadata was incomplete).
        let db = make_db_record(Some(1_700_000_000_000), None);
        assert!(!detect_conflict(1_700_000_001_000, &db, "\"etag-2\""));
    }

    #[test]
    fn no_conflict_when_neither_changed() {
        let db = make_db_record(Some(1_700_000_000_000), Some("\"etag-1\""));
        assert!(!detect_conflict(1_700_000_000_000, &db, "\"etag-1\""));
    }

    #[test]
    fn no_conflict_when_local_older() {
        // Local mtime is older than last sync (shouldn't happen, but handle gracefully).
        let db = make_db_record(Some(1_700_000_001_000), Some("\"etag-1\""));
        assert!(!detect_conflict(1_700_000_000_000, &db, "\"etag-2\""));
    }
}
