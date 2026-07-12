---
feature: "responsiveness"
stage: spec
status: draft
version: 1
req_ids: [REQ-001, REQ-002, REQ-003, REQ-004, REQ-005, REQ-006, REQ-007, REQ-008, REQ-009, REQ-010, REQ-011, REQ-012, REQ-013, REQ-014, REQ-015]
---

# Spec: responsiveness

Source analysis: `responsiveness-recommendations.md` (cross-family reviewed 2026-07-11, revisions folded, PO-approved scope/order 2026-07-11). Approved order: C (covers) → A (instant add) → B1/B2 (cache + batch) → B3/B4 (measure, then tune/consolidate), B5 tucked in. Tier D (central metadata proxy) is explicitly out.

## 0a. Design Principles

- **Never faster by hitting providers harder.** All gains come from waiting less, caching, batching, and connection reuse. No recommendation may raise any provider's request rate or weaken the outbound queue's pacing/caps/breaker (ST-001).
- **Perceived speed first.** User-blocking paths (single add) outrank throughput paths (bulk refresh) whenever they compete.
- **Reuse existing machinery before building.** Thumbnails, convergence recovery, refresh locks, and the cover write gate already exist — new work wires them together; it does not duplicate them (ST-002, ST-007).
- **Honest states.** The UI never fakes completion: in-progress is shown while work runs, failure is visible-but-quiet with a recovery action, and a settled-but-sparse work (Thin) presents as complete, not as an error.
- **Measure before tuning.** Any optimization whose premise is unproven (connection coldness) gets a measurement gate before implementation, and the feature records before/after numbers for its headline claims.

## 0b. System Truths

