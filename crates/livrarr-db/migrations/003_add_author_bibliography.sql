CREATE TABLE IF NOT EXISTS author_bibliography (
    author_id INTEGER PRIMARY KEY REFERENCES authors(id) ON DELETE CASCADE,
    entries TEXT NOT NULL DEFAULT '[]',
    fetched_at TEXT NOT NULL DEFAULT (datetime('now'))
);
