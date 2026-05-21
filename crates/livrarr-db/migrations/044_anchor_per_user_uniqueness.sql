-- Add user_id to work_identity_anchors for per-user uniqueness enforcement.
-- SQLite can't do cross-table partial unique indexes, so we denormalize user_id
-- into the anchors table and enforce uniqueness at the DB level.
ALTER TABLE work_identity_anchors ADD COLUMN user_id INTEGER REFERENCES users(id) ON DELETE CASCADE;

-- Backfill user_id from works table.
UPDATE work_identity_anchors SET user_id = (
    SELECT w.user_id FROM works w WHERE w.id = work_identity_anchors.work_id
);

-- Per-user: same user cannot have two confirmed anchors with the same type+value.
CREATE UNIQUE INDEX IF NOT EXISTS uniq_user_confirmed_ol_anchor
    ON work_identity_anchors(user_id, anchor_type, anchor_value)
    WHERE confidence = 'confirmed';
