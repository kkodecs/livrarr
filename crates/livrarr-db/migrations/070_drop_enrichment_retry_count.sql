-- Drop the dead enrichment_retry_count column (#137).
-- Orphaned by the removal of the S6 background enrichment-retry job: no
-- production writer or reader remained — only a reset-to-zero on manual
-- refresh and two never-called trait methods (both removed with this change).
-- Retry/suppression bookkeeping now lives in provider_retry_state and the
-- convergence dead-end counters.
ALTER TABLE works DROP COLUMN enrichment_retry_count;
