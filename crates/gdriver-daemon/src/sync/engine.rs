// ─── State machine ────────────────────────────────────────────────────────────

/// Internal state of the sync engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncEngineState {
    /// No work to do; waiting for local changes or the next remote poll.
    Idle,
    /// Scanning the local filesystem or Drive for changes to build a task list.
    Scanning,
    /// Actively uploading, downloading, or reconciling files.
    Syncing,
    /// Sync has been suspended by the user; no tasks are processed.
    Paused,
}

impl SyncEngineState {
    /// Map the internal engine state to the user-visible [`SyncStatus`] pushed
    /// to the UI via IPC.
    pub fn to_sync_status(self) -> gdriver_ipc::SyncStatus {
        match self {
            Self::Idle => gdriver_ipc::SyncStatus::UpToDate,
            Self::Scanning | Self::Syncing => gdriver_ipc::SyncStatus::Syncing,
            Self::Paused => gdriver_ipc::SyncStatus::Paused,
        }
    }
}

// ─── Commands ─────────────────────────────────────────────────────────────────

/// Commands sent from IPC handlers to the sync engine.
#[derive(Debug)]
pub enum SyncCommand {
    /// Suspend all sync processing.
    Pause,
    /// Resume processing after a pause.
    Resume,
    /// Switch the sync mode (Stream ↔ Mirror).
    SwitchMode(gdriver_ipc::SyncMode),
}

// ─── SyncContext ──────────────────────────────────────────────────────────────

use std::sync::Arc;

use gdriver_ipc::{PushEvent, SyncStatusPayload};
use sqlx::SqlitePool;
use tokio::sync::mpsc;

use crate::{auth::TokenStore, config::ConfigHandle, db::queue::SyncTask, ipc::PushSender};

/// Shared resources for the sync engine main loop.
pub struct SyncContext {
    /// SQLite connection pool.
    pub db: SqlitePool,
    /// Broadcast channel to notify all IPC clients of state changes.
    pub push_tx: PushSender,
    /// Receives [`SyncCommand`] values from the IPC routing layer.
    pub cmd_rx: mpsc::Receiver<SyncCommand>,
    /// Token store shared with the OAuth flow (in-memory cache + keyring).
    pub tokens: Arc<TokenStore>,
    /// Google OAuth credentials for token refresh.
    pub oauth_config: Option<gdriver_api::auth::OAuthConfig>,
    /// Shared HTTP client for Drive API calls.
    pub http: reqwest::Client,
    /// Receives [`SyncTask`] values from the local filesystem watcher.
    /// `None` when the watcher task is not running or has exited.
    pub watcher_rx: Option<mpsc::Receiver<SyncTask>>,
    /// Application configuration (shared with IPC handlers).
    pub cfg: ConfigHandle,
}

