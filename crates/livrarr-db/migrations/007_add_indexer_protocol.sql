-- Add protocol field to indexers (torrent or usenet).
-- Default to 'torrent' for existing indexers.
ALTER TABLE indexers ADD COLUMN protocol TEXT NOT NULL DEFAULT 'torrent';
