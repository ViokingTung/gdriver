use serde::{Deserialize, Serialize};
use serde_json::Value;

// ─── JSON-RPC 2.0 Protocol Types ────────────────────────────────────────────

/// JSON-RPC request id: string, integer, or absent (None = notification).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonRpcId {
    Str(String),
    Num(i64),
}

/// JSON-RPC 2.0 request (also used for push notifications when `id` is None).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<JsonRpcId>,
}

impl JsonRpcRequest {
    pub fn new(method: impl Into<String>, params: Option<Value>, id: JsonRpcId) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            method: method.into(),
            params,
            id: Some(id),
        }
    }

    /// Create a notification (no id — daemon push events use this form).
    pub fn notification(method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            method: method.into(),
            params,
            id: None,
        }
    }

    /// Returns true when this message is a notification (has no id).
    pub fn is_notification(&self) -> bool {
        self.id.is_none()
    }
}

/// JSON-RPC 2.0 error object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl JsonRpcError {
    pub fn parse_error() -> Self {
        Self {
            code: -32700,
            message: "Parse error".into(),
            data: None,
        }
    }
    pub fn invalid_request() -> Self {
        Self {
            code: -32600,
            message: "Invalid Request".into(),
            data: None,
        }
    }
    pub fn method_not_found(method: &str) -> Self {
        Self {
            code: -32601,
            message: "Method not found".into(),
            data: Some(Value::String(method.to_string())),
        }
    }
    pub fn invalid_params(detail: &str) -> Self {
        Self {
            code: -32602,
            message: "Invalid params".into(),
            data: Some(Value::String(detail.to_string())),
        }
    }
    pub fn internal_error(detail: impl Into<String>) -> Self {
        Self {
            code: -32603,
            message: detail.into(),
            data: None,
        }
    }
    /// Application-level: daemon is busy (e.g. shutting down).
    pub fn daemon_busy() -> Self {
        Self {
            code: -32000,
            message: "Daemon busy".into(),
            data: None,
        }
    }
    /// Application-level: no authenticated account.
    pub fn auth_required() -> Self {
        Self {
            code: -32001,
            message: "Authentication required".into(),
            data: None,
        }
    }
    /// Application-level: requested resource not found.
    pub fn not_found(detail: &str) -> Self {
        Self {
            code: -32002,
            message: "Not found".into(),
            data: Some(Value::String(detail.to_string())),
        }
    }
}

/// JSON-RPC 2.0 response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
    /// Mirrors the request id; null when the request id could not be parsed.
    pub id: Option<JsonRpcId>,
}

impl JsonRpcResponse {
    pub fn success(id: Option<JsonRpcId>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            result: Some(result),
            error: None,
            id,
        }
    }

    pub fn error(id: Option<JsonRpcId>, error: JsonRpcError) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            result: None,
            error: Some(error),
            id,
        }
    }

    pub fn is_success(&self) -> bool {
        self.error.is_none()
    }
}

// ─── Domain Enumerations ─────────────────────────────────────────────────────

/// Overall sync engine status (matches syncStore.status in the frontend).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SyncStatus {
    UpToDate,
    Syncing,
    Paused,
    Error,
    Offline,
}

/// Per-file sync state (stored in drive_files.sync_state).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncState {
    /// File exists only in the cloud; not cached locally (Stream mode default).
    CloudOnly,
    /// File is currently being downloaded.
    Downloading,
    /// File is currently being uploaded.
    Uploading,
    /// File is cached locally; can be evicted.
    Cached,
    /// File is pinned for offline access.
    Offline,
    /// Local file has unsaved changes awaiting upload.
    Modified,
    /// File is fully mirrored to disk (Mirror mode).
    Synced,
    /// Last sync attempt failed.
    Error,
}

/// Whether the user chose Stream (on-demand download) or Mirror (full local copy).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncMode {
    Stream,
    Mirror,
}

impl Default for SyncMode {
    fn default() -> Self {
        Self::Stream
    }
}

/// Folder synchronisation type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FolderType {
    Drive,
    Photos,
}

/// Google Photos upload quality.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhotosQuality {
    OriginalQuality,
    StorageSaver,
}

/// UI colour scheme preference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Appearance {
    Light,
    Dark,
    FollowSystem,
}

/// Host OS identifier returned by `system.get_platform`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    Linux,
    Windows,
    Macos,
}

// ─── Account & Quota ─────────────────────────────────────────────────────────

/// A signed-in Google account.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub photo_url: Option<String>,
    /// BCP-47 locale from the Google account (e.g. "en", "zh-CN").
    pub locale: Option<String>,
    pub created_at: i64,
    pub last_used_at: i64,
}

