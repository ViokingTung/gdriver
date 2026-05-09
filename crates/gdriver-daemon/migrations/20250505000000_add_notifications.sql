-- Notifications table for user-visible alerts (conflicts, storage warnings, sync errors)
CREATE TABLE notifications (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id      TEXT,
    kind            TEXT NOT NULL,       -- 'conflict', 'storage_warning', 'sync_error'
    payload         TEXT NOT NULL,       -- JSON blob with kind-specific fields
    is_read         INTEGER DEFAULT 0,
    created_at      INTEGER NOT NULL
);

CREATE INDEX idx_notifications_unread ON notifications (is_read, created_at);
