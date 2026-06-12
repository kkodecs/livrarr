-- Persisted GR series rosters (REQ-010, sprint-c-series): the parsed
-- primary-works list of a series page, written by the monitor worker
-- (write-through) and by the first expansion of a never-monitored
-- GR-backed series. Mirrors author_series_cache.
CREATE TABLE series_roster (
    series_id   INTEGER PRIMARY KEY REFERENCES series(id) ON DELETE CASCADE,
    entries     TEXT NOT NULL DEFAULT '[]',
    fetched_at  TEXT NOT NULL DEFAULT (datetime('now'))
);