/// Drive storage quota (from `drive/v3/about`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageQuota {
    /// Total available bytes; None when quota is unlimited.
    pub limit: Option<u64>,
    /// Total bytes used across all Google services.
    pub usage: u64,
    pub usage_in_drive: u64,
    pub usage_in_drive_trash: u64,
}

// ─── Sync Items & Errors ──────────────────────────────────────────────────────

/// A single file's current sync status, shown in the activity list and home page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncItem {
    pub file_id: Option<String>,
    pub name: String,
    pub mime_type: Option<String>,
    pub local_path: Option<String>,
    pub sync_state: SyncState,
    /// Upload/download progress in [0.0, 1.0]; present only while transferring.
    pub progress: Option<f32>,
    pub file_size: Option<u64>,
    pub error_msg: Option<String>,
    /// Web URL for the file in Google Drive.
    pub drive_url: Option<String>,
    /// Unix milliseconds of the last state change.
    pub updated_at: i64,
}

/// A recorded sync failure (row in sync_errors table).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncError {
    pub id: i64,
    pub account_id: Option<String>,
    pub file_id: Option<String>,
    pub file_name: Option<String>,
    pub error_code: String,
    pub error_msg: String,
    pub is_resolved: bool,
    pub created_at: i64,
}

/// A page of sync activity results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncActivityPage {
    pub items: Vec<SyncItem>,
    pub page: u32,
    pub has_more: bool,
}

// ─── Folders ─────────────────────────────────────────────────────────────────

/// A configured sync/backup folder returned by `folder.list`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncFolder {
    pub id: i64,
    pub account_id: String,
    pub local_path: String,
    pub folder_type: FolderType,
    pub is_enabled: bool,
    /// Optional: total size in bytes (computed on request).
    pub size_bytes: Option<u64>,
    /// Only relevant for Photos folders.
    pub photos_quality: Option<PhotosQuality>,
}

/// Lightweight info returned after successfully adding a folder.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FolderInfo {
    pub id: i64,
    pub local_path: String,
    pub folder_type: FolderType,
}

/// Aggregated stats about offline-pinned and cached files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OfflineStats {
    /// Bytes used by files explicitly pinned offline.
    pub offline_bytes: u64,
    /// Bytes used by automatically cached (but not pinned) files.
    pub cache_bytes: u64,
}

/// High-level Drive file/folder counts returned by `system.get_drive_stats`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriveStats {
    pub file_count: u64,
    pub folder_count: u64,
}

// ─── Notifications ────────────────────────────────────────────────────────────

/// Type-safe notification payload variants.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NotificationKind {
    /// Two versions of the same file exist after a conflict was detected.
    Conflict {
        file_id: Option<String>,
        file_name: String,
        conflict_copy_name: String,
    },
    /// Drive storage is running low.
    StorageWarning {
        /// Percentage of quota used, 0–100.
        usage_percent: f32,
    },
    /// A file could not be synced after all retries.
    SyncError {
        error_id: i64,
        file_name: Option<String>,
        error_msg: String,
    },
}

/// A user-visible notification stored in the app.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub id: i64,
    pub account_id: Option<String>,
    pub is_read: bool,
    /// Unix milliseconds.
    pub created_at: i64,
    #[serde(flatten)]
    pub kind: NotificationKind,
}

// ─── Preferences ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralPrefs {
    pub launch_on_login: bool,
    pub appearance: Appearance,
    /// "follow_account" or a BCP-47 locale code.
    pub language: String,
    pub prompt_backup_devices: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkPrefs {
    /// "auto" | "direct"
    pub proxy: String,
    /// Download rate limit in KB/s; 0 = unlimited.
    pub download_rate_limit: u32,
    /// Upload rate limit in KB/s; 0 = unlimited.
    pub upload_rate_limit: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotkeyPrefs {
    pub search_enabled: bool,
    /// Platform-formatted key combo string, e.g. "Ctrl+Alt+G".
    pub search_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryPrefs {
    pub auto_send_diagnostics: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VfsPrefs {
    /// Mount point path for the virtual filesystem.
    /// Default: `~/GoogleDrive` (Linux/macOS) or `G:\` (Windows).
    pub mount_point: String,
    /// Sync mode: `stream` (on-demand download) or `mirror` (full local copy).
    #[serde(default)]
    pub sync_mode: SyncMode,
}

/// Full application preferences (mirrors preferences.toml structure).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preferences {
    pub general: GeneralPrefs,
    pub network: NetworkPrefs,
    pub hotkeys: HotkeyPrefs,
    pub telemetry: TelemetryPrefs,
    pub vfs: VfsPrefs,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            general: GeneralPrefs {
                launch_on_login: true,
                appearance: Appearance::FollowSystem,
                language: "follow_account".into(),
                prompt_backup_devices: true,
            },
            network: NetworkPrefs {
                proxy: "auto".into(),
                download_rate_limit: 0,
                upload_rate_limit: 0,
            },
            hotkeys: HotkeyPrefs {
                search_enabled: true,
                search_key: "Ctrl+Alt+G".into(),
            },
            telemetry: TelemetryPrefs {
                auto_send_diagnostics: true,
            },
            vfs: VfsPrefs {
                mount_point: "~/GoogleDrive".into(),
                sync_mode: SyncMode::default(),
            },
        }
    }
}