impl SyncContext {
    /// Push the current internal state as a `sync:status-changed` event.
    async fn push_status(&self, state: SyncEngineState) {
        let payload = SyncStatusPayload {
            status: state.to_sync_status(),
            ts: chrono::Utc::now().timestamp_millis(),
            speed: None,
            pending: None,
        };
        let event = PushEvent::SyncStatusChanged(payload);
        let notif = match event.to_notification() {
            Ok(n) => n,
            Err(e) => {
                tracing::error!("failed to serialise sync status push: {e}");
                return;
            }
        };
        let json = match serde_json::to_string(&notif) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("failed to serialise push notification: {e}");
                return;
            }
        };
        if let Err(e) = self.push_tx.send(json) {
            tracing::debug!("sync status push dropped (no connected clients): {e}");
        }
    }

    /// Transition to a new state and push the change.
    async fn transition_to(&mut self, new_state: SyncEngineState, current: &mut SyncEngineState) {
        if *current == new_state {
            return;
        }
        tracing::info!(?current, ?new_state, "sync engine state change");
        *current = new_state;
        self.push_status(new_state).await;
    }

    /// Build a [`DriveClient`] for the given account.
    ///
    /// Tries the in-memory cache first, then falls back to a keyring refresh.
    /// Returns `None` when no tokens are available for this account.
    async fn build_client_for_account(
        &self,
        account_id: &str,
    ) -> Option<gdriver_api::client::DriveClient> {
        use gdriver_api::client::DriveClient;

        // Try the in-memory access-token cache first.
        if let Some(ts) = self.tokens.get_token_set(account_id) {
            if !ts.should_refresh() {
                tracing::debug!(account_id, "using cached access token");
                let client = DriveClient::new(ts.access_token);
                // Attach refresher for automatic 401 recovery.
                if let Some(ref oauth) = self.oauth_config {
                    let refresher = Arc::new(AccountTokenRefresher {
                        account_id: account_id.to_string(),
                        tokens: Arc::clone(&self.tokens),
                        oauth_config: oauth.clone(),
                        http: self.http.clone(),
                    });
                    return Some(client.with_refresher(refresher));
                }
                return Some(client);
            }
        }

        // Cache is cold or expired — try a keyring refresh.
        let rt = self.tokens.load_refresh_token(account_id).ok()??;
        let oauth = self.oauth_config.as_ref()?;

        match gdriver_api::auth::refresh_access_token(&self.http, oauth, &rt).await {
            Ok(ts) => {
                let access_token = ts.access_token.clone();
                // Persist any new refresh token.
                if let Some(ref new_rt) = ts.refresh_token {
                    if new_rt != &rt {
                        let _ = self.tokens.save_refresh_token(account_id, new_rt);
                    }
                }
                self.tokens.cache_access_token(account_id, ts);

                let refresher = Arc::new(AccountTokenRefresher {
                    account_id: account_id.to_string(),
                    tokens: Arc::clone(&self.tokens),
                    oauth_config: oauth.clone(),
                    http: self.http.clone(),
                });
                Some(DriveClient::new(access_token).with_refresher(refresher))
            }
            Err(e) => {
                tracing::warn!(account_id, error = %e, "token refresh failed");
                None
            }
        }
    }
}

// ─── TokenRefresher ──────────────────────────────────────────────────────────

use async_trait::async_trait;
use gdriver_api::client::TokenRefresher;

/// A [`TokenRefresher`] that loads a refresh token from the keyring, exchanges
/// it for a new access token via the Google OAuth endpoint, and caches the
/// result.
struct AccountTokenRefresher {
    account_id: String,
    tokens: Arc<TokenStore>,
    oauth_config: gdriver_api::auth::OAuthConfig,
    http: reqwest::Client,
}

#[async_trait]
impl TokenRefresher for AccountTokenRefresher {
    async fn refresh(&self) -> anyhow::Result<String> {
        let rt = self
            .tokens
            .load_refresh_token(&self.account_id)?
            .ok_or_else(|| anyhow::anyhow!("no refresh token for {}", self.account_id))?;

        let ts =
            gdriver_api::auth::refresh_access_token(&self.http, &self.oauth_config, &rt).await?;

        let access_token = ts.access_token.clone();

        if let Some(ref new_rt) = ts.refresh_token {
            if new_rt != &rt {
                let _ = self.tokens.save_refresh_token(&self.account_id, new_rt);
            }
        }
        self.tokens.cache_access_token(&self.account_id, ts);

        Ok(access_token)
    }
}

// ─── Main loop ────────────────────────────────────────────────────────────────

use std::time::Duration;

use tracing::{debug, info};

/// Helper: receive from the watcher channel, or never resolve if disabled.
async fn recv_watcher(rx: &mut Option<mpsc::Receiver<SyncTask>>) -> Option<SyncTask> {
    match rx.as_mut() {
        Some(rx) => rx.recv().await,
        None => std::future::pending().await,
    }
}

