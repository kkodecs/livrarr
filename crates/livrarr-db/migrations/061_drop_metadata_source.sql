-- Drop works.metadata_source (REQ-018 / AC-020): a dead column — zero readers
-- and zero writers anywhere in the workspace. Added by 012 for foreign-language
-- provider attribution; superseded by works.language + works.enrichment_source.
-- Plain unindexed TEXT column, safe for DROP COLUMN.
ALTER TABLE works DROP COLUMN metadata_source;
