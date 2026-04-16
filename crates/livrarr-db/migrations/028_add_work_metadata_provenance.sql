CREATE TABLE work_metadata_provenance (
    user_id   INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    work_id   INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    field     TEXT NOT NULL,
    source    TEXT,
    set_at    TEXT NOT NULL,
    setter    TEXT NOT NULL,
    cleared   INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (work_id, field)
);

UPDATE _livrarr_meta SET value = '28' WHERE key = 'schema_version';
