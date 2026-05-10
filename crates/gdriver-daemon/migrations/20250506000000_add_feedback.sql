-- User feedback submissions.
CREATE TABLE IF NOT EXISTS feedback (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    text        TEXT    NOT NULL,
    include_logs INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL  -- Unix ms
);
