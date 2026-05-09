use serde_json::Value;
use tauri::State;

use crate::daemon_client::DaemonState;

// ─── Connectivity ─────────────────────────────────────────────────────────────

/// Verify the round-trip to gdriver-daemon.
///
/// Waits up to 10 s for the daemon connection to become available (giving the
/// background setup task time to connect / spawn the daemon), then sends a
/// JSON-RPC `ping` and returns the daemon's `"pong"` response.
#[tauri::command]
pub async fn ping(state: State<'_, DaemonState>) -> Result<String, String> {
    let client = state
        .wait_for_client(std::time::Duration::from_secs(10))
        .await
        .map_err(|e| e.to_string())?;

    let val = client
        .call("ping", None)
        .await
        .map_err(|e| e.to_string())?;

    serde_json::from_value::<String>(val).map_err(|e| e.to_string())
}

// ─── Sync control ─────────────────────────────────────────────────────────────

/// Return the current sync status from the daemon.
#[tauri::command]
pub async fn get_sync_status(state: State<'_, DaemonState>) -> Result<Value, String> {
    let client = state
        .wait_for_client(std::time::Duration::from_secs(10))
        .await
        .map_err(|e| e.to_string())?;

    client
        .call("sync.get_status", None)
        .await
        .map_err(|e| e.to_string())
}

/// Pause all sync activity.
#[tauri::command]
pub async fn pause_sync(state: State<'_, DaemonState>) -> Result<Value, String> {
    let client = state
        .wait_for_client(std::time::Duration::from_secs(10))
        .await
        .map_err(|e| e.to_string())?;

    client
        .call("sync.pause", None)
        .await
        .map_err(|e| e.to_string())
}

/// Resume sync activity after a pause.
#[tauri::command]
pub async fn resume_sync(state: State<'_, DaemonState>) -> Result<Value, String> {
    let client = state
        .wait_for_client(std::time::Duration::from_secs(10))
        .await
        .map_err(|e| e.to_string())?;

    client
        .call("sync.resume", None)
        .await
        .map_err(|e| e.to_string())
}

/// Return the most recently synced files.
#[tauri::command]
pub async fn get_recent_sync_items(
    limit: Option<u32>,
    state: State<'_, DaemonState>,
) -> Result<Value, String> {
    let client = state
        .wait_for_client(std::time::Duration::from_secs(10))
        .await
        .map_err(|e| e.to_string())?;

    let params = limit.map(|l| serde_json::json!({ "limit": l }));
    client
        .call("sync.get_recent_items", params)
        .await
        .map_err(|e| e.to_string())
}

/// Reveal a file in the system file manager.
#[tauri::command]
pub async fn reveal_in_file_manager(path: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .args(["-R", &path])
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "linux")]
    {
        // xdg-open with the parent directory; some file managers support --select
        let parent = std::path::Path::new(&path)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| path.clone());
        std::process::Command::new("xdg-open")
            .arg(&parent)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .args(["/select,", &path])
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Return a page of sync activity items.
#[tauri::command]
pub async fn get_sync_activity(
    page: Option<u32>,
    state: State<'_, DaemonState>,
) -> Result<Value, String> {
    let client = state
        .wait_for_client(std::time::Duration::from_secs(10))
        .await
        .map_err(|e| e.to_string())?;

    let params = page.map(|p| serde_json::json!({ "page": p }));
    client
        .call("sync.get_activity", params)
        .await
        .map_err(|e| e.to_string())
}

/// Return all unresolved sync errors.
#[tauri::command]
pub async fn get_sync_errors(
    state: State<'_, DaemonState>,
) -> Result<Value, String> {
    let client = state
        .wait_for_client(std::time::Duration::from_secs(10))
        .await
        .map_err(|e| e.to_string())?;

    client
        .call("sync.get_errors", None)
        .await
        .map_err(|e| e.to_string())
}

/// Mark a sync error as resolved and trigger a retry.
#[tauri::command]
pub async fn retry_sync_error(
    error_id: i64,
    state: State<'_, DaemonState>,
) -> Result<Value, String> {
    let client = state
        .wait_for_client(std::time::Duration::from_secs(10))
        .await
        .map_err(|e| e.to_string())?;

    client
        .call("sync.retry_error", Some(serde_json::json!({ "errorId": error_id })))
        .await
        .map_err(|e| e.to_string())
}

/// Open a URL in the system default browser.
#[tauri::command]
pub fn open_url(url: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&url)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&url)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/c", "start", &url.replace('&', "^&")])
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ─── Notifications ───────────────────────────────────────────────────────────

/// Return a list of notifications.
#[tauri::command]
pub async fn get_notifications(
    unread_only: Option<bool>,
    limit: Option<u32>,
    state: State<'_, DaemonState>,
) -> Result<Value, String> {
    let client = state
        .wait_for_client(std::time::Duration::from_secs(10))
        .await
        .map_err(|e| e.to_string())?;

    let mut params = serde_json::Map::new();
    if let Some(u) = unread_only {
        params.insert("unreadOnly".into(), serde_json::json!(u));
    }
    if let Some(l) = limit {
        params.insert("limit".into(), serde_json::json!(l));
    }
    let p = if params.is_empty() { None } else { Some(serde_json::Value::Object(params)) };

    client
        .call("notification.list", p)
        .await
        .map_err(|e| e.to_string())
}

