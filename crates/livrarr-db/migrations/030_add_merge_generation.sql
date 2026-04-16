ALTER TABLE works ADD COLUMN merge_generation INTEGER NOT NULL DEFAULT 0;

UPDATE _livrarr_meta SET value = '30' WHERE key = 'schema_version';
