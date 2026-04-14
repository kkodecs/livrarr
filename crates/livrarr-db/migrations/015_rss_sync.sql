-- RSS Sync: per-media-type monitoring, indexer RSS toggle, RSS state tracking, indexer config.

-- Split single monitored boolean into per-media-type flags.
ALTER TABLE works RENAME COLUMN monitored TO monitor_ebook;
ALTER TABLE works ADD COLUMN monitor_audiobook BOOLEAN NOT NULL DEFAULT 0;
UPDATE works SET monitor_audiobook = monitor_ebook;

-- Per-indexer RSS toggle.
ALTER TABLE indexers ADD COLUMN enable_rss BOOLEAN NOT NULL DEFAULT 1;

-- Per-indexer RSS sync state (gap detection).
CREATE TABLE indexer_rss_state (
    indexer_id INTEGER PRIMARY KEY REFERENCES indexers(id) ON DELETE CASCADE,
    last_publish_date TEXT,
    last_guid TEXT
);

-- Indexer config singleton.
CREATE TABLE indexer_config (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    rss_sync_interval_minutes INTEGER NOT NULL DEFAULT 15,
    rss_match_threshold REAL NOT NULL DEFAULT 0.80
);
INSERT INTO indexer_config (id) VALUES (1);

UPDATE _livrarr_meta SET value = '15' WHERE key = 'schema_version';
