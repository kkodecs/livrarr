# Roadmap

_Updated: 2026-06-10_

## Current Status: Alpha 5 (released, `v0.1.0-alpha5`)

Path to beta = three themed alpha releases:

- **Alpha 6 — Correctness & reliability.** Ship WCC; fix the metadata/identity/series cluster; encrypt credentials at rest; qBittorrent/Prowlarr; startup + privacy bugs.
- **Alpha 7 — Scale, import & distribution.** Large libraries, watch folder, m4b write completion, seedbox transfer, configurable naming, reverse-proxy, packaging, UX consolidation.
- **Alpha 8 — Intelligence & reader modernization.** LLM help/discovery/tagging, modern reader + audio engines, lists/requests, new sources.
- **Then Beta** — gated on the security audit, deferred hardening, and the README "known gaps".

**Status:** ✅ done (this cycle, awaiting release) · 🔄 in progress / in validation · 📋 planned (design pending) · 🔍 triage (may already be fixed — verify) · ⬜ not started · ⛔ blocked · 🚫 declined

---

## Alpha 6 — Correctness & reliability (next release)

**Lead — ✅ WCC / metadata epic SHIPPED to `main`** (PR #140 merged 2026-06-10, 42 commits). Includes the alpha-6 fix batch: cover-URL 404 guard, per-file match fallback + relaxed language guard, matcher language extraction + normalization, cover ranking + hi-res, chapters-on-import, scan folder-context labels + multi-CD grouping, GB-quota backoff. Also on main: the **canonical architecture model** (`architecture/canonical-model.yaml` — entity spine + seams; gates armed; conformance backlog #141/#143) and **PR CI on native arm64** (the 6h QEMU timeout class is dead; node24 action bumps, release path citest-validated). **Remaining before the alpha-6 cut:** GB re-test once quota resets (#97 validation), land community PRs #113/#136, sweep the 🔍 triage cluster below.

| Status | Area | # | Item |
|:------:|------|---|------|
| ✅ | Metadata | #117 | Manual Refresh now best-effort merge (Manual mode) — a transient GB 429 no longer discards Goodreads data |
| 🔍 | Metadata | #110 | Refresh flips whole work to Conflict on single-provider dissent |
| 🔍 | Metadata | #112 | Foreign-edition series leaks into English bibliography |
| 🔍 | Metadata | #111 | Standalone 'Anathem' shown as a 3-book series |
| 🔍 | Metadata | #109 | Series line should be blank, not a placeholder |
| 🚫 | Metadata | #96 | BCP-47 locale tags misroute English — **audit refuted**; close w/ explanation (issue closes deferred, PO) |
| 🚫 | Covers | #59 | Conflict-status works have no `cover_url` — **audit refuted**; close w/ explanation (deferred, PO) |
| ⬜ | Metadata | #133 | **F1 / critical** — foreign enrichment merges a wrong-language edition (GB `zh` onto `fr`) |
| ⬜ | Covers | #134 | Foreign covers: refresh doesn't re-resolve; `cover_width/height` never captured |
| ⬜ | Metadata | #135 | "Refresh All" ignores the language filter; can 409 |
| ⬜ | Import | #132 | Audiobook manual import: nested works not listed + common titles fail match |
| ⬜ | RSS | #142 | RSS sync auto-grabs wrong releases (candidate-substring fallback fabricates matches) |
| ⬜ | Import | #138 | Manual import may rename imported files unexpectedly |
| ⬜ | Covers | #139 | Cannot upload audiobook covers |
| ⬜ | Cleanup | #137 | Drop dead `enrichment_retry_count` column (orphaned by the S6 retry-job removal) |
| ⬜ | Architecture | #141 | Rename `ReleaseSearchResult` → `Release` (canonical-model conformance; gate dogfood) |
| ⬜ | Architecture | #143 | Route import-time save via `livrarr-materialize` (seam conformance; gate dogfood) |
| 🔍 | Series | #58 | Series workflow: find existing works and map |
| 🔍 | Metadata | #53 | Adding works from author biblio creates junk entries |
| 🔍 | Series | #52 | Series monitoring fails ("Failed to monitor") |
| ⬜ | Security | #118 | Encrypt third-party credentials at rest (committed publicly for next build) |
| ⬜ | Indexers | #116 | qBittorrent background-polling failure |
| 🔄 | Indexers | PR #113 | qBit 5.2 auth — review + land (community PR; CI awaiting approval) |
| 🔄 | Indexers | PR #136 | Prowlarr magnet redirects for qBittorrent — review + land (community PR; CI awaiting approval) |
| ⬜ | Indexers | #130 | Prowlarr indexers report connection issues in Livrarr |
| ⬜ | Privacy | #76 | Log-scrubbing for API keys / tokens / credentials |
| ⬜ | Ops | #102 | "Failed to create log directory" |
| ✅ | Ops | #100 | CI → Node.js 24 — done 2026-06-10 (native arm64 matrix, scoped caches, action bumps; citest-validated; close issue) |
| ⬜ | Status page | #131 | System status shows all metadata providers green unconditionally (health never recorded) |
| ⬜ | Import | #18 | Import fails (triage — may be stale) |
| ⬜ | Import | #114 | "Discovered some issues" (umbrella — distribute or close) |
| ⬜ | UX | #80 | Onboarding modal overlays / blocks functionality |
| ✅ | Docs | — | README refreshed (alpha5 pin, multi-arch, stale limitations removed — committed) |
| ⬜ | Docs | — | Backfill CHANGELOG for alpha 5 |

## Alpha 7 — Scale, import & distribution

| Status | Area | # | Item |
|:------:|------|---|------|
| ⬜ | Import | #107 | Scan pagination for large flat libraries |
| ⬜ | Import | #16 | Watch folder |
| ⬜ | Naming | #120 | Configurable naming & folders (tokens + optional per-language roots) |
| ⛔ | Audiobook | — | m4b streaming tag/cover writer (current libs OOM on large files) |
| ⬜ | Audiobook | #99 | Re-pend items wrongly marked synced via the unsupported path |
| 📋 | Seedbox | #34 | Native SFTP transfer (`russh-sftp`, ~0 image cost; pull-vs-push TBD) |
| ⬜ | Calibre | #121 | Optional "let Calibre manage files" mode (Readarr parity) |
| ⬜ | Proxy | #119 | `url_base` for nginx subfolder hosting |
| ⬜ | Storage | #84 | Resize covers on download to cap on-disk footprint |
| ⬜ | Packaging | #106 | Proxmox LXC |
| ⬜ | Packaging | #105/#101 | Unraid community app |
| ⬜ | Packaging | #103 | Windows binary |
| ⬜ | CLI | #81 | Real headless `livrarr-cli` |
| ⬜ | UX | #122 | Bulk edit / multi-select on works |
| ⬜ | UX | #78 | Merge Queue + History → Activity |
| ⬜ | UX | #79 | Drop Calendar / Cutoff-Unmet pages |
| ⬜ | UX | #26 | Collapse-series toggle |

## Alpha 8 — Intelligence & reader modernization

| Status | Area | # | Item |
|:------:|------|---|------|
| ⬜ | LLM | #108 | In-app AI help chat |
| ⬜ | LLM | #77 | AI error-message translation |
| ⬜ | LLM | #31 | Discovery / suggestions |
| ⬜ | LLM | #28 | Auto-tagging / genre curation |
| ⬜ | Reader | #60 | Replace epub.js / react-reader |
| ⬜ | Reader | #61 | HTML5-Audio alternative for audiobooks |
| 🔄 | Reader | #23 | Cross-format resume **shipped 2026-06-09** (Whispersync-style jump + sleep-timer auto-bookmark over `.kash` links); remaining: KASH generation (`kash_gen`) + rescan link-backfill |
| ⬜ | Library | #27 | Lists / bookshelves |
| ⬜ | Library | #129 | One-click add full author bibliography |
| ⬜ | Requests | #127 | Request system / Seerr integration (approach TBD) |
| ⬜ | Metadata | #126 | Audiobook edition / narrator selection (e.g. GraphicAudio) |

## Defer — post-beta (or beta-gate)

| Status | Area | # | Item |
|:------:|------|---|------|
| ⬜ | Beta gate | #29 | Third-party security audit |
| ⬜ | Clients | #124 | nzbget (committed for beta) |
| ⬜ | Clients | #125 | Deluge (committed for beta) |
| ⬜ | Mobile | #123 | Native apps + Android Auto / CarPlay (large) |
| ⬜ | Integrations | #128 | Zotero (academic books) |
| 📋 | Metadata | — | GR resolution ladder (REQ-018) — proper fix for GR wrong-book on foreign titles (#11) |
| ⛔ | Catalog | #21 | Push metadata to OpenLibrary — OL UA cooperation paused |
| 🚫 | Sources | #104 | Anna's Archive / LibGen / Z-Library — declined (legal exposure) |
| ⬜ | Content | #25 | Magazine support (parked) |
| ⬜ | Content | #24 | Anime / manga, incl. `.cbz` comics (parked) |
| ⬜ | Hardening | — | Cursor pagination · HttpOnly cookies · SSRF resolver pinning · `livrarr doctor` CLI |

## Known gaps (from README "Alpha Limitations")

| Status | Area | Item |
|:------:|------|------|
| ⬜ | Multi-user | Effectively single-user — extra users share the admin's indexers/clients (→ beta) |
| ⬜ | Deploy | PUID/PGID not configurable; runs as UID/GID 1000 (→ beta) |
| 🔄 | Covers | Accuracy varies (Goodreads matches); manual refresh usually fixes it — partly addressed by alpha 6 cover work + the GR ladder |

> README was refreshed this cycle: the "no mobile UI" and "cover trust coming in alpha4" limitations were stale (both shipped) and removed; multi-arch + the alpha5 compose pin were corrected.

---

## Shipped since Alpha 5 (on `main`, unreleased)

- **WCC / metadata epic merged to main** (PR #140, 2026-06-10) — everything below plus the alpha-6 fix batch.
- **Cross-format resume + sleep-timer auto-bookmark** — Whispersync-style ebook↔audiobook position sync over `.kash` links.
- **One-road enrichment (metadata-refactor)** — every add/refresh door funnels through the single pipeline; background retry job removed in favor of user-triggered "Retry Incomplete".
- **Canonical architecture model** — authored entity spine + crate seams in-repo (`architecture/canonical-model.yaml`); forward/reverse/amendments gates armed; first-audit baseline recorded.
- **PR CI on native arm64** — 6h QEMU timeouts → ~12-minute native builds; node24-ready actions across CI + release (citest-validated); repo hygiene (branches/tags/releases shelf).

## Shipped since Alpha 4

- **Metadata modularization** — split into `livrarr-external-data` / `livrarr-identity` / `livrarr-enrichment` crates (behavior-preserving carve).
- **Work Creation Consistency (WCC)** — discovery fan-out across all providers incl. Goodreads autocomplete; cached-payload reuse / instant-add; Tier-A manual-import auto-match (#97); anchored-cluster quorum winner rule; two-state identity badge through API + UI.
- Goodreads discovery via the WAF-free autocomplete endpoint.
- Google Books enrichment + onboarding API-key step (#72).
- Audiobook cover auto-selection (#95); provider fallback chain for add-work search (#73).
- OpenLibrary UA fix (#83); Gemini model auto-migration (#82); metadata status on system-status page (#74).
- Release CI on native arm64 runners (#98); numerous reader / UI / import fixes (#65–#71).

## Release history

### Alpha 4 (`v0.1.0-alpha4`, 2026-04-29)

- SSRF trusted origins for private-IP indexers and download clients
- Manual import dedup improvements; download poller fix for private-IP qBittorrent

### Alpha 3 (`v0.1.0-alpha3`, 2026-04-25)

- Full metadata enrichment pipeline with provenance tracking and merge engine
- Series monitoring via Goodreads; Readarr library import (preview + undo); list imports (Goodreads / Hardcover CSV)
- Built-in EPUB reader, PDF viewer, audiobook player; OPDS catalog
- Send to email (Kindle); foreign-language support (10+ languages)
- RSS sync with auto-grab; handler compile-time isolation (`livrarr-handlers` crate)
- Mobile-responsive UI (all 27 pages); Docker image ~76 MB

## Future Ideas (no issue yet)

- Shared collections across users
- Notification integrations (Discord, Telegram, etc.)
