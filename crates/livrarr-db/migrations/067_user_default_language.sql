-- The user's default language for newly added works: applied when a creation
-- door has no explicit language (no file metadata, provider record, or user
-- pick). Lives on the metadata_config singleton like the other user
-- preferences; the 'en' default keeps existing installs unchanged.
ALTER TABLE metadata_config ADD COLUMN default_language TEXT NOT NULL DEFAULT 'en';
