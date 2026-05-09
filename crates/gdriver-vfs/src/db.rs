// ─── VFS Database queries ──────────────────────────────────────────────────
//
// These queries are specific to the VFS layer. They use `rowid` as the inode
// number, which is the plan's recommended strategy.  Root (My Drive) is
// represented by the sentinel inode FUSE_ROOT_ID (= 1).
//
// All queries filter `is_trashed = 0` so that trashed files do not appear in
// directory listings.

use sqlx::SqlitePool;

/// FUSE root inode (My Drive).
pub const ROOT_INODE: u64 = 1;

/// Minimal metadata needed by the VFS layer for every file.
///
/// This is a subset of the full `DriveFile` row, just enough to construct
/// `fuser::FileAttr` responses.
#[derive(Debug, Clone)]
pub struct FileMeta {
    /// SQLite `rowid` used as the FUSE inode number.
    pub inode: u64,
    pub file_id: String,
    pub name: String,
    pub mime_type: String,
    pub size: i64,
    pub modified_time: i64, // Unix milliseconds
}

/// Extended file metadata including sync state and local cache path.
///
/// Used by the `read` FUSE callback to decide whether to trigger a download
/// or read from the local cache directly.
#[derive(Debug, Clone)]
pub struct FileDetails {
    pub inode: u64,
    pub file_id: String,
    pub account_id: String,
    pub name: String,
    pub mime_type: String,
    pub size: i64,
    pub modified_time: i64,
    pub sync_state: String,
    pub local_path: Option<String>,
}

/// Return `FileMeta` for the file with the given inode (rowid), or `None`.
pub async fn get_file_by_inode(
    pool: &SqlitePool,
    inode: u64,
) -> anyhow::Result<Option<FileMeta>> {
    let row = sqlx::query_as::<_, FileMetaRow>(
        "SELECT rowid, id, name, mime_type, COALESCE(size, 0) AS size,
                COALESCE(modified_time, 0) AS modified_time
         FROM drive_files
         WHERE rowid = ? AND is_trashed = 0",
    )
    .bind(inode as i64)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(FileMeta::from))
}

/// Look up a file by parent inode + name.
///
/// Returns `None` when no matching file is found.
pub async fn lookup_by_parent_and_name(
    pool: &SqlitePool,
    parent_inode: u64,
    name: &str,
) -> anyhow::Result<Option<FileMeta>> {
    let row = if parent_inode == ROOT_INODE {
        // Root: files with parent_id IS NULL
        sqlx::query_as::<_, FileMetaRow>(
            "SELECT rowid, id, name, mime_type, COALESCE(size, 0) AS size,
                    COALESCE(modified_time, 0) AS modified_time
             FROM drive_files
             WHERE parent_id IS NULL AND name = ? AND is_trashed = 0",
        )
        .bind(name)
        .fetch_optional(pool)
        .await?
    } else {
        // Non-root: files whose parent has the given inode
        sqlx::query_as::<_, FileMetaRow>(
            "SELECT f.rowid, f.id, f.name, f.mime_type, COALESCE(f.size, 0) AS size,
                    COALESCE(f.modified_time, 0) AS modified_time
             FROM drive_files f
             JOIN drive_files parent ON parent.id = f.parent_id
                AND parent.account_id = f.account_id
             WHERE parent.rowid = ? AND f.name = ? AND f.is_trashed = 0",
        )
        .bind(parent_inode as i64)
        .bind(name)
        .fetch_optional(pool)
        .await?
    };

    Ok(row.map(FileMeta::from))
}

/// List all children (files and folders) under the given parent inode.
pub async fn list_children_by_inode(
    pool: &SqlitePool,
    parent_inode: u64,
) -> anyhow::Result<Vec<FileMeta>> {
    let rows = if parent_inode == ROOT_INODE {
        // Root children: parent_id IS NULL
        sqlx::query_as::<_, FileMetaRow>(
            "SELECT rowid, id, name, mime_type, COALESCE(size, 0) AS size,
                    COALESCE(modified_time, 0) AS modified_time
             FROM drive_files
             WHERE parent_id IS NULL AND is_trashed = 0
             ORDER BY name",
        )
        .fetch_all(pool)
        .await?
    } else {
        // Non-root: children of the file with the given inode
        sqlx::query_as::<_, FileMetaRow>(
            "SELECT f.rowid, f.id, f.name, f.mime_type, COALESCE(f.size, 0) AS size,
                    COALESCE(f.modified_time, 0) AS modified_time
             FROM drive_files f
             JOIN drive_files parent ON parent.id = f.parent_id
                AND parent.account_id = f.account_id
             WHERE parent.rowid = ? AND f.is_trashed = 0
             ORDER BY f.name",
        )
        .bind(parent_inode as i64)
        .fetch_all(pool)
        .await?
    };

    Ok(rows.into_iter().map(FileMeta::from).collect())
}

