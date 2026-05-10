use sqlx::SqlitePool;

// ─── Public domain type ────────────────────────────────────────────────────────

/// A row in the `sync_folders` table representing a configured sync folder.
#[derive(Debug, Clone)]
pub struct SyncFolder {
    #[allow(dead_code)]
    pub id: Option<i64>,
    pub account_id: String,
    pub local_path: String,
    /// One of: `drive`, `photos`.
    pub folder_type: String,
    #[allow(dead_code)]
    pub is_enabled: bool,
}

// ─── Internal row type ────────────────────────────────────────────────────────

#[derive(Debug, sqlx::FromRow)]
pub(crate) struct SyncFolderRow {
    pub(crate) id: i64,
    pub(crate) account_id: String,
    pub(crate) local_path: String,
    pub(crate) folder_type: String,
    pub(crate) is_enabled: i64,
}

impl From<SyncFolderRow> for SyncFolder {
    fn from(r: SyncFolderRow) -> Self {
        Self {
            id: Some(r.id),
            account_id: r.account_id,
            local_path: r.local_path,
            folder_type: r.folder_type,
            is_enabled: r.is_enabled != 0,
        }
    }
}

// ─── CRUD ─────────────────────────────────────────────────────────────────────

/// Return all sync folders that are currently enabled (watcher-ready).
pub async fn list_enabled(pool: &SqlitePool) -> anyhow::Result<Vec<SyncFolder>> {
    let rows = sqlx::query_as::<_, SyncFolderRow>(
        "SELECT id, account_id, local_path, folder_type, is_enabled
         FROM sync_folders
         WHERE is_enabled = 1
         ORDER BY id",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(SyncFolder::from).collect())
}

/// Return all sync folders for a given account.
#[allow(dead_code)]
pub async fn list_by_account(
    pool: &SqlitePool,
    account_id: &str,
) -> anyhow::Result<Vec<SyncFolder>> {
    let rows = sqlx::query_as::<_, SyncFolderRow>(
        "SELECT id, account_id, local_path, folder_type, is_enabled
         FROM sync_folders
         WHERE account_id = ?
         ORDER BY id",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(SyncFolder::from).collect())
}

/// Add a new sync folder. Returns the row with its auto-incremented `id`.
#[allow(dead_code)]
pub async fn add_folder(pool: &SqlitePool, folder: &SyncFolder) -> anyhow::Result<SyncFolder> {
    let row = sqlx::query_as::<_, SyncFolderRow>(
        r#"
        INSERT INTO sync_folders (account_id, local_path, folder_type, is_enabled)
        VALUES (?, ?, ?, ?)
        RETURNING id, account_id, local_path, folder_type, is_enabled
        "#,
    )
    .bind(&folder.account_id)
    .bind(&folder.local_path)
    .bind(&folder.folder_type)
    .bind(folder.is_enabled as i64)
    .fetch_one(pool)
    .await?;

    Ok(SyncFolder::from(row))
}

/// Delete a sync folder by id.
#[allow(dead_code)]
pub async fn delete_folder(pool: &SqlitePool, id: i64) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM sync_folders WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Set the `is_enabled` flag for a sync folder.
#[allow(dead_code)]
pub async fn set_enabled(pool: &SqlitePool, id: i64, enabled: bool) -> anyhow::Result<()> {
    sqlx::query("UPDATE sync_folders SET is_enabled = ? WHERE id = ?")
        .bind(enabled as i64)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

    use super::*;

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

    fn make_folder(account_id: &str, local_path: &str, folder_type: &str) -> SyncFolder {
        SyncFolder {
            id: None,
            account_id: account_id.into(),
            local_path: local_path.into(),
            folder_type: folder_type.into(),
            is_enabled: true,
        }
    }

    // ── add_folder / list_by_account ─────────────────────────────────────────

    #[tokio::test]
    async fn add_folder_returns_id() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;

        let folder = add_folder(&pool, &make_folder("acct-1", "/tmp/drive", "drive"))
            .await
            .unwrap();

        assert!(folder.id.is_some());
        assert_eq!(folder.local_path, "/tmp/drive");
        assert_eq!(folder.folder_type, "drive");
        assert!(folder.is_enabled);
    }

    #[tokio::test]
    async fn list_by_account_returns_correct_folders() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;
        insert_account(&pool, "acct-2").await;

        add_folder(&pool, &make_folder("acct-1", "/a", "drive"))
            .await
            .unwrap();
        add_folder(&pool, &make_folder("acct-1", "/b", "photos"))
            .await
            .unwrap();
        add_folder(&pool, &make_folder("acct-2", "/c", "drive"))
            .await
            .unwrap();

        let a1 = list_by_account(&pool, "acct-1").await.unwrap();
        assert_eq!(a1.len(), 2);

        let a2 = list_by_account(&pool, "acct-2").await.unwrap();
        assert_eq!(a2.len(), 1);
        assert_eq!(a2[0].local_path, "/c");
    }

    // ── list_enabled ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn list_enabled_filters_disabled() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;

        let _a = add_folder(&pool, &make_folder("acct-1", "/enabled", "drive"))
            .await
            .unwrap();
        let b = add_folder(&pool, &make_folder("acct-1", "/disabled", "drive"))
            .await
            .unwrap();

        set_enabled(&pool, b.id.unwrap(), false).await.unwrap();

        let enabled = list_enabled(&pool).await.unwrap();
        assert_eq!(enabled.len(), 1);
        assert_eq!(enabled[0].local_path, "/enabled");
    }

    // ── delete_folder ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn delete_folder_removes_row() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;

        let f = add_folder(&pool, &make_folder("acct-1", "/tmp/x", "drive"))
            .await
            .unwrap();
        let id = f.id.unwrap();

        delete_folder(&pool, id).await.unwrap();

        let rows = list_by_account(&pool, "acct-1").await.unwrap();
        assert!(rows.is_empty());
    }

    // ── set_enabled ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn set_enabled_toggles_flag() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;

        let f = add_folder(&pool, &make_folder("acct-1", "/toggle", "drive"))
            .await
            .unwrap();
        let id = f.id.unwrap();

        set_enabled(&pool, id, false).await.unwrap();

        let rows = list_by_account(&pool, "acct-1").await.unwrap();
        assert!(!rows[0].is_enabled);

        set_enabled(&pool, id, true).await.unwrap();

        let rows = list_by_account(&pool, "acct-1").await.unwrap();
        assert!(rows[0].is_enabled);
    }

    // ── FK cascade ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn account_deletion_cascades_to_folders() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-cascade").await;
        add_folder(&pool, &make_folder("acct-cascade", "/tmp/gone", "drive"))
            .await
            .unwrap();

        sqlx::query("DELETE FROM accounts WHERE id = ?")
            .bind("acct-cascade")
            .execute(&pool)
            .await
            .unwrap();

        let rows = list_enabled(&pool).await.unwrap();
        assert!(rows.is_empty());
    }

    // ── unique constraint ──────────────────────────────────────────────────

    #[tokio::test]
    async fn duplicate_local_path_conflicts() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;

        add_folder(&pool, &make_folder("acct-1", "/same", "drive"))
            .await
            .unwrap();

        let result = add_folder(&pool, &make_folder("acct-1", "/same", "drive")).await;
        assert!(result.is_err());
    }
}
