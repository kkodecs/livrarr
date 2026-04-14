-- Backfill empty imported_at values with a sentinel timestamp.
UPDATE library_items SET imported_at = '1970-01-01T00:00:00Z' WHERE imported_at = '';

UPDATE _livrarr_meta SET value = '22' WHERE key = 'schema_version';
