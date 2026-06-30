-- Drop the never-wired persistent metadata cache table (M-011). The 24h
-- per-provider cache (REQ-009) was never connected to the enrichment pipeline;
-- the live cache is the 5-minute in-memory TransportCache. The table, trait,
-- and impl are all removed in this cleanup pass.

DROP TABLE IF EXISTS metadata_cache;
