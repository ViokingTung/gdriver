use sqlx::SqlitePool;

// ─── Public domain type ────────────────────────────────────────────────────────

/// A row in the `drive_files` table representing one Drive file/folder.
#[derive(Debug, Clone)]
pub struct DriveFile {
    pub id: String,
    pub account_id: String,
    pub name: String,
    pub mime_type: String,
    pub parent_id: Option<String>,
    pub size: Option<i64>,
    pub etag: Option<String>,
    pub version: Option<i64>,
    pub modified_time: Option<i64>,
    pub is_trashed: bool,
    pub is_shared: bool,
    pub local_path: Option<String>,
    pub sync_state: String,
    pub local_mtime: Option<i64>,
}

// ─── Internal row type ────────────────────────────────────────────────────────

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct DriveFileRow {
    id: String,
    account_id: String,
    name: String,
    mime_type: String,
    parent_id: Option<String>,
    size: Option<i64>,
    etag: Option<String>,
    version: Option<i64>,
    modified_time: Option<i64>,
    is_trashed: i64,
    is_shared: i64,
    local_path: Option<String>,
    sync_state: String,
    local_mtime: Option<i64>,
}

impl From<DriveFileRow> for DriveFile {
    fn from(r: DriveFileRow) -> Self {
        Self {
            id: r.id,
            account_id: r.account_id,
            name: r.name,
            mime_type: r.mime_type,
            parent_id: r.parent_id,
            size: r.size,
            etag: r.etag,
            version: r.version,
            modified_time: r.modified_time,
            is_trashed: r.is_trashed != 0,
            is_shared: r.is_shared != 0,
            local_path: r.local_path,
            sync_state: r.sync_state,
            local_mtime: r.local_mtime,
        }
    }
}

impl From<DriveFile> for DriveFileRow {
    fn from(f: DriveFile) -> Self {
        Self {
            id: f.id,
            account_id: f.account_id,
            name: f.name,
            mime_type: f.mime_type,
            parent_id: f.parent_id,
            size: f.size,
            etag: f.etag,
            version: f.version,
            modified_time: f.modified_time,
            is_trashed: f.is_trashed as i64,
            is_shared: f.is_shared as i64,
            local_path: f.local_path,
            sync_state: f.sync_state,
            local_mtime: f.local_mtime,
        }
    }
}

// ─── CRUD ─────────────────────────────────────────────────────────────────────

/// Insert a new file or update all mutable columns when `(id, account_id)`
/// already exists.
pub async fn upsert_file(pool: &SqlitePool, file: &DriveFile) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        INSERT INTO drive_files
            (id, account_id, name, mime_type, parent_id, size, etag, version,
             modified_time, is_trashed, is_shared, local_path, sync_state, local_mtime)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(id, account_id) DO UPDATE SET
            name          = excluded.name,
            mime_type     = excluded.mime_type,
            parent_id     = excluded.parent_id,
            size          = excluded.size,
            etag          = excluded.etag,
            version       = excluded.version,
            modified_time = excluded.modified_time,
            is_trashed    = excluded.is_trashed,
            is_shared     = excluded.is_shared,
            local_path    = excluded.local_path,
            sync_state    = excluded.sync_state,
            local_mtime   = excluded.local_mtime
        "#,
    )
    .bind(&file.id)
    .bind(&file.account_id)
    .bind(&file.name)
    .bind(&file.mime_type)
    .bind(&file.parent_id)
    .bind(file.size)
    .bind(&file.etag)
    .bind(file.version)
    .bind(file.modified_time)
    .bind(file.is_trashed as i64)
    .bind(file.is_shared as i64)
    .bind(&file.local_path)
    .bind(&file.sync_state)
    .bind(file.local_mtime)
    .execute(pool)
    .await?;

    Ok(())
}

