use sqlx::SqlitePool;

// ─── Public domain type ────────────────────────────────────────────────────────

/// A row in the `sync_queue` table representing one pending or in-progress task.
#[derive(Debug, Clone)]
pub struct SyncTask {
    pub id: Option<i64>,
    pub account_id: String,
    pub file_id: Option<String>,
    /// One of: `upload`, `download`, `delete`, `rename`, `move`.
    pub operation: String,
    pub local_path: Option<String>,
    /// 1 (highest) – 10 (lowest).
    pub priority: i32,
    /// One of: `pending`, `in_progress`, `completed`, `failed`.
    pub status: String,
    pub retry_count: i32,
    pub error_msg: Option<String>,
    /// Unix milliseconds.
    pub created_at: i64,
    /// Unix milliseconds.
    pub updated_at: i64,
}

// ─── Internal row type ────────────────────────────────────────────────────────

#[derive(Debug, sqlx::FromRow)]
struct SyncTaskRow {
    id: i64,
    account_id: String,
    file_id: Option<String>,
    operation: String,
    local_path: Option<String>,
    priority: i32,
    status: String,
    retry_count: i32,
    error_msg: Option<String>,
    created_at: i64,
    updated_at: i64,
}

impl From<SyncTaskRow> for SyncTask {
    fn from(r: SyncTaskRow) -> Self {
        Self {
            id: Some(r.id),
            account_id: r.account_id,
            file_id: r.file_id,
            operation: r.operation,
            local_path: r.local_path,
            priority: r.priority,
            status: r.status,
            retry_count: r.retry_count,
            error_msg: r.error_msg,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

// ─── CRUD ─────────────────────────────────────────────────────────────────────

/// Insert a new task into the queue.  The returned `SyncTask` has its `id` field
/// populated with the auto-incremented row id.
pub async fn enqueue(pool: &SqlitePool, task: &SyncTask) -> anyhow::Result<SyncTask> {
    let row = sqlx::query_as::<_, SyncTaskRow>(
        r#"
        INSERT INTO sync_queue
            (account_id, file_id, operation, local_path, priority, status,
             retry_count, error_msg, created_at, updated_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        RETURNING id, account_id, file_id, operation, local_path, priority,
                  status, retry_count, error_msg, created_at, updated_at
        "#,
    )
    .bind(&task.account_id)
    .bind(&task.file_id)
    .bind(&task.operation)
    .bind(&task.local_path)
    .bind(task.priority)
    .bind(&task.status)
    .bind(task.retry_count)
    .bind(&task.error_msg)
    .bind(task.created_at)
    .bind(task.updated_at)
    .fetch_one(pool)
    .await?;

    Ok(SyncTask::from(row))
}

/// Return the next `pending` task ordered by priority (lowest = highest
/// priority), then by `created_at` (FIFO within the same priority).  Returns
/// `None` when the queue is empty.
pub async fn next_pending_task(pool: &SqlitePool) -> anyhow::Result<Option<SyncTask>> {
    let row = sqlx::query_as::<_, SyncTaskRow>(
        "SELECT id, account_id, file_id, operation, local_path, priority,
                status, retry_count, error_msg, created_at, updated_at
         FROM sync_queue
         WHERE status = 'pending'
         ORDER BY priority ASC, created_at ASC
         LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(SyncTask::from))
}

/// Update the status (and optionally the error message) of a task.
pub async fn update_task_status(
    pool: &SqlitePool,
    id: i64,
    status: &str,
    error_msg: Option<&str>,
) -> anyhow::Result<()> {
    let now = chrono::Utc::now().timestamp_millis();

    sqlx::query("UPDATE sync_queue SET status = ?, error_msg = ?, updated_at = ? WHERE id = ?")
        .bind(status)
        .bind(error_msg)
        .bind(now)
        .bind(id)
        .execute(pool)
        .await?;

    Ok(())
}

/// Update a task's status, retry count, and error message in one call.
/// Increments `retry_count` so the engine can enforce a max-retry limit.
pub async fn update_task_retry(
    pool: &SqlitePool,
    id: i64,
    status: &str,
    retry_count: i32,
    error_msg: Option<&str>,
) -> anyhow::Result<()> {
    let now = chrono::Utc::now().timestamp_millis();

    sqlx::query(
        "UPDATE sync_queue SET status = ?, retry_count = ?, error_msg = ?, updated_at = ? WHERE id = ?",
    )
    .bind(status)
    .bind(retry_count)
    .bind(error_msg)
    .bind(now)
    .bind(id)
    .execute(pool)
    .await?;

    Ok(())
}

/// Return all tasks that are currently `in_progress`.  Used at startup to
/// recover tasks that were running when the daemon was killed.
#[allow(dead_code)]
pub async fn get_in_progress_tasks(pool: &SqlitePool) -> anyhow::Result<Vec<SyncTask>> {
    let rows = sqlx::query_as::<_, SyncTaskRow>(
        "SELECT id, account_id, file_id, operation, local_path, priority,
                status, retry_count, error_msg, created_at, updated_at
         FROM sync_queue
         WHERE status = 'in_progress'
         ORDER BY priority ASC, created_at ASC",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(SyncTask::from).collect())
}

/// Reset every `in_progress` task back to `pending` so it will be picked up
/// again on the next worker tick.  Called at startup for crash recovery.
#[allow(dead_code)]
pub async fn reset_in_progress_to_pending(pool: &SqlitePool) -> anyhow::Result<u64> {
    let now = chrono::Utc::now().timestamp_millis();

    let result = sqlx::query(
        "UPDATE sync_queue SET status = 'pending', updated_at = ? WHERE status = 'in_progress'",
    )
    .bind(now)
    .execute(pool)
    .await?;

    Ok(result.rows_affected())
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use sqlx::{
        sqlite::{SqliteConnectOptions, SqlitePoolOptions},
        Row,
    };

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

    fn make_task(operation: &str, priority: i32, status: &str) -> SyncTask {
        SyncTask {
            id: None,
            account_id: "acct-1".into(),
            file_id: Some("file-1".into()),
            operation: operation.into(),
            local_path: Some("/tmp/test.txt".into()),
            priority,
            status: status.into(),
            retry_count: 0,
            error_msg: None,
            created_at: 1_700_000_000_000,
            updated_at: 1_700_000_000_000,
        }
    }

    // ── enqueue ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn enqueue_returns_populated_id() {
        let pool = test_pool().await;
        let task = make_task("upload", 5, "pending");

        let inserted = enqueue(&pool, &task).await.unwrap();

        assert!(inserted.id.is_some());
        assert_eq!(inserted.operation, "upload");
        assert_eq!(inserted.status, "pending");
    }

    #[tokio::test]
    async fn enqueue_persists_all_fields() {
        let pool = test_pool().await;
        let task = SyncTask {
            id: None,
            account_id: "acct-2".into(),
            file_id: Some("drive-abc".into()),
            operation: "download".into(),
            local_path: Some("/downloads/file.pdf".into()),
            priority: 3,
            status: "pending".into(),
            retry_count: 2,
            error_msg: Some("previous timeout".into()),
            created_at: 1_700_000_001_000,
            updated_at: 1_700_000_001_000,
        };

        let inserted = enqueue(&pool, &task).await.unwrap();
        let id = inserted.id.unwrap();

        // Fetch back via raw query to verify all fields
        let row = sqlx::query_as::<_, SyncTaskRow>(
            "SELECT id, account_id, file_id, operation, local_path, priority,
                    status, retry_count, error_msg, created_at, updated_at
             FROM sync_queue WHERE id = ?",
        )
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();

        assert_eq!(row.account_id, "acct-2");
        assert_eq!(row.file_id, Some("drive-abc".into()));
        assert_eq!(row.operation, "download");
        assert_eq!(row.priority, 3);
        assert_eq!(row.retry_count, 2);
        assert_eq!(row.error_msg, Some("previous timeout".into()));
        assert_eq!(row.created_at, 1_700_000_001_000);
    }

    // ── next_pending_task ─────────────────────────────────────────────────

    #[tokio::test]
    async fn next_pending_returns_highest_priority_first() {
        let pool = test_pool().await;

        // Insert tasks with different priorities
        enqueue(&pool, &make_task("upload", 5, "pending"))
            .await
            .unwrap();
        enqueue(&pool, &make_task("download", 1, "pending"))
            .await
            .unwrap(); // highest prio
        enqueue(&pool, &make_task("delete", 10, "pending"))
            .await
            .unwrap();

        let next = next_pending_task(&pool).await.unwrap().unwrap();
        assert_eq!(next.priority, 1);
        assert_eq!(next.operation, "download");
    }

    #[tokio::test]
    async fn next_pending_fifo_within_same_priority() {
        let pool = test_pool().await;

        let mut a = make_task("upload-a", 5, "pending");
        a.created_at = 1_000;
        enqueue(&pool, &a).await.unwrap();

        let mut b = make_task("upload-b", 5, "pending");
        b.created_at = 500;
        enqueue(&pool, &b).await.unwrap();

        // b was created first → should be returned first
        let next = next_pending_task(&pool).await.unwrap().unwrap();
        assert_eq!(next.operation, "upload-b");
    }

    #[tokio::test]
    async fn next_pending_skips_non_pending() {
        let pool = test_pool().await;

        enqueue(&pool, &make_task("upload", 1, "in_progress"))
            .await
            .unwrap();
        enqueue(&pool, &make_task("download", 1, "completed"))
            .await
            .unwrap();
        enqueue(&pool, &make_task("delete", 1, "failed"))
            .await
            .unwrap();

        let next = next_pending_task(&pool).await.unwrap();
        assert!(next.is_none());
    }

    #[tokio::test]
    async fn next_pending_empty_queue_returns_none() {
        let pool = test_pool().await;
        let next = next_pending_task(&pool).await.unwrap();
        assert!(next.is_none());
    }

    // ── update_task_status ────────────────────────────────────────────────

    #[tokio::test]
    async fn update_task_status_changes_status() {
        let pool = test_pool().await;
        let task = enqueue(&pool, &make_task("upload", 5, "pending"))
            .await
            .unwrap();
        let id = task.id.unwrap();

        update_task_status(&pool, id, "completed", None)
            .await
            .unwrap();

        let row = sqlx::query("SELECT status, error_msg FROM sync_queue WHERE id = ?")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();

        assert_eq!(row.get::<String, _>(0), "completed");
        assert!(row.get::<Option<String>, _>(1).is_none());
    }

    #[tokio::test]
    async fn update_task_status_sets_error_msg() {
        let pool = test_pool().await;
        let task = enqueue(&pool, &make_task("upload", 5, "pending"))
            .await
            .unwrap();
        let id = task.id.unwrap();

        update_task_status(&pool, id, "failed", Some("network error"))
            .await
            .unwrap();

        let row = sqlx::query("SELECT status, error_msg FROM sync_queue WHERE id = ?")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();

        assert_eq!(row.get::<String, _>(0), "failed");
        assert_eq!(
            row.get::<Option<String>, _>(1).as_deref(),
            Some("network error")
        );
    }

    // ── update_task_retry ─────────────────────────────────────────────────

    #[tokio::test]
    async fn update_task_retry_increments_count() {
        let pool = test_pool().await;
        let task = enqueue(&pool, &make_task("upload", 5, "pending"))
            .await
            .unwrap();
        let id = task.id.unwrap();

        update_task_retry(&pool, id, "pending", 1, Some("timeout"))
            .await
            .unwrap();

        let row = sqlx::query("SELECT status, retry_count, error_msg FROM sync_queue WHERE id = ?")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();

        assert_eq!(row.get::<String, _>(0), "pending");
        assert_eq!(row.get::<i32, _>(1), 1);
        assert_eq!(row.get::<Option<String>, _>(2).as_deref(), Some("timeout"));
    }

    // ── get_in_progress_tasks ──────────────────────────────────────────────

    #[tokio::test]
    async fn get_in_progress_returns_only_running_tasks() {
        let pool = test_pool().await;

        enqueue(&pool, &make_task("upload", 1, "in_progress"))
            .await
            .unwrap();
        enqueue(&pool, &make_task("download", 2, "in_progress"))
            .await
            .unwrap();
        enqueue(&pool, &make_task("delete", 3, "pending"))
            .await
            .unwrap();

        let in_progress = get_in_progress_tasks(&pool).await.unwrap();
        assert_eq!(in_progress.len(), 2);
        assert!(in_progress.iter().all(|t| t.status == "in_progress"));
    }

    #[tokio::test]
    async fn get_in_progress_empty_when_none_running() {
        let pool = test_pool().await;

        enqueue(&pool, &make_task("upload", 1, "pending"))
            .await
            .unwrap();
        enqueue(&pool, &make_task("download", 2, "completed"))
            .await
            .unwrap();

        let in_progress = get_in_progress_tasks(&pool).await.unwrap();
        assert!(in_progress.is_empty());
    }

    // ── reset_in_progress_to_pending ───────────────────────────────────────

    #[tokio::test]
    async fn reset_in_progress_moves_all_to_pending() {
        let pool = test_pool().await;

        enqueue(&pool, &make_task("upload", 1, "in_progress"))
            .await
            .unwrap();
        enqueue(&pool, &make_task("download", 2, "in_progress"))
            .await
            .unwrap();
        enqueue(&pool, &make_task("delete", 3, "pending"))
            .await
            .unwrap();

        let affected = reset_in_progress_to_pending(&pool).await.unwrap();
        assert_eq!(affected, 2);

        // Now all 3 should be pending
        let pending_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM sync_queue WHERE status = 'pending'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(pending_count, 3);
    }

    #[tokio::test]
    async fn reset_in_progress_noop_when_none() {
        let pool = test_pool().await;

        enqueue(&pool, &make_task("upload", 1, "pending"))
            .await
            .unwrap();
        enqueue(&pool, &make_task("download", 2, "completed"))
            .await
            .unwrap();

        let affected = reset_in_progress_to_pending(&pool).await.unwrap();
        assert_eq!(affected, 0);
    }
}
