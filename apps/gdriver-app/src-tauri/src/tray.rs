use std::sync::Arc;
use std::time::Instant;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Listener, Manager};
use tokio::sync::RwLock;

use crate::daemon_client::DaemonState;

// ─── Tray icon state ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayStatus {
    Syncing,
    Synced,
    Paused,
    Error,
    Offline,
}

impl TrayStatus {
    fn tooltip(self) -> &'static str {
        match self {
            Self::Syncing => "gDriver \u{2014} Syncing\u{2026}",
            Self::Synced => "gDriver \u{2014} Up to date",
            Self::Paused => "gDriver \u{2014} Sync paused",
            Self::Error => "gDriver \u{2014} Sync error",
            Self::Offline => "gDriver \u{2014} Offline",
        }
    }
}

// ─── Setup entry point ───────────────────────────────────────────────────────

/// Build the system tray icon with context menu and event handlers.
pub fn setup_tray(
    app: &AppHandle,
    daemon: DaemonState,
) -> Result<(), Box<dyn std::error::Error>> {
    // ── Menu ──────────────────────────────────────────────────────────────
    let open_item = MenuItem::with_id(app, "open", "Open gDriver", true, None::<&str>)?;
    let pause_item = MenuItem::with_id(app, "toggle_pause", "Pause Sync", true, None::<&str>)?;
    let drive_item =
        MenuItem::with_id(app, "open_drive", "Open Drive folder", true, None::<&str>)?;
    let prefs_item = MenuItem::with_id(app, "preferences", "Preferences", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[
            &open_item,
            &pause_item,
            &drive_item,
            &tauri::menu::PredefinedMenuItem::separator(app)?,
            &prefs_item,
            &quit_item,
        ],
    )?;

    // ── Tray icon ─────────────────────────────────────────────────────────
    let tray = TrayIconBuilder::new()
        .icon(tauri::include_image!("icons/32x32.png"))
        .icon_as_template(false)
        .tooltip(TrayStatus::Synced.tooltip())
        .menu(&menu)
        .on_menu_event({
            let daemon = daemon.clone();
            let pause_item = pause_item.clone();
            move |app, event| {
                let id = event.id().as_ref();
                match id {
                    "open" => show_main_window(app),
                    "toggle_pause" => {
                        let daemon = daemon.clone();
                        let pause_item = pause_item.clone();
                        tauri::async_runtime::spawn(async move {
                            toggle_pause_sync(&daemon, &pause_item).await;
                        });
                    }
                    "open_drive" => {
                        let daemon = daemon.clone();
                        tauri::async_runtime::spawn(async move {
                            if let Some(client) = &*daemon.0.read().await {
                                if let Ok(prefs) = client.call("prefs.get", None).await {
                                    let mount = prefs
                                        .get("vfs")
                                        .and_then(|v| v.get("mount_point"))
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("~/GoogleDrive");
                                    let _ = open_path(mount);
                                }
                            }
                        });
                    }
                    "preferences" => show_main_window(app),
                    "quit" => app.exit(0),
                    _ => {}
                }
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: tauri::tray::MouseButton::Left,
                button_state: tauri::tray::MouseButtonState::Up,
                ..
            } = event
            {
                if let Some(w) = tray.app_handle().get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
        })
        .build(app)?;

    // ── Sync status → tooltip + menu label ───────────────────────────────
    let current_status: Arc<RwLock<TrayStatus>> = Arc::new(RwLock::new(TrayStatus::Synced));
    let last_update: Arc<RwLock<Instant>> = Arc::new(RwLock::new(Instant::now()));

    app.listen("sync:status-changed", {
        let tray = tray.clone();
        let current_status = Arc::clone(&current_status);
        let last_update = Arc::clone(&last_update);
        let pause_item = pause_item.clone();
        move |event| {
            let payload = event.payload();
            // Payload is JSON-encoded string, e.g. "\"syncing\""
            let status_str = payload.trim_matches('"');
            let new_status = match status_str {
                "syncing" => TrayStatus::Syncing,
                "up-to-date" | "up_to_date" => TrayStatus::Synced,
                "paused" => TrayStatus::Paused,
                "error" => TrayStatus::Error,
                "offline" => TrayStatus::Offline,
                _ => return,
            };

            // Debounce rapid updates (skip if <100ms since last, except for
            // state transitions that the user must see immediately).
            let now = Instant::now();
            {
                let mut last = last_update.blocking_write();
                if now.duration_since(*last).as_millis() < 100
                    && new_status != TrayStatus::Paused
                    && new_status != TrayStatus::Error
                {
                    return;
                }
                *last = now;
            }

            *current_status.blocking_write() = new_status;

            let _ = tray.set_tooltip(Some(new_status.tooltip()));

            let label = if new_status == TrayStatus::Paused {
                "Resume Sync"
            } else {
                "Pause Sync"
            };
            let _ = pause_item.set_text(label);
        }
    });

    tracing::info!("system tray initialized");
    Ok(())
}

// ─── Close-to-tray behavior ──────────────────────────────────────────────────

/// Hide the main window to the system tray instead of closing it.
///
/// When the user clicks the window close button, the window is hidden and the
/// app continues running in the background. The `Quit` menu item in the tray
/// context menu calls `app.exit(0)` to actually terminate.
pub fn register_close_to_tray(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let w = window.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = w.hide();
        }
    });
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn show_main_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.set_focus();
    }
}

async fn toggle_pause_sync(daemon: &DaemonState, pause_item: &MenuItem<tauri::Wry>) {
    let is_paused = {
        let guard = daemon.0.read().await;
        match &*guard {
            Some(client) => {
                let val = client.call("sync.get_status", None).await.unwrap_or_default();
                val.get("status").and_then(|s| s.as_str()) == Some("paused")
            }
            None => false,
        }
    };

    if let Some(client) = &*daemon.0.read().await {
        let method = if is_paused { "sync.resume" } else { "sync.pause" };
        let _ = client.call(method, None).await;
    }

    let label = if is_paused { "Pause Sync" } else { "Resume Sync" };
    let _ = pause_item.set_text(label);
}

fn open_path(path: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

