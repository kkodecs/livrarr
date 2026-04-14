-- Finalize schema_version after all migrations.
-- This migration ensures schema_version is set to the latest migration number.
UPDATE _livrarr_meta SET value = '27' WHERE key = 'schema_version';
