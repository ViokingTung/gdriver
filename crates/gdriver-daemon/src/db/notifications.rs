use sqlx::SqlitePool;

// ─── Public domain type ────────────────────────────────────────────────────────

/// A row in the `notifications` table.
#[derive(Debug, Clone)]
pub struct Notification {
    pub id: Option<i64>,
    pub account_id: Option<String>,
    pub kind: String,
    pub payload: String,
    pub is_read: bool,
    pub created_at: i64,
}

// ─── Internal row type ────────────────────────────────────────────────────────

#[derive(Debug, sqlx::FromRow)]
struct NotificationRow {
    id: i64,
    account_id: Option<String>,
    kind: String,
    payload: String,
    is_read: i64,
    created_at: i64,
}

impl From<NotificationRow> for Notification {
    fn from(r: NotificationRow) -> Self {
        Self {
            id: Some(r.id),
            account_id: r.account_id,
            kind: r.kind,
            payload: r.payload,
            is_read: r.is_read != 0,
            created_at: r.created_at,
        }
    }
}

// ─── CRUD ─────────────────────────────────────────────────────────────────────

/// Insert a new notification and return it with the assigned id.
#[allow(dead_code)]
pub async fn insert_notification(
    pool: &SqlitePool,
    notification: &Notification,
) -> anyhow::Result<Notification> {
    let row = sqlx::query_as::<_, NotificationRow>(
        r#"
        INSERT INTO notifications (account_id, kind, payload, is_read, created_at)
        VALUES (?, ?, ?, ?, ?)
        RETURNING id, account_id, kind, payload, is_read, created_at
        "#,
    )
    .bind(&notification.account_id)
    .bind(&notification.kind)
    .bind(&notification.payload)
    .bind(notification.is_read as i64)
    .bind(notification.created_at)
    .fetch_one(pool)
    .await?;

    Ok(Notification::from(row))
}

/// List notifications, most recent first.  If `unreadOnly` is true, only
/// unread notifications are returned.
pub async fn list_notifications(
    pool: &SqlitePool,
    unread_only: bool,
    limit: u32,
) -> anyhow::Result<Vec<Notification>> {
    let rows = if unread_only {
        sqlx::query_as::<_, NotificationRow>(
            "SELECT id, account_id, kind, payload, is_read, created_at
             FROM notifications
             WHERE is_read = 0
             ORDER BY created_at DESC
             LIMIT ?",
        )
        .bind(limit)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, NotificationRow>(
            "SELECT id, account_id, kind, payload, is_read, created_at
             FROM notifications
             ORDER BY created_at DESC
             LIMIT ?",
        )
        .bind(limit)
        .fetch_all(pool)
        .await?
    };

    Ok(rows.into_iter().map(Notification::from).collect())
}

/// Mark a notification as read.
pub async fn mark_read(pool: &SqlitePool, id: i64) -> anyhow::Result<()> {
    sqlx::query("UPDATE notifications SET is_read = 1 WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Mark all notifications as read.
pub async fn mark_all_read(pool: &SqlitePool) -> anyhow::Result<()> {
    sqlx::query("UPDATE notifications SET is_read = 1 WHERE is_read = 0")
        .execute(pool)
        .await?;
    Ok(())
}

/// Delete a notification.
pub async fn dismiss_notification(pool: &SqlitePool, id: i64) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM notifications WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Count unread notifications.
#[allow(dead_code)]
pub async fn count_unread(pool: &SqlitePool) -> anyhow::Result<i64> {
    let row: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM notifications WHERE is_read = 0")
            .fetch_one(pool)
            .await?;
    Ok(row.0)
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

    fn make_notification(kind: &str, payload: &str) -> Notification {
        Notification {
            id: None,
            account_id: Some("acct-1".into()),
            kind: kind.into(),
            payload: payload.into(),
            is_read: false,
            created_at: chrono::Utc::now().timestamp_millis(),
        }
    }

    #[tokio::test]
    async fn insert_and_list() {
        let pool = test_pool().await;
        insert_notification(&pool, &make_notification("sync_error", r#"{"file_name":"a.txt","error_msg":"fail","error_id":1}"#))
            .await
            .unwrap();
        insert_notification(&pool, &make_notification("conflict", r#"{"file_name":"b.txt","conflict_copy_name":"b (conflict).txt"}"#))
            .await
            .unwrap();

        let all = list_notifications(&pool, false, 10).await.unwrap();
        assert_eq!(all.len(), 2);

        let unread = list_notifications(&pool, true, 10).await.unwrap();
        assert_eq!(unread.len(), 2);
    }

    #[tokio::test]
    async fn mark_read_and_count() {
        let pool = test_pool().await;
        let n = insert_notification(&pool, &make_notification("sync_error", "{}"))
            .await
            .unwrap();
        assert_eq!(count_unread(&pool).await.unwrap(), 1);

        mark_read(&pool, n.id.unwrap()).await.unwrap();
        assert_eq!(count_unread(&pool).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn dismiss_removes_notification() {
        let pool = test_pool().await;
        let n = insert_notification(&pool, &make_notification("sync_error", "{}"))
            .await
            .unwrap();
        super::dismiss_notification(&pool, n.id.unwrap()).await.unwrap();
        let all = list_notifications(&pool, false, 10).await.unwrap();
        assert!(all.is_empty());
    }

    #[tokio::test]
    async fn mark_all_as_read() {
        let pool = test_pool().await;
        insert_notification(&pool, &make_notification("sync_error", "{}"))
            .await
            .unwrap();
        insert_notification(&pool, &make_notification("conflict", "{}"))
            .await
            .unwrap();
        assert_eq!(count_unread(&pool).await.unwrap(), 2);

        super::mark_all_read(&pool).await.unwrap();
        assert_eq!(count_unread(&pool).await.unwrap(), 0);
    }
}
