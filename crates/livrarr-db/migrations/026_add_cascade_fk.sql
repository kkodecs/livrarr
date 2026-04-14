-- Add ON DELETE CASCADE to user-scoped tables.
-- SQLite doesn't support ALTER TABLE ADD CONSTRAINT, so we must:
-- 1. Rename the old table
-- 2. Create new table with CASCADE FK
-- 3. Copy data
-- 4. Drop old table
-- 5. Recreate indexes

-- ============================================================================
-- Fix list_import_previews.user_id
-- ============================================================================

-- Rename old table and copy data with new schema
ALTER TABLE list_import_previews RENAME TO list_import_previews_old;

CREATE TABLE list_import_previews (
    id             INTEGER PRIMARY KEY,
    preview_id     TEXT NOT NULL,
    user_id        INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
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

INSERT INTO list_import_previews
  SELECT * FROM list_import_previews_old;

DROP TABLE list_import_previews_old;

-- Recreate indexes
CREATE INDEX idx_lip_preview_user ON list_import_previews(preview_id, user_id);
CREATE UNIQUE INDEX idx_lip_row ON list_import_previews(preview_id, user_id, row_index);

-- ============================================================================
-- Fix series.user_id
-- ============================================================================

ALTER TABLE series RENAME TO series_old;

CREATE TABLE series (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id           INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    author_id         INTEGER NOT NULL REFERENCES authors(id) ON DELETE CASCADE,
    name              TEXT NOT NULL,
    gr_key            TEXT NOT NULL,
    monitor_ebook     BOOLEAN NOT NULL DEFAULT FALSE,
    monitor_audiobook BOOLEAN NOT NULL DEFAULT FALSE,
    work_count        INTEGER NOT NULL DEFAULT 0,
    added_at          TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(user_id, author_id, gr_key)
);

INSERT INTO series
  SELECT * FROM series_old;

DROP TABLE series_old;

-- Recreate index
CREATE INDEX idx_series_user_author ON series(user_id, author_id);

-- ============================================================================
-- Fix imports.user_id
-- ============================================================================

ALTER TABLE imports RENAME TO imports_old;

CREATE TABLE imports (
    id TEXT PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    source TEXT NOT NULL DEFAULT 'readarr',
    status TEXT NOT NULL DEFAULT 'running' CHECK (status IN ('running', 'completed', 'failed', 'undone')),
    started_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    completed_at TEXT,
    authors_created INTEGER NOT NULL DEFAULT 0,
    works_created INTEGER NOT NULL DEFAULT 0,
    files_imported INTEGER NOT NULL DEFAULT 0,
    files_skipped INTEGER NOT NULL DEFAULT 0,
    source_url TEXT,
    target_root_folder_id INTEGER REFERENCES root_folders(id)
);

INSERT INTO imports
  SELECT * FROM imports_old;

DROP TABLE imports_old;

-- Recreate index
CREATE UNIQUE INDEX idx_imports_running ON imports(user_id) WHERE status = 'running';

UPDATE _livrarr_meta SET value = '26' WHERE key = 'schema_version';
