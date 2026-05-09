// Core
pub const PING: &str = "ping";

// Sync control
pub const SYNC_GET_STATUS: &str = "sync.get_status";
pub const SYNC_PAUSE: &str = "sync.pause";
pub const SYNC_RESUME: &str = "sync.resume";
pub const SYNC_GET_RECENT_ITEMS: &str = "sync.get_recent_items";
pub const SYNC_GET_ACTIVITY: &str = "sync.get_activity";
pub const SYNC_RETRY_ERROR: &str = "sync.retry_error";
pub const SYNC_GET_ERRORS: &str = "sync.get_errors";

// Folder management
pub const FOLDER_ADD: &str = "folder.add";
pub const FOLDER_REMOVE: &str = "folder.remove";
pub const FOLDER_LIST: &str = "folder.list";
pub const FOLDER_GET_SIZE: &str = "folder.get_size";
pub const FOLDER_GET_SUGGESTED: &str = "folder.get_suggested";

// Offline files
pub const OFFLINE_GET_STATS: &str = "offline.get_stats";
pub const OFFLINE_CLEAR_CACHE: &str = "offline.clear_cache";

// Authentication & accounts
pub const AUTH_START_FLOW: &str = "auth.start_flow";
pub const AUTH_GET_ACCOUNTS: &str = "auth.get_accounts";
pub const AUTH_DISCONNECT: &str = "auth.disconnect";
pub const AUTH_GET_LOCALE: &str = "auth.get_locale";
pub const AUTH_GET_QUOTA: &str = "auth.get_quota";

// Preferences
pub const PREFS_GET: &str = "prefs.get";
pub const PREFS_SAVE: &str = "prefs.save";

// System operations
pub const SYSTEM_OPEN_DRIVE_FOLDER: &str = "system.open_drive_folder";
pub const SYSTEM_OPEN_URL: &str = "system.open_url";
pub const SYSTEM_SUBMIT_FEEDBACK: &str = "system.submit_feedback";
pub const SYSTEM_GET_VERSION: &str = "system.get_version";
pub const SYSTEM_SET_SYNC_MODE: &str = "system.set_sync_mode";
pub const SYSTEM_GET_DRIVE_STATS: &str = "system.get_drive_stats";
pub const SYSTEM_REVEAL_IN_FILE_MANAGER: &str = "system.reveal_in_file_manager";
pub const SYSTEM_GET_PLATFORM: &str = "system.get_platform";
pub const SYSTEM_QUIT: &str = "system.quit";
pub const SYSTEM_SET_LAUNCH_ON_LOGIN: &str = "system.set_launch_on_login";

// Notifications
pub const NOTIFICATION_LIST: &str = "notification.list";
pub const NOTIFICATION_DISMISS: &str = "notification.dismiss";
pub const NOTIFICATION_MARK_READ: &str = "notification.mark_read";
pub const NOTIFICATION_MARK_ALL_READ: &str = "notification.mark_all_read";

// File system queries (used by file manager extensions — read-only)
pub const FS_GET_SYNC_STATE: &str = "fs.get_sync_state";
pub const FS_SET_OFFLINE: &str = "fs.set_offline";
pub const FS_GET_SHARE_LINK: &str = "fs.get_share_link";

// FileProvider methods (macOS FileProvider extension)
pub const FP_GET_ITEM: &str = "fp.get_item";
pub const FP_LIST_CHILDREN: &str = "fp.list_children";
pub const FP_FETCH_CONTENTS: &str = "fp.fetch_contents";
pub const FP_CREATE_ITEM: &str = "fp.create_item";
pub const FP_MODIFY_ITEM: &str = "fp.modify_item";
pub const FP_DELETE_ITEM: &str = "fp.delete_item";

// Push event method names (Daemon → client, sent as JSON-RPC notifications without id)
pub const EVENT_SYNC_STATUS_CHANGED: &str = "sync:status-changed";
pub const EVENT_SYNC_ITEM_UPDATED: &str = "sync:item-updated";
pub const EVENT_SYNC_ERROR: &str = "sync:error";
pub const EVENT_NOTIFICATION_NEW: &str = "notification:new";
pub const EVENT_ACCOUNT_CHANGED: &str = "account:changed";
pub const EVENT_ACCOUNT_QUOTA_UPDATED: &str = "account:quota-updated";
pub const EVENT_ONBOARDING_OAUTH_COMPLETE: &str = "onboarding:oauth-complete";
