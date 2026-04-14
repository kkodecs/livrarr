-- Series monitoring: series entity, author series cache, works.series_id link.

CREATE TABLE series (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id           INTEGER NOT NULL REFERENCES users(id),
    author_id         INTEGER NOT NULL REFERENCES authors(id) ON DELETE CASCADE,
    name              TEXT NOT NULL,
    gr_key            TEXT NOT NULL,
    monitor_ebook     BOOLEAN NOT NULL DEFAULT FALSE,
    monitor_audiobook BOOLEAN NOT NULL DEFAULT FALSE,
    work_count        INTEGER NOT NULL DEFAULT 0,
    added_at          TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(user_id, author_id, gr_key)
);

CREATE INDEX idx_series_user_author ON series(user_id, author_id);

CREATE TABLE author_series_cache (
    author_id   INTEGER PRIMARY KEY REFERENCES authors(id) ON DELETE CASCADE,
    entries     TEXT NOT NULL DEFAULT '[]',
    fetched_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

ALTER TABLE works ADD COLUMN series_id INTEGER REFERENCES series(id) ON DELETE SET NULL;

CREATE INDEX idx_works_series_id ON works(series_id);
CREATE INDEX idx_works_author_grkey ON works(author_id, gr_key);
CREATE INDEX idx_works_author_series_name ON works(author_id, series_name);

UPDATE _livrarr_meta SET value = '23' WHERE key = 'schema_version';
