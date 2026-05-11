-- Phase 2 of metadata redesign: normalized identity columns for dedup.
-- Backfill performed in Rust startup hook (backfill_normalized_identity).
-- UNIQUE INDEX created by startup hook after backfill + duplicate resolution.
ALTER TABLE works ADD COLUMN normalized_title TEXT NOT NULL DEFAULT '__UNMIGRATED__';
ALTER TABLE works ADD COLUMN normalized_author TEXT NOT NULL DEFAULT '__UNMIGRATED__';
