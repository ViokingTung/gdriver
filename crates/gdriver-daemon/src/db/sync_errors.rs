use sqlx::SqlitePool;

// ─── Public domain type ────────────────────────────────────────────────────────

/// A row in the `sync_errors` table recording a persistent sync error.
#[derive(Debug, Clone)]
pub struct SyncError {
    #[allow(dead_code)]
    pub id: Option<i64>,
    pub account_id: Option<String>,
    pub file_id: Option<String>,
    pub file_name: Option<String>,
    pub error_code: String,
    pub error_msg: String,
    pub is_resolved: bool,
    pub created_at: i64,
}

// ─── Internal row type ────────────────────────────────────────────────────────

#[derive(Debug, sqlx::FromRow)]
struct SyncErrorRow {
    id: i64,
    account_id: Option<String>,
    file_id: Option<String>,
    file_name: Option<String>,
    error_code: String,
    error_msg: String,
    is_resolved: i64,
    created_at: i64,
}

impl From<SyncErrorRow> for SyncError {
    fn from(r: SyncErrorRow) -> Self {
        Self {
            id: Some(r.id),
            account_id: r.account_id,
            file_id: r.file_id,
            file_name: r.file_name,
            error_code: r.error_code,
            error_msg: r.error_msg,
            is_resolved: r.is_resolved != 0,
            created_at: r.created_at,
        }
    }
}

// ─── CRUD ─────────────────────────────────────────────────────────────────────

/// Insert a new sync error record.
pub async fn insert_error(pool: &SqlitePool, error: &SyncError) -> anyhow::Result<SyncError> {
    let row = sqlx::query_as::<_, SyncErrorRow>(
        r#"
        INSERT INTO sync_errors (account_id, file_id, file_name, error_code, error_msg, is_resolved, created_at)
        VALUES (?, ?, ?, ?, ?, ?, ?)
        RETURNING id, account_id, file_id, file_name, error_code, error_msg, is_resolved, created_at
        "#,
    )
    .bind(&error.account_id)
    .bind(&error.file_id)
    .bind(&error.file_name)
    .bind(&error.error_code)
    .bind(&error.error_msg)
    .bind(error.is_resolved as i64)
    .bind(error.created_at)
    .fetch_one(pool)
    .await?;

    Ok(SyncError::from(row))
}

/// List unresolved errors, most recent first.
#[allow(dead_code)]
pub async fn list_unresolved(pool: &SqlitePool) -> anyhow::Result<Vec<SyncError>> {
    let rows = sqlx::query_as::<_, SyncErrorRow>(
        "SELECT id, account_id, file_id, file_name, error_code, error_msg, is_resolved, created_at
         FROM sync_errors
         WHERE is_resolved = 0
         ORDER BY created_at DESC
         LIMIT 100",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(SyncError::from).collect())
}

/// Mark an error as resolved.
pub async fn resolve_error(pool: &SqlitePool, id: i64) -> anyhow::Result<()> {
    sqlx::query("UPDATE sync_errors SET is_resolved = 1 WHERE id = ?")
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

    fn make_error(code: &str, msg: &str, resolved: bool) -> SyncError {
        SyncError {
            id: None,
            account_id: Some("acct-1".into()),
            file_id: Some("file-1".into()),
            file_name: Some("test.txt".into()),
            error_code: code.into(),
            error_msg: msg.into(),
            is_resolved: resolved,
            created_at: chrono::Utc::now().timestamp_millis(),
        }
    }

    #[tokio::test]
    async fn insert_error_returns_id() {
        let pool = test_pool().await;
        let e = insert_error(&pool, &make_error("UPLOAD_FAILED", "timeout", false))
            .await
            .unwrap();
        assert!(e.id.is_some());
        assert_eq!(e.error_code, "UPLOAD_FAILED");
    }

    #[tokio::test]
    async fn list_unresolved_filters_resolved() {
        let pool = test_pool().await;
        insert_error(&pool, &make_error("ERR_A", "msg a", false))
            .await
            .unwrap();
        let e2 = insert_error(&pool, &make_error("ERR_B", "msg b", false))
            .await
            .unwrap();
        resolve_error(&pool, e2.id.unwrap()).await.unwrap();

        let unresolved = list_unresolved(&pool).await.unwrap();
        assert_eq!(unresolved.len(), 1);
        assert_eq!(unresolved[0].error_code, "ERR_A");
    }

    #[tokio::test]
    async fn resolve_error_nonexistent_is_noop() {
        let pool = test_pool().await;
        let result = resolve_error(&pool, 999).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn insert_error_without_file_info() {
        let pool = test_pool().await;
        let e = SyncError {
            id: None,
            account_id: None,
            file_id: None,
            file_name: None,
            error_code: "GENERIC".into(),
            error_msg: "something went wrong".into(),
            is_resolved: false,
            created_at: chrono::Utc::now().timestamp_millis(),
        };
        let inserted = insert_error(&pool, &e).await.unwrap();
        assert!(inserted.file_name.is_none());
    }
}
