-- Phase 2 of metadata redesign: per-item tag sync tracking on library_items.
-- Tag convergence sweep (Phase 7) picks up pending items once works are enriched.
ALTER TABLE library_items ADD COLUMN tag_status TEXT NOT NULL DEFAULT 'pending';
ALTER TABLE library_items ADD COLUMN tagged_at_generation INTEGER NOT NULL DEFAULT 0;
