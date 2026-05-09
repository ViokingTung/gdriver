pub mod accounts;
pub mod files;
pub mod notifications;
pub mod queue;
pub mod sync_errors;
pub mod sync_folders;
pub mod tokens;

use std::path::PathBuf;
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;
use tracing::info;

// ─── Public API ───────────────────────────────────────────────────────────────

/// Open (or create) the SQLite connection pool and return it.
///
/// Configures WAL journal mode and enables foreign-key enforcement for every
/// connection in the pool.
pub async fn create_pool() -> anyhow::Result<SqlitePool> {
    let path = db_path()?;
    info!("database path: {}", path.display());

    let opts = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        // NORMAL sync is the recommended setting for WAL mode: safe and faster
        // than FULL while still preventing corruption on OS crash.
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        // Retry for up to 30 s instead of immediately returning SQLITE_BUSY.
        .busy_timeout(Duration::from_secs(30));

    let pool = SqlitePoolOptions::new()
        // One writer + several concurrent readers is the sweet spot for WAL.
        .max_connections(5)
        .connect_with(opts)
        .await?;

    Ok(pool)
}

/// Run all pending migrations found in `migrations/`.
///
/// Uses `sqlx::migrate!` which embeds the SQL files at compile time and tracks
/// applied migrations in the `_sqlx_migrations` table automatically.
pub async fn run_migrations(pool: &SqlitePool) -> anyhow::Result<()> {
    info!("running database migrations");
    sqlx::migrate!("./migrations").run(pool).await?;
    info!("database migrations up to date");
    Ok(())
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Platform-specific path to `gdriver.db`.
///
/// | Platform | Path                                                 |
/// |----------|------------------------------------------------------|
/// | Linux    | `~/.local/share/gdriver/gdriver.db`                  |
/// | macOS    | `~/Library/Application Support/gdriver/gdriver.db`   |
/// | Windows  | `%APPDATA%\gdriver\gdriver.db`                       |
fn db_path() -> anyhow::Result<PathBuf> {
    let dir = dirs::data_dir()
        .ok_or_else(|| anyhow::anyhow!("cannot determine data directory"))?
        .join("gdriver");

    std::fs::create_dir_all(&dir)?;
    Ok(dir.join("gdriver.db"))
}
