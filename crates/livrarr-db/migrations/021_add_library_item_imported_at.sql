-- No-op: imported_at was present in 001_initial_schema.sql from the beginning.
-- The ALTER TABLE originally here duplicated a column that already existed, causing
-- fresh installs and alpha2→alpha3 upgrades to fail with "duplicate column name".
-- Migration 022 handles the backfill; this migration is intentionally a no-op.
SELECT 1;