/// Run the sync engine main loop.
///
/// Spawn this as a background task from `main.rs`.  The function runs until the
/// command channel is closed (shutdown) or an unrecoverable error occurs.
pub async fn run(mut ctx: SyncContext) -> anyhow::Result<()> {
    use tokio::time::interval;

    let mut state = SyncEngineState::Idle;
    let mut remote_poll = interval(Duration::from_secs(30));

    // Push the initial state immediately so that connected clients see it.
    ctx.push_status(state).await;

    loop {
        tokio::select! {
            // ── IPC commands (pause / resume) ────────────────────────────────
            cmd = ctx.cmd_rx.recv() => {
                match cmd {
                    Some(cmd) => handle_command(&mut ctx, &mut state, cmd).await,
                    // Channel closed — the daemon is shutting down.
                    None => {
                        tracing::info!("sync engine shutting down (cmd channel closed)");
                        break;
                    }
                }
            }

            // ── Remote poll timer ────────────────────────────────────────────
            _ = remote_poll.tick() => {
                handle_remote_poll(&mut ctx, &mut state).await;
            }

            // ── Local watcher tasks ──────────────────────────────────────────
            task = recv_watcher(&mut ctx.watcher_rx), if ctx.watcher_rx.is_some() => {
                match task {
                    Some(task) => handle_watcher_task(&mut ctx, &mut state, task).await,
                    None => {
                        tracing::warn!("watcher channel closed; disabling local watch");
                        ctx.watcher_rx = None;
                    }
                }
            }
        }
    }

    Ok(())
}

/// Process a single command received from the IPC layer.
async fn handle_command(ctx: &mut SyncContext, state: &mut SyncEngineState, cmd: SyncCommand) {
    match cmd {
        SyncCommand::Pause => {
            ctx.transition_to(SyncEngineState::Paused, state).await;
        }
        SyncCommand::Resume => {
            if *state == SyncEngineState::Paused {
                ctx.transition_to(SyncEngineState::Idle, state).await;
            }
        }
        SyncCommand::SwitchMode(new_mode) => {
            handle_mode_switch(ctx, state, new_mode).await;
        }
    }
}

/// Handle a sync mode switch (Stream ↔ Mirror).
///
/// - **Stream → Mirror**: Enqueue download tasks for all non-folder files across
///   all connected accounts, so the entire Drive is mirrored locally.
/// - **Mirror → Stream**: Reset all `cloud_only`-eligible files' local paths and
///   sync states. Local files already downloaded are left in place; the user can
///   clean them up manually or a future enhancement can delete them.
async fn handle_mode_switch(
    ctx: &mut SyncContext,
    state: &mut SyncEngineState,
    new_mode: gdriver_ipc::SyncMode,
) {
    use gdriver_ipc::SyncMode;

    info!(?new_mode, "sync engine: mode switch requested");

    let (mount_point, old_mode) = {
        let prefs = ctx.cfg.read().await;
        (prefs.vfs.mount_point.clone(), prefs.vfs.sync_mode)
    };

    if old_mode == new_mode {
        debug!(?new_mode, "sync engine: already in requested mode, no-op");
        return;
    }

    ctx.transition_to(SyncEngineState::Syncing, state).await;

    let accounts = match crate::db::accounts::list_accounts(&ctx.db).await {
        Ok(a) => a,
        Err(e) => {
            tracing::error!("mode switch: failed to list accounts: {e:#}");
            ctx.transition_to(SyncEngineState::Idle, state).await;
            return;
        }
    };

    match new_mode {
        SyncMode::Mirror => {
            // Stream → Mirror: enqueue downloads for all files.
            for account in &accounts {
                match crate::sync::initial::enqueue_mirror_downloads(
                    &ctx.db,
                    &account.id,
                    &mount_point,
                )
                .await
                {
                    Ok(count) => {
                        info!(
                            account_id = %account.id,
                            download_count = count,
                            "mode switch: mirror download tasks enqueued"
                        );
                    }
                    Err(e) => {
                        tracing::error!(
                            account_id = %account.id,
                            error = %e,
                            "mode switch: failed to enqueue mirror downloads"
                        );
                    }
                }
            }
        }
        SyncMode::Stream => {
            // Mirror → Stream: reset local paths and sync states for files
            // that are not in configured sync folders (sync folders not yet
            // implemented, so reset all non-folder files).
            for account in &accounts {
                match reset_mirror_to_stream(&ctx.db, &account.id).await {
                    Ok(count) => {
                        info!(
                            account_id = %account.id,
                            reset_count = count,
                            "mode switch: files reset to cloud-only"
                        );
                    }
                    Err(e) => {
                        tracing::error!(
                            account_id = %account.id,
                            error = %e,
                            "mode switch: failed to reset files"
                        );
                    }
                }
            }
        }
    }

    ctx.transition_to(SyncEngineState::Idle, state).await;
    info!(?new_mode, "sync engine: mode switch complete");
}