/// Return the file row for the given Drive file ID and account, or `None`.
pub async fn get_file_by_id(
    pool: &SqlitePool,
    id: &str,
    account_id: &str,
) -> anyhow::Result<Option<DriveFile>> {
    let row = sqlx::query_as::<_, DriveFileRow>(
        "SELECT id, account_id, name, mime_type, parent_id, size, etag, version,
                modified_time, is_trashed, is_shared, local_path, sync_state, local_mtime
         FROM drive_files
         WHERE id = ? AND account_id = ?",
    )
    .bind(id)
    .bind(account_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(DriveFile::from))
}

/// Return the file row whose `local_path` column matches exactly, or `None`.
pub async fn get_file_by_local_path(
    pool: &SqlitePool,
    path: &str,
) -> anyhow::Result<Option<DriveFile>> {
    let row = sqlx::query_as::<_, DriveFileRow>(
        "SELECT id, account_id, name, mime_type, parent_id, size, etag, version,
                modified_time, is_trashed, is_shared, local_path, sync_state, local_mtime
         FROM drive_files
         WHERE local_path = ?",
    )
    .bind(path)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(DriveFile::from))
}

/// Return every child (files and sub-folders) of the given parent within one
/// account.  Pass `parent_id = None` for the My Drive root.
pub async fn list_children(
    pool: &SqlitePool,
    parent_id: Option<&str>,
    account_id: &str,
) -> anyhow::Result<Vec<DriveFile>> {
    let rows = if parent_id.is_some() {
        sqlx::query_as::<_, DriveFileRow>(
            "SELECT id, account_id, name, mime_type, parent_id, size, etag, version,
                    modified_time, is_trashed, is_shared, local_path, sync_state, local_mtime
             FROM drive_files
             WHERE parent_id = ? AND account_id = ? AND is_trashed = 0
             ORDER BY name",
        )
        .bind(parent_id)
        .bind(account_id)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, DriveFileRow>(
            "SELECT id, account_id, name, mime_type, parent_id, size, etag, version,
                    modified_time, is_trashed, is_shared, local_path, sync_state, local_mtime
             FROM drive_files
             WHERE parent_id IS NULL AND account_id = ? AND is_trashed = 0
             ORDER BY name",
        )
        .bind(account_id)
        .fetch_all(pool)
        .await?
    };

    Ok(rows.into_iter().map(DriveFile::from).collect())
}

/// Update `sync_state` for the file identified by `(id, account_id)`.
pub async fn set_sync_state(
    pool: &SqlitePool,
    id: &str,
    account_id: &str,
    state: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE drive_files SET sync_state = ? WHERE id = ? AND account_id = ?",
    )
    .bind(state)
    .bind(id)
    .bind(account_id)
    .execute(pool)
    .await?;

    Ok(())
}

/// Mark the file as trashed (soft-delete).
pub async fn mark_trashed(
    pool: &SqlitePool,
    id: &str,
    account_id: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE drive_files SET is_trashed = 1 WHERE id = ? AND account_id = ?",
    )
    .bind(id)
    .bind(account_id)
    .execute(pool)
    .await?;

    Ok(())
}

/// Return up to `limit` recently modified files across all accounts, excluding
/// trashed files.  Ordered by `modified_time DESC` (most recent first).
pub async fn list_recent_files(
    pool: &SqlitePool,
    limit: u32,
) -> anyhow::Result<Vec<DriveFile>> {
    let rows = sqlx::query_as::<_, DriveFileRow>(
        "SELECT id, account_id, name, mime_type, parent_id, size, etag, version,
                modified_time, is_trashed, is_shared, local_path, sync_state, local_mtime
         FROM drive_files
         WHERE is_trashed = 0
         ORDER BY modified_time DESC
         LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(DriveFile::from).collect())
}

