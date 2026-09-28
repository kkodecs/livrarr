-- In-place identity upgrade. A library that has books while the identity
-- authority marker ('identity_authority_v2') is not active is converted from its
-- legacy identifier columns: routes and editions, the user's confirmed picks,
-- one primary contributor per book, identity status, separated look-alikes and
-- closed legacy match questions. It ends with a ready automatic apply run, which
-- the startup readiness check activates. Every identity statement is gated on
-- the m091_go decision row, so a fresh or already-active library is unchanged.
-- The audit actor is 'identity-upgrade'. schema_version stays 83.

-- 0. Audiobook cover choices. run_migrations saves the (user, work) pairs of
--    works.audiobook_cover_trust = 'user' before migration 084 drops that
--    column; 085 added audiobook_cover_manual defaulting to 0. Not gated: with
--    no saved list nothing changes. The EXISTS form is deliberate; a row-value
--    IN over json_each matched no rows.
UPDATE works SET audiobook_cover_manual = 1
 WHERE EXISTS (
       SELECT 1 FROM json_each((SELECT value FROM _livrarr_meta
                                 WHERE key = 'upgrade_audiobook_cover_user_choices')) AS j
        WHERE json_extract(j.value, '$.u') = works.user_id
          AND json_extract(j.value, '$.w') = works.id);
DELETE FROM _livrarr_meta WHERE key = 'upgrade_audiobook_cover_user_choices';

-- 1. The decision, the timestamp and the whitespace set, fixed once.
CREATE TEMP TABLE m091_go AS
SELECT 1 AS go
 WHERE EXISTS (SELECT 1 FROM works)
   AND NOT EXISTS (SELECT 1 FROM _livrarr_meta
                    WHERE key = 'identity_authority_v2' AND value = 'active');

-- RFC 3339 with an explicit offset, as chrono writes it; route readers parse
-- observed_at with parse_from_rfc3339.
CREATE TEMP TABLE m091_now AS
SELECT strftime('%Y-%m-%dT%H:%M:%f', 'now') || '+00:00' AS ts;

-- The 25 code points of Rust's char::is_whitespace. trim(x, ws) equals the
-- legacy staging's str::trim; SQLite's one-argument trim removes spaces only.
-- Applied to identifier and anchor values, never to the identity key columns.
CREATE TEMP TABLE m091_ws AS
SELECT char(9, 10, 11, 12, 13, 32, 133, 160, 5760,
            8192, 8193, 8194, 8195, 8196, 8197, 8198, 8199, 8200, 8201, 8202,
            8232, 8233, 8239, 8287, 12288) AS c;

-- 2. Pending review cards block activation; only an abandoned manual cutover
--    attempt can have left them.
CREATE TEMP TABLE m091_cards AS
SELECT id FROM identity_review_cards
 WHERE status = 'pending' AND EXISTS (SELECT 1 FROM m091_go);
UPDATE identity_review_cards
   SET status = 'cancelled', resolved_at = (SELECT ts FROM m091_now)
 WHERE id IN (SELECT id FROM m091_cards);

-- 3. Identity key columns: the legacy staging's expressions, except that
--    primary_author_id is taken only from an author row that exists.
UPDATE works SET
    normalized_identity_main =
        CASE WHEN trim(normalized_identity_main) = ''
                   OR normalized_identity_main = '__UNMIGRATED__'
             THEN normalized_title ELSE normalized_identity_main END,
    normalized_identity_subtitle =
        COALESCE(NULLIF(trim(normalized_identity_subtitle), ''),
                 lower(trim(COALESCE(subtitle, '')))),
    normalized_identity_volume =
        COALESCE(NULLIF(trim(normalized_identity_volume), ''),
                 NULLIF(trim(identity_volume), ''),
                 CASE WHEN series_position IS NULL THEN ''
                      ELSE CAST(series_position AS TEXT) END),
    primary_author_id = COALESCE(primary_author_id,
        (SELECT a.id FROM authors a WHERE a.id = works.author_id)),
    text_distinction = COALESCE(NULLIF(trim(text_distinction), ''), 'common')
 WHERE EXISTS (SELECT 1 FROM m091_go);

