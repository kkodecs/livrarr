-- Add metadata_source to works table for foreign language provider attribution.
-- Null for existing works (English/OL). Populated from provider name on creation.
ALTER TABLE works ADD COLUMN metadata_source TEXT;

UPDATE _livrarr_meta SET value = '12' WHERE key = 'schema_version';
