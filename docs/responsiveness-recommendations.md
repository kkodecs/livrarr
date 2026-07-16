# Livrarr Responsiveness — Research & Recommendations

**Date:** 2026-07-09 (code citations re-verified 2026-07-11 against post-work-service-split main, `2e623f53`)
**Author:** Claude (main session), grounded against code at `/mnt/opt/livrarr` (main) + 5 online research threads.
**Status:** Cross-family reviewed 2026-07-11 (Codex FAIL: 4 P1; Gemini PASS: 2 P2 + 1 P3 — independent runs, unprimed). All findings verified against source and folded in below; revision markers `[REV]` flag what the review changed. Review record: `build/reviews/responsiveness/review-spec-{google,openai}-r1.json`.

---

## 1. Executive summary

Responsiveness comes from three moves, in priority order:

1. **Stop blocking the one screen that actually freezes** — adding a single book blocks ~2–4s on provider calls + two cover downloads. Return instantly; finish enrichment in the background lane that *already exists*. `[REV]` Reviewers confirmed the freeze but rejected "small": no in-flight enrichment state exists in the domain, so this is a real API/status design, not a moved await (see A1).
2. **Do fewer / cheaper provider calls** — rebuild a persistent response cache and batch Hardcover. Helps foreground and background alike. `[REV]` Connection keepalive (B3) demoted to measure-first: the "we re-pay TLS between paced calls" premise is unproven (reqwest's default pool keeps idle connections 90s; our pacing gaps are 1–3s).
3. **Serve cover images the modern way** — thumbnails in grids + long-lived caching + lazy loading. Cheapest wins on the list; every library page benefits. `[REV]` Even cheaper than drafted: the backend thumbnail endpoint already exists and self-caches; the gap is that the frontend never uses it and lazy-loading attributes are absent (see C1/C2).

A fourth, **strategic** move — a shared metadata cache like the *arr apps run — is powerful but heavy and carries a real privacy/ownership cost. It's a separate decision, not a quick win.

**The governing rule, throughout:** we do **not** get faster by hammering the metadata sites. We are already at/over the polite rate on Goodreads, the throttle is deliberate anti-ban infrastructure, and one ban breaks *every* user. All gains below come from **waiting less, caching, batching, and connection reuse** — never from raising the request rate.

**Recommended order (my POV, revised post-review):** C (covers — cheapest, now known to be mostly frontend wiring, zero pipeline risk) → A (decouple add — the acute freeze, with a real status-contract design pass) → B1/B2 (cache + batch — throughput) → B3/B4 (measure connection reuse first; tune + consolidate only if measured cold) → D (strategic — deliberate, later). `[REV]` B3 moved behind B1/B2 per Codex R-1.

---

## 2. Current state — grounded (so we don't re-propose what's already built)

| Fact | Status | Evidence |
|---|---|---|
| Single add blocks on full enrichment (identity resolve + provider scatter + **two** cover downloads) before returning to the client | **THE blocking wait** (~2–4s) | `handlers/work.rs:259` (`add().await?`); `work_service.rs:1902-1911` (enrichment awaited in `finish_created_work`); `work_service.rs:1849-1858` (phase-1 cover await) + `:2089-2118` (cover-write-gate awaits, ebook + audiobook) |
| A background top-up `refresh()` is already spawned ~5s after add | Pattern already in use | `handlers/work.rs:263-273` (`tokio::spawn` + sleep + `refresh`) |
| Bulk refresh already returns `202 Accepted` immediately and runs in a background task | Already async UX — **but the loop is strictly serial** | `handlers/work.rs:707` (spawn), `:714` (`for work in &works { refresh().await }`), `:754` (202) |
| Background convergence job runs by default | Already on — hourly, 25/batch | `server/config.rs:182-184` (`enabled = true`), `:186` (3600s); `jobs/convergence.rs` |
| Provider scatter within one work | Already parallel (not serial) | `livrarr-enrichment/src/provider_queue.rs` JoinSet + per-provider Semaphore + GCRA token bucket (wiki insights 49/55, since commit `808d47a`) |
| 24-hour persistent metadata cache | **Deleted** (was never wired) | `crates/livrarr-db/migrations/066_drop_metadata_cache.sql`; only a 5-min in-memory `TransportCache` remains |
| Outbound queue: 2 in-flight per provider + per-provider pacing (GR 1.5s, OL/HC/GB 1s, Audnexus 2s, Audible 150ms, OL-covers 3s) | The deliberate anti-ban throttle — **keep it** | grounding pass: `livrarr-http/src/outbound_queue.rs:38` (cap), `:166-181` (pacing); corroborates wiki insight 30 |
| HTTP clients: ~40 `reqwest::Client` instances built at boot, only ~5 actually shared; **zero** pool/keepalive/HTTP-2 tuning anywhere | Tuning absent — but whether connections actually go cold is **unmeasured** `[REV]` (reqwest's default pool keeps idle connections 90s; documented pacing gaps are 1–3s, so client-side expiry between consecutive paced calls is *not* expected — server-side idle closes and long gaps between enrichment bursts are the open question) | `server/main.rs:170-182` (2 shared + 1 fetcher), `:535,:545,:594,:609,:627` (per-service fetchers — the work-service split added one more for discovery), `:549,:598,:614` (per-LLM clients); `livrarr-http/src/lib.rs:64-88` (builder sets only timeout/UA/certs/DNS) |
| Cover **thumbnails already exist server-side**: `/mediacover/{id}/thumb.jpg` + audiobook variant generate a 300px JPEG on first request, cache it on disk, and the cover write gate deletes stale thumbs on every cover change. Covers/thumbs are served with `ETag` + `Cache-Control: public, no-cache` (revalidate each render) by a dedicated handler — **not** header-less `ServeDir`. The frontend never requests thumbs: the shared `BookCover` component loads the **full** cover twice (blur backdrop + main) with no `loading="lazy"`/`decoding`/dimension attrs, and `getCoverThumbUrl` has zero callers and no version param | `[REV]` The Tier-C gap is **frontend wiring**, not backend building (both reviewers, independently) | `router.rs:573-589` (routes); `handlers/mediacover.rs:23-67` (`get_thumb` on-demand generate+cache), `:125-173` (`serve_image` ETag + no-cache); `cover_write_gate.rs:600` (`invalidate_thumbnails`, called at `:579` + recovery); `frontend/src/components/BookCover.tsx:37-58`; `frontend/src/utils/format.ts:48-56` |

---

## 3. Governing constraints (local landmines — reviewers and implementers must respect)

1. **Anti-ban throttle is load-bearing.** No recommendation here raises the provider request rate. Goodreads is already ~5–7× over the polite floor (wiki index); one DataDome ban breaks every install.
2. **The SSRF client split must survive any consolidation.** `http_client` (unrestricted, for admin-configured infra) vs `http_client_safe` (rejects private IPs, for runtime/scraped URLs) is a deliberate split; collapsing it caused the alpha3→alpha4 fire drill (wiki insight 37). Consolidate clients *within* each trust class, never across.
3. **OpenLibrary User-Agent changes are frozen.** OL's "1→3 req/s if you identify with a UA + contact email" lever is **off the table** — new UA identifiers have been burned repeatedly and OL contact is paused. Do not chase it.

---

## 4. Recommendations

### Tier A — Decouple the interactive add (the acute freeze)

**A1. Return the created book immediately after phase-1 (identity seed + fast cover); move the provider scatter + cover-write-gate into the background spawn that already exists.**

- *What "move certain providers to async" means concretely here:* the fast phase-1 cover (3s budget, `work_service.rs:1846-1858`) stays synchronous so a cover is on screen instantly; the slow legs — the Goodreads/Hardcover/OL/GB scatter (`work_service.rs:2037-2047`) and the cover-write-gate (`:2089-2118`), plus the anchor-poor identity settle that runs before the scatter (`:2002-2025`) — move off the response path into a `tokio::spawn`, exactly like the top-up refresh already at `handlers/work.rs:263-273`.
- *Why it's safe:* the add handler already spawns background refreshes; the convergence job already guarantees "never silent limbo" (M9) and re-selects works left incomplete, and `Unenriched` already doubles as a crash-recovery signal (`livrarr-domain/src/lib.rs:84-86`) — so the recovery scaffolding for a backgrounded enrichment exists.
- `[REV]` *Why it's NOT small (Codex R-4, verified):* there is **no in-flight enrichment state** — `EnrichmentStatus` is `{Unenriched, Enriched, Thin, Failed}` (`livrarr-domain/src/lib.rs:83-102`), and `AddWorkResult.enrichment_status` is documented as *final after the synchronous attempt* (`livrarr-domain/src/services/work.rs:72-73`). Decoupling requires a deliberate design pass: a persisted or derivable "enriching now" signal for the UI, the API response contract for a partial record, idempotency for repeat triggers, and an explicit crash-recovery statement. Effort is honestly **medium-plus with a design gate**, not "move one await."
- *Pattern (sourced):* HTTP **202 + status-on-the-record + poll**. Return the book with a `status` field (`pending → enriching → complete/failed`) directly on the record; an **idempotency guard** so a repeat "refresh" click returns current status instead of spawning a second run. [Microsoft — Async Request-Reply](https://learn.microsoft.com/en-us/azure/architecture/patterns/asynchronous-request-reply); [Stripe — Idempotency](https://stripe.com/blog/idempotency)
- *UI (sourced):* **React Query `refetchInterval` polling** while `status === 'enriching'`, stop on complete; seed the cache from the 202 body (free optimistic state); per-field skeletons. **Not SSE/WebSocket** — those push the reverse-proxy-config burden onto self-hosted users, and Sonarr's SignalR is a live, well-documented example of exactly that support pain (works for the author, silently breaks behind the user's nginx/Caddy/Cloudflare). [MDN SSE](https://developer.mozilla.org/en-US/docs/Web/API/Server-sent_events/Using_server-sent_events); [Ably — WebSockets vs SSE](https://ably.com/blog/websockets-vs-sse); [TanStack — Polling](https://tanstack.com/query/latest/docs/framework/react/guides/polling); [Sonarr SignalR-behind-proxy issue](https://feedback.ultra.cc/p/sonarrs-nginx-file-needs-to-include-websockets-to-allow-bazarrsignalr)
- *Expected win:* the frozen 2–4s add becomes instant-with-progress. *Effort:* medium. *Risk:* partial-state visibility → mitigated by the status field + skeletons; the convergence job already prevents stuck states.

### Tier B — Do fewer / cheaper calls (helps foreground + background)

**B1. Rebuild a persistent provider-response cache** (the old 24h table was *deleted*, `migration 066` — this is a rebuild, not a re-wire). Key by stable provider key + anchor; refresh/convergence/re-add read cache first, hitting the network only on miss or explicit hard-refresh. Long TTL is fine — book metadata is near-immutable; hard-refresh bypasses. *Why:* the metadata-pathway doc itself lists "cache provider detail responses" as top speed work; the *arr proxies cache hours-to-days. *Effort:* medium. *Risk:* staleness — bounded by TTL + hard-refresh escape hatch.

**B2. Batch Hardcover in the bulk / convergence / list-import paths via GraphQL aliasing** — many books in **one** HTTP request. Under a 60-requests/min limit, 1 request for 25 books beats 25 requests for 25 books; this respects the rate limit *and* cuts latency. [Hardcover API — 60 req/min](https://docs.hardcover.app/api/getting-started/) *Effort:* medium (a new by-batch Hardcover query; note the existing HC-by-key gap, wiki insight 51). *Flag:* GraphQL **aliasing** (N named sub-queries in one call) is a confirmed language feature; an `id _in [...]` list-filter is inferred from Hasura convention and was **not** shown in the docs fetched — prototype before relying on the `_in` variant.

**B3. `[REV — demoted to measure-first, Codex R-1]` Measure connection reuse; tune keepalive only if measured cold.** The draft claimed pooled connections "likely die between paced calls" — unproven: reqwest's default `pool_idle_timeout` is 90s and our pacing gaps are 1–3s (`outbound_queue.rs:166-181`), so consecutive paced calls should reuse connections *by default*. Where cold connections plausibly DO appear: provider-side idle closes shorter than 90s, and the minutes-long gaps between enrichment bursts. **Step 1: measure** (connection-reuse tracing or handshake-time logging on real provider traffic). **Step 2, only if cold:** `tcp_keepalive`, `http2_keep_alive_interval` + `http2_keep_alive_while_idle(true)`, `pool_idle_timeout` sizing, explicit `connect_timeout`/`read_timeout` (~100–450ms/request at stake per cold handshake on a 50–150ms-RTT link — TLS 1.3 protocol math, [Cloudflare 0-RTT](https://blog.cloudflare.com/introducing-0-rtt/); [reqwest ClientBuilder](https://docs.rs/reqwest/latest/reqwest/struct.ClientBuilder.html)). Zero added requests either way. *Effort:* low (measure) + low (tune). *Flag:* exact defaults depend on the pinned reqwest version — confirm during the measure step.

**B4. Consolidate the ~40 reqwest clients into a few shared, tuned instances** (`main.rs:537,:542,:592,:596` each build throwaway pools). Fewer pools = warmer connections + one place to apply B3. **Preserve the SSRF trust split** (constraint 2). *Effort:* low–medium.

**B5. Make bulk refresh's serial loop bounded-concurrent** (`handlers/work.rs:714`). Secondary to B2 (batching beats concurrency under a rate limit) and **not** a frozen-UI fix (it already returns 202) — this is throughput/freshness only. *Effort:* low. *Landmine:* bulk refresh on a live library is write-heavy and historically triggered wrong-book merges (F1) — **snapshot the DB first** and check the open critical-bug list against the path before any real-data run (wiki insight 49).

### Tier C — Covers / images (pure frontend, independent, cheapest)

**C1. `[REV — rescoped: backend already exists; the gap is frontend, both reviewers]` Wire the grid/list UI to the existing thumbnail endpoints.** The draft proposed building thumbnail generation — it's already built: `get_thumb` generates a 300px JPEG on first request, caches it beside the cover, and the cover write gate invalidates it on every cover change (`handlers/mediacover.rs:23-67`, `cover_write_gate.rs:600`). What's missing: `BookCover` (the shared component every grid/list/detail surface renders) requests the **full-size** cover twice and never calls `getCoverThumbUrl` (`BookCover.tsx:37-58`, `format.ts:54-56`). The work: a size/variant prop on `BookCover` so grids request `thumb.jpg` and only the detail page requests full-res. Calibre-Web measured a **4.5–10×** page-weight cut from serving thumbs in grids. [Calibre-Web thumbnail PR](https://github.com/janeczku/calibre-web/pull/1771) *Optional follow-up, only if measured:* pre-generate at cover-write time to kill first-request latency (`fast_image_resize` if generation cost ever matters — [benchmarks](https://github.com/Cykooz/fast_image_resize/blob/main/benchmarks-x86_64.md)). *Effort:* low (was medium).

**C2. `[REV — current-state corrected + safety precondition added, both reviewers]` Upgrade cover caching from revalidate-every-render to cache-forever — versioned URLs FIRST.** The draft said covers ship without cache headers via `ServeDir` — wrong: a dedicated handler already serves covers *and* thumbs with `ETag` + `Cache-Control: public, no-cache` (`handlers/mediacover.rs:125-173`), so every render costs a conditional request (usually 304) rather than a re-download. The upgrade: `Cache-Control: public, max-age=31536000, immutable` + a versioned URL, so the browser issues **zero** requests for cached covers. **Hard precondition (Codex R-3):** every immutable asset must carry a version in its URL. `getCoverUrl` already takes `?v=`; `getCoverThumbUrl` takes **none** (`format.ts:48-56`) — versioning the thumb (and audiobook) URLs comes first, or users see stale covers forever after a cover change. Unversioned endpoints keep the ETag/no-cache scheme. [MDN HTTP Caching](https://developer.mozilla.org/en-US/docs/Web/HTTP/Guides/Caching)

**C3. Frontend loading hygiene:** explicit `width`/`height` (kills layout shift), `loading="lazy"` on off-screen covers (the single biggest grid win — no request for what's scrolled out of view), `decoding="async"`, and `fetchpriority="high"` on *only* the first cover. WebP for the generated thumbnail is sufficient; AVIF and BlurHash are "nice, not now." [web.dev — lazy loading](https://web.dev/articles/browser-level-image-lazy-loading); [web.dev — fetch priority](https://web.dev/articles/fetch-priority)

### Tier D — Strategic (later, heavy, real tradeoffs)

**D1. Shared metadata proxy/cache (the *arr pattern).** Confirmed: every *arr app (Sonarr/Radarr/Lidarr, and formerly Readarr) routes metadata through one central Servarr-run cache so installs never hit upstream directly — it normalizes, caches, and lets one well-behaved crawler absorb the rate limit instead of thousands of installs replaying the same scrape. A live, self-hostable book precedent already exists: **rreading-glasses** (Hardcover/Goodreads/OpenLibrary behind one crawler, async population, KV cache). [Sonarr Skyhook rationale](https://forums.sonarr.tv/t/why-do-tvdb-api-calls-go-through-skyhook/16104); [rreading-glasses](https://github.com/blampe/rreading-glasses)
- **But this is a commitment, not a quick win.** Readarr is the cautionary tale: its Goodreads proxy degraded, and with no funding/API deal behind it the whole app was **retired in 2025** — one dead proxy breaks *every* user at once. [Readarr status](https://wiki.servarr.com/readarr/status) A central proxy also **sees every user's queries** (no *arr proxy publishes a privacy policy) — a real cost given this project's privacy stance.
- **Recommendation:** adopt the *techniques* internally now (async population — partly done; persistent cache — B1). Treat the *central service* as a separate, deliberate decision, not part of this responsiveness pass.

**D2. Reduce reliance on Goodreads — the slowest, most fragile leg** (HTML scrape + DataDome anti-bot + multi-hop key resolution + LLM parse-repair + 1.5s pacing). The Hardcover API is a real, fast, free alternative that rreading-glasses uses in production and calls "higher quality." Making GR **non-blocking** (Tier A already moves it to the background lane) captures most of the responsiveness benefit safely. Actually **re-weighting** the priority model away from GR is a metadata-*quality* decision, not a pure speed knob — evaluate deliberately, don't just flip it.

---

## 5. What reviewers will likely challenge (and the answer)

| Likely challenge | Response |
|---|---|
| "Bulk refresh is the headline — parallelize it." | It already returns 202 and runs in the background (`work.rs:707/754`); the UI does not freeze. The real frozen wait is **single add**. And under the rate limit, **batching (B2) beats concurrency (B5)**. |
| "Just make the provider scatter parallel." | Already parallel (JoinSet + Semaphore + token bucket since `808d47a`). The scatter is not the serial part. |
| "Raise the in-flight cap / concurrency for speed." | Off the table — anti-ban (constraint 1). GR is already 5–7× over polite; also invites reqwest's HTTP/2 stream-limit failure modes at high concurrency. |
| "Wire up the 24h cache." | It was **deleted** (`migration 066`). B1 is a rebuild. |
| "Get OpenLibrary's 3× rate by identifying the UA." | OL UA changes are frozen/burned (constraint 3). Off the table. |
| "Consolidate all HTTP clients into one." | Yes to consolidation, but the `http_client` / `http_client_safe` SSRF split must survive it (constraint 2) — collapsing it caused the alpha3→4 fire drill. |
| "SSE/WebSocket gives real-time progress." | For a self-hosted app the reverse-proxy-config burden falls on the user; Sonarr's SignalR is live evidence of that support pain. Polling is proportionate for a 2–4s job; revisit SSE only if a shared multi-item activity view is built (Axum supports it natively). |

---

## 6. Open / unverified

- **Per-add latency needs a fresh measure.** The ~2–4s baseline is contested and flagged for re-measure (wiki insight 55). Do not quote a fixed speedup — measure before/after.
- **Connection coldness is unmeasured** — B3 is gated on a measure step (see revised B3); the pinned reqwest version's exact pool defaults get confirmed there (Cargo.lock read was blocked during drafting).
- **Hardcover `_in` id-list batching** (B2) inferred from Hasura convention, not shown in fetched docs — prototype the aliasing path (confirmed) and validate the list-filter before designing around it.
- Several provider numbers (Google Books ~1000/day, ISBNdb/LibraryThing limits) are community-sourced, not primary docs — noted in the research thread, not load-bearing here.

---

## 7. Key sources

- Async request-reply + status resource: Microsoft Learn (Azure Architecture Center); idempotency: Stripe.
- Poll vs SSE vs WebSocket: MDN SSE, Ably (2026), TanStack Query polling, Sonarr SignalR proxy issue.
- Covers: Calibre-Web thumbnail PR (measured), fast_image_resize benchmarks, MDN HTTP caching, tower-http, web.dev (lazy loading / fetch priority).
- reqwest/Tokio tuning: docs.rs/reqwest ClientBuilder, Cloudflare 0-RTT (TLS RTT math), governor docs.
- Providers: docs.hardcover.app (60/min), OpenLibrary search API + dumps, Audnexus (self-hostable), developers.google.com/books.
- *arr proxy pattern: Sonarr forums (Skyhook), Servarr wiki (Readarr status), github.com/blampe/rreading-glasses.
