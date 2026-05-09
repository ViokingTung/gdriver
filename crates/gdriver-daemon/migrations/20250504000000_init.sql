-- Accounts table
CREATE TABLE accounts (
    id              TEXT PRIMARY KEY,   -- Google account ID
    email           TEXT NOT NULL UNIQUE,
    display_name    TEXT,
    photo_url       TEXT,
    locale          TEXT,               -- Account language (zh-CN / en / ja etc.)
    created_at      INTEGER NOT NULL,
    last_used_at    INTEGER NOT NULL
);

-- Drive file metadata
CREATE TABLE drive_files (
    id              TEXT NOT NULL,      -- Google Drive file ID
    account_id      TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    mime_type       TEXT NOT NULL,
    parent_id       TEXT,               -- NULL = My Drive root
    size            INTEGER,
    etag            TEXT,               -- Conflict detection
    version         INTEGER,
    modified_time   INTEGER,            -- Unix ms
    is_trashed      INTEGER DEFAULT 0,
    is_shared       INTEGER DEFAULT 0,
    local_path      TEXT,               -- Local cache path (Stream) or Mirror path
    sync_state      TEXT NOT NULL DEFAULT 'cloud_only',
    local_mtime     INTEGER,            -- Local last modified time (conflict detection)
    PRIMARY KEY (id, account_id)
);

CREATE INDEX idx_drive_files_parent ON drive_files (account_id, parent_id);
CREATE INDEX idx_drive_files_local_path ON drive_files (local_path);

-- Sync task queue
CREATE TABLE sync_queue (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id      TEXT NOT NULL,
    file_id         TEXT,
    operation       TEXT NOT NULL,      -- 'upload' | 'download' | 'delete' | 'rename' | 'move'
    local_path      TEXT,
    priority        INTEGER DEFAULT 5,  -- 1 (high) ~ 10 (low)
    status          TEXT DEFAULT 'pending',
    retry_count     INTEGER DEFAULT 0,
    error_msg       TEXT,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);

CREATE INDEX idx_sync_queue_status_priority ON sync_queue (status, priority);

-- Changes API cursor
CREATE TABLE sync_tokens (
    account_id      TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    page_token      TEXT NOT NULL,
    updated_at      INTEGER NOT NULL
);

-- Local sync folder configuration
CREATE TABLE sync_folders (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id      TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    local_path      TEXT NOT NULL,
    folder_type     TEXT NOT NULL,      -- 'drive' | 'photos'
    is_enabled      INTEGER DEFAULT 1,
    UNIQUE (account_id, local_path)
);

-- Sync error log
CREATE TABLE sync_errors (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id      TEXT,
    file_id         TEXT,
    file_name       TEXT,
    error_code      TEXT NOT NULL,
    error_msg       TEXT NOT NULL,
    is_resolved     INTEGER DEFAULT 0,
    created_at      INTEGER NOT NULL
);

CREATE INDEX idx_sync_errors_unresolved ON sync_errors (is_resolved, created_at);

-- Ignored USB devices
CREATE TABLE ignored_devices (
    device_id       TEXT PRIMARY KEY,
    label           TEXT,
    ignored_at      INTEGER NOT NULL
);
