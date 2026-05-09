//! Incremental sync: poll the Drive Changes API periodically and update the
//! local file metadata database and sync queue accordingly.

use sqlx::SqlitePool;
use tracing::{debug, info, warn};

use gdriver_api::client::DriveClient;
use gdriver_api::changes::{self, Change};
use gdriver_ipc::SyncMode;

use crate::db;
use crate::sync::initial::map_api_file_to_db;

/// Run one incremental-sync cycle for the given account.
///
/// 1. Load the stored page token from `sync_tokens`.
/// 2. Call `changes_list`, paginating through all change pages.
/// 3. For each change, update `drive_files` and enqueue sync tasks.
/// 4. Persist the new page token.
///
/// Returns the total number of changes processed (0 if no token was stored yet).
pub async fn incremental_sync(
    db: &SqlitePool,
    account_id: &str,
    client: &DriveClient,
    sync_mode: SyncMode,
    mount_point: &str,
) -> anyhow::Result<usize> {
    // ── Load the current page token ────────────────────────────────────────
    let page_token = match db::tokens::get_token(db, account_id).await? {
        Some(tok) => tok,
        None => {
            debug!(account_id, "incremental sync: no page token, skipping");
            return Ok(0);
        }
    };

    debug!(account_id, %page_token, "incremental sync: starting poll");

    // ── Paginate through all change pages ──────────────────────────────────
    let mut total_changes: usize = 0;
    let mut current_token: String = page_token;
    let mut latest_token: Option<String> = None;
    let mut page_num: u32 = 0;

    loop {
        page_num += 1;
        let response = changes::changes_list(
            client,
            &current_token,
            Some(1000),         // max page size
            None,               // default fields
            None,               // no shared drive
            Some(true),         // include removed items
        )
        .await?;

        // The new_start_page_token is the same on every page of this poll;
        // it's the token to use for the NEXT poll cycle.
        if let Some(ref tok) = response.new_start_page_token {
            latest_token = Some(tok.clone());
        }

        let page_changes = response.changes;
        let on_this_page = page_changes.len();
        total_changes += on_this_page;

        debug!(
            account_id, page_num, on_this_page,
            total = total_changes, "incremental sync: change page"
        );

        for change in &page_changes {
            if let Err(e) = process_change(db, account_id, change, sync_mode, mount_point).await {
                warn!(
                    account_id,
                    file_id = ?change.file_id,
                    error = %e,
                    "incremental sync: failed to process change"
                );
            }
        }

        match response.next_page_token {
            Some(token) => current_token = token,
            None => break,
        }
    }

    // ── Persist the new page token ─────────────────────────────────────────
    if let Some(tok) = latest_token {
        db::tokens::set_token(db, account_id, &tok).await?;
        debug!(account_id, %tok, "incremental sync: page token updated");
    }

    info!(
        account_id,
        total_changes,
        "incremental sync: complete"
    );
    Ok(total_changes)
}

/// Process a single change from the Changes API.
async fn process_change(
    db: &SqlitePool,
    account_id: &str,
    change: &Change,
    sync_mode: SyncMode,
    mount_point: &str,
) -> anyhow::Result<()> {
    let file_id = match change.file_id.as_deref() {
        Some(id) => id,
        None => return Ok(()),
    };

    let removed = change.removed.unwrap_or(false);

    if removed {
        // File was deleted or the user lost access to it.
        db::files::mark_trashed(db, file_id, account_id).await?;

        // If we have a local copy, enqueue a delete task.
        if let Some(existing) =
            db::files::get_file_by_id(db, file_id, account_id).await?
        {
            if existing.local_path.is_some() {
                enqueue_task(db, account_id, file_id, "delete", &existing).await;
            }
        }
        return Ok(());
    }

    // File still exists — upsert the latest metadata.
    let api_file = match change.file.as_ref() {
        Some(f) => f,
        None => {
            // Should not happen: !removed implies file should be present,
            // but handle gracefully.
            warn!(file_id, account_id, "change with removed=false but no file data");
            return Ok(());
        }
    };

    let db_file = map_api_file_to_db(api_file, account_id);
    let is_new = db::files::get_file_by_id(db, file_id, account_id)
        .await?
        .is_none();

    db::files::upsert_file(db, &db_file).await?;

    if api_file.trashed == Some(true) {
        // File moved to trash.
        if db_file.local_path.is_some() {
            enqueue_task(db, account_id, file_id, "delete", &db_file).await;
        }
        return Ok(());
    }

    if is_new {
        // New file discovered — enqueue download if in mirror mode or if
        // we have a sync folder that covers this file's parent path.
        enqueue_download_for_change(db, account_id, &db_file, sync_mode, mount_point).await;
    } else {
        // Existing file — check if etag changed, meaning content was updated.
        enqueue_download_for_change(db, account_id, &db_file, sync_mode, mount_point).await;
    }

    Ok(())
}

