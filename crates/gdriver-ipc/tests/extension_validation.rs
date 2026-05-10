//! Platform compatibility tests: file-manager extension configuration
//! validation for all three target platforms.
//!
//! Validates that extension manifests, desktop entries, plist files,
//! and Makefiles are correctly structured for their respective platforms:
//! - Linux: Nautilus (GNOME) + Dolphin (KDE)
//! - macOS: FinderSync + FileProvider
//! - Windows: Explorer Shell Extension (COM DLL)

use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    // crates/gdriver-ipc/ → workspace root
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn extensions_dir() -> PathBuf {
    workspace_root().join("extensions")
}

// ── Nautilus (Linux: Ubuntu, Fedora, Arch) ───────────────────────────────

#[test]
fn nautilus_metadata_json_is_valid() {
    let meta_path = extensions_dir().join("nautilus").join("metadata.json");
    assert!(
        meta_path.exists(),
        "Nautilus metadata.json not found at {:?}",
        meta_path
    );

    let content = std::fs::read_to_string(&meta_path).unwrap();
    let meta: serde_json::Value = serde_json::from_str(&content).unwrap();

    assert_eq!(meta["name"], "gDriver");
    assert!(!meta["description"].as_str().unwrap().is_empty());
    assert!(!meta["version"].as_str().unwrap().is_empty());
    assert!(!meta["url"].as_str().unwrap().is_empty());

    // nautilus_versions must be a non-empty array
    let versions = meta["nautilus_versions"].as_array().unwrap();
    assert!(
        !versions.is_empty(),
        "must declare supported Nautilus versions"
    );
}

#[test]
fn nautilus_makefile_exists_and_has_install_target() {
    let makefile = extensions_dir().join("nautilus").join("Makefile");
    assert!(
        makefile.exists(),
        "Nautilus Makefile not found at {:?}",
        makefile
    );

    let content = std::fs::read_to_string(&makefile).unwrap();
    assert!(
        content.contains("install:"),
        "Makefile missing install target"
    );
    assert!(
        content.contains("uninstall:"),
        "Makefile missing uninstall target"
    );
    assert!(
        content.contains("gdriver_nautilus.py"),
        "Makefile should reference gdriver_nautilus.py"
    );
    assert!(
        content.contains("gdriver_ipc.py"),
        "Makefile should reference gdriver_ipc.py"
    );
}

#[test]
fn nautilus_extension_files_exist() {
    let dir = extensions_dir().join("nautilus");
    assert!(dir.join("gdriver_nautilus.py").exists());
    assert!(dir.join("gdriver_ipc.py").exists());
    assert!(dir.join("metadata.json").exists());
}

#[test]
fn nautilus_icon_files_exist() {
    let icon_dir = extensions_dir().join("nautilus").join("icons");
    assert!(icon_dir.exists(), "Nautilus icon directory not found");

    let icons = [
        "emblem-gdriver-cloud.svg",
        "emblem-gdriver-synced.svg",
        "emblem-gdriver-syncing.svg",
        "emblem-gdriver-error.svg",
    ];
    for icon in icons {
        let path = icon_dir.join(icon);
        assert!(path.exists(), "missing Nautilus icon: {:?}", path);
    }
}

// ── Dolphin (Linux: KDE) ─────────────────────────────────────────────────

#[test]
fn dolphin_metadata_json_is_valid() {
    let meta_path = extensions_dir().join("dolphin").join("metadata.json");
    assert!(
        meta_path.exists(),
        "Dolphin metadata.json not found at {:?}",
        meta_path
    );

    let content = std::fs::read_to_string(&meta_path).unwrap();
    let meta: serde_json::Value = serde_json::from_str(&content).unwrap();

    assert!(!meta["name"].as_str().unwrap().is_empty());
    assert!(!meta["description"].as_str().unwrap().is_empty());
    assert!(!meta["version"].as_str().unwrap().is_empty());
}

