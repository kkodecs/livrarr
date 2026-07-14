# Design — Indexer Rate-Limiting Fixes (unit: indexer-citizenship, 2026-07-13)

Status: DRAFT for cross-family design review. Findings source: the 2026-07-13 code
audit (three root causes, re-verified current today post-Wave-3 by a recon pass with
exact line cites). Goal: Livrarr stops earning indexer rate-limits (MaM most visibly)
and recovers gracefully when it gets one. No new entities; no new flows — three narrow
fixes on existing roads.

## System truths (verified at source today)

- ST-1: Grab-time torrent/NZB file downloads bypass all pacing: `fetch_torrent_dispatch_source`
  and the other `dispatch_*` fetches use `RateBucket::None` (release_service.rs:426, :538,
  :614, :787). The indexer name exists one frame up — `grab()` has `req.indexer: String`
  (services/release.rs:55) — and is never threaded down.
- ST-2: Interactive search fans out to every enabled indexer on the Releases tab's FIRST
  mount: `modeRef` initializes to `"search"` (WorkDetailPage.tsx:1000) while the comment
  above it (:997) documents the intended mount mode as `"cacheCheck"`. `useQuery` eager-runs
  it. staleTime Infinity + gcTime 30min bound repeat hits to first-open / >30-min re-open.
- ST-3: The backend accepts `refresh` and `cacheOnly` search params (handlers/release.rs:12-20)
  and threads them into `SearchReleasesRequest` — which the service never reads (zero hits
  for `req.refresh`/`req.cache_only` in release_service.rs). `cache_age_seconds` is
  hardcoded None (:262).
- ST-4: `GrabSearchCache` (livrarr-server/src/infra/cache.rs) — key (title, author,
  indexer_id), 24h TTL, lazy 5-min eviction, returns (results, age_secs) — is constructed
  (main.rs:288) and NEVER consulted: zero business-logic callers. It also lives in the
  wrong crate for the seam that needs it (the service layer in livrarr-download) and holds
  handler DTOs.
- ST-5: A 429 already reports `BreakerSignal::Failure` on the request's bucket
  (fetcher.rs:176-179) — but `Indexer(_)` buckets are pace-only (`breaker: None`), so the
  signal is dropped. `FetchError::RateLimited` carries no Retry-After.
- ST-6: The breaker allowlist comment (breaker.rs:196-201) excludes `Indexer(_)` because "a
  shared breaker over a multi-host bucket lets one bad host suppress the rest" — but
  indexer buckets are keyed PER INDEXER NAME (outbound_queue.rs registry HashMap over the
  full RateBucket value): each is single-host. The written rationale's principle
  (per-host isolation) is not violated by tracking them.
- ST-7: Both indexer callers (manual search release_service.rs:141-150; RSS
  rss_sync_workflow.rs:156-160) treat RateLimited like any error: warn, no cooldown, next
  attempt at full cadence.
- ST-8: Backoff precedent exists for book providers only: GB 429 → 6h + 0-3h jitter
  (google_books.rs:659-673); GR 429 → flat 300s (provider_client.rs:1716-1719). Neither
  is wired to indexers. No per-indexer health/cooldown state exists anywhere.

## Fix 1 — Every indexer-origin fetch joins the indexer's pace lane

Thread the indexer identity from `grab()` into every `dispatch_*` fn and replace
`RateBucket::None` with the indexer bucket (see Fix 3 for the key) at every fetch whose
URL is INDEXER-ORIGIN — classified by URL provenance, not by file location [REV codex
r3 R-7]: the release `download_url` fetches (release_service.rs:426 torrent-file, :675
Transmission URL-fallback, :787 NZB) and the magnet-redirect probe. DOWNLOAD-CLIENT RPC
stays exactly as it is — :538 (qBit auth) and :614 (qBit add) hit `client.host` (verified
:516-521), are local admin infrastructure, and must never be paced or cooled down behind
indexer state; an earlier revision of this design wrongly listed them. At implementation,
the site list is re-derived by an UNBOUNDED `git grep RateBucket::None` over
livrarr-download and each hit is classified by the URL it fetches — indexer-origin joins
the bucket, download-client RPC stays put.

`probe_for_magnet_redirect` (release_service.rs:473-486) currently builds a raw
`reqwest::Client` — bypassing livrarr-http entirely, a standing violation of the
canonical-transport invariant surfaced by this review [REV codex r2 R-4]. It is
rewritten as a normal fetcher call on the indexer bucket: `FetchRequest` gains a
`follow_redirects: bool` (default true — zero change for existing callers) and
`FetchResponse` exposes the redirect `Location` value so the probe can read it. The
probe thereby inherits pacing, cooldown, UA policy, and SSRF posture like every other
outbound call.

Grab-time fetches then share the same pacing (and, after Fix 3, the same cooldown) as
searches. No behavior change beyond pacing; the 500ms interval is invisible at grab
frequency.

