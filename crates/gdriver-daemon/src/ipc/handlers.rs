use std::sync::Arc;

use gdriver_ipc::{
    AccountChangedPayload, JsonRpcError, JsonRpcRequest, JsonRpcResponse, OauthCompletePayload,
    Preferences, PushEvent,
};
use serde_json::{json, Value};
use sqlx::SqlitePool;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::{
    auth::TokenStore, config::ConfigHandle, ipc::server::PushSender, sync::engine::SyncCommand,
};

// ─── Shared context ───────────────────────────────────────────────────────────

/// Resources shared across all JSON-RPC handler invocations.
///
/// Wrapped in `Arc` and cloned cheaply into each connection task.
pub struct RouterContext {
    /// SQLite connection pool (WAL mode).
    pub db: SqlitePool,
    /// In-memory preferences backed by `preferences.toml`.
    pub config: ConfigHandle,
    /// Token storage: keyring for refresh tokens, in-memory for access tokens.
    pub tokens: Arc<TokenStore>,
    /// Broadcast channel for daemon-to-client push events.
    pub push_tx: PushSender,
    /// Send commands to the sync engine (pause / resume).
    pub sync_cmd_tx: mpsc::Sender<SyncCommand>,
    /// Signal the filesystem watcher to reload its watched folders from the DB.
    pub watcher_reload_tx: mpsc::Sender<()>,
}

impl RouterContext {
    pub fn new(
        db: SqlitePool,
        config: ConfigHandle,
        push_tx: PushSender,
        sync_cmd_tx: mpsc::Sender<SyncCommand>,
        tokens: Arc<TokenStore>,
        watcher_reload_tx: mpsc::Sender<()>,
    ) -> Self {
        Self {
            db,
            config,
            tokens,
            push_tx,
            sync_cmd_tx,
            watcher_reload_tx,
        }
    }
}

// ─── Push-event helper ───────────────────────────────────────────────────────

/// Serialise a [`PushEvent`] and broadcast it to all connected IPC clients.
///
/// Errors are logged rather than propagated — push delivery is best-effort by
/// design (clients re-query state on reconnect).
fn push_event(tx: &PushSender, event: PushEvent) {
    match event.to_notification() {
        Ok(notif) => {
            let json = match serde_json::to_string(&notif) {
                Ok(s) => s,
                Err(e) => {
                    error!("failed to serialise push event: {e}");
                    return;
                }
            };
            if let Err(e) = tx.send(json) {
                debug!("push event dropped (no connected clients): {e}");
            }
        }
        Err(e) => error!("failed to build push notification: {e}"),
    }
}

/// Build and push an `account:changed` event reflecting the current account list.
async fn push_account_changed(ctx: &RouterContext) {
    match crate::db::accounts::list_accounts(&ctx.db).await {
        Ok(accounts) => {
            let payload = AccountChangedPayload { accounts };
            push_event(&ctx.push_tx, PushEvent::AccountChanged(payload));
        }
        Err(e) => error!("failed to list accounts for push event: {e:#}"),
    }
}

// ─── FS path resolution helpers ───────────────────────────────────────────────

/// Expand a leading `~` in a path to the current user's home directory.
fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return format!("{}/{}", home.display(), rest);
        }
    } else if path == "~" {
        if let Some(home) = dirs::home_dir() {
            return home.display().to_string();
        }
    }
    path.to_string()
}

/// Extract a required string parameter from a JSON-RPC params object.
fn extract_string_param(params: &Option<Value>, key: &str) -> Result<String, JsonRpcError> {
    params
        .as_ref()
        .and_then(|v| v.get(key))
        .and_then(|v| v.as_str())
        .map(String::from)
        .ok_or_else(|| JsonRpcError::invalid_params(&format!("{key} is required")))
}

/// Resolve a file by its local path — tries exact match first, then falls back
/// to matching the relative path under the VFS mount point.
async fn resolve_file_for_path(
    ctx: &RouterContext,
    path: &str,
) -> Result<crate::db::files::DriveFile, JsonRpcError> {
    // 1. Exact local_path match.
    if let Some(file) = crate::db::files::get_file_by_local_path(&ctx.db, path)
        .await
        .map_err(|e| JsonRpcError::internal_error(e.to_string()))?
    {
        return Ok(file);
    }

    // 2. Prefix match: strip mount point and search by relative path.
    resolve_path_under_mount(ctx, path)
        .await?
        .ok_or_else(|| JsonRpcError::not_found("file not found for path"))
}

/// Try to find a file by stripping the configured mount-point prefix from `path`
/// and searching for the relative portion in the `local_path` column.
async fn resolve_path_under_mount(
    ctx: &RouterContext,
    path: &str,
) -> Result<Option<crate::db::files::DriveFile>, JsonRpcError> {
    let prefs = ctx.config.read().await;
    let mount = expand_tilde(&prefs.vfs.mount_point);
    let mount = mount.trim_end_matches('/');

    // Build the path with a leading '/' for comparison.
    let abs_path = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };

    // Check if the path starts with the mount point.
    if !abs_path.starts_with(mount) && !abs_path.starts_with(&format!("{mount}/")) {
        return Ok(None);
    }

    let relative = &abs_path[mount.len()..].trim_start_matches('/');

    if relative.is_empty() {
        // The path IS the mount point — return the root folder.
        return Ok(None);
    }

    crate::db::files::find_file_by_relative_suffix(&ctx.db, relative)
        .await
        .map_err(|e| JsonRpcError::internal_error(e.to_string()))
}

// ─── Router ───────────────────────────────────────────────────────────────────

/// JSON-RPC method router.
///
/// `handle()` is the single public entry point called by the connection task.
/// Add new methods to `dispatch()` as later milestones land.
pub struct Router {
    ctx: Arc<RouterContext>,
}

impl Router {
    pub fn new(ctx: Arc<RouterContext>) -> Self {
        Self { ctx }
    }

    /// Process one inbound JSON-RPC message.
    ///
    /// Returns `Some(response)` for requests, `None` for push notifications
    /// (which have no `id` and require no reply).
    pub async fn handle(&self, req: JsonRpcRequest) -> Option<JsonRpcResponse> {
        let id = req.id.clone();
        let is_notification = req.is_notification();

        debug!(
            "dispatch: method={} notification={is_notification}",
            req.method
        );

        let result = self.dispatch(&req.method, req.params).await;

        if is_notification {
            return None;
        }

        Some(match result {
            Ok(val) => JsonRpcResponse::success(id, val),
            Err(err) => JsonRpcResponse::error(id, err),
        })
    }

