-- RSS auto-grab failure cap (114a): cap total failures per (work, media_type)
-- within a rolling window so a stuck import doesn't get re-grabbed forever.
-- 0 disables the cap.
ALTER TABLE indexer_config ADD COLUMN rss_grab_failure_limit INTEGER NOT NULL DEFAULT 3;