#[test]
fn dolphin_desktop_entry_format() {
    let desktop_path = extensions_dir()
        .join("dolphin")
        .join("gdriver-dolphin.desktop");
    assert!(
        desktop_path.exists(),
        "Dolphin desktop entry not found at {:?}",
        desktop_path
    );

    let content = std::fs::read_to_string(&desktop_path).unwrap();

    // Desktop entry required keys
    assert!(
        content.contains("[Desktop Entry]"),
        "missing [Desktop Entry] header"
    );
    assert!(
        content.contains("Type=Service"),
        "Type must be Service for Dolphin"
    );
    assert!(content.contains("ServiceTypes="), "missing ServiceTypes");
    assert!(content.contains("Actions="), "missing Actions list");

    // Each action must have a [Desktop Action ...] section
    let actions = [
        "AvailableOffline",
        "OnlineOnly",
        "CopyLink",
        "ViewInDrive",
        "Share",
    ];
    for action in actions {
        assert!(
            content.contains(&format!("[Desktop Action {}]", action)),
            "missing action section: {action}"
        );
        assert!(
            content.contains(&format!("Name=")),
            "each action must have a Name entry"
        );
    }

    // X-KDE metadata
    assert!(content.contains("X-KDE-Priority="));
    assert!(content.contains("X-KDE-Submenu="));
}

#[test]
fn dolphin_makefile_exists_and_has_install_target() {
    let makefile = extensions_dir().join("dolphin").join("Makefile");
    assert!(
        makefile.exists(),
        "Dolphin Makefile not found at {:?}",
        makefile
    );

    let content = std::fs::read_to_string(&makefile).unwrap();
    assert!(
        content.contains("install:"),
        "Makefile missing install target"
    );
    assert!(
        content.contains("uninstall:"),
        "Makefile missing uninstall target"
    );
    assert!(
        content.contains("gdriver_dolphin_ipc.py"),
        "Makefile should reference gdriver_dolphin_ipc.py"
    );
    assert!(
        content.contains("gdriver_dolphin_menu.py"),
        "Makefile should reference gdriver_dolphin_menu.py"
    );
    assert!(
        content.contains("SERVICE_MENU_DIR"),
        "Makefile should reference SERVICE_MENU_DIR"
    );
}

#[test]
fn dolphin_extension_files_exist() {
    let dir = extensions_dir().join("dolphin");
    assert!(dir.join("gdriver_dolphin_ipc.py").exists());
    assert!(dir.join("gdriver_dolphin_menu.py").exists());
    assert!(dir.join("metadata.json").exists());
    assert!(dir.join("gdriver-dolphin.desktop").exists());
}

#[test]
fn dolphin_icon_files_exist() {
    let icon_dir = extensions_dir().join("dolphin").join("icons");
    assert!(icon_dir.exists(), "Dolphin icon directory not found");

    let icons = [
        "emblem-gdriver-cloud.svg",
        "emblem-gdriver-synced.svg",
        "emblem-gdriver-syncing.svg",
        "emblem-gdriver-error.svg",
    ];
    for icon in icons {
        let path = icon_dir.join(icon);
        assert!(path.exists(), "missing Dolphin icon: {:?}", path);
    }
}

// ── FinderSync (macOS: Finder integration) ───────────────────────────────

#[test]
fn findersync_info_plist_exists_and_valid() {
    let plist_path = extensions_dir().join("findersync").join("Info.plist");
    assert!(
        plist_path.exists(),
        "FinderSync Info.plist not found at {:?}",
        plist_path
    );

    let content = std::fs::read_to_string(&plist_path).unwrap();

    // Basic plist structure
    assert!(
        content.contains("<?xml version=\"1.0\""),
        "plist must be XML"
    );
    assert!(
        content.contains("<!DOCTYPE plist"),
        "plist must have DOCTYPE"
    );
    assert!(content.contains("<plist version=\"1.0\">"));

    // Essential keys for Finder Sync extension
    assert!(content.contains("<key>CFBundleIdentifier</key>"));
    assert!(content.contains("<key>CFBundleDisplayName</key>"));
    assert!(content.contains("<key>CFBundlePackageType</key>"));
    assert!(
        content.contains("<string>XPC!</string>"),
        "Finder Sync extension must be XPC! type"
    );

    // Finder Sync specific
    assert!(
        content.contains("com.apple.FinderSync"),
        "must declare NSExtensionPointIdentifier as com.apple.FinderSync"
    );
    assert!(content.contains("<key>NSExtension</key>"));

    // App group for sharing data with main app and daemon
    assert!(content.contains("<key>NSExtensionFileProviderDocumentGroup</key>"));
}

