//! File download: streams a Drive file to a local temp file and atomically
//! renames it to the target path.
//!
//! Strategy:
//! - Regular files: `files_download` (GET with `alt=media`).
//! - Google Workspace files: `files_export` to an appropriate format.
//! - Temp file written alongside the target, then `rename` for atomicity.

use std::path::Path;

use gdriver_api::{client::DriveClient, files};
use sqlx::SqlitePool;
use tokio::io::AsyncWriteExt;
use tracing::{debug, error, info, warn};

use crate::db;

/// Maximum number of retry attempts before logging a persistent error.
const MAX_RETRIES: i32 = 3;

/// Process a single download task.
///
/// Returns `Ok(())` whether the download succeeded or is being retried.
pub async fn download_file(
    db: &SqlitePool,
    client: &DriveClient,
    task: &db::queue::SyncTask,
) -> anyhow::Result<()> {
    let task_id = match task.id {
        Some(id) => id,
        None => {
            warn!("download task has no id, skipping");
            return Ok(());
        }
    };

    let file_id = match task.file_id.as_deref() {
        Some(id) => id,
        None => {
            warn!(task_id, "download task has no file_id, marking completed");
            db::queue::update_task_status(db, task_id, "completed", Some("no file_id")).await?;
            return Ok(());
        }
    };

    let local_path = match task.local_path.as_deref() {
        Some(p) => p,
        None => {
            warn!(
                task_id,
                "download task has no local_path, marking completed"
            );
            db::queue::update_task_status(db, task_id, "completed", Some("no local_path")).await?;
            return Ok(());
        }
    };

    // ── Mark in-progress ──────────────────────────────────────────────────
    db::queue::update_task_status(db, task_id, "in_progress", None).await?;

    // ── Look up the file metadata for mime type ───────────────────────────
    let drive_file = db::files::get_file_by_id(db, file_id, &task.account_id).await?;
    let mime_type = drive_file
        .as_ref()
        .map(|f| f.mime_type.as_str())
        .unwrap_or("application/octet-stream");

    let path = Path::new(local_path);

    // ── Ensure parent directory exists ────────────────────────────────────
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            tokio::fs::create_dir_all(parent).await?;
        }
    }

    // ── Download to temp file ─────────────────────────────────────────────
    let tmp_path = format!("{}.gdriver-tmp", local_path);

    let result = if is_google_workspace_mime(mime_type) {
        let export_mime = export_mime_for(mime_type);
        do_export(client, file_id, export_mime, &tmp_path, task_id).await
    } else {
        do_download(client, file_id, &tmp_path, task_id).await
    };

    match result {
        Ok(()) => {
            // ── Atomic rename ──────────────────────────────────────────
            if let Err(e) = tokio::fs::rename(&tmp_path, local_path).await {
                // Clean up the temp file on rename failure.
                let _ = tokio::fs::remove_file(&tmp_path).await;
                return handle_download_failure(db, task, task_id, &format!("rename failed: {e}"))
                    .await;
            }

            // ── Update file metadata ───────────────────────────────────
            if let Some(ref df) = drive_file {
                let mut updated = df.clone();
                updated.sync_state = "cached".into();
                updated.local_path = Some(local_path.to_string());

                // Set local mtime from the downloaded file.
                if let Ok(meta) = tokio::fs::metadata(local_path).await {
                    if let Ok(mtime) = meta.modified() {
                        if let Ok(dur) = mtime.duration_since(std::time::UNIX_EPOCH) {
                            updated.local_mtime = Some(dur.as_millis() as i64);
                        }
                    }
                }

                if let Err(e) = db::files::upsert_file(db, &updated).await {
                    error!(task_id, error = %e, "failed to update file metadata after download");
                }
            }

            db::queue::update_task_status(db, task_id, "completed", None).await?;

            info!(
                task_id,
                file_id,
                path = %local_path,
                "download completed"
            );
        }
        Err(e) => {
            // Clean up temp file on failure.
            let _ = tokio::fs::remove_file(&tmp_path).await;
            handle_download_failure(db, task, task_id, &format!("{e:#}")).await?;
        }
    }

    Ok(())
}

// ─── Download strategies ───────────────────────────────────────────────────────

async fn do_download(
    client: &DriveClient,
    file_id: &str,
    tmp_path: &str,
    task_id: i64,
) -> anyhow::Result<()> {
    debug!(task_id, file_id, "streaming download");

    let resp = files::files_download(client, file_id).await?;
    let total = resp.content_length();

    let mut file = tokio::fs::File::create(tmp_path).await?;
    let mut stream = resp.bytes_stream();
    let mut downloaded: u64 = 0;

    while let Some(chunk) = futures_util::StreamExt::next(&mut stream).await {
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;

        if let Some(total) = total {
            debug!(task_id, downloaded, total, "download progress");
        }
    }

    file.flush().await?;
    Ok(())
}

