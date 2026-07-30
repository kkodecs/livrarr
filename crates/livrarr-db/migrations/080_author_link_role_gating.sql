-- Which book a parked author question was raised on. Nullable and released on
-- deletion: the question outlives its evidence, the title just stops being
-- shown.
ALTER TABLE author_link_candidates
    ADD COLUMN evidence_work_id INTEGER REFERENCES works(id) ON DELETE SET NULL;

-- Every author currently holding a name-guard question raised before roles were
-- read. Clearing the evaluated fingerprint is what makes the next pass a full
-- re-walk: the road sees changed evidence, opens a new generation, and the
-- generation write supersedes these questions before any provider is called.
--
-- Targeted on purpose. Authors with no such question converge on their own
-- schedule, and a library-wide requeue would put every author in front of the
-- provider queue at once.
UPDATE author_link_progress
   SET state = 'queued',
       next_attempt_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
       updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
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
