-- Add cover trust model and audiobook cover fields
ALTER TABLE works ADD COLUMN cover_source TEXT;
ALTER TABLE works ADD COLUMN cover_trust TEXT DEFAULT 'unvalidated';
ALTER TABLE works ADD COLUMN cover_width INTEGER DEFAULT 0;
ALTER TABLE works ADD COLUMN cover_height INTEGER DEFAULT 0;
ALTER TABLE works ADD COLUMN audiobook_cover_url TEXT;
ALTER TABLE works ADD COLUMN audiobook_cover_source TEXT;
ALTER TABLE works ADD COLUMN audiobook_cover_trust TEXT DEFAULT 'unvalidated';
ALTER TABLE works ADD COLUMN audiobook_cover_width INTEGER DEFAULT 0;
ALTER TABLE works ADD COLUMN audiobook_cover_height INTEGER DEFAULT 0;

-- Backfill cover_trust from cover_manual
UPDATE works SET cover_trust = 'user' WHERE cover_manual = 1;
UPDATE works SET cover_trust = 'unvalidated' WHERE cover_manual = 0 AND cover_url IS NOT NULL;
