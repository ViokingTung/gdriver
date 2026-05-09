// Platform-specific system integration modules.

#[cfg(target_os = "windows")]
pub mod windows;
#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(target_os = "windows")]
pub use windows::{set_launch_on_login, remove_launch_on_login, send_notification};
#[cfg(target_os = "macos")]
pub use macos::{set_launch_on_login, remove_launch_on_login};
