-- Phase 2 of metadata redesign: collapse 7-state EnrichmentStatus to 4-state.
-- Pending/Partial -> Unenriched; Skipped/Exhausted -> Failed.
UPDATE works SET enrichment_status = 'unenriched' WHERE enrichment_status IN ('pending', 'partial');
UPDATE works SET enrichment_status = 'failed' WHERE enrichment_status IN ('exhausted', 'skipped');
