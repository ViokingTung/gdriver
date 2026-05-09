// Account types — mirrors gdriver-ipc/src/types.rs

export interface Account {
  id: string;
  email: string;
  display_name: string | null;
  photo_url: string | null;
  locale: string | null;
  created_at: number;
  last_used_at: number;
}

export interface StorageQuota {
  /** Total storage limit in bytes; null = unlimited. */
  limit: number | null;
  /** Total usage in bytes. */
  usage: number;
  /** Usage by Drive files in bytes. */
  usage_in_drive: number;
  /** Usage by Drive trash in bytes. */
  usage_in_drive_trash: number;
}
