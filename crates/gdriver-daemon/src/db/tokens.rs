use sqlx::SqlitePool;

/// Return the stored page token for an account, or `None` if not present.
pub async fn get_token(pool: &SqlitePool, account_id: &str) -> anyhow::Result<Option<String>> {
    let row = sqlx::query_scalar::<_, String>(
        "SELECT page_token FROM sync_tokens WHERE account_id = ?",
    )
    .bind(account_id)
    .fetch_optional(pool)
    .await?;

    Ok(row)
}

/// Insert or replace the page token for an account.
pub async fn set_token(
    pool: &SqlitePool,
    account_id: &str,
    page_token: &str,
) -> anyhow::Result<()> {
    let now = chrono::Utc::now().timestamp_millis();

    sqlx::query(
        r#"
        INSERT INTO sync_tokens (account_id, page_token, updated_at)
        VALUES (?, ?, ?)
        ON CONFLICT(account_id) DO UPDATE SET
            page_token = excluded.page_token,
            updated_at  = excluded.updated_at
        "#,
    )
    .bind(account_id)
    .bind(page_token)
    .bind(now)
    .execute(pool)
    .await?;

    Ok(())
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

    /// Helper: insert a synthetic account so the FK constraint on sync_tokens
    /// is satisfied.
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

    #[tokio::test]
    async fn get_token_none_when_absent() {
        let pool = test_pool().await;
        let result = get_token(&pool, "acct-1").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn set_and_get_token() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;

        set_token(&pool, "acct-1", "page-token-123").await.unwrap();

        let token = get_token(&pool, "acct-1").await.unwrap();
        assert_eq!(token, Some("page-token-123".into()));
    }

    #[tokio::test]
    async fn set_token_overwrites_existing() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-2").await;

        set_token(&pool, "acct-2", "old-token").await.unwrap();
        set_token(&pool, "acct-2", "new-token").await.unwrap();

        let token = get_token(&pool, "acct-2").await.unwrap();
        assert_eq!(token, Some("new-token".into()));
    }

    #[tokio::test]
    async fn token_cascade_deletes_with_account() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-3").await;

        set_token(&pool, "acct-3", "token-xyz").await.unwrap();

        sqlx::query("DELETE FROM accounts WHERE id = ?")
            .bind("acct-3")
            .execute(&pool)
            .await
            .unwrap();

        let token = get_token(&pool, "acct-3").await.unwrap();
        assert!(token.is_none());
    }
}
