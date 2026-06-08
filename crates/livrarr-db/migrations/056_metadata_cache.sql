-- Persistent (work, provider) metadata cache (REQ-009). The enrichment
-- provider-gateway checks this (<= max_age, default 24h) before fetching a
-- provider; a HardRefresh bypasses it. The payload is stored as opaque JSON so
-- livrarr-db never names external-data's NormalizedWorkDetail (db -> domain
-- only); the enrichment cache adapter (de)serializes it.

CREATE TABLE IF NOT EXISTS metadata_cache (
    work_id      INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    provider     TEXT    NOT NULL,
    fetched_at   TEXT    NOT NULL,
    payload_json TEXT    NOT NULL,
    PRIMARY KEY (work_id, provider)
);
