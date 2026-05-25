CREATE TABLE bookmarks (
    id INTEGER PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    work_id INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    library_item_id INTEGER NOT NULL REFERENCES library_items(id) ON DELETE CASCADE,
    media_type TEXT NOT NULL,
    position TEXT NOT NULL,
    sort_key REAL NOT NULL,
    name TEXT NOT NULL,
    chapter_title TEXT,
    paired_bookmark_id INTEGER REFERENCES bookmarks(id) ON DELETE SET NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
CREATE INDEX idx_bookmarks_user_item ON bookmarks(user_id, library_item_id);
CREATE INDEX idx_bookmarks_paired ON bookmarks(paired_bookmark_id);
