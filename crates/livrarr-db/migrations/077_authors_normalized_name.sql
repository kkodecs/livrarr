-- Stored author identity key (issue #175): the Rust-computed
-- canonical_author_key form of authors.name, NULL when the name does not
-- canonicalize. Column only — the UNIQUE index is created by the startup
-- repair (backfill_author_identity) after existing duplicate rows are
-- merged, mirroring the works precedent (migration 038 + startup backfill).
ALTER TABLE authors ADD COLUMN normalized_name TEXT;
