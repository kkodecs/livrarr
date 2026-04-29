# Changelog

## 0.1.0-alpha4 (2026-04-29)

### Improvements

- **SSRF trusted origins** — user-configured indexers and download clients (including those on private IPs) now work correctly. The SSRF protection maintains a trusted origin allowlist (host:port) derived from configured infrastructure, rebuilt on startup. Untrusted URLs (cover proxy) remain fully protected.
- **Manual import dedup** — scan phase uses shared work dedup function; skips OpenLibrary lookup when a file already matches an existing work in the library
- **Download poller fix** — qBittorrent polling no longer blocked by SSRF when the client is on a private IP or resolves to one via DNS

### Bug Fixes

- Fixed SABnzbd test connection failing on categories check when using a hostname that resolves to a private IP
- Fixed qBittorrent test connection failing on login/version check for the same reason
- Fixed Prowlarr import failing when Prowlarr is on a private network
- Fixed download poller unable to poll qBittorrent via reverse proxy (e.g., `oqbi.petekim.com`)

---

## 0.1.0-alpha3 (2026-04-25)

### New Features

- **Series monitoring** — track book series via Goodreads, auto-add new works when monitored
- **Readarr library import** — three-phase import with preview, undo, and cover download
- **List imports** — bulk import from Goodreads or Hardcover CSV exports
- **File playback** — built-in EPUB reader, PDF viewer, and audiobook player
- **OPDS catalog** — serve your library to any OPDS-compatible reader app
- **Send to email** — push EPUBs to Kindle or other email-based readers
- **Foreign language support** — search and enrich in 10+ languages with per-language providers
- **RSS sync** — automated release discovery with fuzzy matching and auto-grab

### Improvements

- **Metadata overhaul** — new enrichment pipeline with provenance tracking, merge engine, and per-field priority resolution across HC/GR/OL/Audnexus
- **Cover priority HC-first** — Hardcover covers preferred over Goodreads (more reliable matching)
- **GR match safety** — Goodreads enrichment now requires LLM validation to prevent study guide matches
- **Work dedup** — shared dedup logic across all import flows (Readarr, series, manual, list, search add) prevents duplicate works
- **Identity lock** — title and author name locked at add-time, never overwritten by provider enrichment
- **Title case normalization** — search results display proper title capitalization
- **Docker image optimized** — 112MB → 76MB

### Bug Fixes

- Cover cleanup on work delete and Readarr undo (orphaned files removed)
- Series monitor: empty gr_key false match fixed
- Series monitor: works with subtitles now deduplicate correctly
- Series list: HTML source prioritized over book search (proper GR keys)
- BookCover: stop infinite retry loop on missing covers
- Pagination: browser back button works correctly on works page

### Breaking Changes

- Requires migration from alpha2 database (24 new migrations, applied automatically on startup)
- Readarr import is now under Activity in the sidebar (was Settings)

---

## 0.1.0-alpha2 (2026-04-05)

- Core library management, metadata enrichment, download client integration
- Initial Docker deployment

## 0.1.0-alpha1 (2026-03-29)

- First public alpha
