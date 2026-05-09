//! Google Photos backup: reads a local media file and uploads it to Google Photos
//! via the two-step Photos Library API flow (upload bytes → batchCreate).
//!
//! Each file modification generates a **new** Photos entry (no in-place update),
//! matching the requirement that edits produce new media items.

use std::path::Path;

use sqlx::SqlitePool;
use tracing::{debug, error, info, warn};

use gdriver_api::client::DriveClient;
use gdriver_api::photos::{self, BatchCreateRequest, NewMediaItem, SimpleMediaItem};

use crate::db;
use crate::ipc::PushSender;
use gdriver_ipc::{PushEvent, SyncItem, SyncState};

/// Maximum number of retry attempts before marking the task as permanently failed.
const MAX_RETRIES: i32 = 3;

/// Process a single Photos backup task.
///
/// Uploads the local file to Google Photos and pushes a `sync:item-updated`
/// event so the UI can reflect progress.  Returns `Ok(())` in all cases —
/// failures are recorded on the task row itself.
pub async fn backup_photo(
    db: &SqlitePool,
    client: &DriveClient,
    push_tx: &PushSender,
    task: &db::queue::SyncTask,
) -> anyhow::Result<()> {
    let task_id = match task.id {
        Some(id) => id,
        None => {
            warn!("photos_backup task has no id, skipping");
            return Ok(());
        }
    };

    let local_path = match task.local_path.as_deref() {
        Some(p) => p,
        None => {
            warn!(task_id, "photos_backup task has no local_path, marking completed");
            db::queue::update_task_status(db, task_id, "completed", Some("no local_path"))
                .await?;
            return Ok(());
        }
    };

    let path = Path::new(local_path);

    // Validate format before attempting upload.
    if !photos::is_supported_photo_format(path) {
        let msg = format!("unsupported photo format: {}", path.display());
        warn!(task_id, %msg);
        return handle_failure(db, task, task_id, push_tx, &msg).await;
    }

    // Mark in-progress.
    db::queue::update_task_status(db, task_id, "in_progress", None).await?;

    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "Untitled".into());

    // Push an "uploading" event so the UI shows progress.
    push_item_event(push_tx, &file_name, local_path, SyncState::Uploading, None, None);

    // ── Step 1: Upload bytes → upload token ──────────────────────────────
    let upload_token = match photos::upload_media_item_from_path(client, path).await {
        Ok(token) => token,
        Err(e) => {
            let msg = format!("upload failed: {e:#}");
            error!(task_id, %msg);
            return handle_failure(db, task, task_id, push_tx, &msg).await;
        }
    };

    debug!(task_id, token_len = upload_token.len(), "upload token obtained");

    // ── Step 2: batchCreate → media item ─────────────────────────────────
    let request = BatchCreateRequest {
        album_id: None,
        new_media_items: vec![NewMediaItem {
            description: None,
            simple_media_item: SimpleMediaItem {
                upload_token,
                file_name: Some(file_name.clone()),
            },
        }],
    };

    let response = match photos::batch_create(client, &request).await {
        Ok(resp) => resp,
        Err(e) => {
            let msg = format!("batchCreate failed: {e:#}");
            error!(task_id, %msg);
            return handle_failure(db, task, task_id, push_tx, &msg).await;
        }
    };

    // ── Evaluate result ──────────────────────────────────────────────────
    if let Some(result) = response.new_media_item_results.first() {
        if let Some(ref status) = result.status {
            if status.code == Some(0) {
                // Success — push a "synced" event.
                let drive_url = result
                    .media_item
                    .as_ref()
                    .and_then(|m| m.product_url.clone());

                push_item_event(
                    push_tx,
                    &file_name,
                    local_path,
                    SyncState::Synced,
                    None,
                    drive_url,
                );

                db::queue::update_task_status(db, task_id, "completed", None).await?;

                info!(
                    task_id,
                    name = %file_name,
                    media_item_id = ?result.media_item.as_ref().and_then(|m| m.id.as_deref()),
                    "photos backup completed"
                );
            } else {
                let msg = format!(
                    "Photos API error: {}",
                    status.message.as_deref().unwrap_or("unknown")
                );
                error!(task_id, code = status.code, %msg);
                return handle_failure(db, task, task_id, push_tx, &msg).await;
            }
        } else {
            // No status — treat as success if media_item is present.
            if result.media_item.is_some() {
                db::queue::update_task_status(db, task_id, "completed", None).await?;
                info!(task_id, name = %file_name, "photos backup completed (no status)");
            } else {
                let msg = "batchCreate returned no status and no mediaItem";
                error!(task_id, %msg);
                return handle_failure(db, task, task_id, push_tx, msg).await;
            }
        }
    } else {
        let msg = "batchCreate returned empty results";
        error!(task_id, %msg);
        return handle_failure(db, task, task_id, push_tx, msg).await;
    }

    Ok(())
}

