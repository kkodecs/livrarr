-- English work lifecycle: identity anchor table.
-- Stores the authoritative identity resolution per work (OL key, ISBN, etc.).
-- works.ol_key remains as a denormalized cache; this table is the source of truth.

CREATE TABLE IF NOT EXISTS work_identity_anchors (
    work_id       INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    anchor_type   TEXT NOT NULL,
    anchor_value  TEXT NOT NULL,
    confidence    TEXT NOT NULL CHECK (confidence IN ('confirmed', 'pending', 'superseded')),
    setter        TEXT NOT NULL CHECK (setter IN ('user', 'auto_isbn', 'auto_search', 'import', 'redirect')),
    set_at        TEXT NOT NULL,
    superseded_by TEXT,
    PRIMARY KEY (work_id, anchor_type, anchor_value)
);

-- Only one confirmed anchor per (work_id, anchor_type) — e.g. one confirmed OL key per work.
CREATE UNIQUE INDEX IF NOT EXISTS uniq_primary_confirmed_anchor
    ON work_identity_anchors(work_id, anchor_type)
    WHERE confidence = 'confirmed';

-- Lookup by anchor value (e.g. find work by OL key).
CREATE INDEX IF NOT EXISTS idx_anchor_value
    ON work_identity_anchors(anchor_type, anchor_value);
