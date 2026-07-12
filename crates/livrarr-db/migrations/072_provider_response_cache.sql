-- Persistent provider-response cache (REQ-009): successful provider detail
-- payloads keyed by (provider, anchor_type, anchor). Background enrichment
-- passes (Freshness::PreferCache) consult this before fetching a provider; a
-- fresh hit costs zero provider HTTP. User-triggered refresh
-- (Freshness::Bypass) always fetches and overwrites the entry. Only
-- successes are ever cached — TTL/freshness/success-only policy lives at the
-- provider-queue dispatch seam, never here.
-- Global table — deliberately no user_id: provider payloads are
-- user-independent public metadata.
CREATE TABLE IF NOT EXISTS provider_response_cache (
    provider     TEXT NOT NULL,
    anchor_type  TEXT NOT NULL,
    anchor       TEXT NOT NULL,
    payload      TEXT NOT NULL,
    fetched_at   TEXT NOT NULL,
    PRIMARY KEY (provider, anchor_type, anchor)
);

-- Oldest-first eviction scan (count-capped store).
CREATE INDEX IF NOT EXISTS idx_provider_response_cache_fetched_at
    ON provider_response_cache(fetched_at);
