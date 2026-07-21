# Design: Per-Indexer Circuit Breaker (issue #130, alpha6 regression)

**Status:** r3 — amended per round 2 (Codex R-6..R-9; Gemini r2 PASS) · **Author:** CC · **Date:** 2026-07-19
**Fixes:** one indexer's 429 blacking out ALL indexers behind the same Prowlarr host for 30 min.

## Problem

Commit `06748540` (alpha6, "indexer citizenship") keys `RateBucket::Indexer(String)` by
`normalized_origin(indexer.url)` and made `Indexer(_)` breaker-tracked, with any 429 tripping the
breaker immediately for 30 min (`crates/livrarr-http/src/fetcher.rs:199-208`). Prowlarr import
stores every indexer as `{prowlarr_base}/{id}` (`crates/livrarr-handlers/src/indexer.rs:475`), so
all Prowlarr-synced indexers normalize to ONE origin → one shared breaker. Prowlarr fail-fasts 429
for any single indexer in its per-indexer cooldown → that 429 blocks RSS, interactive search, AND
grab-file fetches for every indexer on that host for 30 min; the single half-open probe can hit
another limited indexer and re-trip indefinitely. This is the exact "aggregate bucket lets one bad
host suppress the rest" failure mode `breaker.rs`'s own doc comment forbids — the origin=single-host
premise is false for a proxy origin.

## Decision (the model) — two-level breaker (amended r2)

The bucket key currently conflates two failure domains. Separate them into two breaker levels with
disjoint signal sets:

| Level | Domain | Key | Trips on | Never sees | Gate point |
|---|---|---|---|---|---|
| **Transport** | physical host | pace projection (origin) | connection error, timeout, body-too-large — threshold 5/60s window/60s open (existing `config_for` defaults) | 429 | lane-level, as today (`run_dispatcher` handle gate) |
| **Rate-limit** | logical indexer | full bucket value (origin + indexer id) | a single 429 → `TripImmediately`, open = Retry-After or 30 min | transport failures | per-item, at grant time |

Pacing + in-flight cap stay keyed by origin (unchanged from 06748540 — politeness is to the machine).

*r2 change vs r1:* r1 dropped host-level breaking entirely and accepted the dead-host cost as "log
noise". Codex R-1 showed that is wrong for interactive search: search fans out one task per live
indexer (`release_service.rs:131` JoinSet), all sharing the 500ms lane with in-flight cap 2, so a
hanging (timeout, not refused) dead host would serialize ~N/2 × 30s of user-facing latency with no
breaker to cut the tail. The transport-level breaker restores today's dead-host protection exactly,
while 429s — the #130 cascade — are isolated per indexer.

## Mechanics

### 1. `livrarr-domain/src/services/http.rs` — variant shape

```rust
/// origin: pacing + transport-breaker domain (scheme://host[:port]).
/// indexer: rate-limit-breaker domain — stable DB id of the configured indexer row.
///   None = no rate-limit breaker (release-file fetches where no indexer row is in hand);
///   the transport-level lane gate still applies.
Indexer { origin: String, indexer: Option<String> },
```

Derives unchanged (`Debug, Clone, PartialEq, Eq, Hash`); process-internal, no serde/DB.

### 2. `livrarr-http/src/outbound_queue.rs` — two projections, two breaker levels

- Registry stays `HashMap<RateBucket, BucketHandle>` keyed by the **pace projection**:
  `fn pace_key(b: &RateBucket) -> RateBucket` — identity for all variants except
  `Indexer { origin, .. } → Indexer { origin, indexer: None }`. All indexers on one origin share one
  handle (heap, 500ms interval via `interval_for` struct-pattern update, in-flight semaphore).
- `BucketHandle.breaker` **stays** and becomes the **transport-level** breaker (per pace key;
  created iff `breaker_tracked(pace_key)`). For the six provider buckets it keeps today's exact
  role (their client-layer reporting is untouched).
- New map on `OutboundQueue`: `rate_limit_breakers: Mutex<HashMap<RateBucket, Arc<Mutex<BreakerState>>>>`,
  keyed by the **full bucket value**, created lazily iff `Indexer { indexer: Some(_) }`.
  Bounded: ≤ #configured indexers (see Lifecycle note).
