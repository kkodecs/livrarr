-- Readarr Library Import: import tracking and import attribution.

-- Imports tracking table.
CREATE TABLE imports (
    id TEXT PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id),
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

-- One running import per user.
CREATE UNIQUE INDEX idx_imports_running ON imports(user_id) WHERE status = 'running';

-- Import attribution on works, authors, library_items.
ALTER TABLE works ADD COLUMN import_id TEXT REFERENCES imports(id);
ALTER TABLE authors ADD COLUMN import_id TEXT REFERENCES imports(id);
ALTER TABLE library_items ADD COLUMN import_id TEXT REFERENCES imports(id);