    /// Route a method name + params to the correct handler.
    async fn dispatch(&self, method: &str, params: Option<Value>) -> Result<Value, JsonRpcError> {
        use gdriver_ipc::methods::*;

        match method {
            // ── Core ────────────────────────────────────────────────────────
            PING => {
                debug!("ping → pong");
                Ok(json!("pong"))
            }

            // ── Preferences ──────────────────────────────────────────────────
            PREFS_GET => self.handle_prefs_get().await,
            PREFS_SAVE => self.handle_prefs_save(params).await,

            // ── Auth ────────────────────────────────────────────────────────
            AUTH_START_FLOW => self.handle_auth_start_flow().await,
            AUTH_GET_ACCOUNTS => self.handle_auth_get_accounts().await,
            AUTH_DISCONNECT => self.handle_auth_disconnect(params).await,
            AUTH_GET_LOCALE => self.handle_auth_get_locale(params).await,
            AUTH_GET_QUOTA => self.handle_auth_get_quota(params).await,

            // ── Sync control ────────────────────────────────────────────────
            SYNC_GET_STATUS => self.handle_sync_get_status().await,
            SYNC_PAUSE => self.handle_sync_pause().await,
            SYNC_RESUME => self.handle_sync_resume().await,
            SYNC_GET_RECENT_ITEMS => self.handle_sync_get_recent_items(params).await,
            SYNC_GET_ACTIVITY => self.handle_sync_get_activity(params).await,
            SYNC_RETRY_ERROR => self.handle_sync_retry_error(params).await,
            SYNC_GET_ERRORS => self.handle_sync_get_errors().await,

            // ── Notifications ───────────────────────────────────────────────
            NOTIFICATION_LIST => self.handle_notification_list(params).await,
            NOTIFICATION_DISMISS => self.handle_notification_dismiss(params).await,
            NOTIFICATION_MARK_READ => self.handle_notification_mark_read(params).await,
            NOTIFICATION_MARK_ALL_READ => self.handle_notification_mark_all_read().await,

            // ── Folder management ───────────────────────────────────────────
            FOLDER_LIST => self.handle_folder_list().await,
            FOLDER_ADD => self.handle_folder_add(params).await,
            FOLDER_REMOVE => self.handle_folder_remove(params).await,
            FOLDER_GET_SIZE => self.handle_folder_get_size(params).await,
            FOLDER_GET_SUGGESTED => self.handle_folder_get_suggested().await,

            // ── System ──────────────────────────────────────────────────────
            SYSTEM_SET_SYNC_MODE => self.handle_set_sync_mode(params).await,
            SYSTEM_GET_DRIVE_STATS => self.handle_system_get_drive_stats().await,
            SYSTEM_SUBMIT_FEEDBACK => self.handle_submit_feedback(params).await,
            SYSTEM_GET_PLATFORM => self.handle_get_platform(),

            // ── Offline ───────────────────────────────────────────────────────
            OFFLINE_GET_STATS => self.handle_offline_get_stats().await,
            OFFLINE_CLEAR_CACHE => self.handle_offline_clear_cache().await,

            // ── Filesystem queries (extensions) ─────────────────────────────
            FS_GET_SYNC_STATE => self.handle_fs_get_sync_state(params).await,
            FS_SET_OFFLINE => self.handle_fs_set_offline(params).await,
            FS_GET_SHARE_LINK => self.handle_fs_get_share_link(params).await,

            // ── FileProvider (macOS) ───────────────────────────────────────
            FP_GET_ITEM => self.handle_fp_get_item(params).await,
            FP_LIST_CHILDREN => self.handle_fp_list_children(params).await,
            FP_FETCH_CONTENTS => self.handle_fp_fetch_contents(params).await,
            FP_CREATE_ITEM => self.handle_fp_create_item(params).await,
            FP_MODIFY_ITEM => self.handle_fp_modify_item(params).await,
            FP_DELETE_ITEM => self.handle_fp_delete_item(params).await,

            // ── Unknown ─────────────────────────────────────────────────────
            other => Err(JsonRpcError::method_not_found(other)),
        }
    }

    // ── Preferences handlers ─────────────────────────────────────────────────

    async fn handle_prefs_get(&self) -> Result<Value, JsonRpcError> {
        let prefs = self.ctx.config.read().await;
        serde_json::to_value(&*prefs).map_err(|e| JsonRpcError::internal_error(e.to_string()))
    }

    async fn handle_prefs_save(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let new_prefs: Preferences = serde_json::from_value(params.unwrap_or(Value::Null))
            .map_err(|e| JsonRpcError::invalid_params(&e.to_string()))?;

        // ── Windows / macOS: sync auto-start on preference change ─────
        #[cfg(any(target_os = "windows", target_os = "macos"))]
        {
            let old_prefs = self.ctx.config.read().await;
            let old_launch = old_prefs.general.launch_on_login;
            drop(old_prefs);

            if old_launch != new_prefs.general.launch_on_login {
                let result = if new_prefs.general.launch_on_login {
                    crate::platform::set_launch_on_login()
                } else {
                    crate::platform::remove_launch_on_login()
                };
                if let Err(e) = result {
                    warn!("failed to update auto-start: {e:#}");
                }
            }
        }

        let prefs_to_save = new_prefs.clone();

        tokio::task::spawn_blocking(move || crate::config::save(&prefs_to_save))
            .await
            .map_err(|e| JsonRpcError::internal_error(e.to_string()))?
            .map_err(|e| JsonRpcError::internal_error(e.to_string()))?;

        {
            let mut guard = self.ctx.config.write().await;
            *guard = new_prefs;
        }

        debug!("preferences saved");
        Ok(json!(null))
    }

    // ── Sync control handlers ───────────────────────────────────────────────

    /// Return the current sync status.
    ///
    /// The engine pushes granular state via `sync:status-changed` events, so
    /// this handler returns a snapshot for initial UI hydration.  The initial
    /// "up-to-date" default is corrected within milliseconds by the engine's
    /// startup push.
    async fn handle_sync_get_status(&self) -> Result<Value, JsonRpcError> {
        use gdriver_ipc::SyncStatus;
        let payload = gdriver_ipc::SyncStatusPayload {
            status: SyncStatus::UpToDate,
            ts: chrono::Utc::now().timestamp_millis(),
            speed: None,
            pending: None,
        };
        serde_json::to_value(payload).map_err(|e| JsonRpcError::internal_error(e.to_string()))
    }

    async fn handle_sync_pause(&self) -> Result<Value, JsonRpcError> {
        debug!("sync.pause requested");
        self.ctx
            .sync_cmd_tx
            .send(SyncCommand::Pause)
            .await
            .map_err(|e| JsonRpcError::internal_error(e.to_string()))?;
        Ok(json!({ "paused": true }))
    }

    async fn handle_sync_resume(&self) -> Result<Value, JsonRpcError> {
        debug!("sync.resume requested");
        self.ctx
            .sync_cmd_tx
            .send(SyncCommand::Resume)
            .await
            .map_err(|e| JsonRpcError::internal_error(e.to_string()))?;
        Ok(json!({ "resumed": true }))
    }

