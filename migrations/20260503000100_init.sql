CREATE TABLE IF NOT EXISTS users (
    user_id INTEGER PRIMARY KEY,
    balance INTEGER NOT NULL DEFAULT 3,
    referrer_by INTEGER
);

CREATE TABLE IF NOT EXISTS track_requests (
    id TEXT PRIMARY KEY,
    user_id INTEGER NOT NULL,
    url TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_track_requests_user_id
    ON track_requests(user_id);

CREATE INDEX IF NOT EXISTS idx_track_requests_created_at
    ON track_requests(created_at);
