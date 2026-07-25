-- 1. Drop 044's live per-user all-anchor index (same name as 041's dropped predecessor).
DROP INDEX IF EXISTS uniq_user_confirmed_ol_anchor;

-- 2. Recreate per-user uniqueness for WORK KEYS only (the ratified scope).
CREATE UNIQUE INDEX IF NOT EXISTS uniq_user_confirmed_work_anchor
    ON work_identity_anchors(user_id, anchor_type, anchor_value)
    WHERE confidence = 'confirmed'
      AND anchor_type IN ('ol_work', 'gr_work', 'hc_work');

-- 3. Durable coordination for identity writers; same class as merge_generation.
ALTER TABLE works
    ADD COLUMN identity_generation INTEGER NOT NULL DEFAULT 0;
