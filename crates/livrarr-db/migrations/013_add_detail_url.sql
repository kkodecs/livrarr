-- Add detail_url to works table for foreign work enrichment.
-- Stores the metadata provider detail page URL (e.g., Goodreads book page).
-- Server-side only — never exposed in API responses.
ALTER TABLE works ADD COLUMN detail_url TEXT;

UPDATE _livrarr_meta SET value = '13' WHERE key = 'schema_version';
