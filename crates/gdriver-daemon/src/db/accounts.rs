use sqlx::SqlitePool;

use gdriver_ipc::Account;

// ─── Internal row type ────────────────────────────────────────────────────────

/// SQLite row for the `accounts` table.
///
/// Exists only to bridge sqlx's `FromRow` derive with the shared `Account`
/// type defined in `gdriver-ipc`.  Converting via `From<AccountRow>` is free
/// (no allocation beyond the move).
#[derive(Debug, sqlx::FromRow)]
struct AccountRow {
    id: String,
    email: String,
    display_name: Option<String>,
    photo_url: Option<String>,
    locale: Option<String>,
    created_at: i64,
    last_used_at: i64,
}

impl From<AccountRow> for Account {
    fn from(r: AccountRow) -> Self {
        Self {
            id: r.id,
            email: r.email,
            display_name: r.display_name,
            photo_url: r.photo_url,
            locale: r.locale,
            created_at: r.created_at,
            last_used_at: r.last_used_at,
        }
    }
}

// ─── CRUD ─────────────────────────────────────────────────────────────────────

/// Insert a new account or update all mutable fields if the id already exists.
#[allow(dead_code)]
pub async fn insert_account(pool: &SqlitePool, account: &Account) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        INSERT INTO accounts
            (id, email, display_name, photo_url, locale, created_at, last_used_at)
        VALUES (?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(id) DO UPDATE SET
            email         = excluded.email,
            display_name  = excluded.display_name,
            photo_url     = excluded.photo_url,
            locale        = excluded.locale,
            last_used_at  = excluded.last_used_at
        "#,
    )
    .bind(&account.id)
    .bind(&account.email)
    .bind(&account.display_name)
    .bind(&account.photo_url)
    .bind(&account.locale)
    .bind(account.created_at)
    .bind(account.last_used_at)
    .execute(pool)
    .await?;

    Ok(())
}

/// Return the account with the given Google account ID, or `None` if absent.
#[allow(dead_code)]
pub async fn get_account(pool: &SqlitePool, id: &str) -> anyhow::Result<Option<Account>> {
    let row = sqlx::query_as::<_, AccountRow>(
        "SELECT id, email, display_name, photo_url, locale, created_at, last_used_at
         FROM accounts
         WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(Account::from))
}

/// Return all accounts, ordered by most recently used first.
pub async fn list_accounts(pool: &SqlitePool) -> anyhow::Result<Vec<Account>> {
    let rows = sqlx::query_as::<_, AccountRow>(
        "SELECT id, email, display_name, photo_url, locale, created_at, last_used_at
         FROM accounts
         ORDER BY last_used_at DESC",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(Account::from).collect())
}

/// Delete the account with the given ID.
#[allow(dead_code)]
///
/// Due to `ON DELETE CASCADE` constraints, all related rows in `drive_files`,
/// `sync_queue`, `sync_tokens`, and `sync_folders` are removed automatically.
/// Returns `Ok(())` even if no row matched (idempotent).
pub async fn delete_account(pool: &SqlitePool, id: &str) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM accounts WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;

    Ok(())
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

    /// Open an in-memory SQLite pool and run migrations.
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

    fn make_account(id: &str, email: &str) -> Account {
        Account {
            id: id.into(),
            email: email.into(),
            display_name: Some("Test User".into()),
            photo_url: None,
            locale: Some("en".into()),
            created_at: 1_700_000_000_000,
            last_used_at: 1_700_000_000_000,
        }
    }

    #[tokio::test]
    async fn insert_and_get() {
        let pool = test_pool().await;
        let acct = make_account("uid-1", "test@example.com");

        insert_account(&pool, &acct).await.unwrap();

        let fetched = get_account(&pool, "uid-1").await.unwrap().unwrap();
        assert_eq!(fetched.email, "test@example.com");
        assert_eq!(fetched.display_name, Some("Test User".into()));
    }

    #[tokio::test]
    async fn get_nonexistent_returns_none() {
        let pool = test_pool().await;
        let result = get_account(&pool, "nobody").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn insert_is_upsert() {
        let pool = test_pool().await;
        let acct = make_account("uid-2", "a@example.com");
        insert_account(&pool, &acct).await.unwrap();

        // Update email + display_name via upsert
        let updated = Account {
            email: "b@example.com".into(),
            display_name: Some("Updated".into()),
            last_used_at: 1_700_000_001_000,
            ..acct
        };
        insert_account(&pool, &updated).await.unwrap();

        let fetched = get_account(&pool, "uid-2").await.unwrap().unwrap();
        assert_eq!(fetched.email, "b@example.com");
        assert_eq!(fetched.display_name, Some("Updated".into()));
    }

    #[tokio::test]
    async fn list_accounts_ordered() {
        let pool = test_pool().await;

        let older = Account { last_used_at: 1_000, ..make_account("uid-3", "old@example.com") };
        let newer = Account { last_used_at: 2_000, ..make_account("uid-4", "new@example.com") };
        insert_account(&pool, &older).await.unwrap();
        insert_account(&pool, &newer).await.unwrap();

        let list = list_accounts(&pool).await.unwrap();
        assert_eq!(list.len(), 2);
        // Most recently used first
        assert_eq!(list[0].id, "uid-4");
        assert_eq!(list[1].id, "uid-3");
    }

    #[tokio::test]
    async fn delete_account_removes_row() {
        let pool = test_pool().await;
        let acct = make_account("uid-5", "del@example.com");
        insert_account(&pool, &acct).await.unwrap();

        delete_account(&pool, "uid-5").await.unwrap();

        let result = get_account(&pool, "uid-5").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn delete_nonexistent_is_ok() {
        let pool = test_pool().await;
        // Should not error
        delete_account(&pool, "does-not-exist").await.unwrap();
    }
}