/// Return extended file details including sync state and cache path.
pub async fn get_file_details_by_inode(
    pool: &SqlitePool,
    inode: u64,
) -> anyhow::Result<Option<FileDetails>> {
    let row = sqlx::query_as::<_, FileDetailsRow>(
        "SELECT rowid, id, account_id, name, mime_type, COALESCE(size, 0) AS size,
                COALESCE(modified_time, 0) AS modified_time,
                sync_state, local_path
         FROM drive_files
         WHERE rowid = ? AND is_trashed = 0",
    )
    .bind(inode as i64)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(FileDetails::from))
}

/// Enqueue a download task in the sync queue so the sync engine picks it up.
pub async fn enqueue_download_task(
    pool: &SqlitePool,
    account_id: &str,
    file_id: &str,
    local_path: &str,
) -> anyhow::Result<i64> {
    let now = chrono::Utc::now().timestamp_millis();
    let row = sqlx::query_scalar::<_, i64>(
        "INSERT INTO sync_queue (account_id, file_id, operation, local_path, priority, status,
                                 retry_count, created_at, updated_at)
         VALUES (?, ?, 'download', ?, 1, 'pending', 0, ?, ?)
         RETURNING id",
    )
    .bind(account_id)
    .bind(file_id)
    .bind(local_path)
    .bind(now)
    .bind(now)
    .fetch_one(pool)
    .await?;

    Ok(row)
}

/// Poll until a download task is completed or timeout is reached.
///
/// Returns `true` if the file's `sync_state` changed to `cached`.
pub async fn wait_for_download(
    pool: &SqlitePool,
    file_id: &str,
    account_id: &str,
    timeout_ms: u64,
) -> anyhow::Result<bool> {
    let start = std::time::Instant::now();
    loop {
        let state: Option<String> = sqlx::query_scalar(
            "SELECT sync_state FROM drive_files WHERE id = ? AND account_id = ?",
        )
        .bind(file_id)
        .bind(account_id)
        .fetch_optional(pool)
        .await?
        .flatten();

        match state.as_deref() {
            Some("cached") | Some("synced") | Some("offline") => return Ok(true),
            Some("error") => return Ok(false),
            _ => {}
        }

        if start.elapsed().as_millis() as u64 >= timeout_ms {
            return Ok(false);
        }

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

// ─── Write-path queries ─────────────────────────────────────────────────────

/// Return `(account_id, file_id)` for the given parent inode.
///
/// Returns `None` for the root inode (root has no DB row).
pub async fn get_parent_info(
    pool: &SqlitePool,
    parent_inode: u64,
) -> anyhow::Result<Option<(String, Option<String>)>> {
    if parent_inode == ROOT_INODE {
        return Ok(None);
    }
    let row = sqlx::query_as::<_, ParentInfoRow>(
        "SELECT account_id, id FROM drive_files WHERE rowid = ? AND is_trashed = 0",
    )
    .bind(parent_inode as i64)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| (r.account_id, Some(r.id))))
}

/// Return the ID of the first connected account, if any.
pub async fn get_first_account_id(pool: &SqlitePool) -> anyhow::Result<Option<String>> {
    let row = sqlx::query_scalar::<_, String>(
        "SELECT id FROM accounts ORDER BY last_used_at DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Insert a locally-created file into `drive_files` and return its inode (rowid).
pub async fn insert_local_file(
    pool: &SqlitePool,
    file_id: &str,
    account_id: &str,
    name: &str,
    mime_type: &str,
    parent_id: Option<&str>,
    local_path: &str,
) -> anyhow::Result<u64> {
    let now = chrono::Utc::now().timestamp_millis();
    let rowid = sqlx::query_scalar::<_, i64>(
        "INSERT INTO drive_files (id, account_id, name, mime_type, parent_id, size,
                                  modified_time, is_trashed, is_shared, local_path, sync_state)
         VALUES (?, ?, ?, ?, ?, 0, ?, 0, 0, ?, 'modified')
         RETURNING rowid",
    )
    .bind(file_id)
    .bind(account_id)
    .bind(name)
    .bind(mime_type)
    .bind(parent_id)
    .bind(now)
    .bind(local_path)
    .fetch_one(pool)
    .await?;
    Ok(rowid as u64)
}

/// Insert a locally-created folder into `drive_files` and return its inode (rowid).
pub async fn insert_local_folder(
    pool: &SqlitePool,
    file_id: &str,
    account_id: &str,
    name: &str,
    parent_id: Option<&str>,
) -> anyhow::Result<u64> {
    let now = chrono::Utc::now().timestamp_millis();
    let rowid = sqlx::query_scalar::<_, i64>(
        "INSERT INTO drive_files (id, account_id, name, mime_type, parent_id, size,
                                  modified_time, is_trashed, is_shared, sync_state)
         VALUES (?, ?, ?, 'application/vnd.google-apps.folder', ?, 0, ?, 0, 0, 'cloud_only')
         RETURNING rowid",
    )
    .bind(file_id)
    .bind(account_id)
    .bind(name)
    .bind(parent_id)
    .bind(now)
    .fetch_one(pool)
    .await?;
    Ok(rowid as u64)
}

/// Update the cached size and modification time for a file.
pub async fn update_file_size_mtime(
    pool: &SqlitePool,
    inode: u64,
    size: i64,
    modified_time: i64,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE drive_files SET size = ?, modified_time = ?, sync_state = 'modified'
         WHERE rowid = ?",
    )
    .bind(size)
    .bind(modified_time)
    .bind(inode as i64)
    .execute(pool)
    .await?;
    Ok(())
}

