// Windows platform integration: auto-start and system notifications.

use anyhow::{Context, Result};
use tracing::{info, warn};

// ─── Auto-start via Registry ────────────────────────────────────────────────

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const APP_NAME: &str = "gDriver";
const DAEMON_NAME: &str = "gDriverDaemon";

/// Get the current executable path as a string.
fn current_exe_path() -> Result<String> {
    let path = std::env::current_exe().context("Failed to get current executable path")?;
    Ok(path.to_string_lossy().to_string())
}

/// Register the daemon to launch on user login via the Windows registry.
///
/// Writes to `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` so the
/// daemon starts automatically when the user logs in. This is a per-user
/// setting and does not require administrator privileges.
pub fn set_launch_on_login() -> Result<()> {
    use winreg::{enums::*, RegKey};

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (run_key, _) = hkcu
        .create_subkey(RUN_KEY)
        .context("Failed to open Run registry key")?;

    let exe_path = current_exe_path()?;
    run_key
        .set_string(DAEMON_NAME, &exe_path)
        .context("Failed to set registry value")?;

    info!("Auto-start registered: {} -> {}", DAEMON_NAME, exe_path);
    Ok(())
}

/// Remove the daemon from auto-start on login.
pub fn remove_launch_on_login() -> Result<()> {
    use winreg::{enums::*, RegKey};

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let run_key = hkcu
        .open_subkey_with_flags(RUN_KEY, KEY_SET_VALUE)
        .context("Failed to open Run registry key")?;

    match run_key.delete_value(DAEMON_NAME) {
        Ok(()) => {
            info!("Auto-start removed for {}", DAEMON_NAME);
            Ok(())
        }
        Err(e) => {
            warn!("Auto-start entry not found or already removed: {}", e);
            Ok(()) // Not an error — entry may not exist
        }
    }
}

/// Check if auto-start is currently enabled.
pub fn is_launch_on_login_enabled() -> bool {
    use winreg::{enums::*, RegKey};

    let hkcu = match RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(RUN_KEY, KEY_READ) {
        Ok(key) => key,
        Err(_) => return false,
    };

    match hkcu.get_string::<String, _>(DAEMON_NAME) {
        Ok(_) => true,
        Err(_) => false,
    }
}

// ─── System Notifications ───────────────────────────────────────────────────

/// Send a Windows toast notification using WinRT ToastNotification API.
///
/// Falls back to a simple message box if WinRT is unavailable.
pub fn send_notification(title: &str, body: &str) -> Result<()> {
    // Try WinRT toast notification first
    match send_toast_notification(title, body) {
        Ok(()) => Ok(()),
        Err(e) => {
            warn!(
                "WinRT toast failed ({}), falling back to simple notification",
                e
            );
            send_simple_notification(title, body)
        }
    }
}

/// WinRT ToastNotification implementation.
fn send_toast_notification(title: &str, body: &str) -> Result<()> {
    use windows::{
        core::HSTRING,
        Data::Xml::Dom::XmlDocument,
        UI::Notifications::{ToastNotification, ToastNotificationManager},
    };

    let toast_xml = format!(
        r#"<toast>
            <visual>
                <binding template="ToastGeneric">
                    <text>{}</text>
                    <text>{}</text>
                </binding>
            </visual>
        </toast>"#,
        xml_escape(title),
        xml_escape(body)
    );

    let xml = XmlDocument::new()?;
    xml.LoadXml(&HSTRING::from(&toast_xml))?;

    let toast_notifier = ToastNotificationManager::CreateToastNotificationManager()?;
    let notification = ToastNotification::CreateToastNotification(&xml)?;
    toast_notifier.Show(&notification)?;

    info!("Toast notification sent: {}", title);
    Ok(())
}

/// Simple notification fallback using Win32 MessageBox.
fn send_simple_notification(title: &str, body: &str) -> Result<()> {
    use windows::{
        core::PCWSTR,
        Win32::UI::WindowsAndMessage::{MessageBoxW, MB_ICONINFORMATION, MB_OK},
    };

    let wide_title: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
    let wide_body: Vec<u16> = body.encode_utf16().chain(std::iter::once(0)).collect();

    unsafe {
        MessageBoxW(
            None,
            PCWSTR(wide_body.as_ptr()),
            PCWSTR(wide_title.as_ptr()),
            MB_OK | MB_ICONINFORMATION,
        );
    }

    Ok(())
}

/// Escape XML special characters for toast notification content.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_xml_escape() {
        assert_eq!(xml_escape("hello & <world>"), "hello &amp; &lt;world&gt;");
        assert_eq!(xml_escape("normal text"), "normal text");
    }
}
