-- Add preferred format columns to media_management_config.
-- Stored as JSON arrays of format strings, ordered by preference (first = highest).
-- Defaults: epub for ebooks, m4b for audiobooks.
ALTER TABLE media_management_config ADD COLUMN preferred_ebook_formats TEXT NOT NULL DEFAULT '["epub"]';
ALTER TABLE media_management_config ADD COLUMN preferred_audiobook_formats TEXT NOT NULL DEFAULT '["m4b"]';