/// Return a page of files for the sync activity view, excluding trashed files.
/// Ordered by `modified_time DESC`.  `page` is 0-indexed.
pub async fn list_files_paginated(
    pool: &SqlitePool,
    page: u32,
    page_size: u32,
) -> anyhow::Result<Vec<DriveFile>> {
    let offset = (page as i64) * (page_size as i64);
    let rows = sqlx::query_as::<_, DriveFileRow>(
        "SELECT id, account_id, name, mime_type, parent_id, size, etag, version,
                modified_time, is_trashed, is_shared, local_path, sync_state, local_mtime
         FROM drive_files
         WHERE is_trashed = 0
         ORDER BY modified_time DESC
         LIMIT ? OFFSET ?",
    )
    .bind(page_size)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(DriveFile::from).collect())
}

/// Find a file whose `local_path` ends with the given relative path suffix.
///
/// Used by the FS IPC handlers to resolve a FUSE mount path to a database
/// record.  Returns the shortest matching path (closest match) among
/// non-trashed files.
pub async fn find_file_by_relative_suffix(
    pool: &SqlitePool,
    suffix: &str,
) -> anyhow::Result<Option<DriveFile>> {
    let pattern = format!("%/{}", suffix);
    let row = sqlx::query_as::<_, DriveFileRow>(
        "SELECT id, account_id, name, mime_type, parent_id, size, etag, version,
                modified_time, is_trashed, is_shared, local_path, sync_state, local_mtime
         FROM drive_files
         WHERE local_path LIKE ? AND is_trashed = 0
         ORDER BY length(local_path) ASC
         LIMIT 1",
    )
    .bind(&pattern)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(DriveFile::from))
}

/// Count non-trashed files and folders separately.
///
/// Folders are identified by `mime_type = 'application/vnd.google-apps.folder'`.
pub async fn count_files_and_folders(
    pool: &SqlitePool,
) -> anyhow::Result<(u64, u64)> {
    let row: (i64, i64) = sqlx::query_as(
        "SELECT
            SUM(CASE WHEN mime_type != 'application/vnd.google-apps.folder' THEN 1 ELSE 0 END),
            SUM(CASE WHEN mime_type = 'application/vnd.google-apps.folder' THEN 1 ELSE 0 END)
         FROM drive_files
         WHERE is_trashed = 0",
    )
    .fetch_one(pool)
    .await?;

    Ok((row.0 as u64, row.1 as u64))
}