    /// Return the most recently modified files as `SyncItem` objects.
    async fn handle_sync_get_recent_items(
        &self,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let limit = params
            .as_ref()
            .and_then(|v| v.get("limit"))
            .and_then(|v| v.as_u64())
            .unwrap_or(10) as u32;

        let files = crate::db::files::list_recent_files(&self.ctx.db, limit)
            .await
            .map_err(|e| JsonRpcError::internal_error(e.to_string()))?;

        let items: Vec<gdriver_ipc::SyncItem> = files
            .into_iter()
            .map(|f| gdriver_ipc::SyncItem {
                file_id: Some(f.id.clone()),
                name: f.name,
                mime_type: Some(f.mime_type),
                local_path: f.local_path,
                sync_state: serde_json::from_str(&format!("\"{}\"", f.sync_state))
                    .unwrap_or(gdriver_ipc::SyncState::CloudOnly),
                progress: None,
                file_size: f.size.and_then(|s| u64::try_from(s).ok()),
                error_msg: None,
                drive_url: Some(format!("https://drive.google.com/file/d/{}/view", f.id)),
                updated_at: f.modified_time.unwrap_or(0),
            })
            .collect();

        serde_json::to_value(items).map_err(|e| JsonRpcError::internal_error(e.to_string()))
    }

    /// Return a page of sync activity items.
    async fn handle_sync_get_activity(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let page = params
            .as_ref()
            .and_then(|v| v.get("page"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;

        let page_size: u32 = 50;

        let files = crate::db::files::list_files_paginated(&self.ctx.db, page, page_size)
            .await
            .map_err(|e| JsonRpcError::internal_error(e.to_string()))?;

        let has_more = files.len() as u32 == page_size;

        let items: Vec<gdriver_ipc::SyncItem> = files
            .into_iter()
            .map(|f| gdriver_ipc::SyncItem {
                file_id: Some(f.id.clone()),
                name: f.name,
                mime_type: Some(f.mime_type),
                local_path: f.local_path,
                sync_state: serde_json::from_str(&format!("\"{}\"", f.sync_state))
                    .unwrap_or(gdriver_ipc::SyncState::CloudOnly),
                progress: None,
                file_size: f.size.and_then(|s| u64::try_from(s).ok()),
                error_msg: None,
                drive_url: Some(format!("https://drive.google.com/file/d/{}/view", f.id)),
                updated_at: f.modified_time.unwrap_or(0),
            })
            .collect();

        let page_result = gdriver_ipc::SyncActivityPage {
            items,
            page,
            has_more,
        };

        serde_json::to_value(page_result).map_err(|e| JsonRpcError::internal_error(e.to_string()))
    }

    /// Mark a sync error as resolved and trigger a retry for the associated file.
    async fn handle_sync_retry_error(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let error_id = params
            .as_ref()
            .and_then(|v| v.get("errorId"))
            .and_then(|v| v.as_i64())
            .ok_or_else(|| JsonRpcError::invalid_params("errorId is required"))?;

        crate::db::sync_errors::resolve_error(&self.ctx.db, error_id)
            .await
            .map_err(|e| JsonRpcError::internal_error(e.to_string()))?;

        debug!("sync error {error_id} marked as resolved");
        Ok(json!({ "resolved": true }))
    }

    /// Return all unresolved sync errors.
    async fn handle_sync_get_errors(&self) -> Result<Value, JsonRpcError> {
        let rows = crate::db::sync_errors::list_unresolved(&self.ctx.db)
            .await
            .map_err(|e| JsonRpcError::internal_error(e.to_string()))?;

        let errors: Vec<gdriver_ipc::SyncError> = rows
            .into_iter()
            .map(|r| gdriver_ipc::SyncError {
                id: r.id.unwrap_or(0),
                account_id: r.account_id,
                file_id: r.file_id,
                file_name: r.file_name,
                error_code: r.error_code,
                error_msg: r.error_msg,
                is_resolved: r.is_resolved,
                created_at: r.created_at,
            })
            .collect();

        serde_json::to_value(errors).map_err(|e| JsonRpcError::internal_error(e.to_string()))
    }

    /// Switch the sync mode (Stream ↔ Mirror).
    ///
    /// Updates the config, persists it to disk, and sends a `SwitchMode` command
    /// to the sync engine which handles the actual migration (enqueueing
    /// downloads for Stream→Mirror, resetting files for Mirror→Stream).
    async fn handle_set_sync_mode(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let new_mode: gdriver_ipc::SyncMode = params
            .as_ref()
            .and_then(|v| v.get("mode"))
            .ok_or_else(|| JsonRpcError::invalid_params("mode is required"))
            .and_then(|v| {
                serde_json::from_value(v.clone())
                    .map_err(|e| JsonRpcError::invalid_params(&e.to_string()))
            })?;

        // Update in-memory config.
        {
            let mut prefs = self.ctx.config.write().await;
            prefs.vfs.sync_mode = new_mode;
        }

        // Notify the sync engine (before persisting so the mode switch
        // happens immediately even if disk write is slow).
        self.ctx
            .sync_cmd_tx
            .send(SyncCommand::SwitchMode(new_mode))
            .await
            .map_err(|e| JsonRpcError::internal_error(e.to_string()))?;

        // Persist to disk (best-effort — log error but don't fail the
        // command since the in-memory state and engine are already updated).
        let prefs_clone = {
            let prefs = self.ctx.config.read().await;
            prefs.clone()
        };
        if let Err(e) = tokio::task::spawn_blocking(move || crate::config::save(&prefs_clone))
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))
            .and_then(|r| r)
        {
            tracing::error!("failed to persist sync mode to disk: {e:#}");
        }

        debug!(?new_mode, "sync mode updated");
        Ok(json!({ "sync_mode": new_mode }))
    }

    // ── Notification handlers ────────────────────────────────────────────────