/// Dismiss (delete) a notification.
#[tauri::command]
pub async fn dismiss_notification(
    id: i64,
    state: State<'_, DaemonState>,
) -> Result<Value, String> {
    let client = state
        .wait_for_client(std::time::Duration::from_secs(10))
        .await
        .map_err(|e| e.to_string())?;

    client
        .call("notification.dismiss", Some(serde_json::json!({ "id": id })))
        .await
        .map_err(|e| e.to_string())
}

/// Mark a single notification as read.
#[tauri::command]
pub async fn mark_notification_read(
    id: i64,
    state: State<'_, DaemonState>,
) -> Result<Value, String> {
    let client = state
        .wait_for_client(std::time::Duration::from_secs(10))
        .await
        .map_err(|e| e.to_string())?;

    client
        .call("notification.mark_read", Some(serde_json::json!({ "id": id })))
        .await
        .map_err(|e| e.to_string())
}

/// Mark all notifications as read.
#[tauri::command]
pub async fn mark_all_notifications_read(
    state: State<'_, DaemonState>,
) -> Result<Value, String> {
    let client = state
        .wait_for_client(std::time::Duration::from_secs(10))
        .await
        .map_err(|e| e.to_string())?;

    client
        .call("notification.mark_all_read", None)
        .await
        .map_err(|e| e.to_string())
}

/// Return high-level Drive file/folder counts.
#[tauri::command]
pub async fn get_drive_stats(state: State<'_, DaemonState>) -> Result<Value, String> {
    let client = state
        .wait_for_client(std::time::Duration::from_secs(10))
        .await
        .map_err(|e| e.to_string())?;

    client
        .call("system.get_drive_stats", None)
        .await
        .map_err(|e| e.to_string())
}

// ─── Account / Auth ──────────────────────────────────────────────────────────

/// Return all connected Google accounts.
#[tauri::command]
pub async fn get_accounts(state: State<'_, DaemonState>) -> Result<Value, String> {
    let client = state
        .wait_for_client(std::time::Duration::from_secs(10))
        .await
        .map_err(|e| e.to_string())?;

    client
        .call("auth.get_accounts", None)
        .await
        .map_err(|e| e.to_string())
}

/// Return the storage quota for a specific account.
#[tauri::command]
pub async fn get_storage_quota(
    account_id: String,
    state: State<'_, DaemonState>,
) -> Result<Value, String> {
    let client = state
        .wait_for_client(std::time::Duration::from_secs(10))
        .await
        .map_err(|e| e.to_string())?;

    client
        .call(
            "auth.get_quota",
            Some(serde_json::json!({ "account_id": account_id })),
        )
        .await
        .map_err(|e| e.to_string())
}

/// Start the OAuth2 flow and return the authorization URL.
#[tauri::command]
pub async fn start_oauth_flow(state: State<'_, DaemonState>) -> Result<String, String> {
    let client = state
        .wait_for_client(std::time::Duration::from_secs(10))
        .await
        .map_err(|e| e.to_string())?;

    let val = client
        .call("auth.start_flow", None)
        .await
        .map_err(|e| e.to_string())?;

    // The daemon returns { auth_url: "..." }
    val.get("auth_url")
        .and_then(|v| v.as_str())
        .map(String::from)
        .ok_or_else(|| "missing auth_url in response".to_string())
}

/// Disconnect an account (removes tokens and DB record).
#[tauri::command]
pub async fn disconnect_account(
    account_id: String,
    state: State<'_, DaemonState>,
) -> Result<(), String> {
    let client = state
        .wait_for_client(std::time::Duration::from_secs(10))
        .await
        .map_err(|e| e.to_string())?;

    client
        .call(
            "auth.disconnect",
            Some(serde_json::json!({ "account_id": account_id })),
        )
        .await
        .map_err(|e| e.to_string())?;

    Ok(())
}

// ─── Preferences ─────────────────────────────────────────────────────────────

/// Return the current preferences from the daemon.
#[tauri::command]
pub async fn get_preferences(state: State<'_, DaemonState>) -> Result<Value, String> {
    let client = state
        .wait_for_client(std::time::Duration::from_secs(10))
        .await
        .map_err(|e| e.to_string())?;

    client
        .call("prefs.get", None)
        .await
        .map_err(|e| e.to_string())
}

/// Save preferences to the daemon.
#[tauri::command]
pub async fn save_preferences(prefs: Value, state: State<'_, DaemonState>) -> Result<(), String> {
    let client = state
        .wait_for_client(std::time::Duration::from_secs(10))
        .await
        .map_err(|e| e.to_string())?;

    client
        .call("prefs.save", Some(prefs))
        .await
        .map_err(|e| e.to_string())?;

    Ok(())
}

/// Return the list of configured sync folders.
#[tauri::command]
pub async fn get_sync_folders(state: State<'_, DaemonState>) -> Result<Value, String> {
    let client = state
        .wait_for_client(std::time::Duration::from_secs(10))
        .await
        .map_err(|e| e.to_string())?;

    client
        .call("folder.list", None)
        .await
        .map_err(|e| e.to_string())
}