async fn do_export(
    client: &DriveClient,
    file_id: &str,
    export_mime: &str,
    tmp_path: &str,
    task_id: i64,
) -> anyhow::Result<()> {
    debug!(
        task_id,
        file_id, export_mime, "exporting Google Workspace file"
    );

    let resp = files::files_export(client, file_id, export_mime).await?;
    let total = resp.content_length();

    let mut file = tokio::fs::File::create(tmp_path).await?;
    let mut stream = resp.bytes_stream();
    let mut downloaded: u64 = 0;

    while let Some(chunk) = futures_util::StreamExt::next(&mut stream).await {
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;

        if let Some(total) = total {
            debug!(task_id, downloaded, total, "export progress");
        }
    }

    file.flush().await?;
    Ok(())
}

// ─── Failure handling ──────────────────────────────────────────────────────────

async fn handle_download_failure(
    db: &SqlitePool,
    task: &db::queue::SyncTask,
    task_id: i64,
    error_msg: &str,
) -> anyhow::Result<()> {
    let new_retry = task.retry_count + 1;

    if new_retry > MAX_RETRIES {
        warn!(task_id, retries = new_retry, %error_msg, "download permanently failed");

        db::queue::update_task_status(db, task_id, "failed", Some(error_msg)).await?;

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
            error_code: "DOWNLOAD_FAILED".into(),
            error_msg: error_msg.to_string(),
            is_resolved: false,
            created_at: now,
        };
        if let Err(e) = db::sync_errors::insert_error(db, &sync_error).await {
            error!(task_id, error = %e, "failed to record sync error");
        }
    } else {
        debug!(task_id, retries = new_retry, %error_msg, "download failed, re-queuing for retry");
        db::queue::update_task_retry(db, task_id, "pending", new_retry, Some(error_msg)).await?;
    }

    Ok(())
}

// ─── Helpers ───────────────────────────────────────────────────────────────────

/// Return `true` if the MIME type is a native Google Workspace format that must
/// be exported rather than downloaded directly.
fn is_google_workspace_mime(mime_type: &str) -> bool {
    matches!(
        mime_type,
        "application/vnd.google-apps.document"
            | "application/vnd.google-apps.spreadsheet"
            | "application/vnd.google-apps.presentation"
            | "application/vnd.google-apps.drawing"
            | "application/vnd.google-apps.script"
    )
}

/// Map a Google Workspace MIME type to its most common export format.
fn export_mime_for(workspace_mime: &str) -> &str {
    match workspace_mime {
        "application/vnd.google-apps.document" => {
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        }
        "application/vnd.google-apps.spreadsheet" => {
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        }
        "application/vnd.google-apps.presentation" => {
            "application/vnd.openxmlformats-officedocument.presentationml.presentation"
        }
        "application/vnd.google-apps.drawing" => "image/svg+xml",
        "application/vnd.google-apps.script" => "application/json",
        _ => "application/pdf",
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_detection() {
        assert!(is_google_workspace_mime(
            "application/vnd.google-apps.document"
        ));
        assert!(is_google_workspace_mime(
            "application/vnd.google-apps.spreadsheet"
        ));
        assert!(is_google_workspace_mime(
            "application/vnd.google-apps.presentation"
        ));
        assert!(is_google_workspace_mime(
            "application/vnd.google-apps.drawing"
        ));
        assert!(is_google_workspace_mime(
            "application/vnd.google-apps.script"
        ));
    }

    #[test]
    fn regular_files_not_workspace() {
        assert!(!is_google_workspace_mime("text/plain"));
        assert!(!is_google_workspace_mime("application/pdf"));
        assert!(!is_google_workspace_mime("image/png"));
        assert!(!is_google_workspace_mime(
            "application/vnd.google-apps.folder"
        ));
        assert!(!is_google_workspace_mime("application/octet-stream"));
    }

    #[test]
    fn export_mime_types() {
        assert_eq!(
            export_mime_for("application/vnd.google-apps.document"),
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        );
        assert_eq!(
            export_mime_for("application/vnd.google-apps.spreadsheet"),
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        );
        assert_eq!(
            export_mime_for("application/vnd.google-apps.presentation"),
            "application/vnd.openxmlformats-officedocument.presentationml.presentation"
        );
        assert_eq!(
            export_mime_for("application/vnd.google-apps.drawing"),
            "image/svg+xml"
        );
        assert_eq!(
            export_mime_for("application/vnd.google-apps.script"),
            "application/json"
        );
    }

    #[test]
    fn unknown_workspace_type_falls_back_to_pdf() {
        assert_eq!(
            export_mime_for("application/vnd.google-apps.unknown"),
            "application/pdf"
        );
    }
}
