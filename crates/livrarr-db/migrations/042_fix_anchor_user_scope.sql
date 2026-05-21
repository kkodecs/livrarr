-- Fix: uniq_user_confirmed_ol_anchor was global (anchor_type, anchor_value)
-- but should be per-user. Since work_identity_anchors has no user_id column,
-- we drop the global constraint entirely. Per-user uniqueness is enforced at
-- the application layer via find_work_by_anchor (user-scoped query) before
-- confirm_ol_anchor. The per-work constraint (PRIMARY KEY + partial UNIQUE on
-- work_id,anchor_type WHERE confirmed) already prevents one work from having
-- two confirmed anchors of the same type.
DROP INDEX IF EXISTS uniq_user_confirmed_ol_anchor;
