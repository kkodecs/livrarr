-- Clean up existing duplicate notifications: keep the row with lowest id per
-- (user_id, type, ref_key) group. NULL ref_key is treated as a distinct group
-- via COALESCE sentinel for dedup purposes (only in the index expression; actual
-- column values are NOT modified).

DELETE FROM notifications
WHERE id NOT IN (
    SELECT MIN(id)
    FROM notifications
    GROUP BY user_id, type, COALESCE(ref_key, '__null__')
);

-- Expression-based unique index for deduplication.
-- SQLite treats NULLs as distinct in plain unique indexes, so we use
-- COALESCE(ref_key, '__null__') to make NULL ref_key participate in
-- uniqueness checking. The actual ref_key column value remains NULL.
CREATE UNIQUE INDEX IF NOT EXISTS idx_notifications_dedup
    ON notifications(user_id, type, COALESCE(ref_key, '__null__'));
