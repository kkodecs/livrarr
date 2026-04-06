ALTER TABLE metadata_config ADD COLUMN hardcover_enabled INTEGER NOT NULL DEFAULT 1;
ALTER TABLE metadata_config ADD COLUMN llm_enabled INTEGER NOT NULL DEFAULT 1;
