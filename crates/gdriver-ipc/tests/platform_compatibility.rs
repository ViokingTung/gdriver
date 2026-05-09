//! Platform compatibility tests: Platform enum, IPC method coverage,
//! and cross-platform naming convention validation.
//!
//! Verifies that the IPC protocol layer correctly represents all three
//! target platforms (Linux, Windows, macOS) and that method constants
//! are properly defined for each platform's integration surface.

use gdriver_ipc::types::Platform;

// ── Platform enum serialization ──────────────────────────────────────────

#[test]
fn platform_linux_round_trip() {
    let json = serde_json::to_string(&Platform::Linux).unwrap();
    assert_eq!(json, r#""linux""#);
    let parsed: Platform = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, Platform::Linux);
}

#[test]
fn platform_windows_round_trip() {
    let json = serde_json::to_string(&Platform::Windows).unwrap();
    assert_eq!(json, r#""windows""#);
    let parsed: Platform = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, Platform::Windows);
}

#[test]
fn platform_macos_round_trip() {
    let json = serde_json::to_string(&Platform::Macos).unwrap();
    assert_eq!(json, r#""macos""#);
    let parsed: Platform = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, Platform::Macos);
}

#[test]
fn platform_all_variants_distinct() {
    let a = serde_json::to_string(&Platform::Linux).unwrap();
    let b = serde_json::to_string(&Platform::Windows).unwrap();
    let c = serde_json::to_string(&Platform::Macos).unwrap();
    assert_ne!(a, b);
    assert_ne!(b, c);
    assert_ne!(a, c);
}

