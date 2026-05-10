//! File deletion: handles "delete" sync tasks from two sources.
//!
//! - **Remote-originated** (`file_id` set): the file was trashed/deleted on
//!   Drive — remove the local copy and update the DB.
//! - **Local-originated** (`file_id` is `None`): the user deleted a local
//!   file — trash it on Drive via the API.

use std::path::Path;

use gdriver_api::{client::DriveClient, files};
use sqlx::SqlitePool;
use tracing::{debug, error, info, warn};

use crate::db;

/// Maximum number of retry attempts before logging a persistent error.
const MAX_RETRIES: i32 = 3;

/// Process a single delete task.
///
/// Returns `Ok(())` whether the deletion succeeded or is being retried.
pub async fn delete_file(
    db: &SqlitePool,
    client: &DriveClient,
    task: &db::queue::SyncTask,
) -> anyhow::Result<()> {
    let task_id = match task.id {
        Some(id) => id,
        None => {
            warn!("delete task has no id, skipping");
            return Ok(());
        }
    };

    // ── Mark in-progress ──────────────────────────────────────────────────
    db::queue::update_task_status(db, task_id, "in_progress", None).await?;

    if let Some(file_id) = task.file_id.as_deref() {
        // Remote-originated: file was trashed on Drive — delete local copy.
        handle_remote_delete(db, task, task_id, file_id).await
    } else if let Some(local_path) = task.local_path.as_deref() {
        // Local-originated: user deleted local file — trash on Drive.
        handle_local_delete(db, client, task, task_id, local_path).await
    } else {
        warn!(task_id, "delete task has neither file_id nor local_path, marking completed");
        db::queue::update_task_status(db, task_id, "completed", Some("no file_id or local_path"))
            .await?;
        Ok(())
    }
}

/// File was deleted/trashed on Drive — remove the local copy and update DB.
async fn handle_remote_delete(
    db: &SqlitePool,
    task: &db::queue::SyncTask,
    task_id: i64,
    file_id: &str,
) -> anyhow::Result<()> {
    // Look up the file to find its local_path.
    let drive_file = db::files::get_file_by_id(db, file_id, &task.account_id).await?;

    if let Some(ref df) = drive_file {
        // Remove the local file if it exists.
        if let Some(ref local_path) = df.local_path {
            let path = Path::new(local_path);
            if path.exists() {
                match tokio::fs::remove_file(path).await {
                    Ok(()) => debug!(task_id, path = %local_path, "local file removed"),
                    Err(e) => {
                        // Non-fatal: the file may already be gone.
                        warn!(task_id, error = %e, "failed to remove local file (continuing)");
                    }
                }
            }
        }

        // Clear local_path and update sync state.
        let mut updated = df.clone();
        updated.local_path = None;
        updated.local_mtime = None;
        updated.sync_state = "cloud_only".into();
        if let Err(e) = db::files::upsert_file(db, &updated).await {
            error!(task_id, error = %e, "failed to update file metadata after delete");
        }
    }

    db::queue::update_task_status(db, task_id, "completed", None).await?;
    info!(task_id, file_id, "remote delete completed");
    Ok(())
}

/// User deleted a local file — trash it on Google Drive.
async fn handle_local_delete(
    db: &SqlitePool,
    client: &DriveClient,
    task: &db::queue::SyncTask,
    task_id: i64,
    local_path: &str,
) -> anyhow::Result<()> {
    // Resolve the Drive file ID from the local path.
    let drive_file = db::files::get_file_by_local_path(db, local_path).await?;

    let file_id = match drive_file.as_ref().map(|f| f.id.as_str()) {
        Some(id) => id.to_string(),
        None => {
            // No DB record — file was never synced. Mark completed.
            info!(task_id, path = %local_path, "no DB record for deleted local file, marking completed");
            db::queue::update_task_status(db, task_id, "completed", Some("no DB record"))
                .await?;
            return Ok(());
        }
    };

    // Trash on Drive.
    match files::files_delete(client, &file_id).await {
        Ok(()) => {
            // Mark trashed in local DB.
            if let Some(ref df) = drive_file {
                if let Err(e) = db::files::mark_trashed(db, &df.id, &df.account_id).await {
                    error!(task_id, error = %e, "failed to mark file trashed in DB");
                }
            }

            db::queue::update_task_status(db, task_id, "completed", None).await?;
            info!(task_id, file_id, path = %local_path, "local delete: trashed on Drive");
            Ok(())
        }
        Err(e) => {
            handle_delete_failure(db, task, task_id, &format!("{e:#}")).await
        }
    }
}

// ─── Failure handling ──────────────────────────────────────────────────────────

async fn handle_delete_failure(
    db: &SqlitePool,
    task: &db::queue::SyncTask,
    task_id: i64,
    error_msg: &str,
) -> anyhow::Result<()> {
    let new_retry = task.retry_count + 1;

    if new_retry > MAX_RETRIES {
        warn!(task_id, retries = new_retry, %error_msg, "delete permanently failed");

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
            error_code: "DELETE_FAILED".into(),
            error_msg: error_msg.to_string(),
            is_resolved: false,
            created_at: now,
        };
        if let Err(e) = db::sync_errors::insert_error(db, &sync_error).await {
            error!(task_id, error = %e, "failed to record sync error");
        }
    } else {
        debug!(task_id, retries = new_retry, %error_msg, "delete failed, re-queuing for retry");
        db::queue::update_task_retry(db, task_id, "pending", new_retry, Some(error_msg)).await?;
    }

    Ok(())
}