/// Add a sync folder to the daemon.
#[tauri::command]
pub async fn add_sync_folder(
    state: State<'_, DaemonState>,
    path: String,
    folder_type: String,
) -> Result<Value, String> {
    let client = state
        .wait_for_client(std::time::Duration::from_secs(10))
        .await
        .map_err(|e| e.to_string())?;

    let params = serde_json::json!({ "path": path, "type": folder_type });
    client
        .call("folder.add", Some(params))
        .await
        .map_err(|e| e.to_string())
}

/// Open the Drive mount folder in the system file manager.
#[tauri::command]
pub async fn open_drive_folder(state: State<'_, DaemonState>) -> Result<(), String> {
    let client = state
        .wait_for_client(std::time::Duration::from_secs(10))
        .await
        .map_err(|e| e.to_string())?;

    // Ask the daemon for the mount point, then open it.
    let prefs_val = client
        .call("prefs.get", None)
        .await
        .map_err(|e| e.to_string())?;

    let mount_point = prefs_val
        .get("vfs")
        .and_then(|v| v.get("mount_point"))
        .and_then(|v| v.as_str())
        .unwrap_or("~/GoogleDrive");

    // Expand ~ to home directory.
    let expanded = if let Some(rest) = mount_point.strip_prefix("~/") {
        dirs::home_dir()
            .map(|h| format!("{}/{}", h.display(), rest))
            .unwrap_or_else(|| mount_point.to_string())
    } else {
        mount_point.to_string()
    };

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&expanded)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&expanded)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(&expanded)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ─── System ───────────────────────────────────────────────────────────────────

/// Switch the sync mode (stream ↔ mirror).
#[tauri::command]
pub async fn set_sync_mode(
    state: State<'_, DaemonState>,
    mode: String,
) -> Result<Value, String> {
    let client = state
        .wait_for_client(std::time::Duration::from_secs(10))
        .await
        .map_err(|e| e.to_string())?;

    let params = serde_json::json!({ "mode": mode });
    client
        .call("system.set_sync_mode", Some(params))
        .await
        .map_err(|e| e.to_string())
}

/// Return the current application version from Cargo.toml.
#[tauri::command]
pub fn get_app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Quit the application.
///
/// Note: this only exits the Tauri process.  The daemon keeps running in the
/// background (by design).
#[tauri::command]
pub fn quit_app(app: tauri::AppHandle) {
    app.exit(0);
}

/// Return the hostname of the current machine (e.g. "Vioking's MacBook Pro").
#[tauri::command]
pub fn get_hostname() -> String {
    gethostname::gethostname()
        .into_string()
        .unwrap_or_else(|_| "Computer".to_string())
}

/// Return the current OS platform name.
#[tauri::command]
pub fn get_platform() -> String {
    #[cfg(target_os = "macos")]
    { "macOS".to_string() }
    #[cfg(target_os = "linux")]
    { "Linux".to_string() }
    #[cfg(target_os = "windows")]
    { "Windows".to_string() }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    { "Unknown".to_string() }
}

/// Submit user feedback to the daemon.
#[tauri::command]
pub async fn submit_feedback(
    text: String,
    include_logs: Option<bool>,
    allow_email: Option<bool>,
    state: State<'_, DaemonState>,
) -> Result<(), String> {
    let client = state
        .wait_for_client(std::time::Duration::from_secs(10))
        .await
        .map_err(|e| e.to_string())?;

    let mut params = serde_json::json!({ "text": text });
    if let Some(il) = include_logs {
        params["includeLogs"] = serde_json::json!(il);
    }
    if let Some(ae) = allow_email {
        params["allowEmail"] = serde_json::json!(ae);
    }

    client
        .call("system.submit_feedback", Some(params))
        .await
        .map_err(|e| e.to_string())?;

    Ok(())
}

/// Return the BCP-47 locale for the given account (falls back to "en").
#[tauri::command]
pub async fn get_account_locale(
    account_id: Option<String>,
    state: State<'_, DaemonState>,
) -> Result<String, String> {
    let client = state
        .wait_for_client(std::time::Duration::from_secs(10))
        .await
        .map_err(|e| e.to_string())?;

    let params = account_id.map(|id| serde_json::json!(id));
    let val = client
        .call("auth.get_locale", params)
        .await
        .map_err(|e| e.to_string())?;

    let locale = val
        .get("locale")
        .and_then(|v| v.as_str())
        .unwrap_or("en")
        .to_string();

    Ok(locale)
}

/// Remove a sync folder from the daemon.
#[tauri::command]
pub async fn remove_sync_folder(
    folder_id: String,
    state: State<'_, DaemonState>,
) -> Result<(), String> {
    let client = state
        .wait_for_client(std::time::Duration::from_secs(10))
        .await
        .map_err(|e| e.to_string())?;

    client
        .call("folder.remove", Some(serde_json::json!({ "id": folder_id })))
        .await
        .map_err(|e| e.to_string())?;

    Ok(())
}
