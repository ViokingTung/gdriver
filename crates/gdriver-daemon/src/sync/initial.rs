//! Initial full sync: run once per account after OAuth to populate the local
//! file metadata database.

use gdriver_api::{changes, client::DriveClient, files};
use gdriver_ipc::SyncMode;
use sqlx::SqlitePool;
use tracing::{debug, info, warn};

use crate::db;

/// Run the initial full sync for a newly-authenticated account.
///
/// 1. Obtains and persists the start page token for future incremental syncs.
/// 2. Paginates through ALL Drive files and upserts them into `drive_files`.
/// 3. In Mirror mode, enqueues download tasks for all non-folder files so the
///    entire Drive is fully synced to local disk.
///
/// Called once per account, right after OAuth completes.
pub async fn initial_sync(
    db: &SqlitePool,
    account_id: &str,
    client: &DriveClient,
    sync_mode: SyncMode,
    mount_point: &str,
) -> anyhow::Result<()> {
    info!(account_id, ?sync_mode, "initial sync: starting");

    // ── Step 1: Get and save the start page token ──────────────────────────
    let page_token = changes::changes_get_start_page_token(client, None).await?;
    db::tokens::set_token(db, account_id, &page_token).await?;
    debug!(account_id, %page_token, "initial sync: start page token saved");

    // ── Step 2: Paginate through all files ─────────────────────────────────
    let mut files_count: usize = 0;
    let mut next_page_token: Option<String> = None;
    let mut page_num: u32 = 0;

    loop {
        page_num += 1;
        let response = files::files_list(
            client,
            None, // no query — fetch all files
            next_page_token.as_deref(),
            Some(1000), // max page size
            None,       // default fields
        )
        .await?;

        let page_files = response.files;
        let on_this_page = page_files.len();

        if response.incomplete_search == Some(true) {
            warn!(
                account_id,
                page_num, "initial sync: search incomplete on page"
            );
        }

        for api_file in &page_files {
            let db_file = map_api_file_to_db(api_file, account_id);
            db::files::upsert_file(db, &db_file).await?;
        }
        files_count += on_this_page;

        debug!(
            account_id,
            page_num,
            on_this_page,
            total = files_count,
            "initial sync: page"
        );

        match response.next_page_token {
            Some(token) => next_page_token = Some(token),
            None => break,
        }
    }

    info!(
        account_id,
        total_files = files_count,
        "initial sync: files upserted"
    );

    // ── Step 3: Mirror mode — enqueue downloads for all files ──────────────
    if sync_mode == SyncMode::Mirror {
        let download_count = enqueue_mirror_downloads(db, account_id, mount_point).await?;
        info!(
            account_id,
            download_count, "initial sync: mirror download tasks enqueued"
        );
    }

    info!(account_id, "initial sync: complete");
    Ok(())
}

