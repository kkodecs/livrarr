-- Durable identity-review dismissal ledger (REQ-005). Source card ids are
-- informational and have no foreign key. Work membership does not require the
-- Work row to survive; historical adoption performs its own existence check.

CREATE TABLE identity_review_dismissals (
    id INTEGER PRIMARY KEY,
    user_id INTEGER NOT NULL,
    kind TEXT NOT NULL,
    key_version INTEGER NOT NULL,
    canonical_key TEXT NOT NULL,
    dismissed_at TEXT NOT NULL,
    source_card_id INTEGER NOT NULL,
    revoked_at TEXT,
    revoke_reason TEXT,
    UNIQUE(user_id, kind, key_version, canonical_key)
);

CREATE TABLE identity_review_dismissal_works (
    dismissal_id INTEGER NOT NULL REFERENCES identity_review_dismissals(id) ON DELETE CASCADE,
    user_id INTEGER NOT NULL,
    work_id INTEGER NOT NULL,
    PRIMARY KEY(dismissal_id, work_id)
);

CREATE INDEX idx_identity_review_dismissals_active_key
    ON identity_review_dismissals(user_id, kind, key_version, canonical_key, revoked_at);

CREATE INDEX idx_identity_review_dismissal_works_revoke
    ON identity_review_dismissal_works(user_id, work_id, dismissal_id);
