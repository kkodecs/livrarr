CREATE TABLE audiobook_chapters (
    id INTEGER PRIMARY KEY,
    library_item_id INTEGER NOT NULL REFERENCES library_items(id) ON DELETE CASCADE,
    chapter_index INTEGER NOT NULL,
    title TEXT NOT NULL,
    start_time_secs REAL NOT NULL,
    end_time_secs REAL NOT NULL,
    UNIQUE(library_item_id, chapter_index)
);
CREATE INDEX idx_chapters_item ON audiobook_chapters(library_item_id);
