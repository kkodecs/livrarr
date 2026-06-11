-- Per-field / per-provider merge dissents (REQ-014): the excluded contribution,
-- recorded queryably per work. A dissent isolates one provider's (or one
-- field's) contribution and never blocks the merge; a work's next merge
-- generation supersedes its prior rows.
CREATE TABLE IF NOT EXISTS work_field_dissents (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id          INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    work_id          INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    provider         TEXT    NOT NULL,
    field            TEXT    NOT NULL,
    offered_value    TEXT    NOT NULL,
    winning_value    TEXT,
    reason           TEXT    NOT NULL,
    merge_generation INTEGER NOT NULL,
    recorded_at      TEXT    NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_work_field_dissents_work
    ON work_field_dissents(user_id, work_id);
