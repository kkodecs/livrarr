-- How many authorial-slot observations one key attempt made. Written in the
-- same statement that records the attempt's transition, so an attempt can never
-- reach a completed state without its observation. The Tier-2 gate sums this
-- over the generation instead of trusting a per-pass in-memory tally, which a
-- terminal attempt is never replayed into.
ALTER TABLE author_link_key_attempts
    ADD COLUMN authorial_credits_seen INTEGER NOT NULL DEFAULT 0;

-- When a dismissal stopped suppressing its question. A revoked dismissal keeps
-- status = 'dismissed' — the user's decision and its resolution time stay on the
-- record — and only this stamp says the answer no longer binds. A nullable
-- column rather than a new status value: the shipped status CHECK admits only
-- pending/dismissed/picked/superseded, and widening it would need a table
-- rebuild.
ALTER TABLE author_link_candidates
    ADD COLUMN revoked_at TEXT;

-- Every author currently holding a name-guard question raised before unlabelled
-- credits were told apart from asserted ones. Clearing the evaluated fingerprint
-- is what makes the next pass a full re-walk: the road sees changed evidence,
-- opens a new generation, and the generation write supersedes these questions
-- before any provider is called.
--
-- Selection is per author and deliberately over-inclusive: an author holding one
-- question this unit changes is re-walked whole. Authors holding only
-- legacy_contradiction rows are not selected — nothing in this unit can change
-- their outcome.
UPDATE author_link_progress
   SET state = 'queued',
       next_attempt_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
       updated_at      = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
       claim_token = NULL,
       lease_until = NULL,
       evaluated_fingerprint = NULL
 WHERE author_id IN (
     SELECT c.author_id
       FROM author_link_candidates c
       JOIN author_link_progress p ON p.author_id = c.author_id
      WHERE c.status = 'pending'
        AND c.reason = 'name_guard_failed'
        AND c.evidence_generation = p.evidence_generation
 );