// ─── Failure handling ────────────────────────────────────────────────────────

async fn handle_failure(
    db: &SqlitePool,
    task: &db::queue::SyncTask,
    task_id: i64,
    push_tx: &PushSender,
    error_msg: &str,
) -> anyhow::Result<()> {
    let file_name = task
        .local_path
        .as_deref()
        .and_then(|p| Path::new(p).file_name())
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".into());

    let local_path = task.local_path.clone().unwrap_or_default();

    // Push an error event.
    push_item_event(
        push_tx,
        &file_name,
        &local_path,
        SyncState::Error,
        Some(error_msg.to_string()),
        None,
    );

    let new_retry = task.retry_count + 1;
    if new_retry > MAX_RETRIES {
        warn!(task_id, retries = new_retry, %error_msg, "photos backup permanently failed");
        db::queue::update_task_status(db, task_id, "failed", Some(error_msg)).await?;
    } else {
        debug!(task_id, retries = new_retry, %error_msg, "photos backup failed, re-queuing");
        db::queue::update_task_retry(db, task_id, "pending", new_retry, Some(error_msg)).await?;
    }

    Ok(())
}

// ─── Event helpers ───────────────────────────────────────────────────────────

fn push_item_event(
    push_tx: &PushSender,
    name: &str,
    local_path: &str,
    sync_state: SyncState,
    error_msg: Option<String>,
    drive_url: Option<String>,
) {
    let item = SyncItem {
        file_id: None,
        name: name.to_string(),
        mime_type: None,
        local_path: Some(local_path.to_string()),
        sync_state,
        progress: None,
        file_size: None,
        error_msg,
        drive_url,
        updated_at: chrono::Utc::now().timestamp_millis(),
    };

    let event = PushEvent::SyncItemUpdated(item);
    let notif = match event.to_notification() {
        Ok(n) => n,
        Err(e) => {
            error!("failed to serialise SyncItemUpdated: {e}");
            return;
        }
    };
    let json = match serde_json::to_string(&notif) {
        Ok(s) => s,
        Err(e) => {
            error!("failed to serialise push notification: {e}");
            return;
        }
    };
    if let Err(e) = push_tx.send(json) {
        debug!("SyncItemUpdated push dropped (no connected clients): {e}");
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_item_event_does_not_panic_without_receivers() {
        let (tx, _rx) = tokio::sync::broadcast::channel::<String>(1);
        // Drop the receiver so send() returns Err.
        drop(_rx);

        // Should not panic — just logs a debug message.
        push_item_event(
            &tx,
            "test.jpg",
            "/tmp/test.jpg",
            SyncState::Uploading,
            None,
            None,
        );
    }

    #[test]
    fn push_item_event_with_error_state() {
        let (tx, mut rx) = tokio::sync::broadcast::channel::<String>(16);

        push_item_event(
            &tx,
            "bad.jpg",
            "/tmp/bad.jpg",
            SyncState::Error,
            Some("upload failed".into()),
            None,
        );

        // Verify the event was sent.
        let msg = rx.try_recv().unwrap();
        assert!(msg.contains("sync:item-updated"));
        assert!(msg.contains("error"));
        assert!(msg.contains("upload failed"));
    }

    #[test]
    fn push_item_event_with_synced_state() {
        let (tx, mut rx) = tokio::sync::broadcast::channel::<String>(16);

        push_item_event(
            &tx,
            "photo.jpg",
            "/tmp/photo.jpg",
            SyncState::Synced,
            None,
            Some("https://photos.google.com/abc".into()),
        );

        let msg = rx.try_recv().unwrap();
        assert!(msg.contains("synced"));
        assert!(msg.contains("photos.google.com"));
    }
}
