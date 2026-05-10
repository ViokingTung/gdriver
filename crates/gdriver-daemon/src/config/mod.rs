use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

pub use gdriver_ipc::Preferences;
use tokio::sync::RwLock;
use tracing::{info, warn};

// ─── Config handle ────────────────────────────────────────────────────────────

/// Thread-safe, cloneable reference to the current in-memory preferences.
///
/// Wrap in `Arc` so handlers and subsystems can each hold a copy without
/// extra lifetime complexity.  Reads are non-exclusive; writes are exclusive
/// and always followed by a disk flush.
pub type ConfigHandle = Arc<RwLock<Preferences>>;

/// Create a new `ConfigHandle` from the given initial preferences.
pub fn new_handle(prefs: Preferences) -> ConfigHandle {
    Arc::new(RwLock::new(prefs))
}

// ─── Public file API ──────────────────────────────────────────────────────────

/// Load preferences from the platform-default config path.
///
/// Returns `Preferences::default()` when the file is absent or cannot be
/// parsed so that the daemon always boots successfully.
pub fn load() -> anyhow::Result<Preferences> {
    load_from(&config_path()?)
}

/// Atomically write `prefs` to the platform-default config path.
///
/// Writes to a sibling `.tmp` file first, then renames it over the target.
/// On all supported platforms `std::fs::rename` within the same directory is
/// atomic, so the config file is never left in a partially-written state.
pub fn save(prefs: &Preferences) -> anyhow::Result<()> {
    save_to(&config_path()?, prefs)
}

// ─── Path-parameterised helpers (also used in tests) ─────────────────────────

/// Load preferences from an explicit `path`.
pub fn load_from(path: &Path) -> anyhow::Result<Preferences> {
    if !path.exists() {
        info!(
            "config file not found at {}, using defaults",
            path.display()
        );
        return Ok(Preferences::default());
    }

    let content = std::fs::read_to_string(path)?;
    match toml::from_str::<Preferences>(&content) {
        Ok(prefs) => {
            info!("loaded config from {}", path.display());
            Ok(prefs)
        }
        Err(e) => {
            // Treat a corrupt / outdated config as if it were absent rather
            // than crashing the daemon.
            warn!(
                "config parse error at {} ({e}), using defaults",
                path.display()
            );
            Ok(Preferences::default())
        }
    }
}

/// Atomically write `prefs` to an explicit `path`.
pub fn save_to(path: &Path, prefs: &Preferences) -> anyhow::Result<()> {
    let content = toml::to_string_pretty(prefs)
        .map_err(|e| anyhow::anyhow!("failed to serialise preferences: {e}"))?;

    // Write to a sibling temp file, then rename so the destination is never
    // left in an incomplete state.  The temp file has the same parent
    // directory so that the rename is guaranteed to be within one filesystem.
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, content.as_bytes())?;
    std::fs::rename(&tmp, path)?;

    info!("saved config to {}", path.display());
    Ok(())
}

// ─── Platform path ────────────────────────────────────────────────────────────

