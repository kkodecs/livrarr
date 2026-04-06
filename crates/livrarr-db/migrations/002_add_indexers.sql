-- Add Torznab indexer support (DEFERRED-001)
-- Replaces prowlarr_config singleton with per-indexer configuration.
-- prowlarr_config table left in place (deprecated, no longer read by code).

CREATE TABLE indexers (
    id                        INTEGER PRIMARY KEY AUTOINCREMENT,
    name                      TEXT NOT NULL,
    url                       TEXT NOT NULL,
    api_path                  TEXT NOT NULL DEFAULT '/api',
    api_key                   TEXT,
    categories                TEXT NOT NULL DEFAULT '[7020,3030]',
    priority                  INTEGER NOT NULL DEFAULT 25,
    enable_automatic_search   INTEGER NOT NULL DEFAULT 1,
    enable_interactive_search INTEGER NOT NULL DEFAULT 1,
    supports_book_search      INTEGER NOT NULL DEFAULT 0,
    enabled                   INTEGER NOT NULL DEFAULT 1,
    added_at                  TEXT NOT NULL DEFAULT (datetime('now'))
);