/// Reset files from Mirror state back to cloud-only for Stream mode.
///
/// Clears `local_path` and resets `sync_state` to `"cloud_only"` for all
/// non-folder, non-trashed files that have a local path set.
async fn reset_mirror_to_stream(db: &SqlitePool, account_id: &str) -> anyhow::Result<usize> {
    let result = sqlx::query(
        "UPDATE drive_files
         SET local_path = NULL, sync_state = 'cloud_only'
         WHERE account_id = ?
           AND is_trashed = 0
           AND mime_type != 'application/vnd.google-apps.folder'
           AND local_path IS NOT NULL",
    )
    .bind(account_id)
    .execute(db)
    .await?;

    Ok(result.rows_affected() as usize)
}

/// Called every 30 s.  Polls the Drive Changes API for every known account and
/// updates the local database with any new, modified, or deleted files.
async fn handle_remote_poll(ctx: &mut SyncContext, state: &mut SyncEngineState) {
    if *state == SyncEngineState::Paused {
        tracing::trace!("remote poll skipped (paused)");
        return;
    }

    // List all connected accounts.
    let accounts = match crate::db::accounts::list_accounts(&ctx.db).await {
        Ok(a) => a,
        Err(e) => {
            tracing::error!("failed to list accounts for remote poll: {e:#}");
            return;
        }
    };

    if accounts.is_empty() {
        tracing::trace!("remote poll: no accounts");
        return;
    }

    ctx.transition_to(SyncEngineState::Scanning, state).await;

    let mut any_changes = false;

    for account in &accounts {
        let client = match ctx.build_client_for_account(&account.id).await {
            Some(c) => c,
            None => {
                tracing::debug!(
                    account_id = %account.id,
                    "remote poll: no valid token, skipping account"
                );
                continue;
            }
        };

        let (sync_mode, mount_point) = {
            let prefs = ctx.cfg.read().await;
            (prefs.vfs.sync_mode, prefs.vfs.mount_point.clone())
        };

        match crate::sync::incremental::incremental_sync(
            &ctx.db,
            &account.id,
            &client,
            sync_mode,
            &mount_point,
        )
        .await
        {
            Ok(count) => {
                if count > 0 {
                    any_changes = true;
                }
            }
            Err(e) => {
                tracing::error!(
                    account_id = %account.id,
                    error = %e,
                    "incremental sync failed"
                );
            }
        }
    }

    ctx.transition_to(SyncEngineState::Syncing, state).await;

    process_pending_tasks(ctx).await;

    ctx.transition_to(SyncEngineState::Idle, state).await;
    let _ = any_changes;
}

/// Enqueue a task produced by the local filesystem watcher.
async fn handle_watcher_task(ctx: &mut SyncContext, state: &mut SyncEngineState, task: SyncTask) {
    if *state == SyncEngineState::Paused {
        tracing::trace!("watcher task dropped (paused): {:?}", task.operation);
        return;
    }

    tracing::debug!(
        operation = %task.operation,
        path = ?task.local_path,
        "watcher task received"
    );

    match crate::db::queue::enqueue(&ctx.db, &task).await {
        Ok(enqueued) => {
            tracing::debug!(task_id = enqueued.id, "watcher task enqueued");
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to enqueue watcher task");
            return;
        }
    }

    // Transition to Syncing so the UI reflects activity.
    ctx.transition_to(SyncEngineState::Syncing, state).await;

    // Process the task immediately instead of waiting for the next 30s poll.
    process_pending_tasks(ctx).await;
}