/// Platform-specific path to `preferences.toml` (same directory as the DB).
///
/// | Platform | Path                                                           |
/// |----------|----------------------------------------------------------------|
/// | Linux    | `~/.local/share/gdriver/preferences.toml`                      |
/// | macOS    | `~/Library/Application Support/gdriver/preferences.toml`       |
/// | Windows  | `%APPDATA%\gdriver\preferences.toml`                           |
pub fn config_path() -> anyhow::Result<PathBuf> {
    let dir = dirs::data_dir()
        .ok_or_else(|| anyhow::anyhow!("cannot determine data directory"))?
        .join("gdriver");

    std::fs::create_dir_all(&dir)?;
    Ok(dir.join("preferences.toml"))
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use gdriver_ipc::Appearance;

    use super::*;

    // Each test gets its own uniquely-named temp file to avoid conflicts when
    // tests run in parallel.
    fn tmp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("gdriver_cfg_test_{name}.toml"))
    }

    fn cleanup(path: &Path) {
        let _ = std::fs::remove_file(path);
        let tmp = path.with_extension("toml.tmp");
        let _ = std::fs::remove_file(tmp);
    }

    // ── Serialisation round-trips ────────────────────────────────────────────

    #[test]
    fn toml_roundtrip_default_prefs() {
        let prefs = Preferences::default();
        let toml_str = toml::to_string_pretty(&prefs).unwrap();

        // Must be valid TOML with all expected sections
        assert!(toml_str.contains("[general]"));
        assert!(toml_str.contains("[network]"));
        assert!(toml_str.contains("[hotkeys]"));
        assert!(toml_str.contains("[telemetry]"));
        assert!(toml_str.contains("[vfs]"));

        let parsed: Preferences = toml::from_str(&toml_str).unwrap();
        assert_eq!(
            parsed.general.launch_on_login,
            prefs.general.launch_on_login
        );
        assert_eq!(parsed.general.language, prefs.general.language);
        assert_eq!(parsed.network.proxy, prefs.network.proxy);
        assert_eq!(
            parsed.network.download_rate_limit,
            prefs.network.download_rate_limit
        );
        assert_eq!(parsed.hotkeys.search_key, prefs.hotkeys.search_key);
        assert_eq!(
            parsed.telemetry.auto_send_diagnostics,
            prefs.telemetry.auto_send_diagnostics
        );
    }

    #[test]
    fn appearance_serialises_to_snake_case() {
        let prefs = Preferences::default(); // appearance = FollowSystem
        let toml_str = toml::to_string_pretty(&prefs).unwrap();
        assert!(toml_str.contains("follow_system"), "got:\n{toml_str}");
    }

    // ── File I/O ─────────────────────────────────────────────────────────────

    #[test]
    fn save_creates_readable_toml_file() {
        let path = tmp("save_creates");
        cleanup(&path);

        save_to(&path, &Preferences::default()).unwrap();

        assert!(path.exists());
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("[general]"));

        cleanup(&path);
    }

    #[test]
    fn save_and_load_roundtrip() {
        let path = tmp("roundtrip");
        cleanup(&path);

        let prefs = Preferences::default();
        save_to(&path, &prefs).unwrap();
        let loaded = load_from(&path).unwrap();

        assert_eq!(
            loaded.general.launch_on_login,
            prefs.general.launch_on_login
        );
        assert_eq!(loaded.general.appearance, prefs.general.appearance);
        assert_eq!(loaded.network.proxy, prefs.network.proxy);
        assert_eq!(loaded.hotkeys.search_enabled, prefs.hotkeys.search_enabled);

        cleanup(&path);
    }

    #[test]
    fn modified_prefs_persist_across_save_load() {
        let path = tmp("modified");
        cleanup(&path);

        let mut prefs = Preferences::default();
        prefs.general.launch_on_login = false;
        prefs.general.appearance = Appearance::Dark;
        prefs.general.language = "zh-CN".into();
        prefs.network.upload_rate_limit = 512;
        prefs.hotkeys.search_enabled = false;
        prefs.telemetry.auto_send_diagnostics = false;

        save_to(&path, &prefs).unwrap();
        let loaded = load_from(&path).unwrap();

        assert_eq!(loaded.general.launch_on_login, false);
        assert_eq!(loaded.general.appearance, Appearance::Dark);
        assert_eq!(loaded.general.language, "zh-CN");
        assert_eq!(loaded.network.upload_rate_limit, 512);
        assert_eq!(loaded.hotkeys.search_enabled, false);
        assert_eq!(loaded.telemetry.auto_send_diagnostics, false);

        cleanup(&path);
    }

    #[test]
    fn load_missing_file_returns_default() {
        // Use a path that definitely doesn't exist
        let path = tmp("nonexistent_xyz_abc_123");
        cleanup(&path); // ensure it's gone

        let loaded = load_from(&path).unwrap();
        let default = Preferences::default();

        assert_eq!(
            loaded.general.launch_on_login,
            default.general.launch_on_login
        );
        assert_eq!(loaded.network.proxy, default.network.proxy);
    }

    #[test]
    fn load_invalid_toml_falls_back_to_default() {
        let path = tmp("invalid_toml");
        cleanup(&path);

        std::fs::write(&path, b"this is [ not valid toml ===").unwrap();

        // Should not error — returns default instead
        let loaded = load_from(&path).unwrap();
        let default = Preferences::default();
        assert_eq!(
            loaded.general.launch_on_login,
            default.general.launch_on_login
        );

        cleanup(&path);
    }

    #[test]
    fn atomic_write_leaves_no_tmp_file() {
        let path = tmp("atomic");
        cleanup(&path);

        save_to(&path, &Preferences::default()).unwrap();

        // The .tmp file must be gone after a successful save
        let tmp_path = path.with_extension("toml.tmp");
        assert!(!tmp_path.exists(), "tmp file was not cleaned up");
        assert!(path.exists(), "config file was not created");

        cleanup(&path);
    }
}
