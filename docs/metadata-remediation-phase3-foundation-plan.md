# Phase 3 Foundation Plan — One Outbound Queue (pace + circuit-break, every provider)

Status: PLAN (revision 5) — FINAL (plan review closed). Branch `metadata-remediation`.
Builds on Step A (committed `af50cd5`).

This plan SUPERSEDES the scope framing in
`docs/handoff-metadata-remediation-phase3-wiring.md`. Tracing the code revealed the
enrichment queue also houses a **circuit breaker**, so the real job is
**consolidating two rate-limiters into one** and moving the breaker. The locked
queue-engine design (`docs/metadata-remediation-phase3-queue-design.md`, v4) remains
authoritative for the engine itself.

## Revision log
- **rev 5 (2026-07-01)** — folded the round-5 findings; **plan review CLOSED** (5 rounds,
  both families). R-10: pace ONLY OpenLibrary covers (~3s); other cover hosts stay fast.
  R-11: a `CircuitOpen`/suppressed outcome must NOT consume a background task's enrichment
  retry budget — it's a paused state that resumes when the provider recovers (M9).
  R-12/R-14: `report_outcome` carries a STRUCTURED outcome (Success / Failure /
  TripImmediately) + an optional client-supplied cool-off duration (GB client computes
  its Pacific-midnight reset; queue stays timezone-free). R-13: the Audnexus 304
  conditional request IS queued; a 304 is served from the local cache. C-R11: breaker
  types live in `livrarr-http` (with the queue), NOT `livrarr-domain` (amends R-4).
  GR-1.5s: PO ACCEPTED-RISK (still ~3-5× over the polite floor; backstopped by the 1h
  breaker + immediate-trip-on-anti-bot) — deliberate, not an oversight.
- **rev 4 (2026-07-01)** — folded R-7/R-8/R-9 (Gemini, round 4). **R-7:** the cover
  bucket paces conservatively to respect OpenLibrary's cover limit (stays pace-only; OL
  CoverID caching remains the deferred fast-follow). **R-8:** the breaker's
  success/failure signal comes from the **provider client** (which parses the response),
  not the transport layer — a 200-OK challenge page / error body counts as a failure.
  **R-9:** Google Books gets a long breaker cool-off (~until next Pacific midnight) on
  daily-quota exhaustion, not the 60s default.
- **rev 3 (2026-07-01)** — folded **R-6** (Codex, design round 3): the cover bucket is
  **pace-only, NOT breaker-tracked** — it aggregates many image hosts + the cover-proxy,
  so a shared breaker would let one bad host suppress all covers (P10 / M3).
- **rev 2 (2026-07-01)** — folded five cross-family review findings (Codex + Gemini,
  design round 1/2), PO-approved:
  - **R-1** GR breaker cool-off ~1h now (not deferred) — anti-ban correctness.
  - **R-2** remove the GR ISBN `/search` door (robots.txt-disallowed).
  - **R-3** add a typed circuit-open outcome at the fetcher boundary.
  - **R-4** relocate `CircuitState`/`CircuitBreakerConfig` to `livrarr-domain` before
    moving the breaker (avoids a circular crate dependency).
  - **R-5** breaker tracks only book-provider buckets; exempt `None` + `Indexer`.
- rev 1 — initial plan.

## PO decisions folded in (2026-07-01)
- Goodreads interval → **1.5s** + **circuit breaker** (foundational). GR breaker
  cool-off ~1 hour now (R-1). **ACCEPTED-RISK (C-R10):** 1.5s is a deliberate PO
  speed/politeness tradeoff — still ~3-5× over GR's empirical polite floor (8-12/min ≈
  5-7s); accepted because the 1h breaker + immediate-trip-on-anti-bot (R-12
  `TripImmediately`) backstop a session block. Not an oversight; revisit if GR bans recur.
- **Foundations first, per-provider tuning later** — EXCEPT GR anti-ban correctness
  (interval, `/search` removal, 1h cool-off), which is not tuning.

## Goal
Every outbound call to a book/metadata provider passes through ONE process-global
queue that: (a) paces per provider, (b) caps in-flight per provider, (c)
circuit-breaks a failing provider, (d) orders by priority. No provider path bypasses
it. **Transport** concerns (pacing / concurrency / breaker) live at the queue;
**enrichment-domain** concerns (which providers apply, retry budgets, suppression)
stay in enrichment.

