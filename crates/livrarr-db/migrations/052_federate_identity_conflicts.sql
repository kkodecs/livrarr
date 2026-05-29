-- work-creation-consistency: federate the identity-conflict store.
-- Data-preserving rebuild of work_identity_conflicts: drop the legacy OL-only
-- enum guard constraints (so federated kinds gr/hc/quorum_tie, the
-- series_monitor source, and the renamed replace_anchor action can be stored)
-- and remap the legacy resolution_action 'replace_ol_key' to 'replace_anchor'.
-- Enum validity is enforced in Rust; altering a column guard requires a full
-- table rebuild, so the OL-only migration 040 is rebuilt here rather than
-- altered (ir-v1 D-007 / R-005; ir-v2 db-conflicts-migration). Every existing
-- row is preserved.

DROP TABLE IF EXISTS work_identity_conflicts_new;

CREATE TABLE work_identity_conflicts_new (
    id                    INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id               INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    existing_work_id      INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    kind                  TEXT NOT NULL,
    incoming_payload_json TEXT NOT NULL,
    raised_at             TEXT NOT NULL,
    raised_by             TEXT NOT NULL,
    raised_source_path    TEXT,
    status                TEXT NOT NULL DEFAULT 'open',
    resolved_at           TEXT,
    resolution_action     TEXT,
    resolution_notes      TEXT
);

INSERT INTO work_identity_conflicts_new (
    id, user_id, existing_work_id, kind, incoming_payload_json, raised_at,
    raised_by, raised_source_path, status, resolved_at, resolution_action,
    resolution_notes
)
SELECT
    id, user_id, existing_work_id, kind, incoming_payload_json, raised_at,
    raised_by, raised_source_path, status, resolved_at,
    CASE resolution_action
        WHEN 'replace_ol_key' THEN 'replace_anchor'
        ELSE resolution_action
    END,
    resolution_notes
FROM work_identity_conflicts;

DROP TABLE work_identity_conflicts;

ALTER TABLE work_identity_conflicts_new RENAME TO work_identity_conflicts;

CREATE INDEX IF NOT EXISTS idx_identity_conflicts_user_status
    ON work_identity_conflicts(user_id, status);

CREATE INDEX IF NOT EXISTS idx_identity_conflicts_work
    ON work_identity_conflicts(existing_work_id);
