mod api;
mod auth;
mod config;
mod db;
mod ipc;
mod platform;
mod sync;
mod vfs;
mod watcher;

use std::path::PathBuf;
use std::sync::Arc;

use auth::TokenStore;
use db::queue::SyncTask;
use ipc::{IpcServer, PushSender, Router, RouterContext};
use sync::engine::{SyncCommand, SyncContext};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load .env file if present (dev convenience; ignored in production).
    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("gdriver_daemon=debug".parse()?),
        )
        .init();

    tracing::info!("gdriver-daemon starting");

    // ── Database ──────────────────────────────────────────────────────────
    let pool = db::create_pool().await?;
    db::run_migrations(&pool).await?;

    // ── Config ────────────────────────────────────────────────────────────
    // Load from disk; silently writes the default file on first run so users
    // have a human-readable starting point to customise.
    let prefs = config::load()?;
    let config_path = config::config_path()?;
    if !config_path.exists() {
        config::save(&prefs)?;
    }
    let cfg = config::new_handle(prefs);

    // ── Auto-start registration (Windows / macOS) ────────────────────────
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    {
        let prefs = cfg.read().await;
        if prefs.general.launch_on_login {
            if let Err(e) = platform::set_launch_on_login() {
                tracing::warn!("Failed to register auto-start: {e:#}");
            }
        }
    }

    // ── VFS mount point (read before cfg moves into RouterContext) ────────
    let vfs_mount_point = {
        let prefs = cfg.read().await;
        expand_tilde(&prefs.vfs.mount_point)
    };
    let vfs_cache_dir = dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("gdriver");

    // ── Shared runtime resources ──────────────────────────────────────────
    let tokens = Arc::new(TokenStore::new());
    let oauth_config = gdriver_api::auth::OAuthConfig::from_env()?;
    let http = reqwest::Client::new();

    // ── Sync engine command channel ───────────────────────────────────────
    let (sync_cmd_tx, sync_cmd_rx) = tokio::sync::mpsc::channel::<SyncCommand>(32);

    // ── IPC server ────────────────────────────────────────────────────────
    let server = IpcServer::new();
    let push_tx: PushSender = server.push_sender();

    // ── Watcher channels ──────────────────────────────────────────────────
    let (watcher_tx, watcher_rx) = tokio::sync::mpsc::channel::<SyncTask>(128);
    // Reload channel: IPC handlers signal the watcher when folders change.
    let (watcher_reload_tx, watcher_reload_rx) = tokio::sync::mpsc::channel::<()>(4);

    // ── Build routing context and router ──────────────────────────────────
    let ctx = Arc::new(RouterContext::new(
        pool.clone(),
        cfg.clone(),
        push_tx.clone(),
        sync_cmd_tx,
        tokens.clone(),
        watcher_reload_tx,
    ));
    let router = Arc::new(Router::new(ctx));

    // ── Spawn sync engine ─────────────────────────────────────────────────
    let sync_ctx = SyncContext {
        db: pool.clone(),
        push_tx: push_tx.clone(),
        cmd_rx: sync_cmd_rx,
        tokens,
        oauth_config: Some(oauth_config),
        http,
        watcher_rx: Some(watcher_rx),
        cfg,
    };
    let sync_handle = tokio::spawn(async move {
        if let Err(e) = sync::engine::run(sync_ctx).await {
            tracing::error!("sync engine exited with error: {e:#}");
        }
    });

    // ── Clone DB pool for VFS before watcher takes ownership ─────────────
    let vfs_db_pool = pool.clone();

    // ── Spawn local filesystem watcher ─────────────────────────────────────
    let watcher_handle = tokio::spawn(async move {
        if let Err(e) = watcher::run(pool, watcher_tx, watcher_reload_rx).await {
            tracing::error!("filesystem watcher exited with error: {e:#}");
        }
    });

    // ── Mount virtual filesystem (FUSE on Linux) ─────────────────────────
    let vfs_shutdown = match vfs::mount(
        PathBuf::from(&vfs_mount_point),
        vfs_cache_dir,
        // The watcher task took ownership of `pool` above, but the sync
        // engine and router still hold their own clones that won't be
        // dropped until shutdown.  Re-clone from the sync router's context
        // isn't possible here, so we use a separate clone taken earlier.
        vfs_db_pool,
    )
    .await
    {
        Ok(handle) => {
            tracing::info!("VFS mounted at {vfs_mount_point}");
            Some(vfs::spawn_vfs_task(handle))
        }
        Err(e) => {
            tracing::warn!("VFS mount skipped (non-fatal): {e:#}");
            None
        }
    };

    // ── Run until SIGINT / SIGTERM ────────────────────────────────────────
    tokio::select! {
        result = server.run(Arc::clone(&router)) => {
            if let Err(e) = result {
                tracing::error!("IPC server exited with error: {e:#}");
            }
        }
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("received shutdown signal, stopping daemon");
        }
    }

    // ── Graceful shutdown ─────────────────────────────────────────────────
    // Signal the VFS task to drop the handle (which unmounts).
    if let Some(tx) = vfs_shutdown {
        let _ = tx.send(());
    }

    tracing::info!("waiting for sync engine and watcher to shut down");
    let _ = sync_handle.await;
    let _ = watcher_handle.await;

    tracing::info!("gdriver-daemon stopped");
    Ok(())
}

/// Expand `~` in a path string to the user's home directory.
fn expand_tilde(path: &str) -> String {
    if path.starts_with("~/") || path == "~" {
        if let Some(home) = dirs::home_dir() {
            let home_str = home.to_string_lossy();
            if path == "~" {
                return home_str.into_owned();
            }
            return format!("{}/{}", home_str, &path[2..]);
        }
    }
    path.to_string()
}
