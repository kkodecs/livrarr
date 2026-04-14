-- Import retry support: track retry count and failure time for exponential backoff.
ALTER TABLE grabs ADD COLUMN import_retry_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE grabs ADD COLUMN import_failed_at TEXT;
