-- Add content_path column to grabs table.
-- Stores the raw remote path from the download client (pre-path-mapping).
-- Used to avoid re-querying the download client during import.
ALTER TABLE grabs ADD COLUMN content_path TEXT;

UPDATE _livrarr_meta SET value = '14' WHERE key = 'schema_version';
