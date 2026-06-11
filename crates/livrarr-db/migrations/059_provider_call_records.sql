-- Per-provider call records (REQ-001): one row per fetch attempt across
-- lookup/identity/enrich/cover — whether network, cache-served, or skipped.
-- Append-only log; the server's retention sweep evicts oldest-first at the
-- 30d/100k bounds. System-scoped: provider telemetry carries no user data, so
-- there is no user_id column. work_id is informational, not a FK — records
-- outlive the works they describe.
CREATE TABLE IF NOT EXISTS provider_call_records (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    provider    TEXT    NOT NULL,
    operation   TEXT    NOT NULL,
    work_id     INTEGER,
    started_at  TEXT    NOT NULL,
    duration_ms INTEGER NOT NULL,
    outcome     TEXT    NOT NULL,
    detail      TEXT
);

-- 24h-window scans and oldest-first eviction.
CREATE INDEX IF NOT EXISTS idx_provider_call_records_started_at
    ON provider_call_records(started_at);
-- Per-provider aggregates over the window.
CREATE INDEX IF NOT EXISTS idx_provider_call_records_provider_started
    ON provider_call_records(provider, started_at);
