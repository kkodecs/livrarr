-- REQ-014: schema_meta-style marker (same pattern as schema_version/
-- data_version, migration 010) tracking whether works.normalized_title/
-- normalized_author have been recomputed under the identity_matching
-- authority's identity_key recipe, replacing the retired
-- normalize_for_matching. This migration seeds the marker only — the
-- recompute logic lives in Rust (livrarr_db::pool::backfill_identity_key_recompute),
-- run idempotently at server startup, which bumps this value once every
-- work has been recomputed.
INSERT INTO _livrarr_meta (key, value) VALUES ('identity_key_generation', '0');