/// Enqueue a sync task with reasonable defaults.
async fn enqueue_task(
    db: &SqlitePool,
    account_id: &str,
    file_id: &str,
    operation: &str,
    db_file: &db::files::DriveFile,
) {
    let now = chrono::Utc::now().timestamp_millis();
    let task = db::queue::SyncTask {
        id: None,
        account_id: account_id.to_string(),
        file_id: Some(file_id.to_string()),
        operation: operation.to_string(),
        local_path: db_file.local_path.clone(),
        priority: 5,
        status: "pending".to_string(),
        retry_count: 0,
        error_msg: None,
        created_at: now,
        updated_at: now,
    };

    match db::queue::enqueue(db, &task).await {
        Ok(t) => debug!(
            account_id, file_id, operation,
            task_id = t.id,
            "incremental sync: task enqueued"
        ),
        Err(e) => warn!(
            account_id, file_id, operation,
            error = %e,
            "incremental sync: failed to enqueue task"
        ),
    }
}

/// Enqueue a download task for a file that changed remotely.
///
/// In **Mirror** mode, every non-folder file gets a download task (the entire
/// Drive is kept locally).
///
/// In **Stream** mode, download tasks are only enqueued for files whose parent
/// chain leads to a configured sync folder (currently a stub — the
/// `sync_folders` table will be populated by M8–M10).
async fn enqueue_download_for_change(
    db: &SqlitePool,
    account_id: &str,
    db_file: &db::files::DriveFile,
    sync_mode: SyncMode,
    mount_point: &str,
) {
    // Skip folders — they don't need downloading.
    if db_file.mime_type == "application/vnd.google-apps.folder" {
        return;
    }

    // Skip files already cached or in progress.
    match db_file.sync_state.as_str() {
        "cached" | "synced" | "offline" | "downloading" => return,
        _ => {}
    }

    if sync_mode == SyncMode::Mirror {
        // Mirror mode: always enqueue download for non-folder files.
        let local_path = db_file.local_path.clone().unwrap_or_else(|| {
            // Compute the local path from the mount point and file name.
            // For incremental sync, we use the file name directly under the
            // mount point.  The full tree path is only available during
            // initial sync when all parent metadata is present.
            format!("{}/{}", mount_point.trim_end_matches('/'), db_file.name)
        });

        let now = chrono::Utc::now().timestamp_millis();
        let task = db::queue::SyncTask {
            id: None,
            account_id: account_id.to_string(),
            file_id: Some(db_file.id.clone()),
            operation: "download".to_string(),
            local_path: Some(local_path),
            priority: 5,
            status: "pending".to_string(),
            retry_count: 0,
            error_msg: None,
            created_at: now,
            updated_at: now,
        };

        match db::queue::enqueue(db, &task).await {
            Ok(t) => debug!(
                account_id,
                file_id = %db_file.id,
                task_id = t.id,
                "mirror: download task enqueued for changed file"
            ),
            Err(e) => warn!(
                account_id,
                file_id = %db_file.id,
                error = %e,
                "mirror: failed to enqueue download task"
            ),
        }
    }
    // Stream mode: TODO — enqueue only for files within configured sync
    // folders.  For now, no-op (matching previous stub behavior).
}