-- 4. Look-alike books stay separate. Books sharing all six columns of the
--    activation unique index form a group; every member but the lowest id gets
--    its own distinction. Books without an author never collide (NULL key).
CREATE TEMP TABLE m091_groups AS
SELECT user_id, normalized_identity_main AS m, normalized_identity_subtitle AS s,
       normalized_identity_volume AS v, primary_author_id AS a, text_distinction AS d,
       MIN(id) AS kept_work_id
  FROM works
 WHERE primary_author_id IS NOT NULL AND EXISTS (SELECT 1 FROM m091_go)
 GROUP BY user_id, normalized_identity_main, normalized_identity_subtitle,
          normalized_identity_volume, primary_author_id, text_distinction
HAVING COUNT(*) > 1;
CREATE TEMP TABLE m091_lookalikes AS
SELECT w.user_id, w.id AS work_id, g.kept_work_id
  FROM works w
  JOIN m091_groups g
    ON g.user_id = w.user_id AND g.m = w.normalized_identity_main
   AND g.s = w.normalized_identity_subtitle AND g.v = w.normalized_identity_volume
   AND g.a = w.primary_author_id AND g.d = w.text_distinction
 WHERE w.id <> g.kept_work_id;
UPDATE works SET text_distinction = 'upgrade:separate:' || id
 WHERE id IN (SELECT work_id FROM m091_lookalikes);
INSERT INTO identity_audit_events (user_id, work_id, event_kind, actor, payload, created_at)
SELECT user_id, work_id, 'upgrade-kept-separate', 'identity-upgrade',
       'kept separate from work ' || kept_work_id, (SELECT ts FROM m091_now)
  FROM m091_lookalikes
 ORDER BY user_id, work_id;

-- 5. Legacy identifiers become routes, with the staging's column mapping and
--    encodings. A value that is empty after normalization makes nothing. The
--    owner is an existing active route's book, else the lowest book id.
--    Goodreads, ISBN-13 and ASIN routes are owned by one new edition each.
CREATE TEMP TABLE m091_candidates AS
SELECT * FROM (
    SELECT user_id, id AS work_id, 1 AS col_rank, 'ol_key' AS legacy_field,
           '"OpenLibrary"' AS provider, '"OpenLibraryWork"' AS kind, 'work' AS owner_type,
           trim(ol_key, (SELECT c FROM m091_ws)) AS value
      FROM works
    UNION ALL
    SELECT user_id, id, 2, 'hc_key', '"Hardcover"', '"HardcoverWork"', 'work',
           trim(hc_key, (SELECT c FROM m091_ws))
      FROM works
    UNION ALL
    SELECT user_id, id, 3, 'gr_key', '"Goodreads"', '"GoodreadsBookEdition"', 'edition',
           trim(gr_key, (SELECT c FROM m091_ws))
      FROM works
    UNION ALL
    SELECT user_id, id, 4, 'isbn_13', '"IsbnRegistry"', '"Isbn13Edition"', 'edition',
           trim(isbn_13, (SELECT c FROM m091_ws))
      FROM works
    UNION ALL
    SELECT user_id, id, 5, 'asin', '"Amazon"', '"AsinEdition"', 'edition',
           trim(asin, (SELECT c FROM m091_ws))
      FROM works
)
 WHERE value <> '' AND EXISTS (SELECT 1 FROM m091_go);

CREATE TEMP TABLE m091_owners AS
SELECT c.user_id, c.provider, c.kind, c.value, c.owner_type, c.legacy_field, c.col_rank,
       (SELECT r.resolved_work_id FROM identity_routes r
         WHERE r.user_id = c.user_id AND r.provider = c.provider AND r.kind = c.kind
           AND r.provider_scoped_id = c.value AND r.state = 'active'
         ORDER BY r.id LIMIT 1) AS routed_work_id,
       MIN(c.work_id) AS first_work_id
  FROM m091_candidates c
 GROUP BY c.user_id, c.provider, c.kind, c.value, c.owner_type, c.legacy_field, c.col_rank;

INSERT INTO editions (user_id, work_id, format, source_provider, provider_edition_id, state)
SELECT user_id, first_work_id, '"Unknown"', provider, value, 'active'
  FROM m091_owners
 WHERE routed_work_id IS NULL AND owner_type = 'edition'
 ORDER BY user_id, first_work_id, col_rank;

INSERT INTO identity_routes
    (user_id, owner_type, work_id, edition_id, resolved_work_id, provider, kind,
     provider_scoped_id, state, provenance, user_confirmed, observed_at)