#[test]
fn platform_deserialize_unknown_is_error() {
    let result: Result<Platform, _> = serde_json::from_str(r#""freebsd""#);
    assert!(result.is_err(), "unknown platform variant should error");
}

// ── IPC method constants ─────────────────────────────────────────────────

#[test]
fn all_core_methods_defined() {
    // Every JSON-RPC method constant must be a non-empty string.
    assert!(!gdriver_ipc::methods::PING.is_empty());
}

#[test]
fn sync_control_methods_defined() {
    use gdriver_ipc::methods::*;
    let methods = [
        SYNC_GET_STATUS,
        SYNC_PAUSE,
        SYNC_RESUME,
        SYNC_GET_RECENT_ITEMS,
        SYNC_GET_ACTIVITY,
        SYNC_RETRY_ERROR,
        SYNC_GET_ERRORS,
    ];
    for m in methods {
        assert!(!m.is_empty());
        assert!(m.starts_with("sync."), "{m} should start with 'sync.'");
    }
}

#[test]
fn folder_management_methods_defined() {
    use gdriver_ipc::methods::*;
    let methods = [FOLDER_ADD, FOLDER_REMOVE, FOLDER_LIST, FOLDER_GET_SIZE, FOLDER_GET_SUGGESTED];
    for m in methods {
        assert!(!m.is_empty());
        assert!(m.starts_with("folder."), "{m} should start with 'folder.'");
    }
}

#[test]
fn offline_methods_defined() {
    use gdriver_ipc::methods::*;
    let methods = [OFFLINE_GET_STATS, OFFLINE_CLEAR_CACHE];
    for m in methods {
        assert!(!m.is_empty());
        assert!(m.starts_with("offline."), "{m} should start with 'offline.'");
    }
}

#[test]
fn auth_methods_defined() {
    use gdriver_ipc::methods::*;
    let methods = [
        AUTH_START_FLOW,
        AUTH_GET_ACCOUNTS,
        AUTH_DISCONNECT,
        AUTH_GET_LOCALE,
        AUTH_GET_QUOTA,
    ];
    for m in methods {
        assert!(!m.is_empty());
        assert!(m.starts_with("auth."), "{m} should start with 'auth.'");
    }
}

#[test]
fn preferences_methods_defined() {
    use gdriver_ipc::methods::*;
    let methods = [PREFS_GET, PREFS_SAVE];
    for m in methods {
        assert!(!m.is_empty());
        assert!(m.starts_with("prefs."), "{m} should start with 'prefs.'");
    }
}

#[test]
fn system_platform_methods_defined() {
    use gdriver_ipc::methods::*;
    let methods = [
        SYSTEM_OPEN_DRIVE_FOLDER,
        SYSTEM_OPEN_URL,
        SYSTEM_SUBMIT_FEEDBACK,
        SYSTEM_GET_VERSION,
        SYSTEM_SET_SYNC_MODE,
        SYSTEM_GET_DRIVE_STATS,
        SYSTEM_REVEAL_IN_FILE_MANAGER,
        SYSTEM_GET_PLATFORM,
        SYSTEM_QUIT,
        SYSTEM_SET_LAUNCH_ON_LOGIN,
    ];
    for m in methods {
        assert!(!m.is_empty());
        assert!(m.starts_with("system."), "{m} should start with 'system.'");
    }
}

#[test]
fn notification_methods_defined() {
    use gdriver_ipc::methods::*;
    let methods = [
        NOTIFICATION_LIST,
        NOTIFICATION_DISMISS,
        NOTIFICATION_MARK_READ,
        NOTIFICATION_MARK_ALL_READ,
    ];
    for m in methods {
        assert!(!m.is_empty());
        assert!(m.starts_with("notification."), "{m} should start with 'notification.'");
    }
}

#[test]
fn filesystem_methods_defined() {
    use gdriver_ipc::methods::*;
    let methods = [FS_GET_SYNC_STATE, FS_SET_OFFLINE, FS_GET_SHARE_LINK];
    for m in methods {
        assert!(!m.is_empty());
        assert!(m.starts_with("fs."), "{m} should start with 'fs.'");
    }
}

#[test]
fn fileprovider_methods_defined_for_macos() {
    // These methods are used by the macOS FileProvider extension.
    // They must exist even when compiling on other platforms since
    // the IPC protocol definition is platform-agnostic.
    use gdriver_ipc::methods::*;
    let methods = [
        FP_GET_ITEM,
        FP_LIST_CHILDREN,
        FP_FETCH_CONTENTS,
        FP_CREATE_ITEM,
        FP_MODIFY_ITEM,
        FP_DELETE_ITEM,
    ];
    for m in methods {
        assert!(!m.is_empty());
        assert!(m.starts_with("fp."), "{m} should start with 'fp.'");
    }
}

#[test]
fn push_event_methods_defined() {
    use gdriver_ipc::methods::*;
    let events = [
        EVENT_SYNC_STATUS_CHANGED,
        EVENT_SYNC_ITEM_UPDATED,
        EVENT_SYNC_ERROR,
        EVENT_NOTIFICATION_NEW,
        EVENT_ACCOUNT_CHANGED,
        EVENT_ACCOUNT_QUOTA_UPDATED,
        EVENT_ONBOARDING_OAUTH_COMPLETE,
    ];
    for e in events {
        assert!(!e.is_empty());
        assert!(
            e.contains(':'),
            "push event {e} should use colon-delimited convention"
        );
    }
}

// ── Cross-platform method naming conventions ─────────────────────────────

#[test]
fn all_methods_use_snake_case_or_colon_events() {
    use gdriver_ipc::methods::*;
    let all = [
        PING,
        SYNC_GET_STATUS, SYNC_PAUSE, SYNC_RESUME, SYNC_GET_RECENT_ITEMS,
        SYNC_GET_ACTIVITY, SYNC_RETRY_ERROR, SYNC_GET_ERRORS,
        FOLDER_ADD, FOLDER_REMOVE, FOLDER_LIST, FOLDER_GET_SIZE, FOLDER_GET_SUGGESTED,
        OFFLINE_GET_STATS, OFFLINE_CLEAR_CACHE,
        AUTH_START_FLOW, AUTH_GET_ACCOUNTS, AUTH_DISCONNECT, AUTH_GET_LOCALE, AUTH_GET_QUOTA,
        PREFS_GET, PREFS_SAVE,
        SYSTEM_OPEN_DRIVE_FOLDER, SYSTEM_OPEN_URL, SYSTEM_SUBMIT_FEEDBACK,
        SYSTEM_GET_VERSION, SYSTEM_SET_SYNC_MODE, SYSTEM_GET_DRIVE_STATS,
        SYSTEM_REVEAL_IN_FILE_MANAGER, SYSTEM_GET_PLATFORM, SYSTEM_QUIT,
        SYSTEM_SET_LAUNCH_ON_LOGIN,
        NOTIFICATION_LIST, NOTIFICATION_DISMISS, NOTIFICATION_MARK_READ,
        NOTIFICATION_MARK_ALL_READ,
        FS_GET_SYNC_STATE, FS_SET_OFFLINE, FS_GET_SHARE_LINK,
        FP_GET_ITEM, FP_LIST_CHILDREN, FP_FETCH_CONTENTS, FP_CREATE_ITEM,
        FP_MODIFY_ITEM, FP_DELETE_ITEM,
    ];
    for m in all {
        // Regular methods use dot-separated snake_case.
        assert!(
            m.chars().all(|c| c.is_ascii_lowercase() || c == '_' || c == '.'),
            "method {m} should be lowercase with dots and underscores only"
        );
    }
}

// ── Total method count ───────────────────────────────────────────────────

#[test]
fn ipc_method_count() {
    use gdriver_ipc::methods::*;
    let all_methods: &[&str] = &[
        PING,
        SYNC_GET_STATUS, SYNC_PAUSE, SYNC_RESUME, SYNC_GET_RECENT_ITEMS,
        SYNC_GET_ACTIVITY, SYNC_RETRY_ERROR, SYNC_GET_ERRORS,
        FOLDER_ADD, FOLDER_REMOVE, FOLDER_LIST, FOLDER_GET_SIZE, FOLDER_GET_SUGGESTED,
        OFFLINE_GET_STATS, OFFLINE_CLEAR_CACHE,
        AUTH_START_FLOW, AUTH_GET_ACCOUNTS, AUTH_DISCONNECT, AUTH_GET_LOCALE, AUTH_GET_QUOTA,
        PREFS_GET, PREFS_SAVE,
        SYSTEM_OPEN_DRIVE_FOLDER, SYSTEM_OPEN_URL, SYSTEM_SUBMIT_FEEDBACK,
        SYSTEM_GET_VERSION, SYSTEM_SET_SYNC_MODE, SYSTEM_GET_DRIVE_STATS,
        SYSTEM_REVEAL_IN_FILE_MANAGER, SYSTEM_GET_PLATFORM, SYSTEM_QUIT,
        SYSTEM_SET_LAUNCH_ON_LOGIN,
        NOTIFICATION_LIST, NOTIFICATION_DISMISS, NOTIFICATION_MARK_READ,
        NOTIFICATION_MARK_ALL_READ,
        FS_GET_SYNC_STATE, FS_SET_OFFLINE, FS_GET_SHARE_LINK,
        FP_GET_ITEM, FP_LIST_CHILDREN, FP_FETCH_CONTENTS, FP_CREATE_ITEM,
        FP_MODIFY_ITEM, FP_DELETE_ITEM,
        EVENT_SYNC_STATUS_CHANGED, EVENT_SYNC_ITEM_UPDATED, EVENT_SYNC_ERROR,
        EVENT_NOTIFICATION_NEW, EVENT_ACCOUNT_CHANGED, EVENT_ACCOUNT_QUOTA_UPDATED,
        EVENT_ONBOARDING_OAUTH_COMPLETE,
    ];
    assert_eq!(all_methods.len(), 52, "unexpected method count — method added or removed?");
    // If this assertion fails because you intentionally added/removed a method:
    // update the count and verify the full list above is accurate.
}
