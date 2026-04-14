-- Remove user_id prefix from library_items.path.
-- Old format: "1/Frank Herbert/Dune.epub"
-- New format: "Frank Herbert/Dune.epub"
-- The prefix is "{user_id}/" where user_id is a number.
-- Only strip numeric prefix (before first '/') to avoid corrupting paths like "Frank Herbert/Dune.epub".
UPDATE library_items
SET path = SUBSTR(path, INSTR(path, '/') + 1)
WHERE path LIKE '%/%'
  AND SUBSTR(path, 1, INSTR(path, '/') - 1) GLOB '[0-9]*';

UPDATE _livrarr_meta SET value = '4' WHERE key = 'schema_version';
