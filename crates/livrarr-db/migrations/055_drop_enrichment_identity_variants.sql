-- 055 / status-backport-drop: EnrichmentStatus dropped its three identity-track
-- variants {conflict, identity_pending, needs_review}. They were redundant
-- projections of the identity track. Identity now lives SOLELY on identity_status
-- (added by migration 054), which gains a new value 'not_found': the LLM rejected
-- every provider payload as not-this-book (distinct from an anchor 'conflict').
-- EnrichmentStatus is enrichment-quality only: {unenriched, enriched, thin, failed}.

-- An old enrichment_status='conflict' meant "the LLM rejected all provider payloads"
-- = the work's identity could not be verified -> identity_status 'not_found'.
-- An OPEN anchor-conflict row outranks it (migration 054 already set those works to
-- identity_status='conflict'); do not override those.
UPDATE works SET identity_status = 'not_found'
 WHERE enrichment_status = 'conflict'
   AND id NOT IN (SELECT existing_work_id FROM work_identity_conflicts WHERE status = 'open');

-- Collapse the three dropped identity-track enrichment values to 'unenriched'. Their
-- identity meaning is preserved on identity_status: 'needs_review' and the anchor-derived
-- pending/provisional/confirmed were backfilled by migration 054, and 'conflict'
-- (all-rejected) is mapped to 'not_found' by the statement above.
UPDATE works SET enrichment_status = 'unenriched'
 WHERE enrichment_status IN ('conflict', 'identity_pending', 'needs_review');