    /// Return a list of notifications.
    async fn handle_notification_list(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let unread_only = params
            .as_ref()
            .and_then(|v| v.get("unreadOnly"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let limit = params
            .as_ref()
            .and_then(|v| v.get("limit"))
            .and_then(|v| v.as_u64())
            .unwrap_or(50) as u32;

        let rows = crate::db::notifications::list_notifications(&self.ctx.db, unread_only, limit)
            .await
            .map_err(|e| JsonRpcError::internal_error(e.to_string()))?;

        let items: Vec<Value> = rows
            .into_iter()
            .map(|n| {
                let kind_val: Value = serde_json::from_str(&n.payload).unwrap_or(Value::Null);
                let mut obj = serde_json::json!({
                    "id": n.id,
                    "account_id": n.account_id,
                    "is_read": n.is_read,
                    "created_at": n.created_at,
                    "type": n.kind,
                });
                // Merge the kind-specific payload fields into the top-level object.
                if let Value::Object(ref map) = kind_val {
                    if let Value::Object(ref mut obj_map) = obj {
                        for (k, v) in map {
                            obj_map.insert(k.clone(), v.clone());
                        }
                    }
                }
                obj
            })
            .collect();

        serde_json::to_value(items).map_err(|e| JsonRpcError::internal_error(e.to_string()))
    }

    /// Dismiss (delete) a notification.
    async fn handle_notification_dismiss(
        &self,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let id = params
            .as_ref()
            .and_then(|v| v.get("id"))
            .and_then(|v| v.as_i64())
            .ok_or_else(|| JsonRpcError::invalid_params("id is required"))?;

        crate::db::notifications::dismiss_notification(&self.ctx.db, id)
            .await
            .map_err(|e| JsonRpcError::internal_error(e.to_string()))?;

        debug!("notification {id} dismissed");
        Ok(json!({ "dismissed": id }))
    }

    /// Mark a single notification as read.
    async fn handle_notification_mark_read(
        &self,
        params: Option<Value>,
    ) -> Result<Value, JsonRpcError> {
        let id = params
            .as_ref()
            .and_then(|v| v.get("id"))
            .and_then(|v| v.as_i64())
            .ok_or_else(|| JsonRpcError::invalid_params("id is required"))?;

        crate::db::notifications::mark_read(&self.ctx.db, id)
            .await
            .map_err(|e| JsonRpcError::internal_error(e.to_string()))?;

        debug!("notification {id} marked as read");
        Ok(json!({ "marked_read": id }))
    }

    /// Mark all notifications as read.
    async fn handle_notification_mark_all_read(&self) -> Result<Value, JsonRpcError> {
        crate::db::notifications::mark_all_read(&self.ctx.db)
            .await
            .map_err(|e| JsonRpcError::internal_error(e.to_string()))?;

        debug!("all notifications marked as read");
        Ok(json!({ "marked_all_read": true }))
    }

    // ── Folder management handlers ───────────────────────────────────────────

    /// Return all configured sync folders (enabled and disabled).
    async fn handle_folder_list(&self) -> Result<Value, JsonRpcError> {
        info!("folder.list called");
        let rows = sqlx::query_as::<_, crate::db::sync_folders::SyncFolderRow>(
            "SELECT id, account_id, local_path, folder_type, is_enabled
             FROM sync_folders
             ORDER BY id",
        )
        .fetch_all(&self.ctx.db)
        .await
        .map_err(|e| JsonRpcError::internal_error(e.to_string()))?;

        let folders: Vec<Value> = rows
            .into_iter()
            .map(|r| {
                serde_json::json!({
                    "id": r.id.to_string(),
                    "name": std::path::Path::new(&r.local_path)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(&r.local_path),
                    "path": r.local_path,
                    "type": r.folder_type,
                    "is_enabled": r.is_enabled != 0,
                })
            })
            .collect();

        Ok(json!(folders))
    }

    /// Add a new sync folder.
    async fn handle_folder_add(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        info!("folder.add called with params: {:?}", params);

        let params = params
            .and_then(|v| v.as_object().cloned())
            .ok_or_else(|| JsonRpcError::invalid_params("expected object"))?;

        let local_path = params
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError::invalid_params("path is required"))?
            .to_string();

        let folder_type = params
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("drive")
            .to_string();

        info!("folder.add: path={}, type={}", local_path, folder_type);

        // Use the first available account.
        let account_id: String =
            sqlx::query_scalar("SELECT id FROM accounts ORDER BY last_used_at DESC LIMIT 1")
                .fetch_optional(&self.ctx.db)
                .await
                .map_err(|e| JsonRpcError::internal_error(e.to_string()))?
                .ok_or_else(|| JsonRpcError::internal_error("no account connected"))?;

        let folder = crate::db::sync_folders::SyncFolder {
            id: None,
            account_id,
            local_path: local_path.clone(),
            folder_type: folder_type.clone(),
            is_enabled: true,
        };

        let saved = crate::db::sync_folders::add_folder(&self.ctx.db, &folder)
            .await
            .map_err(|e| {
                error!("folder.add DB error: {e:#}");
                JsonRpcError::internal_error(e.to_string())
            })?;

        info!("folder.add: saved id={:?}", saved.id);

        // Signal the watcher to reload its watched folders.
        let _ = self.ctx.watcher_reload_tx.try_send(());

        let name = std::path::Path::new(&local_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&local_path);

        Ok(json!({
            "id": saved.id.unwrap_or(0).to_string(),
            "name": name,
            "path": local_path,
            "type": folder_type,
        }))
    }

    /// Remove a sync folder by id.
    async fn handle_folder_remove(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let id = params
            .as_ref()
            .and_then(|v| v.get("id"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError::invalid_params("id is required"))?;

        let id: i64 = id
            .parse()
            .map_err(|_| JsonRpcError::invalid_params("id must be a numeric string"))?;

        sqlx::query("DELETE FROM sync_folders WHERE id = ?")
            .bind(id)
            .execute(&self.ctx.db)
            .await
            .map_err(|e| JsonRpcError::internal_error(e.to_string()))?;

        debug!("sync folder {id} removed");

        // Signal the watcher to reload its watched folders.
        let _ = self.ctx.watcher_reload_tx.try_send(());

        Ok(json!({ "removed": id }))
    }

    /// Return the total size (in bytes) of files within a sync folder.
    async fn handle_folder_get_size(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let folder_id = params
            .as_ref()
            .and_then(|v| v.get("id"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError::invalid_params("id is required"))?;

        let folder_id: i64 = folder_id
            .parse()
            .map_err(|_| JsonRpcError::invalid_params("id must be a numeric string"))?;

        let row: Option<(String,)> =
            sqlx::query_as("SELECT local_path FROM sync_folders WHERE id = ?")
                .bind(folder_id)
                .fetch_optional(&self.ctx.db)
                .await
                .map_err(|e| JsonRpcError::internal_error(e.to_string()))?;

        let path = match row {
            Some((p,)) => p,
            None => return Ok(json!({ "bytes": 0 })),
        };

        let total: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(size), 0) FROM drive_files WHERE local_path LIKE ?",
        )
        .bind(format!("{path}%"))
        .fetch_one(&self.ctx.db)
        .await
        .map_err(|e| JsonRpcError::internal_error(e.to_string()))?;

        Ok(json!({ "bytes": total }))
    }

    /// Return platform-suggested sync folders (~Desktop, ~Documents, etc.).
    async fn handle_folder_get_suggested(&self) -> Result<Value, JsonRpcError> {
        let home = dirs::home_dir()
            .map(|h| h.display().to_string())
            .unwrap_or_else(|| "~".to_string());

        let suggestions = vec![
            json!({ "id": "desktop", "name": "Desktop", "path": format!("{home}/Desktop"), "type": "drive" }),
            json!({ "id": "documents", "name": "Documents", "path": format!("{home}/Documents"), "type": "drive" }),
            json!({ "id": "downloads", "name": "Downloads", "path": format!("{home}/Downloads"), "type": "drive" }),
            json!({ "id": "pictures", "name": "Pictures", "path": format!("{home}/Pictures"), "type": "photos" }),
            json!({ "id": "movies", "name": "Movies", "path": format!("{home}/Movies"), "type": "photos" }),
        ];

        Ok(json!(suggestions))
    }

    // ── System handlers ──────────────────────────────────────────────────────

    /// Submit user feedback (stub — logs to tracing until a backend service is wired).
    async fn handle_submit_feedback(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let text = params
            .as_ref()
            .and_then(|v| v.get("text"))
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let include_logs = params
            .as_ref()
            .and_then(|v| v.get("includeLogs"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        info!(
            text_len = text.len(),
            include_logs, "user feedback submitted"
        );

        let feedback = crate::db::feedback::Feedback {
            id: None,
            text: text.to_string(),
            include_logs,
            created_at: chrono::Utc::now().timestamp_millis(),
        };
        if let Err(e) = crate::db::feedback::insert_feedback(&self.ctx.db, &feedback).await {
            warn!(error = %e, "failed to persist feedback");
        }

        Ok(json!({ "submitted": true }))
    }

    /// Return the current OS platform name.
    fn handle_get_platform(&self) -> Result<Value, JsonRpcError> {
        #[cfg(target_os = "macos")]
        {
            Ok(json!("macOS"))
        }
        #[cfg(target_os = "linux")]
        {
            Ok(json!("Linux"))
        }
        #[cfg(target_os = "windows")]
        {
            Ok(json!("Windows"))
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        {
            Ok(json!("Unknown"))
        }
    }

    /// Return high-level file/folder counts from the local database.
    async fn handle_system_get_drive_stats(&self) -> Result<Value, JsonRpcError> {
        let (file_count, folder_count) = crate::db::files::count_files_and_folders(&self.ctx.db)
            .await
            .map_err(|e| JsonRpcError::internal_error(e.to_string()))?;

        let stats = gdriver_ipc::DriveStats {
            file_count,
            folder_count,
        };

        serde_json::to_value(stats).map_err(|e| JsonRpcError::internal_error(e.to_string()))
    }

    // ── Offline handlers ─────────────────────────────────────────────────────

    /// Return aggregated storage stats for offline-pinned and cached files.
    async fn handle_offline_get_stats(&self) -> Result<Value, JsonRpcError> {
        let (offline_bytes, cache_bytes) = crate::db::files::sum_bytes_by_sync_state(&self.ctx.db)
            .await
            .map_err(|e| JsonRpcError::internal_error(e.to_string()))?;

        let stats = gdriver_ipc::OfflineStats {
            offline_bytes,
            cache_bytes,
        };

        serde_json::to_value(stats).map_err(|e| JsonRpcError::internal_error(e.to_string()))
    }

    /// Reset all user-pinned offline files back to `cached` state.
    async fn handle_offline_clear_cache(&self) -> Result<Value, JsonRpcError> {
        sqlx::query(
            "UPDATE drive_files SET sync_state = 'cached' WHERE sync_state = 'offline' AND is_trashed = 0",
        )
        .execute(&self.ctx.db)
        .await
        .map_err(|e| JsonRpcError::internal_error(e.to_string()))?;

        Ok(json!({ "cleared": true }))
    }

    // ── Filesystem query handlers (extensions) ───────────────────────────────

    /// Return the sync state and metadata for a file identified by its local path.
    ///
    /// Used by file manager extensions (Nautilus, Dolphin) to display icon
    /// overlays and context menus.  Path resolution:
    ///   1. Exact match on `drive_files.local_path`.
    ///   2. Prefix match: strip the VFS mount point and match by trailing path.
    async fn handle_fs_get_sync_state(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let path = extract_string_param(&params, "path")?;

        // Try exact local_path match first.
        let file = crate::db::files::get_file_by_local_path(&self.ctx.db, &path)
            .await
            .map_err(|e| JsonRpcError::internal_error(e.to_string()))?;

        // If not found, try matching by relative path under the mount point.
        let file = match file {
            Some(f) => Some(f),
            None => resolve_path_under_mount(&self.ctx, &path).await?,
        };

        let file = file.ok_or_else(|| JsonRpcError::not_found("file not found for path"))?;

        let state: gdriver_ipc::SyncState =
            serde_json::from_str(&format!("\"{}\"", file.sync_state))
                .unwrap_or(gdriver_ipc::SyncState::CloudOnly);

        let is_folder = file.mime_type == "application/vnd.google-apps.folder";

        Ok(serde_json::json!({
            "state": state,
            "file_id": file.id,
            "name": file.name,
            "is_folder": is_folder,
            "drive_url": format!("https://drive.google.com/file/d/{}/view", file.id),
        }))
    }

    /// Change a file's offline availability.
    ///
    /// When `enabled` is true the sync state is set to `offline` (pinned for
    /// offline access).  When false the state reverts to `cached` (local copy
    /// can be evicted).
    async fn handle_fs_set_offline(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let path = extract_string_param(&params, "path")?;
        let enabled = params
            .as_ref()
            .and_then(|v| v.get("enabled"))
            .and_then(|v| v.as_bool())
            .ok_or_else(|| JsonRpcError::invalid_params("enabled (bool) is required"))?;

        let file = resolve_file_for_path(&self.ctx, &path).await?;

        let new_state = if enabled { "offline" } else { "cached" };

        crate::db::files::set_sync_state(&self.ctx.db, &file.id, &file.account_id, new_state)
            .await
            .map_err(|e| JsonRpcError::internal_error(e.to_string()))?;

        debug!(
            path = %path,
            file_id = %file.id,
            enabled,
            "set_offline: state → {new_state}"
        );

        Ok(serde_json::json!({ "state": new_state }))
    }

    /// Return the Google Drive web URL for a file.
    async fn handle_fs_get_share_link(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let path = extract_string_param(&params, "path")?;

        let file = resolve_file_for_path(&self.ctx, &path).await?;

        let url = format!("https://drive.google.com/file/d/{}/view", file.id);

        Ok(serde_json::json!({ "url": url }))
    }

    // ── FileProvider handlers (macOS) ───────────────────────────────────────

    /// Handle `fp.get_item` — return metadata for a single item.
    async fn handle_fp_get_item(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let path = extract_string_param(&params, "path")?;
        let f = resolve_file_for_path(&self.ctx, &path).await?;

        let is_folder = f.mime_type == "application/vnd.google-apps.folder";
        Ok(json!({
            "name": f.name,
            "file_id": f.id,
            "account_id": f.account_id,
            "mime_type": f.mime_type,
            "size": f.size,
            "modified_time": f.modified_time,
            "sync_state": f.sync_state,
            "is_shared": f.is_shared,
            "is_folder": is_folder,
            "parent_file_id": f.parent_id,
            "local_path": f.local_path,
        }))
    }

    /// Handle `fp.list_children` — enumerate directory contents.
    async fn handle_fp_list_children(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let path = extract_string_param(&params, "path")?;

        // For the root path, list top-level files.
        let (parent_id, account_id) = if path == "/" {
            // Get the first account for root listing.
            let acct: Option<String> =
                sqlx::query_scalar("SELECT id FROM accounts ORDER BY last_used_at DESC LIMIT 1")
                    .fetch_optional(&self.ctx.db)
                    .await
                    .map_err(|e| JsonRpcError::internal_error(e.to_string()))?;
            let acct = acct.ok_or_else(|| JsonRpcError::not_found("no accounts configured"))?;
            (None, acct)
        } else {
            let file = resolve_file_for_path(&self.ctx, &path).await?;
            (Some(file.id), file.account_id)
        };

        let children =
            crate::db::files::list_children(&self.ctx.db, parent_id.as_deref(), &account_id)
                .await
                .map_err(|e| JsonRpcError::internal_error(e.to_string()))?;

        let items: Vec<Value> = children
            .into_iter()
            .map(|c| {
                let is_folder = c.mime_type == "application/vnd.google-apps.folder";
                json!({
                    "name": c.name,
                    "file_id": c.id,
                    "account_id": c.account_id,
                    "mime_type": c.mime_type,
                    "size": c.size,
                    "modified_time": c.modified_time,
                    "sync_state": c.sync_state,
                    "is_shared": c.is_shared,
                    "is_folder": is_folder,
                    "parent_file_id": c.parent_id,
                    "local_path": c.local_path,
                })
            })
            .collect();

        Ok(json!({ "items": items }))
    }

    /// Handle `fp.fetch_contents` — trigger download and return local path.
    async fn handle_fp_fetch_contents(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let path = extract_string_param(&params, "path")?;
        let file = resolve_file_for_path(&self.ctx, &path).await?;

        let local_path = file.local_path.clone().unwrap_or_else(|| {
            format!(
                "{}/{}/{}",
                dirs::cache_dir().unwrap_or_default().display(),
                file.account_id,
                file.id
            )
        });

        let now = chrono::Utc::now().timestamp_millis();
        sqlx::query(
            "INSERT INTO sync_queue (account_id, file_id, operation, local_path, priority,
                                     status, retry_count, created_at, updated_at)
             VALUES (?, ?, 'download', ?, 1, 'pending', 0, ?, ?)",
        )
        .bind(&file.account_id)
        .bind(&file.id)
        .bind(&local_path)
        .bind(now)
        .bind(now)
        .execute(&self.ctx.db)
        .await
        .map_err(|e| JsonRpcError::internal_error(e.to_string()))?;

        Ok(json!({
            "name": file.name,
            "file_id": file.id,
            "account_id": file.account_id,
            "local_path": local_path,
            "mime_type": file.mime_type,
            "size": file.size,
            "modified_time": file.modified_time,
            "sync_state": file.sync_state,
            "is_folder": file.mime_type == "application/vnd.google-apps.folder",
            "parent_file_id": file.parent_id,
            "is_shared": file.is_shared,
        }))
    }

    /// Handle `fp.create_item` — create a new file or folder.
    async fn handle_fp_create_item(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let params = params
            .and_then(|v| v.as_object().cloned())
            .ok_or_else(|| JsonRpcError::invalid_params("expected object"))?;

        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError::invalid_params("name is required"))?;
        let is_folder = params
            .get("is_folder")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let account_id = sqlx::query_scalar::<_, String>(
            "SELECT id FROM accounts ORDER BY last_used_at DESC LIMIT 1",
        )
        .fetch_optional(&self.ctx.db)
        .await
        .map_err(|e| JsonRpcError::internal_error(e.to_string()))?
        .ok_or_else(|| JsonRpcError::internal_error("no account connected"))?;

        let temp_id = format!(
            "local-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );

        let mime_type = if is_folder {
            "application/vnd.google-apps.folder".to_string()
        } else {
            crate::vfs::guess_mime(name)
        };

        let now = chrono::Utc::now().timestamp_millis();

        if is_folder {
            sqlx::query(
                "INSERT INTO drive_files (id, account_id, name, mime_type, parent_id, size,
                                          modified_time, is_trashed, is_shared, sync_state)
                 VALUES (?, ?, ?, ?, NULL, 0, ?, 0, 0, 'cloud_only')",
            )
            .bind(&temp_id)
            .bind(&account_id)
            .bind(name)
            .bind(&mime_type)
            .bind(now)
            .execute(&self.ctx.db)
            .await
            .map_err(|e| JsonRpcError::internal_error(e.to_string()))?;
        } else {
            let local_path = params
                .get("local_path")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            sqlx::query(
                "INSERT INTO drive_files (id, account_id, name, mime_type, parent_id, size,
                                          modified_time, is_trashed, is_shared, local_path, sync_state)
                 VALUES (?, ?, ?, ?, NULL, 0, ?, 0, 0, ?, 'modified')",
            )
            .bind(&temp_id)
            .bind(&account_id)
            .bind(name)
            .bind(&mime_type)
            .bind(now)
            .bind(&local_path)
            .execute(&self.ctx.db)
            .await
            .map_err(|e| JsonRpcError::internal_error(e.to_string()))?;

            if let Some(ref lp) = local_path {
                sqlx::query(
                    "INSERT INTO sync_queue (account_id, file_id, operation, local_path, priority,
                                             status, retry_count, created_at, updated_at)
                     VALUES (?, ?, 'upload', ?, 1, 'pending', 0, ?, ?)",
                )
                .bind(&account_id)
                .bind(&temp_id)
                .bind(lp)
                .bind(now)
                .bind(now)
                .execute(&self.ctx.db)
                .await
                .map_err(|e| JsonRpcError::internal_error(e.to_string()))?;
            }
        }

        Ok(json!({
            "name": name,
            "file_id": temp_id,
            "account_id": account_id,
            "mime_type": mime_type,
            "is_folder": is_folder,
            "size": 0,
            "modified_time": now,
            "sync_state": if is_folder { "cloud_only" } else { "modified" },
        }))
    }