## Governing constraints — reviewers, check the plan against these (READ them)
- **Principles** — `build/foundation/principles.md` (highest authority):
  - **P3** ecosystem citizen — respectful, provider-appropriate pacing + honoring
    session-level blocks (GR 403 → ≥1h stop).
  - **P6** enrich eagerly / nothing blocks + **P15** Fast — interactive paths must NOT
    stall behind background; the priority seam is load-bearing.
  - **P10** failure isolation — a circuit-open provider must **skip + degrade**.
  - **P14** opinionated simplicity — don't over-engineer covers/priority classes.
- **Metadata principles** — `wiki/domain/metadata-principles.md`: **M2** (same
  treatment all paths), **M3** (covers matter — don't over-throttle), **M9** (same
  destination).
- **Architecture** — `wiki/architecture/overview.md`: dependency arrows point toward
  `livrarr-domain`; `livrarr-http → domain` only (it CANNOT depend on
  `livrarr-enrichment` — see R-4); `external-data → http` legal; `trait_variant`
  non-dyn.
- **Wiki integration rate facts** — `wiki/integrations/*.md`: GR 1/sec is 5-7× over
  the ~8-12/min floor → 1.5s + breaker interim, 403 → ≥1h stop, `/search`
  robots-disallowed; GB real limit 1,000/day (caching, not pacing — later); Audnexus
  300/min (2s conservative-safe); HC 60/min (1s correct); OL covers 100/IP/5min for
  ISBN keys (CoverID caching is the real fix — later).

## Current state (grounded against source)
- **Two live limiters** (plus dead: `livrarr-http/rate_limit.rs`,
  `goodreads_rate_limiter` field — zero-caller):
  1. **Outbound queue** (`livrarr-http/src/outbound_queue.rs`, Step A) — paces the
     **search/lookup** path (do_fetch), process-global.
  2. **Enrichment queue** (`livrarr-enrichment/src/provider_queue.rs`) — GCRA
     TokenBucket + Semaphore + **circuit breaker** (`BreakerState`; `CircuitState` &
     `CircuitBreakerConfig` at `livrarr-enrichment/src/lib.rs:163/175`; config
     5/60s/60s/1 at `main.rs:240`) + applicability + suppression. Guards
     **enrichment** only.
- **Providers issue HTTP via a raw `HttpClient`**, not the fetcher. Confirmed for
  Hardcover (`self.http`; both `fetch` and `fetch_by_anchor` share `hc_post` /
  `query_hardcover*` / `fetch_hardcover_editions`). Other 5 follow the same shape per
  the door map (confirm per-provider at conversion).
- **Identity fan-out + cover paths have NO limiter and NO breaker** (the M-001 burst).
- **Cover-image download** (`materialize/lib.rs:41`) uses the fetcher but
  `RateBucket::None` (unpaced).

### Door inventory (routing scope — the complete set to move)
Verified via LSP (`ProviderClient::fetch` has 4 production callers; `fetch_by_anchor`
has 1). Raw doors NOT on the queue:
- **Identity fan-out** — `english_identity_resolver.rs:99`.
- **Cover fan-out** — `cover_alternatives.rs:77`, `preadd_cover_service.rs:47`,
  `cover_service.rs:417`.
- **Enrichment** — `provider_queue.rs:557` (paced by TokenBucket + breaker).
- **Standalone raw** — cover ISBN helpers (`cover.rs`), `fast_hc_cover_search` (fires
  on ~every interactive add), cover-proxy (`coverproxy.rs:53`), admin
  `test_hardcover`/`test_audnexus` (`config.rs:282/307`).
- **Cover-image download** — `materialize/lib.rs:53` (`RateBucket::None`).
- **GR ISBN `/search` (R-2)** — `resolve_detail_url` (`provider_client.rs:1236`) →
  `search_goodreads_by_query` → `goodreads.rs:591` `{base}/search?q=isbn:...`. This is
  ON the queue after Step A but is a **robots.txt violation** — pacing ≠ compliance.
Already on the queue (Step A): all other search/lookup FetchRequest sites.

## Target architecture
ONE queue does per-`RateBucket`: pacing + in-flight cap + circuit breaker (book-provider
buckets only) + priority. Every provider `*Client` issues HTTP via `HttpFetcher`. The
enrichment queue keeps applicability + suppression + retry budget and delegates
transport. SSRF split preserved (queue wraps both `http_client` + `http_client_safe`).

