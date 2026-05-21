-- English work lifecycle: identity conflict tracking.
-- Raised when an incoming work candidate's OL key conflicts with an existing work.

CREATE TABLE IF NOT EXISTS work_identity_conflicts (
    id                    INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id               INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    existing_work_id      INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    kind                  TEXT NOT NULL CHECK (kind IN ('incoming_different_ol_key', 'ol_redirect_collision')),
    incoming_payload_json TEXT NOT NULL,
    raised_at             TEXT NOT NULL,
    raised_by             TEXT NOT NULL CHECK (raised_by IN ('manual_add', 'manual_import', 'list_import', 'readarr_import', 'author_monitor', 'refresh')),
    raised_source_path    TEXT,
    status                TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'resolved', 'dismissed')),
    resolved_at           TEXT,
    resolution_action     TEXT CHECK (resolution_action IN ('keep_existing', 'accept_separate', 'replace_ol_key', 'merge') OR resolution_action IS NULL),
    resolution_notes      TEXT
);

CREATE INDEX IF NOT EXISTS idx_identity_conflicts_user_status
    ON work_identity_conflicts(user_id, status);

CREATE INDEX IF NOT EXISTS idx_identity_conflicts_work
    ON work_identity_conflicts(existing_work_id);
