-- Per-monitor seed language (sprint-d-seeds-doors REQ-003): the user's language
-- choice for works created by the author monitor / series monitor. NULL = never
-- configured; the seed builder applies the system default at construction.
ALTER TABLE authors ADD COLUMN monitor_language TEXT;
ALTER TABLE series ADD COLUMN monitor_language TEXT;