**No delegation bypass [REV codex r3 R-8]:** `fetch_torrent_dispatch_source` currently
swallows every fetch error and falls back to `TorrentDispatchSource::Url` — handing the
original indexer URL to qBittorrent, which would then hit the indexer itself from
outside our pacing. With Fix 3 live, that turns a cooldown into a delegated bypass. The
fn preserves the failure kind: `RateLimited`/`CircuitOpen` fail the grab hard with a
clear error ("indexer cooling down — retry later"); the URL fallback remains only for
other failure kinds (parse/transport), where delegating to the client is the existing
intended degradation. Pinned by a test: a 429/open breaker never results in qBittorrent
receiving the indexer URL.

## Fix 2 — The search cache goes live; mount stops hitting indexers

- Move the cache to the seam that owns search policy (insight 56's chokepoint rule):
  a `ReleaseSearchCache` in livrarr-download (release_service module), keyed
  (title, author, indexer_id), holding DOMAIN results (per-indexer parsed release lists),
  same constants (24h TTL, 5-min lazy eviction). The dead server-side `GrabSearchCache` +
  its AppState field + construction are DELETED (one authority).
- `search()` finally honors its request flags, per indexer:
  - `cache_only`: serve cached entries only — ZERO HTTP; indexers with no cached entry
    contribute nothing. An all-miss cache_only search returns a SUCCESSFUL empty
    `ReleaseSearchResponse` — no warnings, `cache_age_seconds` None. The existing
    `AllIndexersFailed` error is reserved for modes that actually attempted live
    fetches and had every attempt fail; it must never fire on a mode that contacted
    nobody [REV codex r2 R-6 — today's `any_success` check would have mislabeled a
    cold-cache mount as "All indexers failed"].
  - default: per indexer, cached-if-fresh (<24h) else live fetch; live results written
    back to the cache.
  - `refresh`: live fetch for every indexer; rewrite cache.
  - `cache_age_seconds` in the response reports the OLDEST consulted entry's age (None if
    everything was live) — the field and its UI display already exist.
- Frontend: fix the one-word initialization bug — `modeRef` starts at `"cacheCheck"`
  (matching its own comment), so mount = cache-only, zero indexer traffic. The Search
  button switches to `"refresh"` (an explicit user click means "go ask the indexers now");
  the plain `"search"` mode remains the default for any other caller. This restores the
  component's documented intent — mount shows what we have, the button does the live work.

## Fix 3 — Indexers get a real cooldown after a 429

- The indexer bucket key changes from configured DISPLAY NAME to the normalized upstream
  ORIGIN (lowercased `scheme://host[:port]` derived from `indexer.url`) at every
  construction site — search, RSS, and the Fix-1 grab sites [REV codex r2 R-5: two
  configured indexers pointing at the same host must share one pace lane and one
  cooldown; a name key would let them evade both]. Warnings and UI keep display names;
  only the bucket/breaker isolation boundary uses the origin. The bucket registry is an
  in-memory process-global map — new keys need no migration.
- Opt `Indexer(_)` buckets into breaker tracking (`breaker_tracked` includes them). With
  origin keying this genuinely honors the allowlist's per-host principle; the comment is
  updated to say so (multi-host AGGREGATE buckets remain the thing that must never opt
  in).
- At the fetcher's 429 arm: for `Indexer(_)` buckets send
  `BreakerSignal::TripImmediately` with a 30-minute cooldown instead of plain `Failure`
  (a 429 is a definitive "stop", not a maybe — same reasoning as the GR anti-bot
  immediate trip). Book-provider buckets keep exactly today's `Failure` signal — their
  429 handling stays at the client layer (ST-8) and this design does NOT touch it.
- Callers need no new arms: an open breaker surfaces as `FetchError::CircuitOpen{retry_after}`,
  which both callers already funnel into their per-indexer warning path — the warning text
  becomes self-explanatory ("circuit open, retry in Ns"), the tick/click simply skips that
  indexer until cooldown. RSS sync's next tick after cooldown resumes normally.
- Deliberately NOT done: parsing Retry-After (no evidence our indexers send it; can be a
  follow-on if logs show it), per-indexer configurable cooldowns (TOML knob creep), and
  any change to book-provider 429 semantics.

## Test surface (red where constructible)

- Grab bucket pin: in-crate (livrarr-download) with the recording fetcher — a grab's
  file download must carry `RateBucket::Indexer(name)`, not None. RED today.
- Cache pins: in-crate — (a) `cache_only` makes zero fetches; (b) second default-mode
  search within TTL makes zero fetches and reports age; (c) `refresh` always fetches.
  All RED today (flags unread, no cache).
- Breaker pin: in livrarr-http — after a 429 on `Indexer("x")`, the next fetch for that
  bucket returns CircuitOpen while a different indexer bucket fetches fine. RED today
  (bucket untracked). NOTE: the outbound queue/breaker is process-global static —
  the pin must use unique bucket names and tolerate the insight-58 parallel-test caveat.
- Frontend mount behavior: not unit-pinnable here; verified in the live smoke (open a
  work's Releases tab → log shows zero indexer requests; click Search → fan-out).

## Out of scope

Readarr-style per-indexer failure statistics/UI, Retry-After parsing, SABnzbd 403
(install issue, separate), search-result pagination.