// ─── Misc Payloads ────────────────────────────────────────────────────────────

/// Payload for `system.submit_feedback`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedbackPayload {
    pub text: String,
    pub include_logs: bool,
    pub allow_email: bool,
}

// ─── Push Event Payloads (Daemon → Client) ───────────────────────────────────

/// Payload for `sync:status-changed`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncStatusPayload {
    pub status: SyncStatus,
    /// Unix milliseconds when the status changed.
    pub ts: i64,
    /// Current transfer speed in bytes/s; absent when not transferring.
    pub speed: Option<u64>,
    /// Number of tasks still queued.
    pub pending: Option<u32>,
}

/// Payload for `account:changed`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountChangedPayload {
    pub accounts: Vec<Account>,
}

/// Payload for `account:quota-updated`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountQuotaPayload {
    pub account_id: String,
    pub quota: StorageQuota,
}

/// Payload for `onboarding:oauth-complete`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OauthCompletePayload {
    pub account_id: String,
}

/// Typed push events dispatched by the daemon without a request id.
///
/// Parse from an incoming `JsonRpcRequest` whose `id` is `None` via
/// [`PushEvent::from_notification`].
#[derive(Debug, Clone)]
pub enum PushEvent {
    SyncStatusChanged(SyncStatusPayload),
    SyncItemUpdated(SyncItem),
    SyncError(SyncError),
    NotificationNew(Notification),
    AccountChanged(AccountChangedPayload),
    AccountQuotaUpdated(AccountQuotaPayload),
    OauthComplete(OauthCompletePayload),
    /// Unknown event forwarded as-is for forward compatibility.
    Unknown {
        method: String,
        params: Option<Value>,
    },
}

impl PushEvent {
    /// Deserialise a push event from an inbound JSON-RPC notification.
    pub fn from_notification(method: &str, params: Option<Value>) -> anyhow::Result<Self> {
        use crate::methods::*;

        fn decode<T: for<'de> Deserialize<'de>>(params: Option<Value>) -> anyhow::Result<T> {
            serde_json::from_value(params.unwrap_or(Value::Null)).map_err(anyhow::Error::from)
        }

        match method {
            EVENT_SYNC_STATUS_CHANGED => Ok(Self::SyncStatusChanged(decode(params)?)),
            EVENT_SYNC_ITEM_UPDATED => Ok(Self::SyncItemUpdated(decode(params)?)),
            EVENT_SYNC_ERROR => Ok(Self::SyncError(decode(params)?)),
            EVENT_NOTIFICATION_NEW => Ok(Self::NotificationNew(decode(params)?)),
            EVENT_ACCOUNT_CHANGED => Ok(Self::AccountChanged(decode(params)?)),
            EVENT_ACCOUNT_QUOTA_UPDATED => Ok(Self::AccountQuotaUpdated(decode(params)?)),
            EVENT_ONBOARDING_OAUTH_COMPLETE => Ok(Self::OauthComplete(decode(params)?)),
            other => Ok(Self::Unknown {
                method: other.to_string(),
                params,
            }),
        }
    }

    /// Serialise this push event as a JSON-RPC notification ready to be sent.
    pub fn to_notification(&self) -> anyhow::Result<JsonRpcRequest> {
        use crate::methods::*;

        fn encode<T: Serialize>(method: &str, payload: &T) -> anyhow::Result<JsonRpcRequest> {
            let params = serde_json::to_value(payload)?;
            Ok(JsonRpcRequest::notification(method, Some(params)))
        }

        match self {
            Self::SyncStatusChanged(p) => encode(EVENT_SYNC_STATUS_CHANGED, p),
            Self::SyncItemUpdated(p) => encode(EVENT_SYNC_ITEM_UPDATED, p),
            Self::SyncError(p) => encode(EVENT_SYNC_ERROR, p),
            Self::NotificationNew(p) => encode(EVENT_NOTIFICATION_NEW, p),
            Self::AccountChanged(p) => encode(EVENT_ACCOUNT_CHANGED, p),
            Self::AccountQuotaUpdated(p) => encode(EVENT_ACCOUNT_QUOTA_UPDATED, p),
            Self::OauthComplete(p) => encode(EVENT_ONBOARDING_OAUTH_COMPLETE, p),
            Self::Unknown { method, params } => {
                Ok(JsonRpcRequest::notification(method.clone(), params.clone()))
            }
        }
    }
}