    /// Handle `fp.modify_item` — rename or write new content.
    async fn handle_fp_modify_item(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let params = params
            .and_then(|v| v.as_object().cloned())
            .ok_or_else(|| JsonRpcError::invalid_params("expected object"))?;

        let path = params
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError::invalid_params("path is required"))?;

        let file = resolve_file_for_path(&self.ctx, &path).await?;

        let now = chrono::Utc::now().timestamp_millis();

        if let Some(new_name) = params.get("new_name").and_then(|v| v.as_str()) {
            sqlx::query(
                "UPDATE drive_files SET name = ?, modified_time = ? WHERE id = ? AND account_id = ?",
            )
            .bind(new_name)
            .bind(now)
            .bind(&file.id)
            .bind(&file.account_id)
            .execute(&self.ctx.db)
            .await
            .map_err(|e| JsonRpcError::internal_error(e.to_string()))?;

            sqlx::query(
                "INSERT INTO sync_queue (account_id, file_id, operation, local_path, priority,
                                         status, retry_count, created_at, updated_at)
                 VALUES (?, ?, 'rename', ?, 1, 'pending', 0, ?, ?)",
            )
            .bind(&file.account_id)
            .bind(&file.id)
            .bind(&file.local_path)
            .bind(now)
            .bind(now)
            .execute(&self.ctx.db)
            .await
            .map_err(|e| JsonRpcError::internal_error(e.to_string()))?;
        }

        if let Some(local_path) = params.get("local_path").and_then(|v| v.as_str()) {
            sqlx::query(
                "UPDATE drive_files SET local_path = ?, sync_state = 'modified', modified_time = ?
                 WHERE id = ? AND account_id = ?",
            )
            .bind(local_path)
            .bind(now)
            .bind(&file.id)
            .bind(&file.account_id)
            .execute(&self.ctx.db)
            .await
            .map_err(|e| JsonRpcError::internal_error(e.to_string()))?;

            sqlx::query(
                "INSERT INTO sync_queue (account_id, file_id, operation, local_path, priority,
                                         status, retry_count, created_at, updated_at)
                 VALUES (?, ?, 'upload', ?, 1, 'pending', 0, ?, ?)",
            )
            .bind(&file.account_id)
            .bind(&file.id)
            .bind(local_path)
            .bind(now)
            .bind(now)
            .execute(&self.ctx.db)
            .await
            .map_err(|e| JsonRpcError::internal_error(e.to_string()))?;
        }

        Ok(json!({
            "name": file.name,
            "file_id": file.id,
            "account_id": file.account_id,
            "mime_type": file.mime_type,
            "sync_state": "modified",
            "modified_time": now,
        }))
    }

    /// Handle `fp.delete_item` — trash a file.
    async fn handle_fp_delete_item(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let path = extract_string_param(&params, "path")?;
        let file = resolve_file_for_path(&self.ctx, &path).await?;

        let now = chrono::Utc::now().timestamp_millis();

        sqlx::query(
            "UPDATE drive_files SET is_trashed = 1, modified_time = ? WHERE id = ? AND account_id = ?",
        )
        .bind(now)
        .bind(&file.id)
        .bind(&file.account_id)
        .execute(&self.ctx.db)
        .await
        .map_err(|e| JsonRpcError::internal_error(e.to_string()))?;

        sqlx::query(
            "INSERT INTO sync_queue (account_id, file_id, operation, local_path, priority,
                                     status, retry_count, created_at, updated_at)
             VALUES (?, ?, 'delete', ?, 1, 'pending', 0, ?, ?)",
        )
        .bind(&file.account_id)
        .bind(&file.id)
        .bind(&file.local_path)
        .bind(now)
        .bind(now)
        .execute(&self.ctx.db)
        .await
        .map_err(|e| JsonRpcError::internal_error(e.to_string()))?;

        Ok(json!({ "deleted": true }))
    }

    // ── Auth handlers ───────────────────────────────────────────────────────

    /// Start the OAuth2 PKCE flow.
    ///
    /// Returns an authorization URL that the UI must open in the system browser.
    /// A background task waits for the callback, exchanges the code for tokens,
    /// persists the account, and broadcasts `onboarding:oauth-complete`.
    async fn handle_auth_start_flow(&self) -> Result<Value, JsonRpcError> {
        let oauth_config = gdriver_api::auth::OAuthConfig::from_env()
            .map_err(|e| JsonRpcError::internal_error(e.to_string()))?;

        let http = reqwest::Client::new();

        let (auth_url, token_future) = gdriver_api::auth::begin_auth_flow(oauth_config, http)
            .await
            .map_err(|e| JsonRpcError::internal_error(e.to_string()))?;

        // Spawn a background task to complete the flow while the UI waits.
        let ctx = Arc::clone(&self.ctx);
        tokio::spawn(async move {
            match token_future.await {
                Ok(token_set) => {
                    complete_oauth_flow(&ctx, token_set).await;
                }
                Err(e) => {
                    error!("OAuth flow failed: {e:#}");
                }
            }
        });

        info!("OAuth flow started; auth_url returned to caller");
        Ok(json!({ "auth_url": auth_url }))
    }

    /// Return all connected accounts.
    async fn handle_auth_get_accounts(&self) -> Result<Value, JsonRpcError> {
        let accounts = crate::db::accounts::list_accounts(&self.ctx.db)
            .await
            .map_err(|e| JsonRpcError::internal_error(e.to_string()))?;
        serde_json::to_value(accounts).map_err(|e| JsonRpcError::internal_error(e.to_string()))
    }

    /// Disconnect an account: wipe tokens and DB record.
    async fn handle_auth_disconnect(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let params: serde_json::Map<String, Value> = match params {
            Some(Value::Object(m)) => m,
            _ => {
                return Err(JsonRpcError::invalid_params(
                    "expected { account_id: string }",
                ));
            }
        };

        let account_id = params
            .get("account_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError::invalid_params("account_id is required"))?;

        // Best-effort keychain cleanup — don't fail the call if the keyring
        // is unavailable (the DB record is the authoritative state).
        if let Err(e) = self.ctx.tokens.delete_all(account_id) {
            warn!("failed to clean up keychain for {account_id}: {e:#}");
        }

        if let Err(e) = crate::db::accounts::delete_account(&self.ctx.db, account_id).await {
            error!("failed to delete account {account_id}: {e:#}");
            return Err(JsonRpcError::internal_error(e.to_string()));
        }

        info!("account {account_id} disconnected");
        push_account_changed(&self.ctx).await;

        Ok(json!({ "disconnected": account_id }))
    }

    /// Return the BCP-47 locale for the given account.
    ///
    /// Currently returns the locale from the DB; will be enriched from the
    /// OAuth2 userinfo endpoint in a future milestone.
    async fn handle_auth_get_locale(&self, params: Option<Value>) -> Result<Value, JsonRpcError> {
        let account_id = params
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_default();

        let account = crate::db::accounts::get_account(&self.ctx.db, &account_id)
            .await
            .map_err(|e| JsonRpcError::internal_error(e.to_string()))?;

        Ok(json!({ "locale": account.and_then(|a| a.locale) }))
    }

    /// Return storage quota for the given account.
    ///
    /// Quota is currently only persisted during the OAuth flow. A future
    /// milestone will add a periodic refresh.
    async fn handle_auth_get_quota(&self, _params: Option<Value>) -> Result<Value, JsonRpcError> {
        // For now the quota is returned as part of the account record.
        // A dedicated quota cache will be added in M5.5.
        Ok(json!({ "quota": null }))
    }
}

