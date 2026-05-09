// macOS platform integration: LaunchAgent auto-start and Notification Center.

use anyhow::{Context, Result};
use std::path::PathBuf;
use tracing::{info, warn};

// ─── Auto-start via LaunchAgent ──────────────────────────────────────────────

const LABEL: &str = "com.gdriver.daemon";
const PLIST_FILENAME: &str = "com.gdriver.daemon.plist";

fn launchd_plist_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("cannot determine home directory")?;
    Ok(home
        .join("Library")
        .join("LaunchAgents")
        .join(PLIST_FILENAME))
}

fn current_exe_path() -> Result<String> {
    let path = std::env::current_exe().context("Failed to get current executable path")?;
    Ok(path.to_string_lossy().to_string())
}

fn generate_plist(executable: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exec}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <false/>
    <key>ProcessType</key>
    <string>Interactive</string>
    <key>MachServices</key>
    <dict>
        <key>com.gdriver.daemon.xpc</key>
        <true/>
    </dict>
    <key>StandardOutPath</key>
    <string>/tmp/gdriver-daemon.log</string>
    <key>StandardErrorPath</key>
    <string>/tmp/gdriver-daemon.log</string>
</dict>
</plist>"#,
        label = LABEL,
        exec = executable,
    )
}

/// Register the daemon with launchd to start at user login.
///
/// Writes a property list to `~/Library/LaunchAgents/com.gdriver.daemon.plist`
/// so the daemon starts automatically on macOS login. This is a per-user
/// setting and does not require administrator privileges.
pub fn set_launch_on_login() -> Result<()> {
    let plist_path = launchd_plist_path()?;

    // Ensure LaunchAgents directory exists.
    if let Some(parent) = plist_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {:?}", parent))?;
    }

    let exe_path = current_exe_path()?;
    let plist_content = generate_plist(&exe_path);

    std::fs::write(&plist_path, plist_content)
        .with_context(|| format!("Failed to write plist to {:?}", plist_path))?;

    // Load the job into launchd for the current session if we are in a GUI session.
    // `$(id -u)` is shell syntax — use `sh -c` so it expands.
    let load_result = std::process::Command::new("sh")
        .args([
            "-c",
            &format!(
                "launchctl bootstrap gui/$(id -u) {}",
                plist_path.display(),
            ),
        ])
        .output();

    match load_result {
        Ok(out) if out.status.success() => {
            info!("LaunchAgent registered and loaded: {:?}", plist_path);
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            warn!(
                "LaunchAgent written but launchctl bootstrap failed: {}",
                stderr.trim()
            );
        }
        Err(e) => {
            warn!(
                "LaunchAgent written but launchctl not found (non-GUI session?): {e}"
            );
        }
    }

    Ok(())
}

/// Remove the daemon from auto-start on login.
pub fn remove_launch_on_login() -> Result<()> {
    let plist_path = launchd_plist_path()?;

    // Unload from launchd first.
    if plist_path.exists() {
        let unload_result = std::process::Command::new("sh")
            .args([
                "-c",
                &format!(
                    "launchctl bootout gui/$(id -u) {}",
                    plist_path.display(),
                ),
            ])
            .output();

        match unload_result {
            Ok(out) if out.status.success() => {
                info!("LaunchAgent unloaded");
            }
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                // "not found" is expected if the job wasn't loaded.
                if !stderr.contains("Could not find") {
                    warn!("launchctl bootout warning: {}", stderr.trim());
                }
            }
            Err(e) => {
                warn!("launchctl unload warning: {e}");
            }
        }
    }

    match std::fs::remove_file(&plist_path) {
        Ok(()) => {
            info!("LaunchAgent plist removed: {:?}", plist_path);
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Ok(()) // Already removed — not an error.
        }
        Err(e) => Err(e).context("Failed to remove LaunchAgent plist"),
    }
}

/// Check if auto-start is currently enabled.
#[allow(dead_code)]
pub fn is_launch_on_login_enabled() -> bool {
    launchd_plist_path().map(|p| p.exists()).unwrap_or(false)
}

// ─── System Notifications ───────────────────────────────────────────────────

/// Send a macOS Notification Center notification via `osascript`.
///
/// Uses `display notification` to deliver a native notification banner.
/// This works without any additional Rust crate dependencies and is
/// available on every macOS system since 10.9.
#[allow(dead_code)]
pub fn send_notification(title: &str, body: &str) -> Result<()> {
    let script = format!(
        r#"display notification "{}" with title "{}""#,
        escape_applescript(body),
        escape_applescript(title),
    );

    let output = std::process::Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .output()
        .context("failed to run osascript")?;

    if output.status.success() {
        info!("notification sent: {title}");
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // osascript may fail in non-GUI sessions (e.g. SSH); not fatal.
        warn!("osascript notification failed: {}", stderr.trim());
        Ok(())
    }
}

/// Escape backslashes and double quotes for embedding in AppleScript strings.
#[allow(dead_code)]
fn escape_applescript(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plist_well_formed_xml() {
        let plist = generate_plist("/usr/local/bin/gdriver-daemon");
        assert!(plist.contains("<key>Label</key>"));
        assert!(plist.contains("<string>com.gdriver.daemon</string>"));
        assert!(plist.contains("<key>RunAtLoad</key>"));
        assert!(plist.contains("<true/>"));
    }

    #[test]
    fn test_escape_applescript() {
        assert_eq!(escape_applescript(r#"hello"world"#), r#"hello\"world"#);
        assert_eq!(escape_applescript(r#"path\to\file"#), r#"path\\to\\file"#);
        assert_eq!(escape_applescript("normal text"), "normal text");
    }

    #[test]
    fn test_is_launch_on_login() {
        // Just ensure it returns a bool without panicking.
        let _ = is_launch_on_login_enabled();
    }
}
