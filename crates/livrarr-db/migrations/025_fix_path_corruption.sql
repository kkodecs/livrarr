-- Fix path corruption from migration 004.
-- Migration 004 stripped the first path segment from ALL rows where path LIKE '%/%',
-- but it didn't validate that the first segment was actually a numeric user_id.
-- This could corrupt paths like "Frank Herbert/Dune.epub" (stripped to "Dune.epub").
--
-- This migration re-runs the path correction, but ONLY for rows whose first segment
-- (before the first '/') is entirely numeric, indicating a user_id prefix.

UPDATE library_items
SET path = SUBSTR(path, INSTR(path, '/') + 1)
WHERE INSTR(path, '/') > 0
  AND SUBSTR(path, 1, INSTR(path, '/') - 1) GLOB '[0-9]*';

UPDATE _livrarr_meta SET value = '25' WHERE key = 'schema_version';
