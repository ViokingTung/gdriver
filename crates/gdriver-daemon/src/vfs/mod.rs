// ─── VFS management (platform-specific) ────────────────────────────────────

use std::path::PathBuf;

use sqlx::SqlitePool;
use tracing::info;

pub use gdriver_vfs::{VfsBackend, VfsHandle};

/// Guess MIME type from a filename based on its extension.
pub fn guess_mime(name: &str) -> String {
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "pdf" => "application/pdf",
        "json" => "application/json",
        "xml" => "application/xml",
        "zip" => "application/zip",
        "gz" => "application/gzip",
        "txt" | "md" | "csv" => "text/plain",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" | "mjs" => "application/javascript",
        "ts" => "application/typescript",
        "doc" | "docx" => "application/msword",
        "xls" | "xlsx" => "application/vnd.ms-excel",
        "ppt" | "pptx" => "application/vnd.ms-powerpoint",
        _ => "application/octet-stream",
    }
    .to_string()
}

/// Mount the virtual filesystem on the given platform.
///
/// On Linux, this spawns a background FUSE session. On Windows, this mounts
/// a virtual drive letter via WinFSP. The returned [`VfsHandle`] must be
/// kept alive for the duration of the mount; dropping it will unmount the
/// filesystem.
///
/// # Errors
///
/// Returns an error if the mount fails (e.g. `fusermount` not installed,
/// mount point in use, or insufficient permissions).
pub async fn mount(
    mount_point: PathBuf,
    cache_dir: PathBuf,
    db: SqlitePool,
) -> anyhow::Result<VfsHandle> {
    let ctx = gdriver_vfs::VfsContext::new(cache_dir, mount_point, db);

    #[cfg(target_os = "linux")]
    {
        let backend = gdriver_vfs::LinuxVfsBackend;
        backend.mount(ctx).await
    }

    #[cfg(target_os = "windows")]
    {
        let backend = gdriver_vfs::WindowsVfsBackend;
        backend.mount(ctx).await
    }

    #[cfg(target_os = "macos")]
    {
        let backend = gdriver_vfs::MacOsVfsBackend;
        backend.mount(ctx).await
    }

    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        let _ = ctx;
        anyhow::bail!("VFS mount is not yet implemented on this platform");
    }
}

/// Unmount the virtual filesystem.
///
/// On Linux, this calls `fusermount -u` as a best-effort cleanup, then
/// drops the handle (which also drops the inner `BackgroundSession`).
/// On Windows, dropping the handle unmounts the virtual drive.
#[allow(dead_code)]
pub async fn unmount(handle: VfsHandle) -> anyhow::Result<()> {
    let mount_point = handle.mount_point.clone();

    #[cfg(target_os = "linux")]
    {
        gdriver_vfs::linux::unmount_fuse(&mount_point)?;
    }

    #[cfg(target_os = "windows")]
    {
        gdriver_vfs::windows::unmount_winfsp(&mount_point)?;
    }

    #[cfg(target_os = "macos")]
    {
        // For FUSE-T mode, unmount via system command.
        // FileProvider mode cleans up on handle drop.
        #[cfg(feature = "fuse")]
        gdriver_vfs::macos::unmount_fuse(&mount_point)?;
    }

    // Drop the handle to clean up the background session.
    drop(handle);

    info!("VFS unmounted from {}", mount_point.display());
    Ok(())
}

/// Spawn a background task that holds the VFS handle until the daemon exits.
///
/// Returns a oneshot sender that can be used to trigger unmount during
/// graceful shutdown.
pub fn spawn_vfs_task(handle: VfsHandle) -> tokio::sync::oneshot::Sender<()> {
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    tokio::spawn(async move {
        // Hold the handle until shutdown is signaled.
        let _handle = handle;
        let _ = shutdown_rx.await;
    });

    shutdown_tx
}
