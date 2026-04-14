-- Playback progress tracking for ebook reading and audiobook listening.

CREATE TABLE playback_progress (
    id INTEGER PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id),
    library_item_id INTEGER NOT NULL REFERENCES library_items(id),
    position TEXT NOT NULL,
    progress_pct REAL NOT NULL DEFAULT 0.0,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE(user_id, library_item_id)
);

CREATE INDEX idx_playback_progress_user ON playback_progress(user_id);

UPDATE _livrarr_meta SET value = '18' WHERE key = 'schema_version';
