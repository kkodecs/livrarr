-- Add ON DELETE CASCADE to user_id FK on imports, list_import_previews, and series.
-- SQLite requires table recreation to alter FK constraints.
--
-- Because foreign_keys=ON prevents DROP TABLE when child rows reference the parent,
-- we save attribution links to TEMP tables, NULL out all references, do the swaps,
-- then restore links from TEMP tables. All inside a single transaction.

CREATE TEMP TABLE _work_import_map  AS SELECT id, import_id  FROM works         WHERE import_id  IS NOT NULL;
CREATE TEMP TABLE _auth_import_map  AS SELECT id, import_id  FROM authors        WHERE import_id  IS NOT NULL;
CREATE TEMP TABLE _item_import_map  AS SELECT id, import_id  FROM library_items  WHERE import_id  IS NOT NULL;
CREATE TEMP TABLE _work_series_map  AS SELECT id, series_id  FROM works           WHERE series_id  IS NOT NULL;

UPDATE works         SET import_id = NULL, series_id = NULL;
UPDATE authors       SET import_id = NULL;
UPDATE library_items SET import_id = NULL;

-- ── imports ─────────────────────────────────────────────────────────────────
DELETE FROM imports WHERE user_id NOT IN (SELECT id FROM users);
DELETE FROM imports
WHERE target_root_folder_id IS NOT NULL
  AND target_root_folder_id NOT IN (SELECT id FROM root_folders);

CREATE TABLE imports_new (
    id                    TEXT    PRIMARY KEY,
    user_id               INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    source                TEXT    NOT NULL DEFAULT 'readarr',
    status                TEXT    NOT NULL DEFAULT 'running'
                                  CHECK (status IN ('running', 'completed', 'failed', 'undone')),
    started_at            TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    completed_at          TEXT,
    authors_created       INTEGER NOT NULL DEFAULT 0,
    works_created         INTEGER NOT NULL DEFAULT 0,
    files_imported        INTEGER NOT NULL DEFAULT 0,
    files_skipped         INTEGER NOT NULL DEFAULT 0,
    source_url            TEXT,
    target_root_folder_id INTEGER REFERENCES root_folders(id)
);
INSERT INTO imports_new SELECT * FROM imports;
DROP TABLE imports;
ALTER TABLE imports_new RENAME TO imports;
CREATE UNIQUE INDEX idx_imports_running ON imports(user_id) WHERE status = 'running';

-- ── list_import_previews ────────────────────────────────────────────────────
DELETE FROM list_import_previews WHERE user_id NOT IN (SELECT id FROM users);

CREATE TABLE list_import_previews_new (
    id             INTEGER PRIMARY KEY,
    preview_id     TEXT    NOT NULL,
    user_id        INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    row_index      INTEGER NOT NULL,
    title          TEXT    NOT NULL,
    author         TEXT    NOT NULL,
    isbn_13        TEXT,
    isbn_10        TEXT,
    year           INTEGER,
    source_status  TEXT,
    source_rating  REAL,
    preview_status TEXT    NOT NULL
                           CHECK (preview_status IN ('new', 'already_exists', 'parse_error')),
    source         TEXT    NOT NULL
                           CHECK (source IN ('goodreads', 'hardcover')),
    created_at     TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
INSERT INTO list_import_previews_new SELECT * FROM list_import_previews;
DROP TABLE list_import_previews;
ALTER TABLE list_import_previews_new RENAME TO list_import_previews;
CREATE INDEX idx_lip_preview_user ON list_import_previews(preview_id, user_id);
CREATE UNIQUE INDEX idx_lip_row ON list_import_previews(preview_id, user_id, row_index);

-- ── series ──────────────────────────────────────────────────────────────────
DELETE FROM series WHERE user_id   NOT IN (SELECT id FROM users);
DELETE FROM series WHERE author_id NOT IN (SELECT id FROM authors);

CREATE TABLE series_new (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id           INTEGER NOT NULL REFERENCES users(id)    ON DELETE CASCADE,
    author_id         INTEGER NOT NULL REFERENCES authors(id)  ON DELETE CASCADE,
    name              TEXT    NOT NULL,
    gr_key            TEXT    NOT NULL,
    monitor_ebook     BOOLEAN NOT NULL DEFAULT FALSE,
    monitor_audiobook BOOLEAN NOT NULL DEFAULT FALSE,
    work_count        INTEGER NOT NULL DEFAULT 0,
    added_at          TEXT    NOT NULL DEFAULT (datetime('now')),
    UNIQUE(user_id, author_id, gr_key)
);
INSERT INTO series_new SELECT * FROM series;
DROP TABLE series;
ALTER TABLE series_new RENAME TO series;
CREATE INDEX idx_series_user_author ON series(user_id, author_id);

-- ── restore attribution links ────────────────────────────────────────────────
UPDATE works SET import_id = (
    SELECT import_id FROM _work_import_map WHERE _work_import_map.id = works.id)
WHERE id IN (
    SELECT id FROM _work_import_map WHERE import_id IN (SELECT id FROM imports));

UPDATE authors SET import_id = (
    SELECT import_id FROM _auth_import_map WHERE _auth_import_map.id = authors.id)
WHERE id IN (
    SELECT id FROM _auth_import_map WHERE import_id IN (SELECT id FROM imports));

UPDATE library_items SET import_id = (
    SELECT import_id FROM _item_import_map WHERE _item_import_map.id = library_items.id)
WHERE id IN (
    SELECT id FROM _item_import_map WHERE import_id IN (SELECT id FROM imports));

UPDATE works SET series_id = (
    SELECT series_id FROM _work_series_map WHERE _work_series_map.id = works.id)
WHERE id IN (
    SELECT id FROM _work_series_map WHERE series_id IN (SELECT id FROM series));