SELECT o.user_id, o.owner_type,
       CASE WHEN o.owner_type = 'work' THEN o.first_work_id END,
       CASE WHEN o.owner_type = 'edition' THEN
           (SELECT MAX(e.id) FROM editions e
             WHERE e.user_id = o.user_id AND e.work_id = o.first_work_id
               AND e.source_provider = o.provider AND e.provider_edition_id = o.value
               AND e.state = 'active') END,
       o.first_work_id, o.provider, o.kind, o.value, 'active',
       '{"Migrated":{"legacy_field":"' || o.legacy_field || '"}}', 0,
       (SELECT ts FROM m091_now)
  FROM m091_owners o
 WHERE o.routed_work_id IS NULL
 ORDER BY o.user_id, o.first_work_id, o.col_rank;

-- 6. A shared identifier stays with the book that owns it. Every other book
--    holding it gets no route and no edition, and one audit event naming the
--    normalized value and the book that kept it.
CREATE TEMP TABLE m091_shared AS
SELECT c.user_id, c.work_id, c.col_rank, c.legacy_field, c.value,
       COALESCE(o.routed_work_id, o.first_work_id) AS kept_work_id
  FROM m091_candidates c
  JOIN m091_owners o
    ON o.user_id = c.user_id AND o.provider = c.provider AND o.kind = c.kind
   AND o.value = c.value
 WHERE c.work_id <> COALESCE(o.routed_work_id, o.first_work_id);
INSERT INTO identity_audit_events (user_id, work_id, event_kind, actor, payload, created_at)
SELECT user_id, work_id, 'upgrade-shared-identifier', 'identity-upgrade',
       legacy_field || '=' || value || ' kept on work ' || kept_work_id,
       (SELECT ts FROM m091_now)
  FROM m091_shared
 ORDER BY user_id, work_id, col_rank;

-- 7. The user's pick survives: a migrated route is confirmed when the same book
--    has a user-set, confirmed anchor of the matching type whose normalized
--    value equals the route value.
UPDATE identity_routes SET user_confirmed = 1
 WHERE EXISTS (SELECT 1 FROM m091_go)
   AND state = 'active' AND user_confirmed = 0
   AND EXISTS (
       SELECT 1 FROM work_identity_anchors a
        WHERE a.work_id = identity_routes.resolved_work_id
          AND a.setter = 'user' AND a.confidence = 'confirmed'
          AND trim(a.anchor_value, (SELECT c FROM m091_ws)) = identity_routes.provider_scoped_id
          AND a.anchor_type = CASE identity_routes.provenance
                WHEN '{"Migrated":{"legacy_field":"ol_key"}}'  THEN 'ol_work'
                WHEN '{"Migrated":{"legacy_field":"hc_key"}}'  THEN 'hc_work'
                WHEN '{"Migrated":{"legacy_field":"gr_key"}}'  THEN 'gr_work'
                WHEN '{"Migrated":{"legacy_field":"isbn_13"}}' THEN 'isbn_13'
                WHEN '{"Migrated":{"legacy_field":"asin"}}'    THEN 'asin'
              END);

-- 8. One ordinal-0 contributor, the primary author, for a book that has none.
INSERT INTO work_contributors (user_id, work_id, author_id, ordinal)
SELECT w.user_id, w.id, w.primary_author_id, 0
  FROM works w
 WHERE EXISTS (SELECT 1 FROM m091_go)
   AND w.primary_author_id IS NOT NULL
   AND EXISTS (SELECT 1 FROM authors a WHERE a.user_id = w.user_id AND a.id = w.primary_author_id)
   AND NOT EXISTS (SELECT 1 FROM work_contributors c
                    WHERE c.user_id = w.user_id AND c.work_id = w.id);

-- 9. Status from the book's active routes.
UPDATE works SET identity_status_v2 = CASE
    WHEN EXISTS (SELECT 1 FROM identity_routes r
                  WHERE r.user_id = works.user_id AND r.resolved_work_id = works.id
                    AND r.state = 'active' AND r.user_confirmed = 1) THEN 'user_confirmed'
    WHEN EXISTS (SELECT 1 FROM identity_routes r
                  WHERE r.user_id = works.user_id AND r.resolved_work_id = works.id
                    AND r.state = 'active') THEN 'connected'
    ELSE 'not_connected' END
 WHERE EXISTS (SELECT 1 FROM m091_go);

