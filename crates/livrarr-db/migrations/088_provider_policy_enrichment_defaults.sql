-- Enrichment provider priorities (REQ-002/REQ-003/REQ-004). The `ebook` list is
-- the ordinary-metadata order (content AND description); the `audiobook` list is
-- the audio-detail order (narrator, duration, narration type, abridged).
-- Cover ordering is NOT stored here — covers keep their own rank table.
--
-- Migration 057 seeded only the generic ('*') group, and nothing loaded it. This
-- migration writes the requested defaults without disturbing a list an operator
-- already customised. A (language, kind) list is written when it is:
--   * absent entirely — no rows at all. That covers a fresh database, the English
--     groups 057 never seeded, and a generic group whose rows were removed. An
--     empty list is not a choice an operator can express: the loader requires
--     both lists before the server will serve, so leaving it empty would only
--     block startup.
--   * still byte-for-byte the 057 generic seed (ebook = google_books@0;
--     audiobook = audible@0 + audnexus@1), which is a default, not a preference.
-- Every other stored list — any nonempty list, including explicit
-- other-language groups — is left exactly as it is.
-- The targets are decided BEFORE anything is written, so the decision can never
-- see a row this migration inserted.

CREATE TEMP TABLE provider_policy_seed_target (
    language TEXT NOT NULL,
    kind     TEXT NOT NULL
);

-- Generic ebook: absent, or exactly the 057 seed.
INSERT INTO provider_policy_seed_target (language, kind)
SELECT '*', 'ebook'
WHERE (SELECT COUNT(*) FROM provider_policy WHERE language = '*' AND kind = 'ebook') = 0
   OR (
       (SELECT COUNT(*) FROM provider_policy WHERE language = '*' AND kind = 'ebook') = 1
       AND EXISTS (
           SELECT 1 FROM provider_policy
           WHERE language = '*' AND kind = 'ebook'
             AND provider = 'google_books' AND rank = 0
       )
   );

-- Generic audiobook: absent, or exactly the 057 seed.
INSERT INTO provider_policy_seed_target (language, kind)
SELECT '*', 'audiobook'
WHERE (SELECT COUNT(*) FROM provider_policy WHERE language = '*' AND kind = 'audiobook') = 0
   OR (
       (SELECT COUNT(*) FROM provider_policy WHERE language = '*' AND kind = 'audiobook') = 2
       AND EXISTS (
           SELECT 1 FROM provider_policy
           WHERE language = '*' AND kind = 'audiobook'
             AND provider = 'audible' AND rank = 0
       )
       AND EXISTS (
           SELECT 1 FROM provider_policy
           WHERE language = '*' AND kind = 'audiobook'
             AND provider = 'audnexus' AND rank = 1
       )
   );

-- English groups: absent only — 057 never seeded them.
INSERT INTO provider_policy_seed_target (language, kind)
SELECT 'en', 'ebook'
WHERE NOT EXISTS (SELECT 1 FROM provider_policy WHERE language = 'en' AND kind = 'ebook');

INSERT INTO provider_policy_seed_target (language, kind)
SELECT 'en', 'audiobook'
WHERE NOT EXISTS (SELECT 1 FROM provider_policy WHERE language = 'en' AND kind = 'audiobook');

-- Clear only the targeted groups, then write their defaults. Ranks are dense and
-- zero-based; the loader reads them in rank order.
DELETE FROM provider_policy
WHERE EXISTS (
    SELECT 1 FROM provider_policy_seed_target t
    WHERE t.language = provider_policy.language
      AND t.kind = provider_policy.kind
);

-- English ordinary metadata: HC -> GB -> GR -> Readarr -> OL -> Audible.
-- English audio details: Audible -> Audnexus -> HC -> GR -> OL -> GB.
-- Generic (every other language) drops Hardcover and OpenLibrary from BOTH
-- lists: those two are English-centric sources a foreign-language work must
-- never take metadata from (REQ-014/#133), and the merge boundary drops them
-- anyway. Removing them leaves the remaining order unchanged.
INSERT INTO provider_policy (language, kind, provider, rank)
SELECT d.language, d.kind, d.provider, d.rank
FROM (
             SELECT 'en' AS language, 'ebook'     AS kind, 'hardcover'    AS provider, 0 AS rank
    UNION ALL SELECT 'en',            'ebook',            'google_books',           1
    UNION ALL SELECT 'en',            'ebook',            'goodreads',              2
    UNION ALL SELECT 'en',            'ebook',            'readarr',                3
    UNION ALL SELECT 'en',            'ebook',            'open_library',           4
    UNION ALL SELECT 'en',            'ebook',            'audible',                5
    UNION ALL SELECT 'en',            'audiobook',        'audible',                0
    UNION ALL SELECT 'en',            'audiobook',        'audnexus',               1
    UNION ALL SELECT 'en',            'audiobook',        'hardcover',              2
    UNION ALL SELECT 'en',            'audiobook',        'goodreads',              3
    UNION ALL SELECT 'en',            'audiobook',        'open_library',           4
    UNION ALL SELECT 'en',            'audiobook',        'google_books',           5
    UNION ALL SELECT '*',             'ebook',            'google_books',           0
    UNION ALL SELECT '*',             'ebook',            'goodreads',              1
    UNION ALL SELECT '*',             'ebook',            'readarr',                2
    UNION ALL SELECT '*',             'ebook',            'audible',                3
    UNION ALL SELECT '*',             'audiobook',        'audible',                0
    UNION ALL SELECT '*',             'audiobook',        'audnexus',               1
    UNION ALL SELECT '*',             'audiobook',        'goodreads',              2
    UNION ALL SELECT '*',             'audiobook',        'google_books',           3
) AS d
JOIN provider_policy_seed_target t
  ON t.language = d.language AND t.kind = d.kind;

DROP TABLE provider_policy_seed_target;
