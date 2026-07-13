-- The enrichment suppression machinery is retired: ProviderOutcome::Suppressed
-- no longer exists, so no code writes last_outcome = 'suppressed' or the
-- suppression bookkeeping columns. Clear any legacy suppressed rows (a deleted
-- row makes the provider re-eligible on its next enrichment pass — the
-- conservative recovery) and drop the dead columns.
DELETE FROM provider_retry_state WHERE last_outcome = 'suppressed';
ALTER TABLE provider_retry_state DROP COLUMN suppressed_passes;
ALTER TABLE provider_retry_state DROP COLUMN first_suppressed_at;