/// Enqueue an upload task for a locally-modified file.
pub async fn enqueue_upload_task(
    pool: &SqlitePool,
    account_id: &str,
    file_id: &str,
    local_path: &str,
) -> anyhow::Result<i64> {
    let now = chrono::Utc::now().timestamp_millis();
    let row = sqlx::query_scalar::<_, i64>(
        "INSERT INTO sync_queue (account_id, file_id, operation, local_path, priority, status,
                                 retry_count, created_at, updated_at)
         VALUES (?, ?, 'upload', ?, 1, 'pending', 0, ?, ?)
         RETURNING id",
    )
    .bind(account_id)
    .bind(file_id)
    .bind(local_path)
    .bind(now)
    .bind(now)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

// ─── File-operation queries (unlink / rename / mkdir / rmdir) ─────────────────

/// Soft-delete a file by inode: set `is_trashed = 1`.
pub async fn soft_delete_by_inode(pool: &SqlitePool, inode: u64) -> anyhow::Result<()> {
    sqlx::query("UPDATE drive_files SET is_trashed = 1 WHERE rowid = ?")
        .bind(inode as i64)
        .execute(pool)
        .await?;
    Ok(())
}

/// Rename a file and optionally move it to a new parent.
pub async fn rename_file(
    pool: &SqlitePool,
    inode: u64,
    new_name: &str,
    new_parent_id: Option<&str>,
) -> anyhow::Result<()> {
    let now = chrono::Utc::now().timestamp_millis();
    sqlx::query(
        "UPDATE drive_files SET name = ?, parent_id = ?, modified_time = ?, sync_state = 'modified'
         WHERE rowid = ?",
    )
    .bind(new_name)
    .bind(new_parent_id)
    .bind(now)
    .bind(inode as i64)
    .execute(pool)
    .await?;
    Ok(())
}

/// Return `true` if the directory at `inode` has any non-trashed children.
pub async fn has_children(pool: &SqlitePool, inode: u64) -> anyhow::Result<bool> {
    if inode == ROOT_INODE {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM drive_files WHERE parent_id IS NULL AND is_trashed = 0",
        )
        .fetch_one(pool)
        .await?;
        return Ok(count > 0);
    }

    // Find the directory's file_id first, then check for children.
    let dir_id: Option<String> = sqlx::query_scalar(
        "SELECT id FROM drive_files WHERE rowid = ?",
    )
    .bind(inode as i64)
    .fetch_optional(pool)
    .await?
    .flatten();

    match dir_id {
        Some(ref dir_id) => {
            let count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM drive_files WHERE parent_id = ? AND is_trashed = 0",
            )
            .bind(dir_id)
            .fetch_one(pool)
            .await?;
            Ok(count > 0)
        }
        None => Ok(false),
    }
}

/// Generic task enqueue for sync operations (delete, rename, move).
pub async fn enqueue_task(
    pool: &SqlitePool,
    account_id: &str,
    file_id: &str,
    operation: &str,
    local_path: Option<&str>,
) -> anyhow::Result<i64> {
    let now = chrono::Utc::now().timestamp_millis();
    let row = sqlx::query_scalar::<_, i64>(
        "INSERT INTO sync_queue (account_id, file_id, operation, local_path, priority, status,
                                 retry_count, created_at, updated_at)
         VALUES (?, ?, ?, ?, 1, 'pending', 0, ?, ?)
         RETURNING id",
    )
    .bind(account_id)
    .bind(file_id)
    .bind(operation)
    .bind(local_path)
    .bind(now)
    .bind(now)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

// ─── Inner row types ────────────────────────────────────────────────────────────

#[derive(Debug, sqlx::FromRow)]
struct FileMetaRow {
    rowid: i64,
    id: String,
    name: String,
    mime_type: String,
    size: i64,
    modified_time: i64,
}

impl From<FileMetaRow> for FileMeta {
    fn from(r: FileMetaRow) -> Self {
        Self {
            inode: r.rowid as u64,
            file_id: r.id,
            name: r.name,
            mime_type: r.mime_type,
            size: r.size,
            modified_time: r.modified_time,
        }
    }
}

#[derive(Debug, sqlx::FromRow)]
struct FileDetailsRow {
    rowid: i64,
    id: String,
    account_id: String,
    name: String,
    mime_type: String,
    size: i64,
    modified_time: i64,
    sync_state: String,
    local_path: Option<String>,
}

impl From<FileDetailsRow> for FileDetails {
    fn from(r: FileDetailsRow) -> Self {
        Self {
            inode: r.rowid as u64,
            file_id: r.id,
            account_id: r.account_id,
            name: r.name,
            mime_type: r.mime_type,
            size: r.size,
            modified_time: r.modified_time,
            sync_state: r.sync_state,
            local_path: r.local_path,
        }
    }
}

#[derive(Debug, sqlx::FromRow)]
struct ParentInfoRow {
    account_id: String,
    id: String,
}