// ─── OAuth flow completion (background task) ─────────────────────────────────

/// Called in a spawned task after `token_future` resolves successfully.
///
/// 1. Fetch user profile + quota from the Drive About API.
/// 2. Persist refresh token to the OS keychain.
/// 3. Cache the access token in memory.
/// 4. Save the account record to SQLite.
/// 5. Push `onboarding:oauth-complete` and `account:changed` events.
async fn complete_oauth_flow(ctx: &RouterContext, token_set: gdriver_api::auth::TokenSet) {
    let client = gdriver_api::client::DriveClient::new(&token_set.access_token);

    // Fetch user info so we know the account identity before persisting tokens.
    let result = crate::api::fetch_and_store_account(&ctx.db, &client).await;

    let (account, _quota) = match result {
        Ok(v) => v,
        Err(e) => {
            error!("failed to fetch account info after OAuth: {e:#}");
            return;
        }
    };

    // Persist tokens keyed by account email.
    if let Some(ref rt) = token_set.refresh_token {
        if let Err(e) = ctx.tokens.save_refresh_token(&account.id, rt) {
            error!("failed to save refresh token to keyring: {e:#}");
        }
    }

    // Cache the access token (and the refresh token it may carry).
    client.set_access_token(&token_set.access_token);
    ctx.tokens.cache_access_token(&account.id, token_set);

    info!(
        email = %account.email,
        "OAuth flow completed, account stored"
    );

    // Notify all connected UI clients.
    push_event(
        &ctx.push_tx,
        PushEvent::OauthComplete(OauthCompletePayload {
            account_id: account.id.clone(),
        }),
    );
    push_account_changed(ctx).await;

    // Run initial sync to populate the drive_files table.
    let (sync_mode, mount_point) = {
        let prefs = ctx.config.read().await;
        (prefs.vfs.sync_mode, prefs.vfs.mount_point.clone())
    };
    if let Err(e) =
        crate::sync::initial::initial_sync(&ctx.db, &account.id, &client, sync_mode, &mount_point)
            .await
    {
        error!("initial sync failed for {}: {e:#}", account.id);
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_tilde_replaces_home() {
        let home = dirs::home_dir().unwrap().display().to_string();
        assert_eq!(expand_tilde("~/GoogleDrive"), format!("{home}/GoogleDrive"));
        assert_eq!(expand_tilde("~/foo/bar"), format!("{home}/foo/bar"));
    }

    #[test]
    fn expand_tilde_bare_tilde() {
        let home = dirs::home_dir().unwrap().display().to_string();
        assert_eq!(expand_tilde("~"), home);
    }

    #[test]
    fn expand_tilde_no_prefix() {
        assert_eq!(expand_tilde("/absolute/path"), "/absolute/path");
        assert_eq!(expand_tilde("relative/path"), "relative/path");
    }

    #[test]
    fn extract_string_param_ok() {
        let params = Some(serde_json::json!({ "path": "/foo/bar" }));
        assert_eq!(extract_string_param(&params, "path").unwrap(), "/foo/bar");
    }

    #[test]
    fn extract_string_param_missing_key() {
        let params = Some(serde_json::json!({ "other": 42 }));
        assert!(extract_string_param(&params, "path").is_err());
    }

    #[test]
    fn extract_string_param_none_params() {
        assert!(extract_string_param(&None, "path").is_err());
    }

    #[tokio::test]
    async fn set_sync_mode_updates_config_and_returns_mode() {
        use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

        let pool = {
            let opts = SqliteConnectOptions::new()
                .filename(":memory:")
                .foreign_keys(true);
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(opts)
                .await
                .unwrap();
            sqlx::migrate!("./migrations").run(&pool).await.unwrap();
            pool
        };

        let cfg = crate::config::new_handle(gdriver_ipc::Preferences::default());
        let (sync_cmd_tx, mut sync_cmd_rx) = tokio::sync::mpsc::channel(8);
        let (push_tx, _) = tokio::sync::broadcast::channel(16);
        let tokens = std::sync::Arc::new(crate::auth::TokenStore::new());
        let (watcher_reload_tx, _) = tokio::sync::mpsc::channel(4);

        let ctx = std::sync::Arc::new(RouterContext {
            db: pool,
            config: cfg.clone(),
            tokens,
            push_tx,
            sync_cmd_tx,
            watcher_reload_tx,
        });
        let router = Router::new(ctx);

        // Initial mode should be Stream (default).
        {
            let prefs = cfg.read().await;
            assert_eq!(prefs.vfs.sync_mode, gdriver_ipc::SyncMode::Stream);
        }

        // Switch to Mirror. The handler updates in-memory config and sends
        // the sync command immediately; disk persistence is best-effort.
        let params = Some(serde_json::json!({ "mode": "mirror" }));
        let result = router
            .dispatch("system.set_sync_mode", params)
            .await
            .unwrap();
        assert_eq!(result["sync_mode"], "mirror");

        // Config should be updated in memory.
        {
            let prefs = cfg.read().await;
            assert_eq!(prefs.vfs.sync_mode, gdriver_ipc::SyncMode::Mirror);
        }

        // Sync engine should receive SwitchMode command.
        let cmd = sync_cmd_rx.recv().await.unwrap();
        match cmd {
            SyncCommand::SwitchMode(mode) => {
                assert_eq!(mode, gdriver_ipc::SyncMode::Mirror);
            }
            other => panic!("expected SwitchMode, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn set_sync_mode_rejects_invalid_mode() {
        use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

        let pool = {
            let opts = SqliteConnectOptions::new()
                .filename(":memory:")
                .foreign_keys(true);
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(opts)
                .await
                .unwrap();
            sqlx::migrate!("./migrations").run(&pool).await.unwrap();
            pool
        };

        let cfg = crate::config::new_handle(gdriver_ipc::Preferences::default());
        let (sync_cmd_tx, _sync_cmd_rx) = tokio::sync::mpsc::channel(8);
        let (push_tx, _) = tokio::sync::broadcast::channel(16);
        let tokens = std::sync::Arc::new(crate::auth::TokenStore::new());
        let (watcher_reload_tx, _) = tokio::sync::mpsc::channel(4);

        let ctx = std::sync::Arc::new(RouterContext {
            db: pool,
            config: cfg.clone(),
            tokens,
            push_tx,
            sync_cmd_tx,
            watcher_reload_tx,
        });
        let router = Router::new(ctx);

        // Invalid mode value.
        let params = Some(serde_json::json!({ "mode": "invalid_mode" }));
        let result = router.dispatch("system.set_sync_mode", params).await;
        assert!(result.is_err(), "should reject invalid mode");

        // Missing mode key.
        let params = Some(serde_json::json!({}));
        let result = router.dispatch("system.set_sync_mode", params).await;
        assert!(result.is_err(), "should reject missing mode");
    }

    #[tokio::test]
    async fn set_sync_mode_is_idempotent() {
        use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

        let pool = {
            let opts = SqliteConnectOptions::new()
                .filename(":memory:")
                .foreign_keys(true);
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(opts)
                .await
                .unwrap();
            sqlx::migrate!("./migrations").run(&pool).await.unwrap();
            pool
        };

        let cfg = crate::config::new_handle(gdriver_ipc::Preferences::default());
        let (sync_cmd_tx, mut sync_cmd_rx) = tokio::sync::mpsc::channel(8);
        let (push_tx, _) = tokio::sync::broadcast::channel(16);
        let tokens = std::sync::Arc::new(crate::auth::TokenStore::new());
        let (watcher_reload_tx, _) = tokio::sync::mpsc::channel(4);

        let ctx = std::sync::Arc::new(RouterContext {
            db: pool,
            config: cfg.clone(),
            tokens,
            push_tx,
            sync_cmd_tx,
            watcher_reload_tx,
        });
        let router = Router::new(ctx);

        // Switch to Mirror.
        let params = Some(serde_json::json!({ "mode": "mirror" }));
        router
            .dispatch("system.set_sync_mode", params)
            .await
            .unwrap();
        sync_cmd_rx.recv().await.unwrap();

        // Switch to Mirror again — should still succeed and send command.
        let params = Some(serde_json::json!({ "mode": "mirror" }));
        let result = router
            .dispatch("system.set_sync_mode", params)
            .await
            .unwrap();
        assert_eq!(result["sync_mode"], "mirror");

        // Engine receives the command (idempotent at handler level; engine handles dedup).
        let cmd = sync_cmd_rx.recv().await.unwrap();
        assert!(matches!(
            cmd,
            SyncCommand::SwitchMode(gdriver_ipc::SyncMode::Mirror)
        ));
    }
}