| ID | Source | Guarantee | Forbids | Confidence |
|----|--------|-----------|---------|------------|
| ST-001 | `crates/livrarr-http/src/outbound_queue.rs:38` (`OUTBOUND_IN_FLIGHT_CAP = 2`), `:166-181` (pacing: GR 1.5s, OL/HC/GB 1s, Audnexus 2s, Audible 150ms, OL-covers 3s) — sampled 2026-07-11 | ALL outbound provider HTTP flows through one process-global queue with per-bucket pacing, in-flight cap 2, priority ordering, and a circuit breaker | Any design that raises request rates, bypasses the queue, or expects concurrency above the cap to reach providers | high |
| ST-002 | `crates/livrarr-handlers/src/mediacover.rs:23-67` (`get_thumb`: generates 300px JPEG on first request, caches `{id}_thumb.jpg` beside the cover), `crates/livrarr-metadata/src/cover_write_gate.rs:600` (`invalidate_thumbnails`, called at `:579` and from recovery) — sampled 2026-07-11 | Server-side thumbnails already exist for ebook + audiobook slots, self-cache on disk, and are deleted whenever the cover write gate commits a new cover | Building new thumbnail generation; assuming thumbs go stale on disk after a cover change | high |
| ST-003 | `crates/livrarr-handlers/src/mediacover.rs:125-173` (`serve_image`: ETag from mtime + `Cache-Control: public, no-cache`; 304 on If-None-Match), `:117-123` (missing cover → 404 `no-store`) — sampled 2026-07-11 | Covers and thumbs are served by a dedicated handler with per-request revalidation semantics today; placeholders are never cached | Assuming covers ship without cache headers; long-caching 404/placeholder responses | high |
| ST-004 | `frontend/src/utils/format.ts:48-56` (`getCoverUrl(workId, v?, mediaType?)` carries `?v=`; `getCoverThumbUrl(workId)` has NO version param), `frontend/src/components/BookCover.tsx:37-58` (loads full `cover.jpg` twice — blur backdrop + main — no `loading`/`decoding`/dimension attrs, never uses thumbs) — sampled 2026-07-11 | The shared cover component fetches full-size covers everywhere; thumb URLs are unversioned and unused | Applying cache-forever headers to thumb URLs before they carry versions (stale-forever covers); assuming grids already use thumbs | high |
| ST-005 | `crates/livrarr-domain/src/lib.rs:83-102` (`EnrichmentStatus` = Unenriched, Enriched, Thin, Failed; comment: Unenriched doubles as crash-recovery pickup signal), `crates/livrarr-domain/src/services/work.rs:72-73` (`AddWorkResult.enrichment_status`: "Final enrichment status after synchronous enrichment attempt") — sampled 2026-07-11 | No persisted in-flight enrichment state exists; the add contract today is final-after-synchronous | UI depending on a persisted "enriching" status existing today; treating the current add response's status as anything but final |  high |
| ST-006 | `crates/livrarr-handlers/src/work.rs:259` (handler awaits `add`), `crates/livrarr-metadata/src/work_service.rs:1849-1858` (phase-1 cover await, 3s budget), `:1902-1911` (enrichment awaited), `:2037-2047` (provider scatter), `:2089-2118` (cover write gate awaits) — sampled 2026-07-11 | Interactive add currently blocks the HTTP response on identity + provider scatter + two cover paths; the ~2–4s figure is UNMEASURED (flagged in the reviewed analysis) | Quoting a fixed speedup without the REQ-014 baseline | high (structure) / low (latency number) |
| ST-007 | `crates/livrarr-server/src/config.rs:182-196` (convergence defaults: enabled=true, 3600s interval, batch 25) — sampled 2026-07-11; selection semantics per `convergence_service.rs` (wiki insight 57: Completed requires Enriched/Thin; incomplete works are re-selected, Low priority) | A default-on hourly background job re-selects and completes works whose enrichment is incomplete | Building new crash-recovery machinery for backgrounded enrichment; "never silent limbo" designs that ignore the existing lane | high |
| ST-008 | `crates/livrarr-db/migrations/066_drop_metadata_cache.sql` ("never-wired persistent metadata cache table" dropped; live cache = 5-min in-memory TransportCache) — read 2026-07-11; 15-min discovery lookup cache per wiki insight 64 | No persistent provider-response cache exists; every refresh/convergence pass pays full network | Designs that "re-wire" the old cache (it's gone — B1 is a rebuild); assuming any cross-restart cache exists | high |
| ST-009 | `crates/livrarr-http/src/lib.rs:64-88` (builder sets only timeout/UA/certs/DNS-resolver) — sampled 2026-07-11; reqwest default `pool_idle_timeout` = 90s per docs.rs (external; pinned version unconfirmed) | HTTP client tuning is absent, but default pooling should already survive the 1–3s pacing gaps; whether real provider connections go cold is UNMEASURED | Implementing keepalive tuning before the REQ-011 measurement; claiming a handshake tax as fact | high (absence) / medium (external default) / low (coldness) |
| ST-010 | docs.hardcover.app (60 req/min limit; GraphQL aliasing is a confirmed language feature); `id _in [...]` list-filter inferred from Hasura convention, NOT shown in fetched docs — deferred to code-stage prototype | Hardcover allows batching many lookups into one request in principle | Designing REQ-010 around the `_in` filter before a live prototype confirms it | medium (aliasing) / low (`_in`) |
| ST-011 | `crates/livrarr-server/src/main.rs:170-182` (2 shared clients: unrestricted + SSRF-safe, plus shared fetcher) — sampled 2026-07-11; trust-split semantics per wiki insight 37 (alpha3→4 incident) | Two trust classes of HTTP client exist deliberately: admin-configured infra (unrestricted) vs runtime-derived URLs (SSRF-safe) | Consolidating clients ACROSS the trust split; routing cover/scrape fetches through the unrestricted client | high |
| ST-012 | Wiki insights 30/64 + ST-001: every `HttpFetcherImpl` instance shares the one process-global outbound queue | Consolidating client instances (REQ-012) cannot change provider-facing request rates; bulk-refresh concurrency (REQ-013) is throughput-bounded by the queue's caps | Expecting provider-rate changes from client consolidation; expecting linear speedup from refresh concurrency | high |
| ST-013 | `frontend/src/pages/search/SearchPage.tsx:318-339` (add → navigate to `/work/{id}`), `frontend/src/pages/author-detail/AuthorDetailPage.tsx:368-392` (add → stays on author page, marks entry added) — both sampled 2026-07-11; both call the same `addWork` API → `POST /works` | Exactly two interactive add doors exist and share one endpoint; their post-add UX differs (navigate vs mark-in-place) | Wiring the fast return for one door only (door→road lesson, wiki insight 46); assuming the author door navigates to the detail page | high |
| ST-014 | `crates/livrarr-handlers/src/work.rs:199-223` (handler awaits `resolve_identity(..., LatencyTier::Interactive)` BEFORE creating the work; comment: "an isbn/asin-only pick still fans out to find a work anchor"), `:224-235` (a pre-create conflict returns the existing work), `crates/livrarr-handlers/src/types/work.rs:137-143` (`AddWorkResponse` = work + author_created + messages — no created/duplicate flag, no progress field) — sampled 2026-07-11 | Provider-backed identity resolution sits on the synchronous add path today, and the add response exposes neither the duplicate outcome nor any progress signal | Treating "don't wait on the scatter" as sufficient for a fast add (the identity fan-out can freeze the response alone); assuming the current response already carries the REQ-004/005 contract | high |

## 1. Problem Statement

Livrarr works correctly but feels slow in three user-facing ways: (1) adding a single book freezes the interaction for seconds while providers and covers are fetched synchronously; (2) library grids ship full-size cover images with no lazy loading, making every page heavier than needed; (3) every metadata pass re-pays full provider network cost because nothing persists between runs, and bulk refresh is strictly serial. The anti-ban throttle (ST-001) is load-bearing and non-negotiable, so responsiveness must come from waiting less, caching, batching, and reuse — never from more provider traffic.

## 2. Requirements

**Tier C — covers (first):**

- **REQ-001** — *Thumbnails in grids.* Every grid/list surface displays covers via the existing thumbnail variant; full-size covers load only where the large image is actually shown (detail-page hero and equivalent large renditions). Applies to both ebook and audiobook cover slots.
- **REQ-002** — *Loading hygiene.* Covers outside the initial viewport are not fetched until needed; cover slots reserve their dimensions (no layout shift as images arrive); above-the-fold covers are not deprioritized.
- **REQ-003** — *Cache-forever covers, safely.* Cover and thumbnail responses become long-term browser-cacheable with zero revalidation, under the precondition that every such URL carries a version that changes whenever the underlying image changes (all four variants: cover, thumb, audiocover, audiocover_thumb). A cover change is visible in grids and detail without a hard reload. Unversioned requests keep today's revalidation behavior; missing-cover responses are never long-cached (ST-003). The version token's only function is cache identity — the endpoint ignores its value and remains public-by-design (ST-003), so the token grants no access; it must change on every image change (content-derived or per-image mtime acceptable; never a global sequential counter).

**Tier A — instant add:**

- **REQ-004** — *Fast add return.* The interactive add (`POST /works`, both doors per ST-013) returns as soon as the work record and the fast phase-1 cover are persisted. The response never waits on ANY provider-bound network work: not the provider scatter, not the cover write gate, and not provider-backed identity resolution (which sits on the synchronous path today — ST-014; it moves to the background phase). Identity work needing no network — anchors already on the candidate, the local conflict check against existing works — may stay synchronous, and a conflict detectable from local data still returns the existing work immediately, as today. A conflict that only emerges from background identity completion surfaces through the existing identity-conflict machinery (badge + review page), like the batch doors already do. The response carries the created work, an explicit created-vs-existing outcome field, and the enrichment progress signal (contract in REQ-005).
- **REQ-005** — *Enrichment progress is observable.* The work detail surface exposes whether enrichment is currently in progress for the work. The UI presents exactly three pill states derived from (in-progress, enrichment status): *fetching* (in progress), *complete* (settled as Enriched or Thin), *attention* (settled as Failed, or progress signal lost while unsettled) with a Retry action. The pill reflects enrichment only; identity states (Conflict / NeedsReview / NotFound) keep their existing badges and surfaces, and the pill never masks them. **Contract:** the work payload (detail response AND the work embedded in the add response — one shape, not two) carries an explicit boolean in-progress indicator that is true exactly while an enrichment run is executing for that work. It must never read stale-true: after a server restart, an unsettled work reads not-in-progress (REQ-008's convergence lane and the UI's bounded-wait degradation cover completion). `enrichment_status` in the add response reflects the state at response time and is no longer final — this deliberately supersedes the ST-005 response contract; clients must not treat it as terminal. The created-vs-existing outcome is an explicit response field, not inferred from messages. (Whether the indicator is persisted or derived is a design-stage choice; the observable semantics above are the requirement.)
- **REQ-006** — *Progressive fill.* On the work detail page: title, author, year, phase-1 cover, and all controls are present and usable immediately after add. Description, genres, series, publisher, language, and narrator render as loading placeholders that fill in as enrichment lands, without manual reload — including replacing the phase-1 cover when the cover write gate accepts a better one. Content changes reflect within ~2 seconds of landing; no push channel (polling is sufficient; SSE/WebSocket are non-requirements).
- **REQ-007** — *Idempotent add and retry.* Re-adding the same book returns the existing work (no duplicate record, no second enrichment pipeline). The pill's Retry action has the same semantics and guards as today's refresh (per-work lock; concurrent retries don't double-run).
- **REQ-008** — *No permanent "fetching".* A work whose background enrichment is interrupted (crash, restart) is completed by the existing background convergence lane (ST-007) without user action. The UI never spins forever: if the progress signal is lost while the work is unsettled, the pill degrades to the *attention* state after a bounded wait (≤60s).

**Tier B — fewer / cheaper provider calls:**

- **REQ-009** — *Persistent provider-response cache.* Provider detail responses are cached persistently, keyed by provider + anchor. Background metadata flows consult the cache first; a fresh hit costs zero provider HTTP. Default TTL 7 days; the store is size-capped with oldest-first eviction; TTL and cap are configurable via TOML only (project rule: no env overrides). Interactive discovery keeps its existing short-lived caches. **Coverage matrix:** the cache covers the six enrichment detail providers, keyed by their dispatch anchors — GoogleBooks←isbn13, Goodreads←gr_key, Hardcover←isbn13 or hc_key, OpenLibrary←ol_key or isbn13, Audnexus←asin, Audible←asin. Only successful detail payloads cache; errors, not-found, and unparseable/partial responses are NEVER cached (a transient failure must not be pinned for a TTL). Cover/image bytes are excluded — the cover pipeline owns those. **Who reads it:** convergence, background retries, list-import enrichment, monitor-created works, and re-adds. **Who bypasses it:** the user's per-work Refresh and Refresh All — a user asking for fresh data gets real fetches, which also overwrite the cache entries.
- **REQ-010** — **CANCELLED by PO 2026-07-12.** Documented reason, verbatim: "Claude was wrong. This is NOT worth it. Gemini and Codex are unanimous in their judgement. This was a goose chase." Basis: measured sweep is bound by the Goodreads/Audnexus pacing buckets (batching Hardcover cannot move the wall); the U-B1 cache already eliminates repeat Hardcover fetches for background flows; cross-family confer was unanimous against building the cross-work path (Gemini DO-NOT-PURSUE, Codex PURSUE-REDUCED-only-if-profiling-demands; design-review round 1 had already FAILED the draft from both families). AC-016 is void with this cancellation. Probe artifact retained for the record: `docs/hc-batch-probe-2026-07-11.md`. ~~*Hardcover batching.*~~ Multi-work Hardcover flows (convergence batches, list import, bulk refresh) fetch many works in one HTTP request instead of one request per work, within Hardcover's documented limits. A recorded live prototype of Hardcover batch retrieval is the ENTRY gate for this requirement's design work — no design or implementation before the probe confirms the mechanism (ST-010 stays low-confidence until then; the spec-time deferral is template-sanctioned). If batching fails at runtime, the flow falls back to per-item fetches without losing works.
- **REQ-011** — *Measure connection reuse before tuning.* Instrument real provider traffic to report connection reuse vs new-handshake counts over a representative refresh run. Keepalive/pool tuning happens only if the measurement shows meaningful coldness; the measurement report is the gate artifact.
- **REQ-012** — *Client consolidation within trust classes.* Reduce the per-service HTTP client instances to a small documented shared set, preserving the SSRF trust split absolutely (ST-011): admin-infra clients and runtime-URL-safe clients never merge. No behavior change beyond fewer pools.
- **REQ-013** — *Bounded-concurrent bulk refresh.* `refresh_all` processes works with small bounded concurrency instead of strictly one-by-one. Per-work semantics, failure isolation (one failure never stops the sweep), and completion notification are unchanged; provider pacing remains governed by the queue (ST-012 — the win is overlapping different providers across works, not more provider traffic).

**Cross-cutting:**

- **REQ-014** — *Before/after evidence.* Add latency and bulk-refresh timing are measured on current main before any code lands, and re-measured as tiers land. The headline "add feels instant" claim is backed by numbers, not asserted.
- **REQ-015** — *Dead code removal.* The unreferenced `AddWorkRequest` struct in the domain services (leftover from a deleted overload) is removed.

## 3. UI/Interface Design

Mockup: `ui/responsiveness-add-progress.html` (interactive timeline: instant → filling → done → failure). PO-approved 2026-07-11:

- Failure presentation: **quiet amber pill with Retry** on the work detail page — no toasts/banners/modals for enrichment failure.
- Progress indicator lives on the **work detail page only** — no grid-tile badges in this feature.
- Tier C has **zero visual change** — same layouts, lighter images.
- Door-specific UX (ST-013): search-page add navigates to the detail page (pill visible immediately); author-page add stays in place and marks the entry added (progress visible if the user navigates to the book).

## 4. Non-Requirements

- **Tier D (central shared metadata proxy)** — separate strategic/privacy decision, explicitly out.
- **SSE/WebSocket push** — polling is proportionate; reverse-proxy burden on self-hosted users rejected in the reviewed analysis.
- **Grid-tile "fetching" badges** — PO call 2026-07-11: detail page only.
- **Manual metadata editing** — separate feature (PO call 2026-07-11).
- **Goodreads de-prioritization / provider re-weighting** — metadata-quality decision, not a speed knob; out.
- **Pre-generating thumbnails at cover-write time** — only if REQ-014 measurements show first-request generation latency matters; not in this feature.
- **Raising any provider request rate, in-flight cap, or pacing change** — forbidden by design principle 1.
- **OpenLibrary UA/rate-identity changes** — frozen project-wide; out.

## 5. Open Questions

| ID | Question | Status | Resolution |
|----|----------|--------|------------|
| Q-001 | Persistent cache default TTL — is 7 days right? | proposed | Proposed 7d (book metadata is near-immutable; hard-refresh bypasses). TOML-configurable either way. PO may adjust at spec walkthrough. |
| Q-002 | Failure pill wording ("Couldn't fetch everything") and Thin wording ("Details complete" vs "Limited info found") | proposed | Mockup wording stands unless PO objects; Thin presents as complete per design principle 4. |
| Q-003 | Cover version token: does predictability matter? (round-1 review, gemini R-004) | resolved | No access implication — the endpoint ignores the token's value and is public-by-design (ST-003); requirement folded into REQ-003: token changes on every image change, content-derived or mtime, never a global sequential counter. |

## 6. Acceptance Criteria

**Tier C:**

- [ ] **AC-001** (REQ-001): Loading a library grid page issues thumbnail requests for tiles and zero full-size cover requests; opening a work detail issues the full-size request. Verified for ebook and audiobook slots.
- [ ] **AC-002** (REQ-002): With a library larger than one viewport, covers below the fold produce no network requests until scrolled near; grid tiles reserve image dimensions (no visible reflow when images arrive); first-row covers are not lazy-deprioritized.
- [ ] **AC-003** (REQ-003): A versioned cover/thumb response carries immutable long-term caching; a repeat visit issues zero requests for unchanged covers (no 304 revalidations for versioned URLs).
- [ ] **AC-004** (REQ-003): Changing a work's cover (upload, select, enrichment swap) changes the URLs grids and detail request, and the new image appears without hard reload; the stale cached entry is never served on any surface.
- [ ] **AC-005** (REQ-003): Missing-cover/placeholder responses remain non-cacheable; all four image variants carry versions before any immutable header ships (gate: no immutable header on an unversioned variant).
- [ ] **AC-006** (REQ-001, REQ-002): Measured grid page image bytes drop materially vs baseline (target ≥4× lighter on a representative library page, per the Calibre-Web precedent; actual number recorded in the REQ-014 report).

**Tier A:**

- [ ] **AC-007** (REQ-004): With ALL provider responses artificially delayed, `POST /works` returns within the phase-1 envelope for every seed shape — fully-anchored search pick, isbn/asin-only pick, and title/author-only add — proving the response waits on no provider-bound work (identity fan-out, scatter, and cover gate all off the response path). A duplicate/conflict detectable from local data still returns the existing work synchronously.
- [ ] **AC-008** (REQ-004): Both interactive doors get the fast return: search-page add navigates immediately; author-page add clears its per-entry busy state immediately (ST-013).
- [ ] **AC-009** (REQ-005): The detail surface reports in-progress=true from add-return until enrichment settles, then false; the pill shows *fetching* while true, *complete* for Enriched AND Thin, *attention* with Retry for Failed. After a server restart mid-run, the indicator reads false (never stale-true). The add response and detail response expose the same indicator and an explicit created-vs-existing field.
- [ ] **AC-010** (REQ-005): For a work parked in an identity state (Conflict/NeedsReview/NotFound), the existing identity badge renders exactly as today; the enrichment pill neither replaces nor contradicts it.
- [ ] **AC-011** (REQ-006): On an open detail page: description/genres/series/publisher/language/narrator fill in without manual reload as enrichment lands, and a gate-accepted better cover replaces the phase-1 cover on the open page (image version observed to change).
- [ ] **AC-012** (REQ-007): Adding the same book twice yields one work row, one enrichment pipeline (provider call records show a single run), and the second response marks the duplicate outcome; clicking Retry during an active refresh does not start a concurrent second run (existing per-work lock observed).
- [ ] **AC-013** (REQ-008): Enrichment interrupted by a server restart leaves the work unsettled; the convergence lane completes it on its next tick with no user action; meanwhile the UI degrades to *attention* within 60s instead of spinning indefinitely.

**Tier B:**

- [ ] **AC-014** (REQ-009): A background pass (convergence or re-add) over a work whose matrix-covered providers were fetched within TTL issues zero provider HTTP for those providers (verified via provider call records); errors and not-found responses from a prior pass are provably not served from cache. The user's per-work Refresh and Refresh All issue real provider requests and overwrite the cache entries.
- [ ] **AC-015** (REQ-009): TTL and size cap load from TOML config; exceeding the cap evicts oldest entries; no environment-variable override path exists.
- [ ] **AC-016** (REQ-010 — VOID, requirement cancelled by PO 2026-07-12, see REQ-010): A convergence/list batch of N Hardcover-anchored works produces one Hardcover HTTP request (N within the batch max); a failed batch request falls back to per-item fetches with zero works lost; a recorded live prototype of the `_in` filter exists before the design gate.
- [ ] **AC-017** (REQ-011): A measurement report exists quantifying connection reuse vs new handshakes over a representative refresh run; the tuning decision (proceed/skip) cites it.
- [ ] **AC-018** (REQ-012): The client inventory shrinks to the documented shared set; SSRF-safe behavior is preserved (private-IP fetch still rejected on the safe path, admin-infra on private IPs still works); full workspace tests green.
- [ ] **AC-019** (REQ-013): Bulk refresh over a multi-work library completes measurably faster than the serial baseline while queue pacing intervals remain respected (no bucket paced faster than ST-001); one work's failure doesn't abort the sweep; the completion notification still fires with correct counts.

**Cross-cutting:**

- [ ] **AC-020** (REQ-014): A baseline report (add latency, bulk-refresh timing, grid page weight) exists from before the first code change, and per-tier after-measurements are appended as tiers land.
- [ ] **AC-021** (REQ-015): `AddWorkRequest` is gone from the domain services; the workspace compiles with zero references to it.
