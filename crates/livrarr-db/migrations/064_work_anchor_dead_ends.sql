-- Durable per-(work, anchor) convergence retry counter (id-completeness REQ-009):
-- how many background attempts have failed to obtain a missing anchor. At or
-- above the configured threshold the anchor is a dead-end and is no longer
-- chased; a successful harvest clears the row. Unlike provider_retry_state it
-- survives manual refresh, so a give-up decision stays durable.
CREATE TABLE IF NOT EXISTS work_anchor_dead_ends (
    work_id INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    anchor_type TEXT NOT NULL,
    attempt_count INTEGER NOT NULL DEFAULT 0,
    last_attempt_at TEXT NOT NULL,
    user_id INTEGER NOT NULL,
    PRIMARY KEY (work_id, anchor_type)
);
