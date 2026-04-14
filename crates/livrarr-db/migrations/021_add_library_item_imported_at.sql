-- imported_at was present in 001_initial_schema.sql from alpha1 onward.
-- This migration was added in error (the column was never missing on any real install).
-- Making it a no-op keeps the migration history intact without breaking fresh installs.
SELECT 1;