/// Enqueue download tasks for every non-folder, non-trashed file that is not
/// yet present locally.
///
/// Uses the Drive file tree to compute the correct local path under
/// `mount_point` by walking each file's parent chain.
pub(crate) async fn enqueue_mirror_downloads(
    db: &SqlitePool,
    account_id: &str,
    mount_point: &str,
) -> anyhow::Result<usize> {
    // Fetch all non-trashed files for this account.
    let all_files = sqlx::query_as::<_, crate::db::files::DriveFileRow>(
        "SELECT id, account_id, name, mime_type, parent_id, size, etag, version,
                modified_time, is_trashed, is_shared, local_path, sync_state, local_mtime
         FROM drive_files
         WHERE account_id = ? AND is_trashed = 0",
    )
    .bind(account_id)
    .fetch_all(db)
    .await?;

    // Build a parent_id → name map for path resolution.
    let mut name_map: std::collections::HashMap<String, (String, Option<String>)> =
        std::collections::HashMap::new();
    let mut folders: std::collections::HashSet<String> = std::collections::HashSet::new();

    for row in &all_files {
        let file: crate::db::files::DriveFile = row.clone().into();
        if file.mime_type == "application/vnd.google-apps.folder" {
            folders.insert(file.id.clone());
        }
        name_map.insert(file.id.clone(), (file.name.clone(), file.parent_id.clone()));
    }

    let now = chrono::Utc::now().timestamp_millis();
    let mut count: usize = 0;

    for row in &all_files {
        let file: crate::db::files::DriveFile = row.clone().into();

        // Skip folders — they don't need downloading.
        if folders.contains(&file.id) {
            continue;
        }

        // Skip files that are already cached/synced locally.
        if file.sync_state == "cached"
            || file.sync_state == "synced"
            || file.sync_state == "offline"
            || file.sync_state == "downloading"
        {
            continue;
        }

        // Skip files that already have a local_path set.
        if file.local_path.is_some() {
            continue;
        }

        // Compute the local path by walking the parent chain.
        let relative_path = resolve_relative_path(&file.id, &name_map);
        let local_path = format!("{}/{}", mount_point.trim_end_matches('/'), relative_path);

        // Set the local_path on the drive_file record.
        let mut updated = file.clone();
        updated.local_path = Some(local_path.clone());
        db::files::upsert_file(db, &updated).await?;

        // Enqueue a download task.
        let task = db::queue::SyncTask {
            id: None,
            account_id: account_id.to_string(),
            file_id: Some(file.id.clone()),
            operation: "download".to_string(),
            local_path: Some(local_path),
            priority: 5,
            status: "pending".to_string(),
            retry_count: 0,
            error_msg: None,
            created_at: now,
            updated_at: now,
        };

        if let Err(e) = db::queue::enqueue(db, &task).await {
            warn!(
                account_id,
                file_id = %file.id,
                error = %e,
                "mirror: failed to enqueue download task"
            );
        } else {
            count += 1;
        }
    }

    Ok(count)
}

/// Walk the parent chain to build a relative path for a file.
///
/// Returns `"folder/subfolder/file.txt"` for a file nested inside folders.
fn resolve_relative_path(
    file_id: &str,
    name_map: &std::collections::HashMap<String, (String, Option<String>)>,
) -> String {
    let mut segments: Vec<String> = Vec::new();
    let mut current_id = file_id.to_string();

    while let Some((name, parent_id)) = name_map.get(&current_id) {
        segments.push(name.clone());
        match parent_id {
            Some(pid) => current_id = pid.clone(),
            None => break, // Reached root
        }
    }

    segments.reverse();
    segments.join("/")
}

// ─── API → DB mapping ─────────────────────────────────────────────────────────

/// Map a Google Drive API [`files::DriveFile`] to our database
/// [`db::files::DriveFile`].
pub(crate) fn map_api_file_to_db(
    api_file: &files::DriveFile,
    account_id: &str,
) -> db::files::DriveFile {
    db::files::DriveFile {
        id: api_file.id.clone().unwrap_or_default(),
        account_id: account_id.to_string(),
        name: api_file.name.clone().unwrap_or_else(|| "Untitled".into()),
        mime_type: api_file
            .mime_type
            .clone()
            .unwrap_or_else(|| "application/octet-stream".into()),
        parent_id: api_file.parents.as_ref().and_then(|p| p.first().cloned()),
        size: api_file.size.as_ref().and_then(|s| s.parse::<i64>().ok()),
        etag: api_file.etag.clone(),
        version: api_file
            .version
            .as_ref()
            .and_then(|v| v.parse::<i64>().ok()),
        modified_time: api_file
            .modified_time
            .as_deref()
            .and_then(parse_rfc3339_to_ms),
        is_trashed: api_file.trashed.unwrap_or(false),
        is_shared: api_file.shared.unwrap_or(false),
        local_path: None,
        sync_state: "cloud_only".into(),
        local_mtime: None,
    }
}

