# Changelog

## 0.1.0-alpha6 (unreleased)

### New Features

- **Cross-format resume** — Whispersync-style position sync between an ebook and its audiobook: jump to the furthest position in either format, with sleep-timer auto-bookmarks (#23)
- **Add directly from search** — works are added straight from the search results; covers resolve automatically (no pre-add picker step)
- **Goodreads discovery restored** — book search reaches Goodreads again via its autocomplete endpoint (the old search path is bot-blocked)
- **Identity badge** — works show Verified/Unverified, so you can tell whether the metadata identity was confirmed against a provider

### Improvements

- **Consistent work creation** — search fans out across all providers (Hardcover, OpenLibrary, Google Books, Goodreads, Audnexus) and adding reuses the already-fetched result, so the work you add is exactly the one you picked — and it's instant
- **Manual import auto-match** — high-confidence files match automatically (#97), with per-file fallback and a relaxed language guard for mixed and foreign libraries
- **One enrichment pipeline** — every add and refresh path runs through the same road; the background retry job is replaced by an explicit "Retry Incomplete" action
- **Audiobook scanning** — multi-disc audiobooks group into a single work, scan results carry folder context, and chapters are extracted at import
- **Cover quality** — covers are ranked with a high-resolution preference, and dead cover URLs (404) are no longer saved

### Bug Fixes

- **qBittorrent 5.2 authentication** — accepts the new `QBT_SID_*` session cookies and 2xx responses; fixes the background poller's authentication-failure loop (#116). Contributed by @Jandalslap (#113)
- **Prowlarr magnet redirects** — when an indexer download URL resolves to a magnet, the magnet itself is sent to qBittorrent/Transmission; fixes grabs where the download client can't reach Prowlarr, e.g. behind a VPN. Contributed by @Vandypointe2 (#136)
- Google Books daily-quota exhaustion now backs off cleanly instead of failing enrichment
- Manual Refresh keeps best-effort results — a transient provider error no longer discards data from the providers that succeeded (#117)

### Internal

- Metadata stack split into dedicated crates (external-data / identity / enrichment) with behavior preserved
- Canonical architecture model authored in-repo (entity spine + crate seams) with conformance gates
- Pull-request CI builds arm64 natively (~12 minutes; previously hit 6-hour QEMU timeouts)

---

## 0.1.0-alpha5 (2026-05-28)

### New Features

- **Transmission support** — Transmission joins qBittorrent and SABnzbd as a download client (#17)
- **Google Books enrichment** — foreign-language metadata via Google Books, with an onboarding step for the API key (#72)
- **Trust-aware covers** — multi-source cover system with a picker UI; trusted sources preferred, manual choices stick
- **Playback upgrades** — audiobook chapters, bookmarks, and a full progress lifecycle in the built-in player
- **System status page** — infrastructure health summary with a sidebar indicator (#74)
- **Search fallback chain** — Google-Books-first discovery with an Audible provider and ISBN bridging (#73)

### Improvements

- **Native arm64 release builds** — multi-arch images build natively (#98)
- Language filter on the works page, denser overview, cover play button (#57, #71, #72)
- Poster view shows the series name with a link; recently-downloaded sort option
- qBittorrent grabs fetch the .torrent server-side, so qBittorrent doesn't need to reach the indexer (#88)
- Audiobook covers auto-select by media-aware resolution and priority (#95)
- Gemini default moved to the stable model, deprecated model names auto-migrate (#89)
- 12 hardening fixes from a cross-model audit

### Bug Fixes

- PID-file deadlock on container restart (#86), including the self-deadlock variant found in the first alpha5 image
- qBittorrent add results are read from the response body instead of trusting HTTP 200 (#85)
- Trusted origins rebuild after indexer/download-client changes — no restart needed (#87)
- CJK titles match correctly via bigram tokenization (#93)
- Deleting already-missing library files no longer errors (#94)
- Monitoring toggles work from the works-overview chips (#92)
- EPUB reader: right-arrow advances past the cover page (#91)
- Poster tiles keep equal heights (#90)
- API-key fields show "leave blank to keep" instead of clearing saved keys

### Known Limitations

- Audiobook tag writing (m4b/mp3) is disabled in this release — the underlying writers exhaust memory on large files. EPUB tag writing is unaffected
- The first published alpha5 image had a startup deadlock and an out-of-memory crash on very large audiobook files; the image was re-issued within the hour — `docker compose pull` if you grabbed it early

---

## 0.1.0-alpha4 (2026-04-29)

### Improvements

- **SSRF trusted origins** — user-configured indexers and download clients (including those on private IPs) now work correctly. The SSRF protection maintains a trusted origin allowlist (host:port) derived from configured infrastructure, rebuilt on startup. Untrusted URLs (cover proxy) remain fully protected.
- **Manual import dedup** — scan phase uses shared work dedup function; skips OpenLibrary lookup when a file already matches an existing work in the library
- **Download poller fix** — qBittorrent polling no longer blocked by SSRF when the client is on a private IP or resolves to one via DNS

### Bug Fixes

- Fixed SABnzbd test connection failing on categories check when using a hostname that resolves to a private IP
- Fixed qBittorrent test connection failing on login/version check for the same reason
- Fixed Prowlarr import failing when Prowlarr is on a private network
- Fixed download poller unable to poll qBittorrent via reverse proxy with hostname resolving to private IP

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
