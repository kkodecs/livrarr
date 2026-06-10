-- Provider selection policy by language (REQ-003/REQ-014). Each language has two
-- self-contained priority lists (ebook, audiobook); a provider's rank orders it
-- within one list. DB is the source of truth; the server loads it into an
-- in-memory snapshot at startup and atomically swaps on edit. The composite PK
-- enforces "no provider appears twice within one (language, kind) list" (AC-015)
-- at the storage layer. No CHECK constraint on the enum columns (kind, provider):
-- validity is enforced in Rust (altering a CHECK requires a full table rebuild).

CREATE TABLE IF NOT EXISTS provider_policy (
    language TEXT    NOT NULL,
    kind     TEXT    NOT NULL,
    provider TEXT    NOT NULL,
    rank     INTEGER NOT NULL,
    PRIMARY KEY (language, kind, provider)
);

-- The generic row (language '*'): the standalone policy for any language without
-- its own row. It excludes Hardcover and OpenLibrary so a foreign-language work
-- is never enriched from those (REQ-014). Google Books covers foreign ebook
-- metadata; Audible + Audnexus cover audiobooks.
INSERT INTO provider_policy (language, kind, provider, rank) VALUES
    ('*', 'ebook',     'google_books', 0),
    ('*', 'audiobook', 'audible',      0),
    ('*', 'audiobook', 'audnexus',     1)
ON CONFLICT (language, kind, provider) DO NOTHING;