/// Parse an RFC 3339 timestamp string to Unix milliseconds.
fn parse_rfc3339_to_ms(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

// ─── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use files::DriveFile as ApiDriveFile;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

    use super::*;

    // ── helpers ──────────────────────────────────────────────────────────────

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

    fn sample_api_file(id: &str, name: &str, parent_id: Option<&str>) -> ApiDriveFile {
        ApiDriveFile {
            id: Some(id.into()),
            name: Some(name.into()),
            mime_type: Some("text/plain".into()),
            parents: parent_id.map(|p| vec![p.to_string()]),
            size: Some("1024".into()),
            etag: Some(format!("\"etag_{id}\"")),
            version: Some("5".into()),
            modified_time: Some("2026-05-01T12:00:00.000Z".into()),
            created_time: Some("2026-04-01T12:00:00.000Z".into()),
            trashed: Some(false),
            shared: Some(false),
            md5_checksum: Some("abc123".into()),
            web_view_link: Some(format!("https://drive.google.com/file/d/{id}")),
        }
    }

    // ── map_api_file_to_db tests ─────────────────────────────────────────────

    #[test]
    fn map_full_file() {
        let api = sample_api_file("f1", "report.pdf", Some("parent-1"));
        let db_file = map_api_file_to_db(&api, "acct-1");

        assert_eq!(db_file.id, "f1");
        assert_eq!(db_file.account_id, "acct-1");
        assert_eq!(db_file.name, "report.pdf");
        assert_eq!(db_file.mime_type, "text/plain");
        assert_eq!(db_file.parent_id.as_deref(), Some("parent-1"));
        assert_eq!(db_file.size, Some(1024));
        assert_eq!(db_file.etag.as_deref(), Some("\"etag_f1\""));
        assert_eq!(db_file.version, Some(5));
        assert_eq!(db_file.modified_time, Some(1_777_636_800_000)); // 2026-05-01T12:00:00Z
        assert!(!db_file.is_trashed);
        assert!(!db_file.is_shared);
        assert_eq!(db_file.local_path, None);
        assert_eq!(db_file.sync_state, "cloud_only");
        assert_eq!(db_file.local_mtime, None);
    }

    #[test]
    fn map_file_missing_id_uses_empty() {
        let api = ApiDriveFile {
            id: None,
            name: Some("no-id.txt".into()),
            mime_type: None,
            parents: None,
            size: None,
            etag: None,
            version: None,
            modified_time: None,
            created_time: None,
            trashed: None,
            shared: None,
            md5_checksum: None,
            web_view_link: None,
        };
        let db_file = map_api_file_to_db(&api, "acct-1");
        assert_eq!(db_file.id, "");
        assert_eq!(db_file.name, "no-id.txt");
    }

    #[test]
    fn map_file_missing_name_uses_untitled() {
        let api = ApiDriveFile {
            id: Some("f2".into()),
            name: None,
            mime_type: None,
            parents: None,
            size: None,
            etag: None,
            version: None,
            modified_time: None,
            created_time: None,
            trashed: None,
            shared: None,
            md5_checksum: None,
            web_view_link: None,
        };
        let db_file = map_api_file_to_db(&api, "acct-1");
        assert_eq!(db_file.name, "Untitled");
        assert_eq!(db_file.mime_type, "application/octet-stream");
    }

    #[test]
    fn map_file_multiple_parents_uses_first() {
        let api = ApiDriveFile {
            id: Some("f3".into()),
            name: Some("multi-parent.txt".into()),
            mime_type: None,
            parents: Some(vec!["first".into(), "second".into()]),
            size: None,
            etag: None,
            version: None,
            modified_time: None,
            created_time: None,
            trashed: None,
            shared: None,
            md5_checksum: None,
            web_view_link: None,
        };
        let db_file = map_api_file_to_db(&api, "acct-1");
        assert_eq!(db_file.parent_id.as_deref(), Some("first"));
    }

    #[test]
    fn map_file_no_parents() {
        let api = sample_api_file("f4", "root-file.txt", None);
        let db_file = map_api_file_to_db(&api, "acct-1");
        assert_eq!(db_file.parent_id, None);
    }

    #[test]
    fn map_file_trashed_and_shared() {
        let api = ApiDriveFile {
            id: Some("f5".into()),
            name: Some("shared-trash.txt".into()),
            mime_type: None,
            parents: None,
            size: None,
            etag: None,
            version: None,
            modified_time: None,
            created_time: None,
            trashed: Some(true),
            shared: Some(true),
            md5_checksum: None,
            web_view_link: None,
        };
        let db_file = map_api_file_to_db(&api, "acct-1");
        assert!(db_file.is_trashed);
        assert!(db_file.is_shared);
    }

    #[test]
    fn map_file_invalid_size_ignored() {
        let api = ApiDriveFile {
            id: Some("f6".into()),
            name: Some("bad-size.txt".into()),
            mime_type: None,
            parents: None,
            size: Some("not-a-number".into()),
            etag: None,
            version: None,
            modified_time: None,
            created_time: None,
            trashed: None,
            shared: None,
            md5_checksum: None,
            web_view_link: None,
        };
        let db_file = map_api_file_to_db(&api, "acct-1");
        assert_eq!(db_file.size, None);
    }

    #[test]
    fn map_file_invalid_version_ignored() {
        let api = ApiDriveFile {
            id: Some("f7".into()),
            name: Some("bad-ver.txt".into()),
            mime_type: None,
            parents: None,
            size: None,
            etag: None,
            version: Some("abc".into()),
            modified_time: None,
            created_time: None,
            trashed: None,
            shared: None,
            md5_checksum: None,
            web_view_link: None,
        };
        let db_file = map_api_file_to_db(&api, "acct-1");
        assert_eq!(db_file.version, None);
    }

    #[test]
    fn map_file_size_zero() {
        let api = ApiDriveFile {
            id: Some("f8".into()),
            name: Some("empty.txt".into()),
            mime_type: None,
            parents: None,
            size: Some("0".into()),
            etag: None,
            version: None,
            modified_time: None,
            created_time: None,
            trashed: None,
            shared: None,
            md5_checksum: None,
            web_view_link: None,
        };
        let db_file = map_api_file_to_db(&api, "acct-1");
        assert_eq!(db_file.size, Some(0));
    }

    // ── parse_rfc3339_to_ms tests ────────────────────────────────────────────

    #[test]
    fn parse_valid_rfc3339() {
        let ms = parse_rfc3339_to_ms("2026-05-01T12:00:00.000Z");
        assert_eq!(ms, Some(1_777_636_800_000));
    }

    #[test]
    fn parse_rfc3339_with_offset() {
        // +08:00 timezone — 8 hours behind UTC
        let ms = parse_rfc3339_to_ms("2026-05-01T12:00:00.000+08:00");
        assert_eq!(ms, Some(1_777_608_000_000));
    }

    #[test]
    fn parse_invalid_rfc3339() {
        assert_eq!(parse_rfc3339_to_ms("not-a-date"), None);
        assert_eq!(parse_rfc3339_to_ms(""), None);
    }

    // ── initial_sync DB integration tests ────────────────────────────────────

    /// Verify the full flow of initial sync DB operations: save page token,
    /// upsert files from API response, and verify children listing.
    ///
    /// This test exercises the DB operations that `initial_sync` performs.
    /// The actual Drive API calls are tested in the `gdriver-api` crate.
    #[tokio::test]
    async fn initial_sync_persists_page_token_and_files() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;

        // Save token (as initial_sync does after getting it from the API).
        crate::db::tokens::set_token(&pool, "acct-1", "init-tok-42")
            .await
            .unwrap();
        let saved = crate::db::tokens::get_token(&pool, "acct-1").await.unwrap();
        assert_eq!(saved.as_deref(), Some("init-tok-42"));

        // Insert files (as initial_sync does for each page of API results).
        for (id, name, parent) in [
            ("f1", "alpha.txt", Some("root")),
            ("f2", "beta.txt", None),
            ("f3", "gamma.txt", Some("f1")),
        ] {
            let api = sample_api_file(id, name, parent);
            let db_file = map_api_file_to_db(&api, "acct-1");
            crate::db::files::upsert_file(&pool, &db_file)
                .await
                .unwrap();
        }

        // Verify files are in the DB
        let f1 = crate::db::files::get_file_by_id(&pool, "f1", "acct-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(f1.name, "alpha.txt");
        assert_eq!(f1.parent_id.as_deref(), Some("root"));
        assert_eq!(f1.sync_state, "cloud_only");

        let f2 = crate::db::files::get_file_by_id(&pool, "f2", "acct-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(f2.name, "beta.txt");
        assert_eq!(f2.parent_id, None);

        let f3 = crate::db::files::get_file_by_id(&pool, "f3", "acct-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(f3.name, "gamma.txt");
        assert_eq!(f3.parent_id.as_deref(), Some("f1"));

        // Verify children listing works
        let root_children = crate::db::files::list_children(&pool, None, "acct-1")
            .await
            .unwrap();
        assert_eq!(root_children.len(), 1);
        assert_eq!(root_children[0].name, "beta.txt");

        let f1_children = crate::db::files::list_children(&pool, Some("f1"), "acct-1")
            .await
            .unwrap();
        assert_eq!(f1_children.len(), 1);
        assert_eq!(f1_children[0].name, "gamma.txt");
    }

    /// Test that initial_sync works with an empty Drive (no files).
    #[tokio::test]
    async fn initial_sync_empty_drive() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-empty").await;

        // Save token and verify no files
        crate::db::tokens::set_token(&pool, "acct-empty", "empty-tok")
            .await
            .unwrap();

        let children = crate::db::files::list_children(&pool, None, "acct-empty")
            .await
            .unwrap();
        assert!(children.is_empty());

        let token = crate::db::tokens::get_token(&pool, "acct-empty")
            .await
            .unwrap();
        assert_eq!(token.as_deref(), Some("empty-tok"));
    }

    /// Verify that file upsert during initial sync handles trashed files.
    #[tokio::test]
    async fn initial_sync_persists_trashed_files() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;

        let api = ApiDriveFile {
            id: Some("trashed-1".into()),
            name: Some("deleted.txt".into()),
            mime_type: Some("text/plain".into()),
            parents: None,
            size: Some("512".into()),
            etag: Some("\"etag_old\"".into()),
            version: Some("3".into()),
            modified_time: Some("2026-05-01T12:00:00.000Z".into()),
            created_time: None,
            trashed: Some(true),
            shared: Some(false),
            md5_checksum: None,
            web_view_link: None,
        };

        let db_file = map_api_file_to_db(&api, "acct-1");
        crate::db::files::upsert_file(&pool, &db_file)
            .await
            .unwrap();

        let fetched = crate::db::files::get_file_by_id(&pool, "trashed-1", "acct-1")
            .await
            .unwrap()
            .unwrap();
        assert!(fetched.is_trashed);
        assert_eq!(fetched.name, "deleted.txt");

        // Trashed files should NOT appear in list_children
        let children = crate::db::files::list_children(&pool, None, "acct-1")
            .await
            .unwrap();
        assert!(children.is_empty());
    }

    /// Verify that folder mime types are persisted correctly.
    #[tokio::test]
    async fn initial_sync_handles_folders() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;

        let api = ApiDriveFile {
            id: Some("folder-1".into()),
            name: Some("Documents".into()),
            mime_type: Some("application/vnd.google-apps.folder".into()),
            parents: Some(vec!["root".into()]),
            size: None, // folders have no size
            etag: Some("\"etag_folder\"".into()),
            version: Some("1".into()),
            modified_time: Some("2026-05-01T12:00:00.000Z".into()),
            created_time: None,
            trashed: Some(false),
            shared: Some(true),
            md5_checksum: None,
            web_view_link: Some("https://drive.google.com/drive/folders/folder-1".into()),
        };

        let db_file = map_api_file_to_db(&api, "acct-1");
        crate::db::files::upsert_file(&pool, &db_file)
            .await
            .unwrap();

        let fetched = crate::db::files::get_file_by_id(&pool, "folder-1", "acct-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(fetched.name, "Documents");
        assert_eq!(fetched.mime_type, "application/vnd.google-apps.folder");
        assert_eq!(fetched.size, None);
        assert!(fetched.is_shared);
    }

    // ── enqueue_mirror_downloads tests ─────────────────────────────────────

    #[tokio::test]
    async fn mirror_downloads_enqueues_non_trashed_files() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;

        for (id, name) in [("f1", "file-a.txt"), ("f2", "file-b.txt")] {
            let api = sample_api_file(id, name, Some("root"));
            crate::db::files::upsert_file(&pool, &map_api_file_to_db(&api, "acct-1"))
                .await
                .unwrap();
        }

        let count = enqueue_mirror_downloads(&pool, "acct-1", "/tmp/drive")
            .await
            .unwrap();
        assert_eq!(count, 2, "should enqueue downloads for both files");

        let t1 = crate::db::queue::next_pending_task(&pool)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(t1.operation, "download");
        let t2 = crate::db::queue::next_pending_task(&pool)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(t2.operation, "download");
    }

    #[tokio::test]
    async fn mirror_downloads_skips_trashed_files() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;

        let mut api = sample_api_file("f-trash", "trashed.txt", Some("root"));
        api.trashed = Some(true);
        crate::db::files::upsert_file(&pool, &map_api_file_to_db(&api, "acct-1"))
            .await
            .unwrap();

        let count = enqueue_mirror_downloads(&pool, "acct-1", "/tmp/drive")
            .await
            .unwrap();
        assert_eq!(count, 0, "should skip trashed files");
    }

    #[tokio::test]
    async fn mirror_downloads_skips_folders() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;

        let mut api = sample_api_file("folder-1", "My Folder", Some("root"));
        api.mime_type = Some("application/vnd.google-apps.folder".into());
        crate::db::files::upsert_file(&pool, &map_api_file_to_db(&api, "acct-1"))
            .await
            .unwrap();

        let count = enqueue_mirror_downloads(&pool, "acct-1", "/tmp/drive")
            .await
            .unwrap();
        assert_eq!(count, 0, "should skip folders");
    }

    #[tokio::test]
    async fn mirror_downloads_skips_already_cached_files() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;

        let api = sample_api_file("f-cached", "cached.txt", Some("root"));
        let mut db_file = map_api_file_to_db(&api, "acct-1");
        db_file.sync_state = "cached".to_string();
        crate::db::files::upsert_file(&pool, &db_file)
            .await
            .unwrap();

        let count = enqueue_mirror_downloads(&pool, "acct-1", "/tmp/drive")
            .await
            .unwrap();
        assert_eq!(count, 0, "should skip already cached files");
    }

    #[tokio::test]
    async fn mirror_downloads_resolves_nested_path() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;

        // root folder
        let mut root_api = sample_api_file("root-1", "My Drive", None);
        root_api.mime_type = Some("application/vnd.google-apps.folder".into());
        let mut root_db = map_api_file_to_db(&root_api, "acct-1");
        root_db.parent_id = None;
        crate::db::files::upsert_file(&pool, &root_db)
            .await
            .unwrap();

        // subfolder under root
        let mut sub_api = sample_api_file("sub-1", "Documents", Some("root-1"));
        sub_api.mime_type = Some("application/vnd.google-apps.folder".into());
        crate::db::files::upsert_file(&pool, &map_api_file_to_db(&sub_api, "acct-1"))
            .await
            .unwrap();

        // file inside subfolder
        let file_api = sample_api_file("f-nested", "report.pdf", Some("sub-1"));
        crate::db::files::upsert_file(&pool, &map_api_file_to_db(&file_api, "acct-1"))
            .await
            .unwrap();

        let count = enqueue_mirror_downloads(&pool, "acct-1", "/tmp/drive")
            .await
            .unwrap();
        assert_eq!(count, 1);

        let task = crate::db::queue::next_pending_task(&pool)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(task.operation, "download");
        let path = task.local_path.unwrap();
        assert!(
            path.contains("report.pdf"),
            "path should end with filename: {path}"
        );
    }
}
