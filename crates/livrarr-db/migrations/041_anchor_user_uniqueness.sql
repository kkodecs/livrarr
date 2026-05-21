-- Prevent duplicate confirmed OL anchors for the same user.
-- Two works owned by the same user cannot share a confirmed ol_work anchor.
CREATE UNIQUE INDEX IF NOT EXISTS uniq_user_confirmed_ol_anchor
    ON work_identity_anchors(anchor_type, anchor_value)
    WHERE confidence = 'confirmed';
