-- Fix: add ON DELETE CASCADE to playback_progress foreign keys.
-- SQLite requires table recreation to alter foreign key constraints.

CREATE TABLE playback_progress_new (
    id INTEGER PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    library_item_id INTEGER NOT NULL REFERENCES library_items(id) ON DELETE CASCADE,
    position TEXT NOT NULL,
    progress_pct REAL NOT NULL DEFAULT 0.0,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE(user_id, library_item_id)
);

INSERT INTO playback_progress_new SELECT * FROM playback_progress;
DROP TABLE playback_progress;
ALTER TABLE playback_progress_new RENAME TO playback_progress;

CREATE INDEX idx_playback_progress_user ON playback_progress(user_id);