/// Dequeue and process pending sync tasks (upload, download, delete).
///
/// Processes up to 10 tasks per invocation to keep the event loop responsive.
async fn process_pending_tasks(ctx: &mut SyncContext) {
    for _ in 0..10 {
        let task = match crate::db::queue::next_pending_task(&ctx.db).await {
            Ok(Some(t)) => t,
            Ok(None) => {
                tracing::trace!("sync queue drained");
                break;
            }
            Err(e) => {
                tracing::error!("failed to fetch pending task: {e:#}");
                break;
            }
        };

        let task_id = task.id;
        tracing::debug!(
            task_id,
            operation = %task.operation,
            path = ?task.local_path,
            "processing task"
        );

        match task.operation.as_str() {
            "upload" => {
                let client = match ctx.build_client_for_account(&task.account_id).await {
                    Some(c) => c,
                    None => {
                        tracing::warn!(
                            account_id = %task.account_id,
                            "no valid token for upload task; deferring"
                        );
                        continue;
                    }
                };

                // Check for conflicts before uploading (only when the file has
                // been synced before — i.e. we have a file_id and local_path).
                if let (Some(ref file_id), Some(ref local_path)) =
                    (task.file_id.as_deref(), task.local_path.as_deref())
                {
                    match crate::sync::conflict::check_and_resolve(
                        &ctx.db,
                        &client,
                        &ctx.push_tx,
                        &task,
                        file_id,
                        local_path,
                    )
                    .await
                    {
                        Ok(Some(())) => {
                            // Conflict resolved — skip the normal upload.
                            continue;
                        }
                        Ok(None) => {
                            // No conflict — fall through to normal upload.
                        }
                        Err(e) => {
                            tracing::error!(task_id, error = %e, "conflict check failed; falling back to normal upload");
                        }
                    }
                }

                if let Err(e) = crate::sync::uploader::upload_file(&ctx.db, &client, &task).await {
                    tracing::error!(task_id, error = %e, "upload processing failed");
                }
            }
            "download" => {
                let client = match ctx.build_client_for_account(&task.account_id).await {
                    Some(c) => c,
                    None => {
                        tracing::warn!(
                            account_id = %task.account_id,
                            "no valid token for download task; deferring"
                        );
                        continue;
                    }
                };
                if let Err(e) =
                    crate::sync::downloader::download_file(&ctx.db, &client, &task).await
                {
                    tracing::error!(task_id, error = %e, "download processing failed");
                }
            }
            "photos_backup" => {
                let client = match ctx.build_client_for_account(&task.account_id).await {
                    Some(c) => c,
                    None => {
                        tracing::warn!(
                            account_id = %task.account_id,
                            "no valid token for photos_backup task; deferring"
                        );
                        continue;
                    }
                };
                if let Err(e) =
                    crate::sync::photos::backup_photo(&ctx.db, &client, &ctx.push_tx, &task).await
                {
                    tracing::error!(task_id, error = %e, "photos_backup processing failed");
                }
            }
            "delete" => {
                let client = match ctx.build_client_for_account(&task.account_id).await {
                    Some(c) => c,
                    None => {
                        tracing::warn!(
                            account_id = %task.account_id,
                            "no valid token for delete task; deferring"
                        );
                        continue;
                    }
                };
                if let Err(e) = crate::sync::deleter::delete_file(&ctx.db, &client, &task).await {
                    tracing::error!(task_id, error = %e, "delete processing failed");
                }
            }
            other => {
                tracing::warn!(
                    task_id,
                    operation = other,
                    "unknown operation, marking completed"
                );
                if let Some(id) = task_id {
                    let _ = crate::db::queue::update_task_status(
                        &ctx.db,
                        id,
                        "completed",
                        Some("unknown operation"),
                    )
                    .await;
                }
            }
        }
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    async fn dummy_context() -> (mpsc::Sender<SyncCommand>, SyncContext) {
        let (tx, rx) = mpsc::channel::<SyncCommand>(8);
        let (push_tx, _push_rx) = tokio::sync::broadcast::channel(16);
        let (_watcher_tx, watcher_rx) = mpsc::channel::<SyncTask>(128);

        let db = {
            use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
            let opts = SqliteConnectOptions::new()
                .filename(":memory:")
                .foreign_keys(true);
            SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(opts)
                .await
                .expect("in-memory pool")
        };

        let cfg = crate::config::new_handle(gdriver_ipc::Preferences::default());

        let ctx = SyncContext {
            db,
            push_tx,
            cmd_rx: rx,
            tokens: Arc::new(TokenStore::new()),
            oauth_config: None,
            http: reqwest::Client::new(),
            watcher_rx: Some(watcher_rx),
            cfg,
        };

        (tx, ctx)
    }

    #[test]
    fn idle_maps_to_up_to_date() {
        assert_eq!(
            SyncEngineState::Idle.to_sync_status(),
            gdriver_ipc::SyncStatus::UpToDate
        );
    }

    #[test]
    fn scanning_maps_to_syncing() {
        assert_eq!(
            SyncEngineState::Scanning.to_sync_status(),
            gdriver_ipc::SyncStatus::Syncing
        );
    }

    #[test]
    fn syncing_maps_to_syncing() {
        assert_eq!(
            SyncEngineState::Syncing.to_sync_status(),
            gdriver_ipc::SyncStatus::Syncing
        );
    }

    #[test]
    fn paused_maps_to_paused() {
        assert_eq!(
            SyncEngineState::Paused.to_sync_status(),
            gdriver_ipc::SyncStatus::Paused
        );
    }

    #[tokio::test]
    async fn engine_starts_in_idle_and_shuts_down() {
        let (tx, ctx) = dummy_context().await;
        drop(tx);

        let result = run(ctx).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn pause_transitions_to_paused_state() {
        let (tx, ctx) = dummy_context().await;
        let (push_tx, mut push_rx) = tokio::sync::broadcast::channel(16);
        let ctx = SyncContext { push_tx, ..ctx };

        let ctx = std::panic::AssertUnwindSafe(ctx);
        let handle = tokio::spawn(async move { run(ctx.0).await });

        // Wait for the initial idle status push.
        let msg = push_rx.recv().await.unwrap();
        assert!(
            msg.contains("up-to-date"),
            "initial push should be up-to-date, got: {msg}"
        );

        // Send pause command.
        tx.send(SyncCommand::Pause).await.unwrap();

        // Wait for the paused status push.
        let msg = push_rx.recv().await.unwrap();
        assert!(
            msg.contains("paused"),
            "pause push should be paused, got: {msg}"
        );

        // Send resume command.
        tx.send(SyncCommand::Resume).await.unwrap();

        // Wait for the up-to-date (resumed) status push.
        let msg = push_rx.recv().await.unwrap();
        assert!(
            msg.contains("up-to-date"),
            "resume push should be up-to-date, got: {msg}"
        );

        drop(tx);
        let _ = handle.await;
    }

    #[tokio::test]
    async fn pause_is_idempotent() {
        let (tx, ctx) = dummy_context().await;
        let (push_tx, mut push_rx) = tokio::sync::broadcast::channel(16);
        let ctx = SyncContext { push_tx, ..ctx };

        let ctx = std::panic::AssertUnwindSafe(ctx);
        let handle = tokio::spawn(async move { run(ctx.0).await });

        // Consume initial idle push.
        push_rx.recv().await.unwrap();

        // First pause: should get a "paused" push.
        tx.send(SyncCommand::Pause).await.unwrap();
        let msg1 = push_rx.recv().await.unwrap();
        assert!(msg1.contains("paused"));

        // Second pause: should NOT get another push (idempotent).
        tx.send(SyncCommand::Pause).await.unwrap();
        let result = tokio::time::timeout(Duration::from_millis(200), push_rx.recv()).await;
        assert!(
            result.is_err(),
            "should time out — no duplicate paused push expected"
        );

        drop(tx);
        let _ = handle.await;
    }

    // ── reset_mirror_to_stream tests ──────────────────────────────────────

    async fn test_pool() -> sqlx::SqlitePool {
        use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
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

    async fn insert_account(pool: &sqlx::SqlitePool, id: &str) {
        sqlx::query(
            "INSERT INTO accounts (id, email, created_at, last_used_at) VALUES (?, ?, ?, ?)",
        )
        .bind(id)
        .bind(format!("{id}@example.com"))
        .bind(1_700_000_000_000_i64)
        .bind(1_700_000_000_000_i64)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn insert_file(
        pool: &sqlx::SqlitePool,
        id: &str,
        account_id: &str,
        name: &str,
        mime_type: &str,
        local_path: Option<&str>,
    ) {
        sqlx::query(
            "INSERT INTO drive_files (id, account_id, name, mime_type, size, etag, version,
                    modified_time, is_trashed, is_shared, local_path, sync_state)
             VALUES (?, ?, ?, ?, 1024, ?, 5, 1700000000000, 0, 0, ?, ?)",
        )
        .bind(id)
        .bind(account_id)
        .bind(name)
        .bind(mime_type)
        .bind(format!("\"etag_{id}\""))
        .bind(local_path)
        .bind(if local_path.is_some() {
            "cached"
        } else {
            "cloud_only"
        })
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn reset_mirror_clears_local_paths() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;

        // Insert files with local paths (simulating mirror mode).
        insert_file(
            &pool,
            "f1",
            "acct-1",
            "file-a.txt",
            "text/plain",
            Some("/tmp/drive/file-a.txt"),
        )
        .await;
        insert_file(
            &pool,
            "f2",
            "acct-1",
            "file-b.pdf",
            "application/pdf",
            Some("/tmp/drive/file-b.pdf"),
        )
        .await;

        let count = reset_mirror_to_stream(&pool, "acct-1").await.unwrap();
        assert_eq!(count, 2);

        // Verify local paths are cleared.
        let f1 = crate::db::files::get_file_by_id(&pool, "f1", "acct-1")
            .await
            .unwrap()
            .unwrap();
        assert!(f1.local_path.is_none());
        assert_eq!(f1.sync_state, "cloud_only");

        let f2 = crate::db::files::get_file_by_id(&pool, "f2", "acct-1")
            .await
            .unwrap()
            .unwrap();
        assert!(f2.local_path.is_none());
        assert_eq!(f2.sync_state, "cloud_only");
    }

    #[tokio::test]
    async fn reset_mirror_skips_folders() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;

        insert_file(
            &pool,
            "folder-1",
            "acct-1",
            "My Folder",
            "application/vnd.google-apps.folder",
            Some("/tmp/drive/My Folder"),
        )
        .await;
        insert_file(
            &pool,
            "f1",
            "acct-1",
            "file.txt",
            "text/plain",
            Some("/tmp/drive/file.txt"),
        )
        .await;

        let count = reset_mirror_to_stream(&pool, "acct-1").await.unwrap();
        assert_eq!(count, 1, "should only reset non-folder files");

        // Folder should keep its local path.
        let folder = crate::db::files::get_file_by_id(&pool, "folder-1", "acct-1")
            .await
            .unwrap()
            .unwrap();
        assert!(folder.local_path.is_some());
    }

    #[tokio::test]
    async fn reset_mirror_skips_already_cloud_only() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;

        // File without local path (already cloud-only).
        insert_file(&pool, "f1", "acct-1", "file.txt", "text/plain", None).await;

        let count = reset_mirror_to_stream(&pool, "acct-1").await.unwrap();
        assert_eq!(count, 0, "should skip files without local_path");
    }

    #[tokio::test]
    async fn reset_mirror_skips_trashed_files() {
        let pool = test_pool().await;
        insert_account(&pool, "acct-1").await;

        // Insert a trashed file with local path.
        sqlx::query(
            "INSERT INTO drive_files (id, account_id, name, mime_type, size, etag, version,
                    modified_time, is_trashed, is_shared, local_path, sync_state)
             VALUES (?, ?, ?, ?, 1024, ?, 5, 1700000000000, 1, 0, ?, ?)",
        )
        .bind("f-trash")
        .bind("acct-1")
        .bind("trashed.txt")
        .bind("text/plain")
        .bind("\"etag_f-trash\"")
        .bind(Some("/tmp/drive/trashed.txt"))
        .bind("cached")
        .execute(&pool)
        .await
        .unwrap();

        let count = reset_mirror_to_stream(&pool, "acct-1").await.unwrap();
        assert_eq!(count, 0, "should skip trashed files");
    }
}
