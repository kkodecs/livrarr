CREATE TABLE provider_retry_state (
    user_id                INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    work_id                INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    provider               TEXT NOT NULL,
    attempts               INTEGER NOT NULL DEFAULT 0,
    suppressed_passes      INTEGER NOT NULL DEFAULT 0,
    last_outcome           TEXT,
    last_attempt_at        TEXT,
    next_attempt_at        TEXT,
    normalized_payload_json TEXT,
    first_suppressed_at    TEXT,
    PRIMARY KEY (work_id, provider)
);

UPDATE _livrarr_meta SET value = '29' WHERE key = 'schema_version';