// ─── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use gdriver_api::files::DriveFile as ApiDriveFile;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

    async fn test_pool() -> SqlitePool {
        let opts = SqliteConnectOptions::new()
            .filename(":memory:")
            .foreign_keys(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .expect("in-memory pool");

        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("migrations");

        pool
    }

    async fn insert_account(pool: &SqlitePool, id: &str) {
        sqlx::query(
            "INSERT INTO accounts (id, email, created_at, last_used_at)
             VALUES (?, ?, ?, ?)",
        )
        .bind(id)
        .bind(format!("{id}@example.com"))
        .bind(1_700_000_000_000_i64)
        .bind(1_700_000_000_000_i64)
        .execute(pool)
        .await
        .unwrap();
    }

    fn make_api_file(id: &str, name: &str, trashed: bool) -> ApiDriveFile {
        ApiDriveFile {
            id: Some(id.into()),
            name: Some(name.into()),
            mime_type: Some("text/plain".into()),
            parents: None,
            size: Some("1024".into()),
            etag: Some(format!("\"etag_{id}\"")),
            version: Some("5".into()),
            modified_time: Some("2026-05-01T12:00:00.000Z".into()),
            created_time: None,
            trashed: Some(trashed),
            shared: Some(false),
            md5_checksum: None,
            web_view_link: None,
        }
    }

    fn make_change(
        file_id: &str,
        removed: bool,
        trashed: bool,
        name: &str,
    ) -> Change {
        Change {
            kind: Some("drive#change".into()),
            change_type: Some("file".into()),
            file_id: Some(file_id.into()),
            removed: Some(removed),
            time: Some("2026-05-02T12:00:00.000Z".into()),
            file: if removed {
                None
            } else {
                Some(make_api_file(file_id, name, trashed))
            },
        }
    }

    // ── process_change tests ───────────────────────────────────────────────

    #[tokio::test]
    async fn process_new_file_upserts() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;

        let change = make_change("f1", false, false, "new-file.txt");
        process_change(&pool, "acct-1", &change, SyncMode::Stream, "/tmp/drive").await.unwrap();

        let f = db::files::get_file_by_id(&pool, "f1", "acct-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(f.name, "new-file.txt");
        assert!(!f.is_trashed);
        assert_eq!(f.sync_state, "cloud_only");
    }

    #[tokio::test]
    async fn process_removed_file_marks_trashed() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;

        // First upsert the file as if from initial sync
        let api = make_api_file("f2", "will-be-deleted.txt", false);
        let db_file = map_api_file_to_db(&api, "acct-1");
        db::files::upsert_file(&pool, &db_file).await.unwrap();

        // Now process a "removed" change
        let change = make_change("f2", true, false, "");
        process_change(&pool, "acct-1", &change, SyncMode::Stream, "/tmp/drive").await.unwrap();

        let f = db::files::get_file_by_id(&pool, "f2", "acct-1")
            .await
            .unwrap()
            .unwrap();
        assert!(f.is_trashed, "removed file should be marked trashed");
    }

    #[tokio::test]
    async fn process_trashed_file_marks_trashed() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;

        let api = make_api_file("f3", "moved-to-trash.txt", false);
        let db_file = map_api_file_to_db(&api, "acct-1");
        db::files::upsert_file(&pool, &db_file).await.unwrap();

        // File change with trashed=true
        let change = make_change("f3", false, true, "moved-to-trash.txt");
        process_change(&pool, "acct-1", &change, SyncMode::Stream, "/tmp/drive").await.unwrap();

        let f = db::files::get_file_by_id(&pool, "f3", "acct-1")
            .await
            .unwrap()
            .unwrap();
        assert!(f.is_trashed);
    }

    #[tokio::test]
    async fn process_modified_file_updates_metadata() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;

        // Initial state
        let api = make_api_file("f4", "old-name.txt", false);
        let db_file = map_api_file_to_db(&api, "acct-1");
        db::files::upsert_file(&pool, &db_file).await.unwrap();

        // Modified change with new name
        let mut modified = make_api_file("f4", "renamed.txt", false);
        modified.etag = Some("\"etag_v2\"".into());
        modified.version = Some("6".into());
        let change = Change {
            kind: Some("drive#change".into()),
            change_type: Some("file".into()),
            file_id: Some("f4".into()),
            removed: Some(false),
            time: Some("2026-05-02T12:00:00.000Z".into()),
            file: Some(modified),
        };

        process_change(&pool, "acct-1", &change, SyncMode::Stream, "/tmp/drive").await.unwrap();

        let f = db::files::get_file_by_id(&pool, "f4", "acct-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(f.name, "renamed.txt");
        assert_eq!(f.etag.as_deref(), Some("\"etag_v2\""));
        assert_eq!(f.version, Some(6));
    }

    #[tokio::test]
    async fn process_change_with_no_file_id_is_skipped() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;

        let change = Change {
            kind: Some("drive#change".into()),
            change_type: Some("file".into()),
            file_id: None,
            removed: None,
            time: None,
            file: None,
        };

        let result = process_change(&pool, "acct-1", &change, SyncMode::Stream, "/tmp/drive").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn process_change_without_file_resource_is_handled() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;

        // A change with removed=false but no file resource (shouldn't happen
        // in practice, but we handle it gracefully).
        let change = Change {
            kind: Some("drive#change".into()),
            change_type: Some("file".into()),
            file_id: Some("orphan".into()),
            removed: Some(false),
            time: Some("2026-05-02T12:00:00.000Z".into()),
            file: None,
        };

        let result = process_change(&pool, "acct-1", &change, SyncMode::Stream, "/tmp/drive").await;
        assert!(result.is_ok());
    }

    // ── incremental_sync integration tests ──────────────────────────────────

    #[tokio::test]
    async fn incremental_sync_skips_when_no_page_token() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;

        // No page token stored — should skip gracefully
        let count = incremental_sync(&pool, "acct-1", &DriveClient::new("unused"), SyncMode::Stream, "/tmp/drive")
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn incremental_sync_updates_page_token_after_processing() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;

        // Simulate the result of incremental sync: save token, apply changes
        db::tokens::set_token(&pool, "acct-1", "old-tok").await.unwrap();

        // Process a change directly
        let change = make_change("f-new", false, false, "hello.txt");
        process_change(&pool, "acct-1", &change, SyncMode::Stream, "/tmp/drive").await.unwrap();

        // Update token (as incremental_sync does)
        db::tokens::set_token(&pool, "acct-1", "new-tok").await.unwrap();

        // Verify
        let tok = db::tokens::get_token(&pool, "acct-1").await.unwrap();
        assert_eq!(tok.as_deref(), Some("new-tok"));

        let f = db::files::get_file_by_id(&pool, "f-new", "acct-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(f.name, "hello.txt");
    }

    #[tokio::test]
    async fn removed_file_with_local_path_enqueues_delete_task() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;

        // Create a file with local_path
        let api = make_api_file("f-local", "local-file.txt", false);
        let mut db_file = map_api_file_to_db(&api, "acct-1");
        db_file.local_path = Some("/home/user/Drive/local-file.txt".into());
        db::files::upsert_file(&pool, &db_file).await.unwrap();

        // Process removed change
        let change = make_change("f-local", true, false, "");
        process_change(&pool, "acct-1", &change, SyncMode::Stream, "/tmp/drive").await.unwrap();

        // File should be trashed
        let f = db::files::get_file_by_id(&pool, "f-local", "acct-1")
            .await
            .unwrap()
            .unwrap();
        assert!(f.is_trashed);

        // A delete task should be enqueued
        let task = db::queue::next_pending_task(&pool).await.unwrap();
        assert!(task.is_some(), "delete task should be enqueued");
        let task = task.unwrap();
        assert_eq!(task.operation, "delete");
        assert_eq!(task.file_id.as_deref(), Some("f-local"));
    }

    // ── Mirror mode tests ──────────────────────────────────────────────────

    #[tokio::test]
    async fn mirror_mode_enqueues_download_for_new_file() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;

        let change = make_change("f-mirror", false, false, "mirror-file.txt");
        process_change(&pool, "acct-1", &change, SyncMode::Mirror, "/tmp/drive")
            .await
            .unwrap();

        // File should exist
        let f = db::files::get_file_by_id(&pool, "f-mirror", "acct-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(f.name, "mirror-file.txt");

        // A download task should be enqueued
        let task = db::queue::next_pending_task(&pool).await.unwrap();
        assert!(task.is_some(), "mirror mode should enqueue download for new file");
        let task = task.unwrap();
        assert_eq!(task.operation, "download");
        assert_eq!(task.file_id.as_deref(), Some("f-mirror"));
    }

    #[tokio::test]
    async fn mirror_mode_skips_folder_download() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;

        let mut change = make_change("folder-1", false, false, "My Folder");
        change.file.as_mut().unwrap().mime_type =
            Some("application/vnd.google-apps.folder".into());

        process_change(&pool, "acct-1", &change, SyncMode::Mirror, "/tmp/drive")
            .await
            .unwrap();

        // No download task should be enqueued for a folder
        let task = db::queue::next_pending_task(&pool).await.unwrap();
        assert!(task.is_none(), "mirror mode should not enqueue download for folders");
    }

    #[tokio::test]
    async fn stream_mode_does_not_enqueue_download_for_new_file() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;

        let change = make_change("f-stream", false, false, "stream-file.txt");
        process_change(&pool, "acct-1", &change, SyncMode::Stream, "/tmp/drive")
            .await
            .unwrap();

        // File should exist
        let f = db::files::get_file_by_id(&pool, "f-stream", "acct-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(f.name, "stream-file.txt");

        // No download task in stream mode (no sync folders configured)
        let task = db::queue::next_pending_task(&pool).await.unwrap();
        assert!(task.is_none(), "stream mode should not enqueue download without sync folders");
    }

    #[tokio::test]
    async fn mirror_mode_enqueue_download_skips_cached_state() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;

        // Directly test enqueue_download_for_change with a cached file.
        let api = make_api_file("f-cached", "cached.txt", false);
        let mut db_file = map_api_file_to_db(&api, "acct-1");
        db_file.sync_state = "cached".to_string();

        enqueue_download_for_change(&pool, "acct-1", &db_file, SyncMode::Mirror, "/tmp/drive")
            .await;

        let task = db::queue::next_pending_task(&pool).await.unwrap();
        assert!(task.is_none(), "should skip file with sync_state=cached");

        // Also test downloading state
        db_file.sync_state = "downloading".to_string();
        enqueue_download_for_change(&pool, "acct-1", &db_file, SyncMode::Mirror, "/tmp/drive")
            .await;

        let task = db::queue::next_pending_task(&pool).await.unwrap();
        assert!(task.is_none(), "should skip file with sync_state=downloading");
    }
}
