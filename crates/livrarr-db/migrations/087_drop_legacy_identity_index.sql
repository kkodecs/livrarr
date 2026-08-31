-- The legacy pre-F2 work-identity dedup index. It was created at runtime by the
-- retired startup backfill (never by a migration), and the cutover ceremony used
-- to drop it at activation. Dropping it here makes the upgrade path explicit and
-- keeps the dead-symbol build gate and the schema in agreement.
DROP INDEX IF EXISTS idx_works_identity;
