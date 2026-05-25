ALTER TABLE playback_progress ADD COLUMN finished_at TEXT;
ALTER TABLE library_items ADD COLUMN duration_seconds REAL;
ALTER TABLE library_items ADD COLUMN chapter_scan_status TEXT;
