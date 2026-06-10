-- 4b / REQ-014 / D-013: the identity track of the two-state-machine split.
-- Adds a persisted identity-confidence status to works, backfilled from each
-- work's ACTUAL identity (anchors / ISBN), NOT from the conflated
-- enrichment_status. Anchor-derived per D-013 (PO-confirmed).

ALTER TABLE works ADD COLUMN identity_status TEXT NOT NULL DEFAULT 'pending';

-- Backfill in ascending priority (each later statement overrides earlier ones
-- where it matches), yielding precedence:
--   conflict > needs_review > confirmed > provisional > pending(default).

-- Provisional: an ISBN/ASIN bridge resolved but no work anchor
-- (REQ-016 de-facto identity).
UPDATE works SET identity_status = 'provisional'
 WHERE (isbn_13 IS NOT NULL OR asin IS NOT NULL)
   AND ol_key IS NULL AND gr_key IS NULL AND hc_key IS NULL;

-- Confirmed: has a work anchor (OL/GR/HC work key).
UPDATE works SET identity_status = 'confirmed'
 WHERE ol_key IS NOT NULL OR gr_key IS NOT NULL OR hc_key IS NOT NULL;

-- NeedsReview: preserve works the old conflated status had flagged for review.
UPDATE works SET identity_status = 'needs_review'
 WHERE enrichment_status = 'needs_review';

-- Conflict: an open identity contradiction outranks all of the above.
UPDATE works SET identity_status = 'conflict'
 WHERE id IN (SELECT existing_work_id FROM work_identity_conflicts WHERE status = 'open');

-- AUDIT (run manually post-migration; intentionally NOT enforced here): rows
-- whose old status implied an identity the anchors don't support. Review before
-- trusting the backfill on these.
--   SELECT id, title, enrichment_status, identity_status,
--          ol_key, gr_key, hc_key, isbn_13, asin
--     FROM works
--    WHERE (enrichment_status = 'identity_pending' AND identity_status = 'confirmed')
--       OR (enrichment_status = 'conflict'        AND identity_status = 'pending');