## The work (sequenced; each stage its own reviewed unit)
### B0 — Goodreads anti-ban (tiny, ships first)
- `outbound_queue::interval_for`: `Goodreads` 1s → **1500ms**.
- **(R-2) Remove the GR ISBN `/search` tier** in `resolve_detail_url`
  (`provider_client.rs:1236`). GR-by-ISBN is redundant (HC/OL/GB resolve ISBN → key);
  the GR path keeps only gr_key-direct + title/author **autocomplete**
  (`/book/auto_complete`, already WAF-free). Result: **zero `/search` calls on the GR
  queue path**. (If a legal GR-ISBN endpoint is later wanted, that's a separate item.)

### B1 — Provider transport conversion (the big one). Hardcover first as template.
Convert each provider `*Client` from raw `HttpClient` to `HttpFetcher`: build a
`FetchRequest` (method, auth headers, serialized body, provider `RateBucket`,
priority), call `fetcher.fetch`, parse `FetchResponse.body`. Preserve per-provider
auth, GraphQL bodies, parsing, and the **Audnexus 304 cache** (R-13): the conditional request (`If-Modified-Since`)
IS queued through the fetcher like any GET — a `304` response is served from the local
cache (no re-parse), so revalidation still passes through the queue. Order: **Hardcover → review → GR / OL / GB / Audnexus / Audible.** Shared
send code means converting one client routes its enrichment + identity + cover-search
traffic at once.

### B2 — Circuit breaker at the queue
- **(R-4, C-R11, prerequisite) Relocate `CircuitState` + `CircuitBreakerConfig` out of
  `livrarr-enrichment`** (they block moving the breaker — `livrarr-http` cannot depend on
  `livrarr-enrichment` → circular). **Home = `livrarr-http`** (the breaker is a TRANSPORT
  concern; it lives with the queue there — do NOT dump transport machinery into
  `livrarr-domain`, C-R11). Everything that still needs the types (enrichment status
  reporting, handlers, server) depends on `livrarr-http`, so they can see them. IF a
  `livrarr-domain` trait is found to reference `CircuitState`, expose a minimal SEMANTIC
  availability type in domain and keep breaker mechanics/config in http. Update import
  paths at all consumers.
- Move `BreakerState`'s state-machine logic (REUSE — do not reinvent) into
  `outbound_queue`, keyed per `RateBucket`.
- **(R-5, R-6) Breaker tracks ONLY the single-host provider API buckets**: OpenLibrary,
  Goodreads, Hardcover, GoogleBooks, Audnexus, Audible. `RateBucket::None` (bypass),
  `RateBucket::Indexer(_)` (admin-configured infra — trusted, insight 37), AND the
  cover bucket are EXEMPT (pace-only): no breaker state, no suppression. The cover
  bucket is exempt because it aggregates many image hosts + the cover-proxy — a shared
  breaker would let one failing host suppress all covers (R-6; P10/M3).
- **Failure-feedback path (R-8, R-12, R-14)**: the breaker's outcome signal originates in
  the **provider client**, which parses the response and can tell a real success from a
  block — NOT `do_fetch` alone (a `200 OK` DataDome challenge or a GraphQL/`Throttled`
  error body looks like success to the transport layer). Each client calls the queue's
  **`report_outcome(bucket, Outcome)`** after interpreting the response, where `Outcome`
  is STRUCTURED (R-12): `Success` | `Failure` (counts toward the 5-in-window threshold) |
  `TripImmediately` (403/anti-bot → open now), plus an OPTIONAL client-supplied
  `open_for: Duration` (R-14) so the GB client computes its Pacific-midnight reset and the
  low-level queue stays timezone-/provider-rule-free. `do_fetch`'s transport failures
  (429/5xx/timeout/anti-bot) feed in too. Recording is O(1), no lock across `.await`.
- **(R-3, R-11) Typed circuit-open outcome.** Add `FetchError::CircuitOpen`. The
  dispatcher checks the breaker BEFORE granting a turn; Open → `do_fetch` returns
  `FetchError::CircuitOpen` (no HTTP). Each provider-client conversion MAPS it explicitly:
  enrichment surface → the existing suppressed/`WillRetry`-without-network outcome;
  identity/cover surface → abstain / partial (skip provider, use what we have, P10). **Do
  NOT collapse it into `RateLimited`** (corrupts retry accounting). **(R-11) A
  `CircuitOpen`/suppressed outcome must NOT consume a background task's enrichment retry
  budget** — it is a PAUSED state (provider temporarily unavailable), resumed when the
  breaker closes; else convergence tasks dead-end terminally during an outage without ever
  making a real attempt (violates M9).
- **(R-1, R-9) Per-bucket breaker config.** Default 5 failures / 60s window / 60s open /
  1 probe. **Goodreads override: `open_duration` ~3600s** (a 403/DataDome block lasts
  ≥1h; a 60s reopen loop trains DataDome); treat GR 403/anti-bot as an immediate trip.
  **Google Books override (R-9): on daily-quota exhaustion (`403 quotaExceeded`),
  `open_duration` ~until next Pacific midnight** — a 60s reopen loop against an exhausted
  daily quota burns guaranteed-fail requests all day.

### B3 — Covers
- `download_cover_to_disk`: `RateBucket::None` → a **dedicated cover bucket** (pace
  cover-image downloads; keep off providers' API budgets — M3). This bucket is
  **PACE-ONLY, NOT breaker-tracked** (R-6): it mixes many image hosts, so a shared
  breaker would let one failing host suppress healthy covers (P10/M3).
  **(R-7, R-10) Pace ONLY the OpenLibrary cover host** (`covers.openlibrary.org` by ISBN
  = 100/IP/5min ≈ 1 per 3s) at ~3s to respect its hard limit; **other cover hosts
  (Hardcover / Audnexus / Audible / Amazon / Google) stay fast** (light or no pacing) — a
  single ~3s bucket for ALL covers would throttle a 50-book import to 150s+ and break
  P15/M3 (R-10). Covers were fully UNPACED before, so even OL-only pacing is strictly
  safer; OL CoverID caching (deferred fast-follow, insight 44 — CoverID fetches are
  unlimited) later removes even the OL 3s cost. Per-host cover breaking, if ever needed,
  is later tuning.
- Cover-proxy (`coverproxy.rs`): route through the fetcher; **Interactive priority**.
- Standalone cover ISBN helpers (`cover.rs`) route through the fetcher.
- Real OL cover fix (CoverID caching, insight 44) is DEFERRED; pacing is the interim.

### B4 — Priorities (seam exists from Step A)
Interactive Add / cover-proxy → `Interactive`/`High`; background / bulk-refresh /
convergence → `Low`; else `Normal`. (Honors P6/P15.)

### C — Retire the enrichment queue's transport duties
Remove the enrichment TokenBucket pacing (redundant) and the enrichment breaker (now
at the queue). FIRST trace `WillRetry{RateLimit}` consumers (`provider_queue.rs:549`).
Keep applicability + suppression + retry budget.

## Semantics to get right (reviewer focus)
- **Pace (wait) vs suppress (breaker Open → `FetchError::CircuitOpen`, fast degrade)** —
  the ONE deliberate drop; P10; R-3 typing prevents mislabel.
- **Cancellation-safety** preserved (Step A `QueuePermit` RAII).
- **SSRF split** preserved.
- **Transitional double-pacing** (enrichment TokenBucket + queue until C) — harmless
  (more conservative); C removes it.

## Out of scope (explicit — "foundations first")
- Per-provider interval tuning EXCEPT GR 1.5s.
- GB daily-quota caching; OL CoverID caching; adaptive `Retry-After` honoring.
- LIFO/FIFO ordering policy (field wired; policy later).
- LLM caller; Readarr client; download-client pollers; indexer search.
- (No longer deferred — folded into the foundation: GR 1h cool-off (R-1), GR `/search`
  removal (R-2).)

## Risks / open questions
- Failure-feedback on the hot path — keep O(1), no lock across `.await`. The signal now
  originates in the provider client (R-8), so the queue exposes a small
  `report_outcome(bucket, ok|fail)` API the clients call; the breaker (transport) thus
  consumes a domain-interpreted signal — acceptable, since only the client reads a
  soft-block.
- Provider conversion is wide (6 clients) — Hardcover-first template de-risks.
- Breaker-Open changes enrichment's reject/`WillRetry` shape — trace consumers before C.
- Type relocation (R-4) touches every `CircuitState`/`CircuitBreakerConfig` consumer —
  mechanical but wide; land it as its own small commit before B2's breaker move.
- Two provider-client construction sets in `main.rs` (~251, ~444) both rebuild.

## Test plan
- Keep the 1030-test baseline green.
- New behavioral tests: breaker trips/resets AT THE QUEUE (book-provider bucket);
  Open → `FetchError::CircuitOpen`, zero HTTP; `None`/`Indexer`/cover buckets never
  trip (R-5/R-6); failure-feedback wiring; GR path issues **zero `/search`** (R-2); one
  door-routing test per converted provider (insight 46).

## Confidence
- Structural claims (two limiters; breaker in enrichment; providers on raw HttpClient;
  shared send code; GR `/search` door; type-location for R-4) — HIGH; traced directly.
- "Other 5 providers follow the same shape" — MEDIUM-HIGH (confirm each at conversion).
- Rate-fact judgments (GR 5-7× over floor; 403 ≥1h) — from the wiki's operational
  rules (inferred from community evidence, not provider-published) — strong guidance.