- `QueuedItem` gains `rl_breaker: Option<Arc<Mutex<BreakerState>>>`, resolved once in `acquire()`.
- `run_dispatcher` (fixes Gemini R-1 TOCTOU + R-2 double-lock):
  1. **Early drain, one lock hold:** under a single `handle.state` lock: if heap empty → exit;
     while the TOP item's `rl_breaker` is Open → pop that item and send `Err(retry_after)`
     (no pacing consumed, no TOCTOU — peek/check/pop share the lock hold). Transport-breaker
     Open → pop top item, `Err`, continue (as today, per iteration).
  2. Pacing sleep, then semaphore acquire (unchanged).
  3. **Grant-time re-check:** pop the item under the state lock and re-check BOTH breakers
     *before* granting. If either is Open (tripped during the sleep/semaphore wait, or the heap
     reordered under priority): send `Err(retry_after)`, drop the permit (frees the slot),
     `continue` — `last_dispatch` does NOT advance. Only a successful `send(Ok)` advances the
     pacing clock (existing commit-point rule, unchanged).
- `report_outcome` routes by signal target (see §4); stale doc comments at
  `outbound_queue.rs:357-359` and `:214-218` (both still claim `Indexer(_)` is breaker-exempt)
  rewritten.

### 3. `livrarr-http/src/breaker.rs` — allowlist + honest comment

`breaker_tracked` gains the pace-projection semantics: six providers unchanged;
`Indexer { .. }` tracked at the transport level (the handle breaker exists for every indexer lane).
Rate-limit-level existence is decided by `indexer: Some(_)` in the queue, not here. Doc comment
rewritten: two levels, disjoint signals; the forbidden aggregate-suppression mode is what #130 hit.

### 4. `livrarr-http/src/fetcher.rs` — signal routing + Retry-After

`do_fetch` owns the full signal lifecycle for Indexer buckets (they have no client reporting layer;
provider buckets keep their existing client-layer reporting untouched):

- **Transport failures** (connection error, timeout, body-too-large): `Failure` → transport level.
  Unchanged behavior for provider buckets.
- **429 on `Indexer { indexer: Some(_) }`**: `TripImmediately { open_for }` → rate-limit level only.
  A 429 also reports **success to the transport level** (the host is alive and answering).
  429 on `Indexer { indexer: None }` (unresolved-identity fallback only, see §5): transport-success
  only, no trip, error still returned to the caller as `RateLimited`.
- **Anti-bot (Codex R-7):** intentionally out of contract for Indexer buckets — every indexer call
  site sets `anti_bot_check: false`. If a future indexer site ever enables it, the signal routes
  **transport-level** `TripImmediately` (an interstitial is a host-level block); add a pin test at
  that point, not now.
- **Any completed response** (any status) on an Indexer bucket: success → transport level; non-429
  → success → rate-limit level (closes a half-open cleanly).
- **`open_for`** (fixes Gemini R-3 by simplification, narrows Codex R-3): honor `Retry-After`
  **delta-seconds only**. Parse: integer string → seconds; value ≤ 0, non-integer (incl. HTTP-date
  form), or absent → default `INDEXER_RATE_LIMITED_COOLDOWN` (30 min). Clamp parsed values to
  **[10s, 6h]** (1–9s floors up to 10s; >6h caps to 6h). No HTTP-date parsing — delta-seconds is
  the only form honored because it is unambiguous under clock drift. No claim is made about what
  Prowlarr emits (Codex R-9): the contract IS the fallback — any other form → 30-minute default.

### 5. Call sites (6)

| Site | New bucket |
|---|---|
| `rss_sync_workflow.rs:120` (RSS fetch) | `Indexer { origin, indexer: Some(indexer.id.to_string()) }` |
| `release_service.rs:179` (search) | same, from the indexer row in scope |
| `release_service.rs:529, :596, :798, :911` (grab-file/magnet/transmission/usenet) | `Indexer { origin, indexer }` — identity resolved once in `grab()` (below) |

Identity = DB `indexer.id` (global table, stable, unique; survives renames). Compiler enumerates
any further sites via the struct-pattern change.

**Grab-path identity (amended r3, Codex R-6):** r2 exempted all grab-file fetches from the
rate-limit breaker ("user intent"). That contradicted the code's own shipped philosophy: RSS
auto-grab reaches the same path (`rss_sync_workflow.rs` grab loop → `release_service.grab`), and
`fetch_torrent_dispatch_source` already hard-fails on `RateLimited`/`CircuitOpen` precisely so a
cooldown is never bypassed (`release_service.rs:503-506`). So: **no exemption for anyone.**
`grab()` resolves the indexer row by the `GrabRequest.indexer` name (one `list_indexers` lookup)
and threads `indexer: Some(id)` into all four dispatch-path fetches — RSS and manual alike. A
manual grab against an open breaker fails **instantly** with `CircuitOpen{retry_after}` surfaced
in the grab error/notification — an honest fast "this indexer is cooling down, retry in Xm"
instead of a doomed attempt; failed RSS grabs keep feeding the existing 114a failure caps.
`indexer: None` remains only as the graceful fallback when the name no longer resolves (indexer
renamed/deleted between feed and grab): pace + transport gate still apply, no rate-limit breaker.
`GrabSource` stays as-is (history/notifications), unused for bucket identity.

