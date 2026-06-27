-- Per-work background-convergence clock (id-completeness): when the work is next
-- due for a convergence pass. NULL = due now. A plain index on
-- (user_id, next_convergence_at) so all three convergence-selection branches can
-- use it; a partial index excluding enriched works would not cover the widened
-- ID-chasing branch (which may select enriched-but-ID-incomplete works).
ALTER TABLE works ADD COLUMN next_convergence_at TEXT;
CREATE INDEX IF NOT EXISTS idx_works_convergence_due ON works(user_id, next_convergence_at);