/// Sum file sizes grouped by sync_state for states that use local storage.
///
/// Returns `(offline_bytes, cache_bytes)` where:
/// - `offline_bytes`: total size of files pinned for offline access (`sync_state = 'offline'`)
/// - `cache_bytes`: total size of files cached locally (`sync_state = 'cached'`)
pub async fn sum_bytes_by_sync_state(
    pool: &SqlitePool,
) -> anyhow::Result<(u64, u64)> {
    let row: (i64, i64) = sqlx::query_as(
        "SELECT
            COALESCE(SUM(CASE WHEN sync_state = 'offline' THEN size ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN sync_state = 'cached' THEN size ELSE 0 END), 0)
         FROM drive_files
         WHERE is_trashed = 0",
    )
    .fetch_one(pool)
    .await?;

    Ok((row.0 as u64, row.1 as u64))
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
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

    /// Insert a synthetic account so the FK constraint on `drive_files.account_id` is satisfied.
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

    fn make_file(id: &str, account_id: &str, name: &str, parent_id: Option<&str>) -> DriveFile {
        DriveFile {
            id: id.into(),
            account_id: account_id.into(),
            name: name.into(),
            mime_type: "application/vnd.google-apps.folder".into(),
            parent_id: parent_id.map(String::from),
            size: None,
            etag: Some("\"etag-1\"".into()),
            version: Some(1),
            modified_time: Some(1_700_000_000_000),
            is_trashed: false,
            is_shared: false,
            local_path: None,
            sync_state: "cloud_only".into(),
            local_mtime: None,
        }
    }

    // ── upsert_file ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn upsert_inserts_new_file() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;
        let f = make_file("file-1", "acct-1", "report.pdf", Some("parent-1"));

        upsert_file(&pool, &f).await.unwrap();

        let fetched = get_file_by_id(&pool, "file-1", "acct-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(fetched.name, "report.pdf");
        assert_eq!(fetched.mime_type, "application/vnd.google-apps.folder");
    }

    #[tokio::test]
    async fn upsert_updates_existing_file() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;
        let f = make_file("file-2", "acct-1", "old-name.txt", None);
        upsert_file(&pool, &f).await.unwrap();

        let mut updated = f.clone();
        updated.name = "new-name.txt".into();
        updated.etag = Some("\"etag-2\"".into());
        updated.version = Some(2);
        upsert_file(&pool, &updated).await.unwrap();

        let fetched = get_file_by_id(&pool, "file-2", "acct-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(fetched.name, "new-name.txt");
        assert_eq!(fetched.etag, Some("\"etag-2\"".into()));
        assert_eq!(fetched.version, Some(2));
    }

    // ── get_file_by_id ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn get_by_id_nonexistent_returns_none() {
        let pool = test_pool().await;
        let result = get_file_by_id(&pool, "nobody", "acct-1")
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn get_by_id_different_account_returns_none() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;
        let f = make_file("file-3", "acct-1", "shared.txt", None);
        upsert_file(&pool, &f).await.unwrap();

        // Same file id but different account should not match
        let result = get_file_by_id(&pool, "file-3", "acct-2")
            .await
            .unwrap();
        assert!(result.is_none());
    }

    // ── get_file_by_local_path ─────────────────────────────────────────────

    #[tokio::test]
    async fn get_by_local_path_finds_file() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;
        let mut f = make_file("file-4", "acct-1", "notes.txt", None);
        f.local_path = Some("/home/user/Drive/notes.txt".into());
        upsert_file(&pool, &f).await.unwrap();

        let fetched = get_file_by_local_path(&pool, "/home/user/Drive/notes.txt")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(fetched.id, "file-4");
    }

    #[tokio::test]
    async fn get_by_local_path_nonexistent_returns_none() {
        let pool = test_pool().await;
        let result = get_file_by_local_path(&pool, "/no/such/path")
            .await
            .unwrap();
        assert!(result.is_none());
    }

    // ── list_children ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn list_children_returns_direct_children_only() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;

        // Root folder
        upsert_file(&pool, &make_file("root", "acct-1", "My Drive", None))
            .await
            .unwrap();
        // Children of root
        upsert_file(
            &pool,
            &make_file("child-1", "acct-1", "alpha.txt", Some("root")),
        )
        .await
        .unwrap();
        upsert_file(
            &pool,
            &make_file("child-2", "acct-1", "beta.txt", Some("root")),
        )
        .await
        .unwrap();
        // Grandchild (should NOT appear)
        upsert_file(
            &pool,
            &make_file("grandchild", "acct-1", "nested.txt", Some("child-1")),
        )
        .await
        .unwrap();

        let children = list_children(&pool, Some("root"), "acct-1")
            .await
            .unwrap();
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].name, "alpha.txt");
        assert_eq!(children[1].name, "beta.txt");
    }

    #[tokio::test]
    async fn list_children_root_with_null_parent() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;

        // File at root (parent_id IS NULL)
        let mut f = make_file("orphan", "acct-1", "at-root.txt", None);
        f.parent_id = None;
        upsert_file(&pool, &f).await.unwrap();

        let children = list_children(&pool, None, "acct-1").await.unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].name, "at-root.txt");
    }

    #[tokio::test]
    async fn list_children_excludes_trashed() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;

        let mut f = make_file("alive", "acct-1", "keep.txt", Some("folder-x"));
        upsert_file(&pool, &f).await.unwrap();

        f.id = "dead".into();
        f.name = "trash.txt".into();
        f.is_trashed = true;
        upsert_file(&pool, &f).await.unwrap();

        let children = list_children(&pool, Some("folder-x"), "acct-1")
            .await
            .unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].id, "alive");
    }

    #[tokio::test]
    async fn list_children_empty_folder() {
        let pool = test_pool().await;
        let children = list_children(&pool, Some("empty-dir"), "acct-1")
            .await
            .unwrap();
        assert!(children.is_empty());
    }

    // ── set_sync_state ────────────────────────────────────────────────────

    #[tokio::test]
    async fn set_sync_state_updates_row() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;
        let f = make_file("file-5", "acct-1", "data.csv", None);
        upsert_file(&pool, &f).await.unwrap();

        set_sync_state(&pool, "file-5", "acct-1", "synced")
            .await
            .unwrap();

        let fetched = get_file_by_id(&pool, "file-5", "acct-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(fetched.sync_state, "synced");
    }

    #[tokio::test]
    async fn set_sync_state_nonexistent_is_noop() {
        let pool = test_pool().await;
        // Should not error on non-existent row
        let result = set_sync_state(&pool, "ghost", "acct-1", "synced").await;
        assert!(result.is_ok());
    }

    // ── mark_trashed ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn mark_trashed_sets_flag() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;
        let f = make_file("file-6", "acct-1", "to-delete.txt", None);
        upsert_file(&pool, &f).await.unwrap();

        mark_trashed(&pool, "file-6", "acct-1").await.unwrap();

        let fetched = get_file_by_id(&pool, "file-6", "acct-1")
            .await
            .unwrap()
            .unwrap();
        assert!(fetched.is_trashed);
    }

    #[tokio::test]
    async fn mark_trashed_nonexistent_is_noop() {
        let pool = test_pool().await;
        let result = mark_trashed(&pool, "ghost", "acct-1").await;
        assert!(result.is_ok());
    }

    // ── boolean round-trip ────────────────────────────────────────────────

    #[tokio::test]
    async fn boolean_fields_roundtrip() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;
        let f = DriveFile {
            id: "file-bool".into(),
            account_id: "acct-1".into(),
            name: "shared-trash.txt".into(),
            mime_type: "text/plain".into(),
            parent_id: None,
            size: Some(1024),
            etag: None,
            version: None,
            modified_time: None,
            is_trashed: true,
            is_shared: true,
            local_path: None,
            sync_state: "error".into(),
            local_mtime: Some(1_700_000_001_000),
        };

        upsert_file(&pool, &f).await.unwrap();

        let fetched = get_file_by_id(&pool, "file-bool", "acct-1")
            .await
            .unwrap()
            .unwrap();
        assert!(fetched.is_trashed);
        assert!(fetched.is_shared);
        assert_eq!(fetched.sync_state, "error");
        assert_eq!(fetched.local_mtime, Some(1_700_000_001_000));
    }

    // ── cascade delete ────────────────────────────────────────────────────

    #[tokio::test]
    async fn deleting_account_cascades_to_files() {
        let pool = test_pool().await;

        // Insert an account first (needed for FK)
        sqlx::query(
            "INSERT INTO accounts (id, email, created_at, last_used_at)
             VALUES (?, ?, ?, ?)",
        )
        .bind("acct-cascade")
        .bind("cascade@example.com")
        .bind(1_700_000_000_000_i64)
        .bind(1_700_000_000_000_i64)
        .execute(&pool)
        .await
        .unwrap();

        upsert_file(&pool, &make_file("file-c", "acct-cascade", "gone.txt", None))
            .await
            .unwrap();

        // Delete account → file should be cascade-deleted
        sqlx::query("DELETE FROM accounts WHERE id = ?")
            .bind("acct-cascade")
            .execute(&pool)
            .await
            .unwrap();

        let fetched = get_file_by_id(&pool, "file-c", "acct-cascade")
            .await
            .unwrap();
        assert!(fetched.is_none());
    }

    // ── find_file_by_relative_suffix ──────────────────────────────────────

    #[tokio::test]
    async fn find_by_relative_suffix_matches_trailing_path() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;

        let mut f = make_file("file-sfx", "acct-1", "report.pdf", None);
        f.local_path = Some("/home/user/.cache/gdriver/acct-1/docs/report.pdf".into());
        f.mime_type = "application/pdf".into();
        upsert_file(&pool, &f).await.unwrap();

        let found = find_file_by_relative_suffix(&pool, "docs/report.pdf")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.id, "file-sfx");
    }

    #[tokio::test]
    async fn find_by_relative_suffix_returns_none_for_no_match() {
        let pool = test_pool().await;
        let found = find_file_by_relative_suffix(&pool, "nonexistent/file.txt")
            .await
            .unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn find_by_relative_suffix_excludes_trashed() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;

        let mut f = make_file("file-trashed-sfx", "acct-1", "gone.pdf", None);
        f.local_path = Some("/home/user/.cache/gdriver/acct-1/gone.pdf".into());
        f.is_trashed = true;
        upsert_file(&pool, &f).await.unwrap();

        let found = find_file_by_relative_suffix(&pool, "gone.pdf")
            .await
            .unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn find_by_relative_suffix_picks_shortest_match() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;

        // Two files with the same suffix — the shorter path should win.
        let mut f1 = make_file("file-short", "acct-1", "data.csv", None);
        f1.local_path = Some("/home/user/.cache/gdriver/acct-1/data.csv".into());
        upsert_file(&pool, &f1).await.unwrap();

        let mut f2 = make_file("file-long", "acct-1", "data.csv", None);
        f2.local_path = Some("/home/user/.cache/gdriver/acct-1/sub/dir/data.csv".into());
        upsert_file(&pool, &f2).await.unwrap();

        let found = find_file_by_relative_suffix(&pool, "data.csv")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.id, "file-short");
    }

    // ── sum_bytes_by_sync_state ──────────────────────────────────────

    #[tokio::test]
    async fn sum_bytes_empty_table() {
        let pool = test_pool().await;
        let (offline, cached) = sum_bytes_by_sync_state(&pool).await.unwrap();
        assert_eq!(offline, 0);
        assert_eq!(cached, 0);
    }

    #[tokio::test]
    async fn sum_bytes_offline_files() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;

        let mut f = make_file("f1", "acct-1", "doc.pdf", None);
        f.sync_state = "offline".into();
        f.size = Some(1024);
        upsert_file(&pool, &f).await.unwrap();

        let mut f2 = make_file("f2", "acct-1", "img.png", None);
        f2.sync_state = "offline".into();
        f2.size = Some(2048);
        upsert_file(&pool, &f2).await.unwrap();

        let (offline, cached) = sum_bytes_by_sync_state(&pool).await.unwrap();
        assert_eq!(offline, 3072);
        assert_eq!(cached, 0);
    }

    #[tokio::test]
    async fn sum_bytes_cached_files() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;

        let mut f = make_file("f1", "acct-1", "video.mp4", None);
        f.sync_state = "cached".into();
        f.size = Some(4096);
        upsert_file(&pool, &f).await.unwrap();

        let (offline, cached) = sum_bytes_by_sync_state(&pool).await.unwrap();
        assert_eq!(offline, 0);
        assert_eq!(cached, 4096);
    }

    #[tokio::test]
    async fn sum_bytes_excludes_trashed() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;

        let mut f = make_file("f1", "acct-1", "trashed.pdf", None);
        f.sync_state = "offline".into();
        f.size = Some(1024);
        f.is_trashed = true;
        upsert_file(&pool, &f).await.unwrap();

        let (offline, cached) = sum_bytes_by_sync_state(&pool).await.unwrap();
        assert_eq!(offline, 0);
        assert_eq!(cached, 0);
    }

    #[tokio::test]
    async fn sum_bytes_ignores_other_states() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;

        for state in &["cloud_only", "synced", "downloading", "uploading", "modified", "error"] {
            let mut f = make_file(
                &format!("f-{state}"),
                "acct-1",
                &format!("{state}.txt"),
                None,
            );
            f.sync_state = (*state).into();
            f.size = Some(512);
            upsert_file(&pool, &f).await.unwrap();
        }

        let (offline, cached) = sum_bytes_by_sync_state(&pool).await.unwrap();
        assert_eq!(offline, 0);
        assert_eq!(cached, 0);
    }
}
