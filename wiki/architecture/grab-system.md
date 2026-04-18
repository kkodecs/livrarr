# Grab System

The grab system handles release discovery, download initiation, and download tracking. Spans `livrarr-download` (client APIs) and `livrarr-server` (orchestration).

## Components

### Indexer System

Accepts any Torznab/Newznab URL directly (url + api_path + api_key). Prowlarr is optional — the system works with direct indexer configuration. Resolved from DEFERRED-001.

### Release Search

1. User or RSS sync triggers search
2. Query sent to configured indexers (Torznab XML API)
3. Results parsed, filtered, scored
4. Presented to user (manual) or auto-grabbed (RSS sync)

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
