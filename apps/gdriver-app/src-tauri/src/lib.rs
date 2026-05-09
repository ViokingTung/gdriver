mod commands;
mod daemon_client;
mod tray;

use std::sync::Arc;

use daemon_client::DaemonState;
use tauri::{Emitter, Manager};
use tracing::{error, info, warn};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("gdriver_app=debug".parse().unwrap()),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        // Register the daemon connection state before setup runs.
        .manage(DaemonState::new())
        .setup(|app| {
            let app_handle = app.handle().clone();

            // ── Close-to-tray: hide window instead of quitting ────────────
            tray::register_close_to_tray(&app_handle);

            // ── System tray icon + context menu ───────────────────────────
            let daemon_state = DaemonState::clone(&app.state::<DaemonState>());
            if let Err(e) = tray::setup_tray(&app_handle, daemon_state) {
                error!("failed to set up system tray: {e:#}");
            }

            // ── Connect (or spawn) the background daemon ──────────────────
            let client_lock = Arc::clone(&app.state::<DaemonState>().0);
            tauri::async_runtime::spawn(async move {
                match daemon_client::DaemonClient::connect_or_spawn(&app_handle).await {
                    Ok(client) => {
                        *client_lock.write().await = Some(client);
                        info!("daemon client ready");
                    }
                    Err(e) => {
                        error!("failed to connect to daemon: {e:#}");
                    }
                }
            });

            // ── Detect system theme and emit to frontend ─────────────────
            if let Some(window) = app.get_webview_window("main") {
                match window.theme() {
                    Ok(theme) => {
                        let theme_str = if theme == tauri::Theme::Dark {
                            "dark"
                        } else {
                            "light"
                        };
                        info!("system theme detected: {theme_str}");
                        let _ = window.emit("system:theme-changed", theme_str);
                    }
                    Err(e) => {
                        warn!("failed to detect system theme: {e}");
                    }
                }
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::ping,
            commands::get_sync_status,
            commands::pause_sync,
            commands::resume_sync,
            commands::get_recent_sync_items,
            commands::get_sync_activity,
            commands::get_sync_errors,
            commands::retry_sync_error,
            commands::reveal_in_file_manager,
            commands::open_url,
            commands::get_notifications,
            commands::dismiss_notification,
            commands::mark_notification_read,
            commands::mark_all_notifications_read,
            commands::get_drive_stats,
            commands::get_accounts,
            commands::get_storage_quota,
            commands::start_oauth_flow,
            commands::disconnect_account,
            commands::get_preferences,
            commands::save_preferences,
            commands::get_sync_folders,
            commands::add_sync_folder,
            commands::open_drive_folder,
            commands::set_sync_mode,
            commands::get_app_version,
            commands::get_hostname,
            commands::get_platform,
            commands::submit_feedback,
            commands::get_account_locale,
            commands::remove_sync_folder,
            commands::quit_app,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
