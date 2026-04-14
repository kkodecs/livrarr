-- Version tracking table for startup compatibility gate.
-- schema_version tracks DDL level (bumped by migrations).
-- data_version tracks semantic data compatibility (bumped manually).
CREATE TABLE IF NOT EXISTS _livrarr_meta (
    key   TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
);

INSERT OR IGNORE INTO _livrarr_meta (key, value) VALUES ('schema_version', '10');
INSERT OR IGNORE INTO _livrarr_meta (key, value) VALUES ('data_version', '1');

UPDATE _livrarr_meta SET value = '10' WHERE key = 'schema_version';
