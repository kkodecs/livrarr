-- Add imported_at timestamp to library_items (was in the original combined migration but lost in the split).
ALTER TABLE library_items ADD COLUMN imported_at TEXT NOT NULL DEFAULT '1970-01-01T00:00:00Z';
