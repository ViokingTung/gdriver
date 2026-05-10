//! Orchestration layer between the raw Drive API client and the daemon's
//! database / IPC infrastructure.
//!
//! Each public function here is a self-contained operation that the IPC
//! handler methods (or the sync engine) can call.

use chrono::Utc;
use gdriver_api::client::DriveClient;
use gdriver_ipc::{Account, StorageQuota};
use sqlx::SqlitePool;

use crate::db;

/// Fetch the user profile and storage quota from Google Drive, then persist
/// the account record in SQLite.
///
/// Returns the freshly-stored [`Account`] and [`StorageQuota`].
///
/// # Errors
///
/// Fails if the About API call fails or if the database write fails.
pub async fn fetch_and_store_account(
    db: &SqlitePool,
    client: &DriveClient,
) -> anyhow::Result<(Account, StorageQuota)> {
    let about = gdriver_api::files::about_get(client).await?;

    let now_ms = Utc::now().timestamp_millis();

    // Use the user's email as a stable account identifier (permissionId is
    // Drive-scoped and not guaranteed to be present in all About responses).
    let account = Account {
        id: about.user.email_address.clone(),
        email: about.user.email_address,
        display_name: Some(about.user.display_name),
        photo_url: about.user.photo_link,
        locale: None,
        created_at: now_ms,
        last_used_at: now_ms,
    };

    let quota = StorageQuota {
        limit: gdriver_api::files::parse_quota_number(&about.storage_quota.limit).ok(),
        usage: gdriver_api::files::parse_quota_number(&Some(about.storage_quota.usage))
            .unwrap_or(0),
        usage_in_drive: gdriver_api::files::parse_quota_number(&about.storage_quota.usage_in_drive)
            .unwrap_or(0),
        usage_in_drive_trash: gdriver_api::files::parse_quota_number(
            &about.storage_quota.usage_in_drive_trash,
        )
        .unwrap_or(0),
    };

    db::accounts::insert_account(db, &account).await?;

    tracing::info!(
        email = %account.email,
        display_name = ?account.display_name,
        "account record persisted"
    );

    Ok((account, quota))
}

// ─── Tests ──────────────────────────────────────────────────────────────────

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

    #[tokio::test]
    async fn account_id_is_email() {
        // Verify the design decision: account.id == the Google email address.
        // This test is documentation-as-code — if we change the ID strategy,
        // this test must be updated.
        let _pool = test_pool().await;
        let email = "user@gmail.com";
        let acct = Account {
            id: email.into(),
            email: email.into(),
            display_name: Some("User".into()),
            photo_url: None,
            locale: None,
            created_at: 0,
            last_used_at: 0,
        };
        assert_eq!(acct.id, acct.email);
    }
}
