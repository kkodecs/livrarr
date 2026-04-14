-- Consistent external ID columns: ol_key, gr_key, hc_key on both works and authors.
ALTER TABLE works RENAME COLUMN hardcover_id TO hc_key;
ALTER TABLE works ADD COLUMN gr_key TEXT;
ALTER TABLE authors ADD COLUMN gr_key TEXT;
ALTER TABLE authors ADD COLUMN hc_key TEXT;
