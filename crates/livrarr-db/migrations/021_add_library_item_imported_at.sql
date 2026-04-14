-- Add imported_at timestamp to library_items (was in the original combined migration but lost in the split).
-- Using IF NOT EXISTS to handle fresh installs where imported_at is already in the schema.
ALTER TABLE library_items ADD COLUMN IF NOT EXISTS imported_at TEXT NOT NULL DEFAULT '1970-01-01T00:00:00Z';

UPDATE _livrarr_meta SET value = '21' WHERE key = 'schema_version';
