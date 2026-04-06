-- Remove user_id prefix from library_items.path.
-- Old format: "1/Frank Herbert/Dune.epub"
-- New format: "Frank Herbert/Dune.epub"
-- The prefix is "{user_id}/" where user_id is a number.
UPDATE library_items
SET path = SUBSTR(path, INSTR(path, '/') + 1)
WHERE path LIKE '%/%';
