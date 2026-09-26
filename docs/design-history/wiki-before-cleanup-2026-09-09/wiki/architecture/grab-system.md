# Grab System

The grab system handles release discovery, download initiation, and download tracking. Spans `livrarr-download` (client APIs) and `livrarr-server` (orchestration).

## Components

### Indexer System

Accepts any Torznab/Newznab URL directly (url + api_path + api_key). Prowlarr is optional — the system works with direct indexer configuration. Resolved from DEFERRED-001.

### Release Search

1. A user triggers the search. **RSS sync does not use this path** — it has its own feed
   fetch with no query (`crates/livrarr-metadata/src/rss_sync_workflow.rs:786-807`), and only
   the two paths' *grab* step converges. See [rss-sync](rss-sync.md).
2. Query sent to indexers with interactive search enabled, in parallel
   (`crates/livrarr-download/src/release_service.rs:73-77`, `:132-141`)
3. Torznab XML parsed; items missing a `guid` or download URL are dropped with a warning
   (`:213-230`); results deduped by `(guid, indexer)` (`:295-297`) and sorted — torrents
   before usenet, torrents by seeders then size, usenet by date then size (`:299-319`).
   **Nothing is scored here**; scoring belongs to the RSS match step, not to search.
4. Presented to the user

### Download Clients

- **qBittorrent** — primary. API v2 client with session management (cookie cache, 403 re-auth retry, config-update invalidation).
- **SABnzbd** — Usenet. Caution: `search=<nzo_id>` searches by name, not ID.

### Grab Flow

1. User or automation selects a release
2. Torrent/NZB sent to download client
3. Grab record created (user-scoped, tracks status)
4. Download poller (60s interval) monitors progress
5. On completion: triggers import pipeline

## Import Lock

Key: `(user_id, work_id)` — not per-grab. Prevents filesystem races when multiple grabs complete for the same work simultaneously.

## Orphan File Adoption

On retry: if target file exists but no DB record, adopt the file instead of re-importing. Handles crash recovery gracefully.
