// Sync status types — mirrors gdriver-ipc/src/types.rs

export type SyncStatusValue = "up-to-date" | "syncing" | "paused" | "error" | "offline";

export interface SyncStatusPayload {
  status: SyncStatusValue;
  /** Unix milliseconds when the status changed. */
  ts: number;
  /** Current transfer speed in bytes/s; absent when not transferring. */
  speed?: number;
  /** Number of tasks still queued. */
  pending?: number;
}

export type SyncStateValue =
  | "cloud_only"
  | "downloading"
  | "uploading"
  | "cached"
  | "offline"
  | "modified"
  | "synced"
  | "error";

export interface SyncItem {
  file_id?: string;
  name: string;
  mime_type?: string;
  local_path?: string;
  sync_state: SyncStateValue;
  /** Upload/download progress [0.0, 1.0]; present only while transferring. */
  progress?: number;
  file_size?: number;
  error_msg?: string;
  drive_url?: string;
  /** Unix milliseconds of the last state change. */
  updated_at: number;
}

export interface SyncError {
  id: number;
  account_id?: string;
  file_id?: string;
  file_name?: string;
  error_code: string;
  error_msg: string;
  is_resolved: boolean;
  created_at: number;
}

export interface SyncActivityPage {
  items: SyncItem[];
  page: number;
  has_more: boolean;
}

// ─── Notification types ──────────────────────────────────────────────────────

export type NotificationKind = "conflict" | "storage_warning" | "sync_error";

export interface NotificationConflict {
  type: "conflict";
  id: number;
  account_id?: string;
  is_read: boolean;
  created_at: number;
  file_id?: string;
  file_name: string;
  conflict_copy_name: string;
}

export interface NotificationStorageWarning {
  type: "storage_warning";
  id: number;
  account_id?: string;
  is_read: boolean;
  created_at: number;
  usage_percent: number;
}

export interface NotificationSyncError {
  type: "sync_error";
  id: number;
  account_id?: string;
  is_read: boolean;
  created_at: number;
  error_id: number;
  file_name?: string;
  error_msg: string;
}

export type Notification =
  | NotificationConflict
  | NotificationStorageWarning
  | NotificationSyncError;
