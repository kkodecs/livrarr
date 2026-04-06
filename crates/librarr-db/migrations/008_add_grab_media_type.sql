-- Add media_type to grabs so we know ebook vs audiobook at grab time.
-- Nullable for existing grabs.
ALTER TABLE grabs ADD COLUMN media_type TEXT;
