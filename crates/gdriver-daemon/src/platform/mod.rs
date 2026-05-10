// Platform-specific system integration modules.

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "windows")]
pub mod windows;

#[cfg(target_os = "macos")]
pub use macos::{remove_launch_on_login, set_launch_on_login};
#[cfg(target_os = "windows")]
pub use windows::{remove_launch_on_login, set_launch_on_login};