#[test]
fn findersync_package_swift_exists() {
    let pkg = extensions_dir().join("findersync").join("Package.swift");
    assert!(
        pkg.exists(),
        "FinderSync Package.swift not found at {:?}",
        pkg
    );
}

// ── FileProvider (macOS: on-demand file fetching) ────────────────────────

#[test]
fn fileprovider_info_plist_exists() {
    let plist_path = extensions_dir().join("fileprovider").join("Info.plist");
    assert!(
        plist_path.exists(),
        "FileProvider Info.plist not found at {:?}",
        plist_path
    );

    let content = std::fs::read_to_string(&plist_path).unwrap();
    assert!(content.contains("<plist version=\"1.0\">"));
    assert!(content.contains("<key>CFBundleIdentifier</key>"));
    assert!(content.contains("<key>NSExtension</key>"));
}

#[test]
fn fileprovider_package_swift_exists() {
    let pkg = extensions_dir().join("fileprovider").join("Package.swift");
    assert!(
        pkg.exists(),
        "FileProvider Package.swift not found at {:?}",
        pkg
    );
}

// ── Cross-platform: both Linux extensions share icon names ───────────────

#[test]
fn nautilus_and_dolphin_share_same_icon_names() {
    // Both Linux file manager extensions use the same icon set for sync state
    // emblems — this test verifies they remain consistent.
    let nautilus_icons = extensions_dir().join("nautilus").join("icons");
    let dolphin_icons = extensions_dir().join("dolphin").join("icons");

    let nautilus_files: Vec<String> = std::fs::read_dir(&nautilus_icons)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();

    let dolphin_files: Vec<String> = std::fs::read_dir(&dolphin_icons)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();

    for name in &nautilus_files {
        assert!(
            dolphin_files.contains(name),
            "Dolphin extension missing icon: {name} (present in Nautilus)"
        );
    }
}

// ── macOS: FileProvider and FinderSync with same app group ───────────────

#[test]
fn macos_extensions_use_same_app_group() {
    // Both macOS extensions must use the same App Group identifier so they
    // can share data with the main app and daemon.
    let finder_plist = extensions_dir().join("findersync").join("Info.plist");
    let fp_plist = extensions_dir().join("fileprovider").join("Info.plist");

    let finder_content = std::fs::read_to_string(&finder_plist).unwrap();
    let fp_content = std::fs::read_to_string(&fp_plist).unwrap();

    // Both reference APP_GROUP_IDENTIFIER (set via Xcode build settings)
    assert!(
        finder_content.contains("APP_GROUP_IDENTIFIER"),
        "FinderSync must reference APP_GROUP_IDENTIFIER"
    );
    assert!(
        fp_content.contains("APP_GROUP_IDENTIFIER"),
        "FileProvider must reference APP_GROUP_IDENTIFIER"
    );
}

// ── Cross-platform: all extension directories exist ──────────────────────

#[test]
fn all_platform_extension_dirs_exist() {
    let dirs = ["nautilus", "dolphin", "findersync", "fileprovider"];
    for dir in dirs {
        let path = extensions_dir().join(dir);
        assert!(path.exists(), "extension directory missing: {:?}", path);
        assert!(path.is_dir(), "expected directory: {:?}", path);
    }
}

#[test]
fn windows_shell_extension_exists() {
    // Windows Shell Extension is a Rust COM DLL crate (not a workspace member).
    // It lives in its own directory with its own Cargo.toml.
    let win_shell = extensions_dir().join("windows-shell");
    assert!(
        win_shell.exists(),
        "Windows Shell Extension directory not found"
    );
    assert!(
        win_shell.join("Cargo.toml").exists(),
        "Windows Shell Extension Cargo.toml not found"
    );
    assert!(
        win_shell.join("src").join("lib.rs").exists(),
        "Windows Shell Extension lib.rs not found"
    );
}