-- 10. Old unanswered match questions are closed keeping the current match. The
--     audit event carries the question's payload; no user decision is recorded.
CREATE TEMP TABLE m091_questions AS
SELECT id, user_id, existing_work_id, incoming_payload_json
  FROM work_identity_conflicts
 WHERE status = 'open' AND EXISTS (SELECT 1 FROM m091_go);
INSERT INTO identity_audit_events (user_id, work_id, event_kind, actor, payload, created_at)
SELECT user_id, existing_work_id, 'upgrade-legacy-conflict-closed', 'identity-upgrade',
       incoming_payload_json, (SELECT ts FROM m091_now)
  FROM m091_questions
 ORDER BY id;
UPDATE work_identity_conflicts
   SET status = 'dismissed', resolved_at = (SELECT ts FROM m091_now),
       resolution_notes = 'closed by identity upgrade; existing match kept'
 WHERE id IN (SELECT id FROM m091_questions);

-- 11. Route clashes left pending by an abandoned manual attempt: the route
--     stays on the book that has it.
CREATE TEMP TABLE m091_clashes AS
SELECT id FROM identity_conflicts_v2
 WHERE status = 'pending' AND EXISTS (SELECT 1 FROM m091_go);
UPDATE identity_conflicts_v2
   SET status = 'resolved', resolution = 'identity upgrade: kept on first book'
 WHERE id IN (SELECT id FROM m091_clashes);

-- 12. A ready automatic apply run and its report, in the shape the readiness
--     check reads: latest apply run 'ready', index_ready 1, blocker_count 0.
--     Fingerprints are zero-filled; report_json is the upgrade summary.
INSERT INTO identity_cutover_runs
    (mode, branch, source_schema_version, source_fingerprint,
     canonical_output_fingerprint, status, report_json, created_at, updated_at)
SELECT 'apply', 'automatic',
       CAST((SELECT value FROM _livrarr_meta WHERE key = 'schema_version') AS INTEGER),
       zeroblob(32), zeroblob(32), 'ready',
       json_object(
           'books', (SELECT COUNT(*) FROM works),
           'identifiers', (SELECT COUNT(*) FROM identity_routes WHERE state = 'active'),
           'confirmed_identifiers', (SELECT COUNT(*) FROM identity_routes
                                      WHERE state = 'active' AND user_confirmed = 1),
           'lookalike_books_kept_separate', (SELECT COUNT(*) FROM m091_lookalikes),
           'shared_identifiers', (SELECT COUNT(*) FROM m091_shared),
           'old_questions_closed', (SELECT COUNT(*) FROM m091_questions),
           'review_cards_cancelled', (SELECT COUNT(*) FROM m091_cards),
           'pending_clashes_resolved', (SELECT COUNT(*) FROM m091_clashes)),
       (SELECT ts FROM m091_now), (SELECT ts FROM m091_now)
 WHERE EXISTS (SELECT 1 FROM m091_go);
INSERT INTO identity_cutover_reports
    (run_id, source_schema_version, source_fingerprint, canonical_output_fingerprint,
     mapped_route_count, edition_count, blocker_count, index_ready, trivially_empty)
SELECT r.id, r.source_schema_version, r.source_fingerprint, r.canonical_output_fingerprint,
       (SELECT COUNT(*) FROM identity_routes WHERE state = 'active'),
       (SELECT COUNT(*) FROM editions), 0, 1, 0
  FROM identity_cutover_runs r
 WHERE r.id = (SELECT MAX(id) FROM identity_cutover_runs
                WHERE mode = 'apply' AND branch = 'automatic' AND status = 'ready'
                  AND created_at = (SELECT ts FROM m091_now))
   AND EXISTS (SELECT 1 FROM m091_go);

-- 13. Temporary tables.
DROP TABLE temp.m091_clashes;
DROP TABLE temp.m091_questions;
DROP TABLE temp.m091_shared;
DROP TABLE temp.m091_owners;
DROP TABLE temp.m091_candidates;
DROP TABLE temp.m091_lookalikes;
DROP TABLE temp.m091_groups;
DROP TABLE temp.m091_cards;
DROP TABLE temp.m091_ws;
DROP TABLE temp.m091_now;
DROP TABLE temp.m091_go;
