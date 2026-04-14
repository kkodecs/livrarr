-- List import preview session state for CSV imports (Goodreads, Hardcover).
-- Rows are ephemeral — cleaned up after 1 hour by session_cleanup job.

CREATE TABLE list_import_previews (
    id             INTEGER PRIMARY KEY,
    preview_id     TEXT NOT NULL,
    user_id        INTEGER NOT NULL REFERENCES users(id),
    row_index      INTEGER NOT NULL,
    title          TEXT NOT NULL,
    author         TEXT NOT NULL,
    isbn_13        TEXT,
    isbn_10        TEXT,
    year           INTEGER,
    source_status  TEXT,
    source_rating  REAL,
    preview_status TEXT NOT NULL CHECK (preview_status IN ('new', 'already_exists', 'parse_error')),
    source         TEXT NOT NULL CHECK (source IN ('goodreads', 'hardcover')),
    created_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX idx_lip_preview_user ON list_import_previews(preview_id, user_id);
CREATE UNIQUE INDEX idx_lip_row ON list_import_previews(preview_id, user_id, row_index);

UPDATE _livrarr_meta SET value = '20' WHERE key = 'schema_version';
