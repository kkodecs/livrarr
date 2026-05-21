-- Backfill work_identity_anchors from existing works.ol_key values.
-- Ensures upgraded libraries have anchor rows for dedup/adopt lookups.
INSERT INTO work_identity_anchors (work_id, anchor_type, anchor_value, confidence, setter, set_at)
SELECT id, 'ol_work', ol_key, 'confirmed', 'import', datetime('now')
FROM works
WHERE ol_key IS NOT NULL AND ol_key != ''
  AND NOT EXISTS (
    SELECT 1 FROM work_identity_anchors a
    WHERE a.work_id = works.id AND a.anchor_type = 'ol_work'
  );
