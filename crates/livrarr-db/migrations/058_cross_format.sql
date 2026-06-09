-- Cross-format resume (Whispersync model): .kash-established 1:1 links between
-- one audiobook and one ebook LibraryItem, plus per-(user, link) monotonic
-- furthest marks in audio-timestamp space with per-format decline thresholds.

CREATE TABLE kash_links (
    id INTEGER PRIMARY KEY,
    audio_item_id INTEGER NOT NULL UNIQUE REFERENCES library_items(id) ON DELETE CASCADE,
    ebook_item_id INTEGER NOT NULL UNIQUE REFERENCES library_items(id) ON DELETE CASCADE,
    container_duration_secs REAL NOT NULL,
    epub_hash TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE TABLE cross_format_state (
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    kash_link_id INTEGER NOT NULL REFERENCES kash_links(id) ON DELETE CASCADE,
    furthest_ts REAL NOT NULL DEFAULT 0,
    ebook_declined_at_ts REAL,
    audio_declined_at_ts REAL,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    PRIMARY KEY (user_id, kash_link_id)
);

CREATE INDEX idx_cross_format_state_link ON cross_format_state(kash_link_id);