## Lifecycle & bounds (Codex R-4, accepted-as-documented)

Rate-limit breaker entries are keyed by DB indexer id and have no deletion path; deleting an
indexer strands its entry (a few hundred bytes) for the process lifetime. Accepted: entries are
tiny, indexer deletion is rare, restart clears them, and a recreated indexer gets a fresh id (no
stale-state carryover). A pruning hook from the settings service into the queue is deliberately
NOT added — cross-crate plumbing for negligible gain. Revisit only if indexer churn becomes real.

## Test plan (r2 — additions from Gemini R-4/R-5, Codex R-2/R-3)

Unit (in `outbound_queue.rs` tests + `fetcher.rs`):

1. **#130 regression (load-bearing):** two indexers, same origin — `TripImmediately` on A →
   A's `acquire` → `Err`; B's `acquire` → `Ok` without waiting an extra pacing interval.
2. **Mixed-priority, mixed-breaker lane (Codex R-2):** ≥2 same-origin items, different indexer
   identities and priorities; open only one breaker → open item gets `Err` with no pacing slot
   consumed; closed sibling gets `Ok`; subsequent pacing advances only on the successful handoff;
   no starvation of the lower-priority closed item.
3. **Grant-time re-check race (Gemini R-1/R-4):** enqueue healthy item; trip its rate-limit breaker
   during the pacing sleep (or push a higher-priority item for a tripped indexer during the sleep)
   → the tripped item is rejected at grant time, permit released, clock not advanced.
4. **Transport level isolation:** connection-failure storm on one origin trips the lane after the
   threshold (dead-host protection preserved); a 429 storm does NOT move the transport breaker;
   a sibling origin is unaffected.
5. **`Indexer { indexer: None }`** (unresolved-identity fallback) is never rate-limit-gated
   regardless of 429 reports; it IS subject to the transport lane gate.
5b. **Grab-path workflow test (Codex R-8):** RSS-triggered grab whose indexer's rate-limit breaker
   is open → grab fails fast with no HTTP attempt, the failure is recorded (feeds the 114a caps);
   manual grab against the same open breaker → same fast `CircuitOpen` failure surfaced in the
   grab error. Identity-resolution fallback: a `GrabRequest.indexer` name that matches no
   configured indexer still grabs (bucket falls back to `indexer: None`).
6. **Retry-After matrix (Codex R-3):** absent → 30 min; `"120"` → 120s; `"3"` → 10s (floor);
   `"999999"` → 6h (cap); `"0"` / `"-5"` / `"Wed, 21 Oct 2026 07:28:00 GMT"` / garbage → 30 min.
7. **Half-open recovery per level:** rate-limit half-open closes on a non-429 completed response;
   transport half-open reopens on a transport failure (existing tests keep passing).
7b. **Half-open admission semantics (Codex r3 R-10, accepted-as-documented):** probes are not
   reserved at grant time, so a half-open breaker may admit up to the lane's in-flight cap (2)
   before the first outcome reports — at most one extra request against an indexer whose cooldown
   has already elapsed, and a second 429 simply re-trips. This is the pre-existing queue-wide
   property (provider breakers behave identically today); a probe-reservation mechanism is
   deliberately out of scope for this fix. Pin it: queue two same-indexer acquires while the
   rate-limit breaker transitions to half-open with no outcome reported → both may be granted;
   a subsequent 429 re-trips for the full cooldown.
8. **Update existing suites:** `indexer_bucket_is_breaker_tracked_and_trips_at_the_failure_threshold`
   (semantics now transport-level) and `crates/livrarr-http/tests/indexer_breaker_pins.rs`
   (Gemini R-5 — struct-pattern + two-level semantics), plus all existing pacing/priority/
   cancel-safety tests unchanged.
9. Quality gate: fmt zero diffs, clippy zero warnings, `cargo test --workspace --no-fail-fast`
   zero failures.

## Out of scope

- Per-indexer *pacing* fairness within one origin's lane (bounded by RSS being sequential and
  search's 30s per-task timeout; revisit only if observed).
- Prowlarr-specific URL parsing anywhere in livrarr-http (the fix is topology-agnostic).
- Wiki: insight 30 amendment + `breaker.rs` narrative ship with the implementation commit.

## Rollout

No migration, no config change, no API change. Ships as a normal fix; robgates can verify on the
next alpha (his repro: multiple Prowlarr indexers, one rate-limited).
