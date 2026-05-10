pub mod backend;
pub mod db;

#[cfg(target_os = "linux")]
pub mod linux;

#[cfg(target_os = "windows")]
pub mod windows;

#[cfg(target_os = "macos")]
pub mod macos;

// ─── Re-exports ──────────────────────────────────────────────────────────────

pub use backend::{VfsBackend, VfsContext, VfsHandle};
#[cfg(target_os = "linux")]
pub use linux::LinuxVfsBackend;
#[cfg(target_os = "macos")]
pub use macos::MacOsVfsBackend;
#[cfg(target_os = "windows")]
pub use windows::WindowsVfsBackend;
