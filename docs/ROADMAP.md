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

**Lead — ✅ WCC / metadata epic SHIPPED to `main`** (PR #140 merged 2026-06-10, 42 commits). Includes the alpha-6 fix batch: cover-URL 404 guard, per-file match fallback + relaxed language guard, matcher language extraction + normalization, cover ranking + hi-res, chapters-on-import, scan folder-context labels + multi-CD grouping, GB-quota backoff. Also on main: the **canonical architecture model** (`architecture/canonical-model.yaml` — entity spine + seams; gates armed) and **PR CI on native arm64** (6h QEMU timeout class dead; node24 actions, citest-validated).

Open items are grouped into **sprints** (settled with the PO, 2026-06-10). Triage statuses below are grounded in the metadata-lifecycle audit (`docs/metadata-lifecycle-audit-scout.md` + `-deep-passes.md`, verdict IDs DP1–DP5 / fix backlog F1–F8) — not guesses.

### Sprint A — Cut the release (days, release-blocking)

| Status | Area | # | Item |
|:------:|------|---|------|
| 🔄 | Validation | #97 | GB re-test now that quota is reset — confirm the merged manual-import fix batch end-to-end, then close |
| 🔍 | Import | #132 | Verify-then-close: the merged fix batch shipped the grouping + folder-label halves and the match fallback; re-test nested audiobooks + common titles with GB up |
| 🔄 | Indexers | PR #113 | qBit 5.2 auth — approve CI, review, land (community PR; likely also closes #116 — verify) |
| 🔄 | Indexers | PR #136 | Prowlarr magnet redirects for qBittorrent — approve CI, review, land (community PR) |
| ⬜ | Docs | — | Backfill CHANGELOG for alpha 5; write alpha-6 notes |
| ⬜ | Release | — | Cut `v0.1.0-alpha6` (release.yml node24 path already citest-validated); close resolved issues with explanations |
| ⬜ | Perf | — | **Speed baseline capture** (1 day, feeds Sprint E): time add-work / enrichment-per-provider / refresh / bulk-refresh on the live library, before any fixes change the numbers |

### Sprint B — Metadata correctness core (audit F1/F3/F4/F5)

First feature(s) through the armed gates — report gate friction verbatim (insight 48). #141/#143 are the deliberate warm-ups.

| Status | Area | # | Item |
|:------:|------|---|------|
| ⬜ | Architecture | #141 | Rename `ReleaseSearchResult` → `Release` (conformance; gate-dogfood warm-up) |
| ⬜ | Architecture | #143 | Route import-time save via `livrarr-materialize` (seam conformance; warm-up) |
| ⬜ | Metadata | #133 | **F1 / critical** — foreign wrong-language merge. Audit DP5: the cached-path fix is incomplete; the network/refresh path has **no language guard at all** (English HC/OL values win empty fields on foreign works) |
| ⬜ | Metadata | — | **F3** — stop stamping query-language onto GR autocomplete hits (`lookup_goodreads`); root of #11 / `三体`→`es` (narrow fix, distinct from the deferred GR ladder) |
| ⬜ | Metadata | — | **F4** — COALESCE all anchor keys in `apply_enrichment_merge` (gr_key NULLed on 82/120 enriched works) + defensive `cover_url` COALESCE (latent footgun, DP3) |
| ⬜ | Metadata | #110 | **F5** — per-field/per-provider conflict instead of whole-work block (one dissent currently discards every merged field; confirmed by scout + DP1) |
| ⬜ | Metadata | #135 | "Refresh All": add language to `WorkFilter`; replace the held-for-whole-loop HashSet flag (panic = permanent 409) |
| ⬜ | Covers | #134 | **F7** — wire `update_cover_dimensions` (writer exists, zero callers); cover re-resolve on refresh; decide phantom `metadata_source` (wire or delete) |

### Sprint C — Series reconcile (audit F6: 3 sharp roots)

Series entity + FK exist and work; **0 of 125 works use them** (only the GR series-monitor flow writes them; 106 works carry orphan `series_name` strings).

| Status | Area | # | Item |
|:------:|------|---|------|
| ⬜ | Series | #58 | Back-fill `series_id` from `series_name` (the prerequisite — Library→Series renders empty today) |
| ⬜ | Series | #112 | Canonical, language-aware series key (exact-string grouping leaks foreign editions / drops variants) |
| ⬜ | Series | #111 | Authoritative `book_count` (today synthesized from search-result repetition; empty `gr_key` makes these unmonitorable) |
| ⬜ | Series | #52 | Series monitoring — sound FK traversal, but meaningless until the back-fill lands (depends on #58) |
| ⬜ | Series | #109 | Blank series line instead of placeholder (display nit; ride along) |

### Sprint D — Seeds & doors (audit F2) + cleanup (F8)

| Status | Area | # | Item |
|:------:|------|---|------|
| ⬜ | Metadata | — | **F2** — single `SeedBuilder`; kill the 4× hardcoded `language="en"` doors (list, author-monitor, series, Add-box); each door derives language from its source |
| ⬜ | Metadata | #53 | Author-biblio junk entries — quality screen on author-monitor adds (today every "eligible" OL entry lands as Confirmed) |
| ⬜ | Cleanup | — | **F8** — delete the dead `MetadataProvider` trait (zero impls); begin the `work_service.rs` god-object split (3,383 lines; do last) |

### Sprint E — Metadata speed: testing & optimization (NEW)

Measure first, then tune — against the Sprint-A baseline so before/after is real.

| Status | Area | # | Item |
|:------:|------|---|------|
| ⬜ | Perf | — | Test harness: timed scenarios per door (add, refresh, bulk refresh, manual-import batch) + per-provider latency/budget burn; repeatable against the dev library |
| ⬜ | Perf | — | Pacing/budget tuning: GB daily budget + reason-aware 429 backoff, foreground-beats-background verification, per-provider rates (the R1 research-shelf scope) |
| ⬜ | Perf | — | Cache effectiveness: 24h `(work, provider)` cache hit-rate; skip-already-enriched sweep coverage |
| ⬜ | Status page | #131 | Provider health recording — the pacing gate is supposed to write outcomes to the status page; today nothing is written (also fixes "all providers green unconditionally") |

### Sprint F — Acquisition & ops correctness

| Status | Area | # | Item |
|:------:|------|---|------|
| ⬜ | RSS | #142 | RSS sync auto-grabs wrong releases (candidate-substring fallback fabricates matches) |
| ⬜ | Indexers | #116 | qBittorrent background-polling failure (verify against landed PR #113 first) |
| ⬜ | Indexers | #130 | Prowlarr indexers report connection issues in Livrarr |
| ⬜ | Import | #138 | Manual import may rename imported files unexpectedly |
| ⬜ | Covers | #139 | Cannot upload audiobook covers |
| ⬜ | Cleanup | #137 | Drop dead `enrichment_retry_count` column (orphaned by the S6 retry-job removal) |
| ⬜ | Ops | #102 | "Failed to create log directory" |
| ⬜ | Security | #118 | Encrypt third-party credentials at rest (committed publicly for next build) |
| ⬜ | Privacy | #76 | Log-scrubbing for API keys / tokens / credentials |
| ⬜ | UX | #80 | Onboarding modal overlays / blocks functionality |
| ⬜ | Import | #18 | Import fails (triage — may be stale) |
| ⬜ | Import | #114 | "Discovered some issues" (umbrella — ask for specifics, distribute or close) |

### Resolved / refuted this cycle (close with explanations at the Sprint-A cut)

| Status | Area | # | Item |
|:------:|------|---|------|
| ✅ | Metadata | #117 | Manual Refresh best-effort merge — a transient GB 429 no longer discards Goodreads data |
| ✅ | Ops | #100 | CI → Node.js 24 — done 2026-06-10 (native arm64 matrix, scoped caches, action bumps; citest-validated) |
| ✅ | Docs | — | README refreshed (alpha5 pin, multi-arch, stale limitations removed) |
| 🚫 | Metadata | #96 | BCP-47 misroute — **refuted (DP5)**: every `works.language` write is normalized upstream; test DB holds zero region tags |
| 🚫 | Covers | #59 | Cover-NULL on conflict — **refuted (DP3)**: nothing nulls covers; held works never *reach* cover resolution. Product intent already settled by metadata-refactor REQ-015 (covers independent of identity) — verify implemented, then close |

**Two product calls the audit surfaced — both already answered by the metadata-refactor spec:** (a) covers for held/unverified works → **yes** (REQ-015: cover resolution is not gated behind identity), pending an implementation check; (b) Hardcover excluded from foreign enrichment → **intentional** (REQ-014 names HC + OL exclusion as policy) — Sprint B's F1 fix should encode it as the documented rule at the merge site, not an accident-shaped hardcode.

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
