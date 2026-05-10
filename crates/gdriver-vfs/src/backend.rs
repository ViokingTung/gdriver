use std::path::PathBuf;

use async_trait::async_trait;
#[cfg(all(target_os = "macos", feature = "fuse"))]
use fuser;
use sqlx::SqlitePool;

/// Shared context available to every VFS operation.
///
/// This is the VFS layer's window into the rest of the system. Platform
/// implementations (FUSE, WinFSP, FileProvider) receive this context at mount
/// time and use it to serve filesystem requests.
#[derive(Clone)]
pub struct VfsContext {
    /// Path to the local cache directory, e.g. `~/.cache/gdriver/{account_id}/`.
    pub cache_dir: PathBuf,
    /// Mount point path on the local filesystem, e.g. `~/GoogleDrive`.
    pub mount_point: PathBuf,
    /// Database connection pool for looking up file metadata.
    pub db: SqlitePool,
}

impl VfsContext {
    /// Create a new VFS context.
    pub fn new(cache_dir: PathBuf, mount_point: PathBuf, db: SqlitePool) -> Self {
        Self {
            cache_dir,
            mount_point,
            db,
        }
    }
}

/// Platform-agnostic virtual filesystem backend.
///
/// Each platform provides its own implementation (FUSE on Linux, WinFSP on
/// Windows, FileProvider on macOS). The daemon calls [`mount`] once on startup
/// and [`unmount`] on shutdown.
///
/// [`mount`]: VfsBackend::mount
/// [`unmount`]: VfsBackend::unmount
#[async_trait]
pub trait VfsBackend: Send + Sync + 'static {
    /// Mount the virtual filesystem at the configured mount point.
    ///
    /// This call is expected to block until the filesystem is unmounted (or
    /// return immediately after spawning a background thread that handles FUSE
    /// requests). The returned [`VfsHandle`] is used to signal unmount.
    async fn mount(&self, ctx: VfsContext) -> anyhow::Result<VfsHandle>;

    /// Unmount the virtual filesystem, releasing the mount point.
    async fn unmount(handle: VfsHandle) -> anyhow::Result<()>;
}

/// Opaque handle representing a mounted filesystem.
///
/// Dropping this handle should trigger unmount (platform implementations
/// should hook into `Drop` to ensure clean shutdown).
pub struct VfsHandle {
    /// Platform-specific session data.
    ///
    /// - Linux: `fuser::BackgroundSession`
    /// - Windows: `winfsp::filesystem::FileSystemHost`
    /// - macOS: `crate::macos::XpcService` (FileProvider) or
    ///   `fuser::BackgroundSession` (FUSE-T fallback)
    #[cfg(target_os = "linux")]
    pub(crate) inner: Option<fuser::BackgroundSession>,

    #[cfg(all(target_os = "windows", feature = "winfsp-vfs"))]
    pub(crate) inner: Option<winfsp::host::FileSystemHost>,

    #[cfg(all(target_os = "windows", not(feature = "winfsp-vfs")))]
    pub(crate) inner: Option<()>,

    #[cfg(target_os = "macos")]
    pub(crate) inner: Option<crate::macos::VfsHandleInner>,

    /// Mount point path, used for cleanup on drop.
    pub mount_point: PathBuf,
}

impl VfsHandle {
    /// Create a new VFS handle for the Linux FUSE backend.
    #[cfg(target_os = "linux")]
    pub fn new_linux(session: fuser::BackgroundSession, mount_point: PathBuf) -> Self {
        Self {
            inner: Some(session),
            mount_point,
        }
    }

    /// Create a new VFS handle for the Windows WinFSP backend.
    #[cfg(all(target_os = "windows", feature = "winfsp-vfs"))]
    pub fn new_windows(host: winfsp::host::FileSystemHost, mount_point: PathBuf) -> Self {
        Self {
            inner: Some(host),
            mount_point,
        }
    }

    /// Create a new VFS handle stub (WinFSP feature disabled).
    #[cfg(all(target_os = "windows", not(feature = "winfsp-vfs")))]
    pub fn new_windows(_host: (), mount_point: PathBuf) -> Self {
        Self {
            inner: Some(_host),
            mount_point,
        }
    }

    /// Create a new VFS handle for the macOS FileProvider backend (XPC).
    #[cfg(target_os = "macos")]
    pub fn new_macos_fileprovider(xpc: crate::macos::XpcService, mount_point: PathBuf) -> Self {
        Self {
            inner: Some(crate::macos::VfsHandleInner::FileProvider(xpc)),
            mount_point,
        }
    }

    /// Create a new VFS handle for the macOS FUSE-T backend.
    #[cfg(all(target_os = "macos", feature = "fuse"))]
    pub fn new_macos_fuse(session: fuser::BackgroundSession, mount_point: PathBuf) -> Self {
        Self {
            inner: Some(crate::macos::VfsHandleInner::Fuse(session)),
            mount_point,
        }
    }
}

impl Drop for VfsHandle {
    fn drop(&mut self) {
        // On Linux, dropping the BackgroundSession unmounts the filesystem.
        // We explicitly take it out to ensure deterministic drop order.
        #[cfg(target_os = "linux")]
        {
            if let Some(session) = self.inner.take() {
                tracing::info!(
                    "unmounting FUSE filesystem at {}",
                    self.mount_point.display()
                );
                drop(session);
            }
        }

        // On Windows, dropping the FileSystemHost unmounts the virtual drive.
        #[cfg(all(target_os = "windows", feature = "winfsp-vfs"))]
        {
            if let Some(host) = self.inner.take() {
                tracing::info!(
                    "unmounting WinFSP filesystem at {}",
                    self.mount_point.display()
                );
                drop(host);
            }
        }

        #[cfg(all(target_os = "windows", not(feature = "winfsp-vfs")))]
        {
            let _ = self.inner.take();
        }

        // On macOS, drop the platform-specific handle.
        #[cfg(target_os = "macos")]
        {
            if let Some(inner) = self.inner.take() {
                match inner {
                    crate::macos::VfsHandleInner::FileProvider(xpc) => {
                        tracing::info!("stopping FileProvider XPC service");
                        drop(xpc);
                    }
                    #[cfg(feature = "fuse")]
                    crate::macos::VfsHandleInner::Fuse(session) => {
                        tracing::info!(
                            "unmounting FUSE-T filesystem at {}",
                            self.mount_point.display()
                        );
                        drop(session);
                    }
                }
            }
        }
    }
}
