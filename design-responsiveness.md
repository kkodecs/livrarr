# Design: responsiveness

**Date:** 2026-07-11 · **Spec:** `spec-responsiveness.md` (v1, review-passed 2026-07-11) · **Stage:** design (lightened path per PO process decision — design doc + cross-family review; no IR/contract ceremony)

Every code claim below was sampled against main `2e623f53` on 2026-07-11. Call-site annotations: **NEW** (to be written), **EXISTING** (verified present), **LIB** (external crate, already approved in-workspace).

## 1. Unit map — spec REQs → build units → files

| Unit | REQs | Files touched | Lane |
|------|------|---------------|------|
| U-0 baseline | REQ-014 | `scripts/speed-baseline.py` (EXISTING, untracked local — extend), new report in `docs/` | first, before any code |
| U-C1 thumbs+lazy | REQ-001, REQ-002 | `frontend/src/components/BookCover.tsx`, `frontend/src/utils/format.ts`, grid/list callers of BookCover | **Lane F** (frontend) |
| U-C2 versioned caching | REQ-003 | `format.ts` (+version params), `crates/livrarr-handlers/src/mediacover.rs`, `crates/livrarr-handlers/src/cover.rs` (conditional Cache-Control) | Lane F, after U-C1 |
| U-A instant add | REQ-004..008 | `crates/livrarr-handlers/src/work.rs` (add), `crates/livrarr-handlers/src/types/work.rs`, `crates/livrarr-metadata/src/work_service.rs`, `crates/livrarr-domain/src/services/work.rs` (trait), frontend work-detail page + poll | **Lane A** (backend) |
| U-B1 provider cache | REQ-009 | new migration `072_provider_response_cache.sql`, `crates/livrarr-db/` (trait+impl), `crates/livrarr-enrichment/src/provider_queue.rs` (seam), enrichment entry points (freshness param), `crates/livrarr-server/src/config.rs` (TOML) | **Lane B** (backend) |
| U-B2 HC batching | REQ-010 | probe script + recorded artifact ONLY (batching design deferred to a post-probe follow-up — spec entry gate) | Lane B, after U-B1; **probe-gated** |
| U-B3 reuse measurement | REQ-011 | log-capture script + `docs/` report; tuning unit only if measured cold | anytime, independent |
| U-B4 client consolidation | REQ-012 | `crates/livrarr-server/src/main.rs` only | after lanes converge |
| U-B5 bounded bulk refresh | REQ-013 | `crates/livrarr-handlers/src/work.rs` (`refresh_all`) | after U-A (same file) |
| U-X dead code | REQ-015 | `crates/livrarr-domain/src/services/work.rs` (`AddWorkRequest` deletion) | fold into U-A |

**Parallelization:** Lane F ∥ Lane A ∥ Lane B (file-disjoint). Sonnet subagents implement; Serena-based edits serialize globally (no parallel worktrees — standing constraint); Lane F uses built-in edits (TS/TSX). U-B4/U-B5 run last, serial. Tests-first per unit (red gate before implementation, Codex authors test bodies from the TDD directives below).

**Dependencies:** no new external crates anywhere. `approved_libraries` unchanged: sqlx/serde (U-B1), `image` crate (already used by `get_thumb`), std `Mutex<HashSet>` for the registry (same pattern as `refresh_locks` — EXISTING `work_service.rs:17`).

## 2. U-A — instant add (the core design)

### 2.1 Current synchronous chain (EXISTING, all sampled)

`handlers/work.rs:203-223` awaits `resolve_identity(.., LatencyTier::Interactive)` (provider fan-out possible) → `:224-235` resolver-conflict branch returns existing work → `:259` awaits `work_service().add()` → inside `finish_created_work` (`work_service.rs:1807`): phase-1 cover await (`:1849-1858`, 3s budget) → `ensure_identity_and_enrichment` await (`:1902-1911`) → `run_unified_enrichment` (`:1969-2237`: anchor-poor settle `:2002-2025`, scatter `:2037-2047`, cover gates `:2089-2118`, materialize, dims backfill).

### 2.2 Target chain

**Synchronous (response path):** sanitize seed → **local** badge derivation → dedup-aware create → phase-1 cover → respond. **Background (spawned task):** identity completion + enrichment + cover gates + materialize — i.e., exactly today's `ensure_identity_and_enrichment` call, moved.

Pseudocode (handler `add`, NEW structure over EXISTING pieces):

```
add(state, ctx, req):
    language = resolve seed language                       # EXISTING :186-187
    local = work_service.resolve_identity_local(harvest)   # NEW — see 2.3
    candidate = seed_add_box(seed_input, local.identity, req.candidate_id, cover_is_manual)  # EXISTING :241
    result = work_service.add_fast(user, candidate)        # NEW variant — see 2.4
    if result.created:
        spawn work_service.complete_add(user, work_id, source_data, candidate_id, mode, source)  # NEW — see 2.4
        (keep the existing +5s top-up refresh spawn        # EXISTING :263-273)
    respond AddWorkResponse{ work (with enriching=true when spawned), created: result.created,
                             author_created, messages }    # types NEW fields — see 2.6
```

### 2.3 `resolve_identity_local` (NEW, factored from EXISTING)

`resolve_identity` (`work_service.rs:678-805`) already has a clean local prefix: `WorkSeed::sanitized` + anchor-presence check (`:695-701`), anchorless → `Pending{NoCandidates}` (`:703-716`), no-resolver → Pending with captured seed (`:729-744`). Only `resolver.resolve(user_id, &seed, tier)` (`:747-750`) goes to the network. The local function:

```
resolve_identity_local(harvest) -> ResolvedIdentity:      # zero network, zero DB
    seed = WorkSeed::sanitized(harvest), keep if any anchor  # EXISTING logic :695-701
    none -> Pending{NoCandidates}                            # EXISTING :703-716
    some -> badge from the D-013 derivation rules the backfill already uses:
              work anchor (ol/gr/hc key) present -> Confirmed{anchors: captured, method: SeedAnchors}
              isbn/asin only                     -> Pending with captured seed (Provisional badge
                                                    derives at create exactly as today's derived_identity write)
    conflict: always None here                               # see D-002
```

The full `resolve_identity` stays for every other caller (unchanged signature and behavior).

### 2.4 `add_fast` + `complete_add` (NEW, split of EXISTING `add`/`finish_created_work`)

- `add_fast` = today's `add` up THROUGH the phase-1 cover write and badge persist (`finish_created_work` `:1823-1900`), then **returns** — dedup paths (`try_dedup_by_normalized`, anchor dedup, `handle_race_loser` — all EXISTING) unchanged and still synchronous (they are local DB checks; a duplicate still returns `created=false` with the existing work immediately, satisfying REQ-004's local-conflict clause). Dedup surface today (sampled): anchor dedup matches WORK anchors only — ol/gr/hc (`work_service.rs:256-278`), deliberately never isbn/asin ("bridge-anchor policy" comment `:258-260`); Pending candidates dedup by normalized title/author (`try_dedup_by_normalized` `:1592-1632`).
- **NEW dedup step — verdict-gated local bridge dedup (round-2 fix, replaces the round-1 "accept the window" stance):** for a bridge-only candidate (isbn/asin, no work anchor), `add_fast` runs a local DB lookup over the user's works by that isbn_13/asin (NEW small `livrarr-db` query, `find_works_by_bridge`). On a hit, the match is gated by the ONE matching authority (`livrarr-domain/src/identity_matching.rs` — EXISTING pure functions over strings; sampled `:70-98`). Exact predicate (round-3 fix — exact enum variants): dedup ⟺ `title_verdict ∈ {Same, Grey{..}}` **AND** `author_verdict ∈ {Agree, Grey, Abstain}` (i.e., reject only `Disagree`; `Abstain` passes because bridge seeds may carry no author and the isbn is the lookup evidence). `TitleVerdict::Different` or `::VetoVolume`, or `AuthorVerdict::Disagree` → NOT a dedup: proceed to create (the ISBN-collision arm the bridge-anchor policy exists for — same ISBN + disagreeing titles must never merge, insight 52). Zero network, zero new matching logic, and the bridge key is a LOOKUP hint gated by verdicts — not a merge anchor — so the bridge-anchor policy stands. Boundary note: the verdict functions live in `livrarr-domain` and `livrarr-metadata` already depends on it (every workspace arrow points toward domain), so calling them from `add_fast` (service logic, not DB code) is the normal direction — no new edge, no cycle. See D-007.
- `complete_add` = the remainder of `finish_created_work` (`:1902-1923`: `ensure_identity_and_enrichment` + NotFound badge write) wrapped in the enriching-registry guard (2.5). `ensure_identity_and_enrichment` (EXISTING `:1679-1770`) already runs the add-time identity leg (`settle_identity`) for anchorless works and the anchor-poor completion inside `run_unified_enrichment` (`:2002-2025`) — the background task needs NO new identity machinery; the batch doors already work exactly this way (M9).
- Spawn mechanics: handler-level `tokio::spawn` (insight 9g — handlers own `state.clone()`; services can't spawn themselves). The spawned future calls a `WorkService` trait method; no `AppState` capture inside the service.

### 2.5 The `enriching` registry (NEW, REQ-005 contract)

```
WorkServiceImpl field: enriching: Arc<Mutex<HashSet<(UserId, WorkId)>>>   # same shape as EXISTING refresh_locks :17
complete_add / refresh-triggered runs:  insert on start (RAII guard), remove on exit (Drop — panic-safe)
trait method: is_enriching(user, work) -> bool                            # NEW on WorkService
```

- True exactly while a run executes. Server restart ⇒ empty set ⇒ false — never stale-true (spec REQ-005). Recovery of interrupted runs: EXISTING convergence job (config defaults `config.rs:182-196`) re-selects unsettled works — no new machinery.
- The registry guard also wraps the existing `refresh` path so a user-triggered Retry shows *fetching* too. The per-work `refresh_locks` (EXISTING) still prevents concurrent duplicate runs (REQ-007) — registry is signal, lock is mutual exclusion; they stay separate.

### 2.6 API contract (NEW fields, additive)

- `AddWorkResponse` (`types/work.rs:139-143`): add `created: bool`, and the embedded `work` carries `enriching`.
- `WorkDetailResponse` (`:168-221`): add `enriching: bool` (populated via `is_enriching` in the detail handler and everywhere `work_to_detail*` builds it).
- `enrichment_status` semantics in the add response change from final to at-return (spec supersedes ST-005; frontend must not treat it as terminal).

### 2.7 Frontend (work-detail page, per approved mockup `ui/responsiveness-add-progress.html`)

- Pill from `(enriching, enrichment_status)`: fetching / complete (Enriched|Thin) / attention+Retry (Failed, or `enriching=false` while Unenriched after a 60s poll cap).
- React Query polling while `enriching===true`: 1.5s for the first 15s, then 5s, hard cap 60s (D-006), stop on settle; skeletons on the fill-in fields; cover img re-renders when polled `cover_mtime` changes (BookCover `coverVersion` prop — EXISTING).
- Identity badges unchanged (pill is enrichment-only, REQ-005/AC-010).

## 3. Lane F — covers

### 3.1 U-C1

- `BookCover` (`BookCover.tsx:27-58`) gains `variant: "thumb" | "full"` (default `"thumb"` — grids are the overwhelming callers; detail hero passes `"full"`). Both `<img>`s (blur backdrop + main) use the variant URL.
- `getCoverThumbUrl(workId, v?, mediaType?)` (extend EXISTING `format.ts:54-56`) → `/mediacover/{id}/thumb.jpg` or `/audiocover_thumb.jpg`, `?v=` when provided.
- Loading hygiene on both imgs: `loading="lazy"` + `decoding="async"` on grid variant; explicit `width`/`height` (or aspect-ratio via the existing sized container — the component already reserves box dimensions through `className` h/w classes, so CLS is handled; verify in AC); `fetchpriority="high"` on the first-row hero only (detail page).
- Callers audit: every grid/list caller passes `coverVersion` (from `cover_mtime` already present in list/detail payloads — EXISTING `types/work.rs:217-220`); wanted/works/series/author pages inherit the thumb default by not passing `variant`.

### 3.2 U-C2

- `serve_image` (`mediacover.rs:125-173`) gains the request query: **versioned request** (`?v=` present) → `Cache-Control: public, max-age=31536000, immutable` (ETag kept harmlessly); **unversioned** → today's `public, no-cache` + ETag exactly as now. 404/placeholder stays `no-store` (`:117-123`). Same change where the audiobook handlers serve (`cover.rs` paths).
- Version value = the mtime integers already flowing (`cover_mtime`/`audiobook_cover_mtime`); thumbs share the parent cover's version (a cover change deletes thumbs via EXISTING `invalidate_thumbnails` `cover_write_gate.rs:600` and bumps mtime → new URL → fresh fetch). Token is cache-identity only (spec REQ-003).

## 4. Lane B — fewer calls

### 4.1 U-B1 provider cache

- **Migration 072** `provider_response_cache(provider TEXT, anchor_type TEXT, anchor TEXT, payload TEXT/JSON, fetched_at TEXT, PRIMARY KEY(provider, anchor_type, anchor))` + count-capped eviction (oldest `fetched_at` first). Global (not per-user): provider payloads are user-independent public metadata.
- **Seam:** the ONE dispatch point `provider_queue.rs:286` (`client.fetch_by_anchor(anchor, language, priority)` — EXISTING, sole enrichment network call per insight 30/51). Wrap:

```
fetch_with_cache(provider, anchor, freshness):
    if freshness == PreferCache and fresh row (age < ttl) exists -> return cached payload (no queue, no HTTP)
    outcome = client.fetch_by_anchor(...)                  # EXISTING :286
    if outcome is Success(payload) -> upsert cache row     # success ONLY: errors/not-found/partial never cached
    return outcome
```

- **Freshness knob (NEW):** `Freshness { PreferCache, Bypass }` threaded through `enrich_work`/`run_unified_enrichment` entry points. `refresh(Interactive|Bulk)` → `Bypass` (both `RefreshSurface` variants are user-triggered — EXISTING enum `services/work.rs:338-346`); add/re-add, convergence, list import, monitors → `PreferCache`. Note: freshness is orthogonal to `RequestPriority` (refresh_all is Low-priority but Bypass — priority orders the queue, freshness decides cache).
- **Config (TOML only):** `[metadata_cache] ttl_days = 7, max_rows = 100_000` in server config (defaults per spec Q-001).
- Call records: a cache hit makes NO provider call and therefore writes NO call record (call records instrument real HTTP — semantics preserved; AC-014's zero-HTTP assertion reads call records, consistent).

### 4.2 U-B2 HC batching — CANCELLED (PO decision, 2026-07-12)

PO's documented reason, verbatim: "Claude was wrong. This is NOT worth it. Gemini and Codex are unanimous in their judgement. This was a goose chase."

Trail: probe artifact recorded (`docs/hc-batch-probe-2026-07-11.md`, both batch mechanisms live); a batching design draft (prefetch-through-the-cache) FAILED cross-family review round 1 from BOTH families (traffic accounting false for uncovered anchors; GraphQL partial semantics unspecified; axis/scope findings — `build/reviews/responsiveness/review-design-{google,openai}-r8.json`); a cross-family confer on the corrected concept was unanimous against building it — the sweep wall is bound by the Goodreads (161 works x 1.5s) and Audnexus (134 x 2s) pacing buckets, so removing Hardcover requests cannot move it, and the U-B1 cache already zeroes repeat Hardcover fetches on every background flow. Residual value (quota headroom on first-contact large list imports) judged not worth the cross-work machinery. REQ-010 carries the cancellation; AC-016 is void.


### 4.3 U-B3 measurement

`RUST_LOG` connect-level tracing (hyper/reqwest connect targets) captured during a scripted N-work refresh on the dev instance; parse new-connection count vs request count per provider bucket → `docs/connection-reuse-report-<date>.md`. Tuning (keepalive/pool sizing) is a FOLLOW-UP unit only if cold-rate is material; not designed now (spec REQ-011).

### 4.4 U-B4 consolidation (main.rs only)

From the verified inventory (subagent sweep 2026-07-11, spot-checked): 5 `HttpClient` builds, 12 `HttpFetcherImpl::new` calls, 3 LLM `HttpClient::builder().build()` calls **with no timeout/UA at all** (`main.rs:549-551, 598-600, 614-616`). Target: ONE shared `HttpFetcherImpl` (clone into work/discovery/author/series-query/release/rss/list/monitor constructions — all fetchers already share the process-global outbound queue, ST-012, so behavior is unchanged); ONE shared LLM `HttpClient` **with timeout + UA set** (fixes the unbounded-LLM-call latent bug); the `http_client`/`http_client_safe` trust split untouched (ST-011). The 3 duplicate `WorkServiceImpl`/`ReleaseServiceImpl` constructions stay as-is (out of scope — service topology, not client pooling).

### 4.5 U-B5 bounded bulk refresh

`refresh_all`'s serial loop (`handlers/work.rs:714-731`) → `futures::stream::iter(works).map(|w| refresh(w)).buffer_unordered(3)` with the same per-work match arms (enriched/failed counters), same `_bulk_guard` RAII, same completion notification. N=3 constant (queue caps make higher N pointless — ST-012). Failure isolation preserved by the per-item match (REQ-013).

### 2.8 `WorkService` trait surface (the compile-wall API — fixes review R-001)

The trait (`livrarr-domain/src/services/work.rs`) gains exactly FOUR methods, all called by the generic handler through `HasWorkService`:

```
resolve_identity_local(harvest) -> ResolvedIdentity      # 2.3 — zero network
add_fast(user, candidate) -> AddWorkResult               # 2.4 — response path
complete_add(user, work_id, source_data, candidate_id, mode, source)  # 2.4 — background body
is_enriching(user, work_id) -> bool                      # 2.5 — signal read
```

`add` (EXISTING) keeps its name, signature, and synchronous semantics for every other caller (batch doors, list import, Readarr, monitors — untouched), but its IMPLEMENTATION becomes `add_fast` + an awaited `complete_add` — one pipeline, one implementation, two entry shapes. No divergent road: `complete_add` IS the existing `ensure_identity_and_enrichment` behind the trait.

## 5. Design decisions

- **D-001 — `enriching` is in-memory, not persisted.** Registry (2.5) over a DB column: restart-false comes free (spec demands never-stale-true), no migration, no writes per run; the convergence lane owns durable recovery. Rejected: DB in-flight status (needs crash-cleanup logic, duplicates what convergence already guarantees).
- **D-002 — add-time badge derives locally; resolver conflicts move post-create.** The handler's pre-create `resolve_identity` call (network-capable) is replaced by `resolve_identity_local` (2.3). A user-confirmed search pick carrying a work anchor is Confirmed by the same D-013 derivation the DB backfill uses; isbn/asin-only and title-only seeds create Pending/Provisional and converge in background — exactly the batch-door pattern (M9). Consequence (spec-sanctioned, REQ-004): the rare resolver-detected cross-anchor conflict surfaces post-create via the existing identity-conflict machinery instead of pre-create. Local DEDUP (same-anchor, normalized-title) still returns the existing work synchronously — that path never needed the resolver.
- **D-003 — cache is global, success-only, at the single dispatch seam.** One seam (`provider_queue.rs:286`) = policy lives in one place (the merge-chokepoint lesson, insight 56). Success-only prevents TTL-pinned transient failures (spec REQ-009).
- **D-004 — freshness is an explicit parameter, not inferred from priority.** refresh_all proves priority (Low) and freshness (Bypass) are orthogonal — inferring one from the other would silently serve a user stale data or de-prioritize background cache reads.
- **D-005 — thumb version = parent cover mtime.** Thumbs are derived artifacts; `invalidate_thumbnails` already couples their lifetime to the cover file. One version value per slot, already present in every payload — no new plumbing.
- **D-006 — poll with decay: 1.5s for the first 15s, then 5s, hard cap 60s.** Per spec REQ-006/-008 and the reviewed analysis's SSE rejection. Cap → attention state (never infinite spinner). Decay keeps an open detail page from hammering the handler when enrichment runs long (review R-004).
- **D-007 — bridge-only dedup is verdict-gated local lookup; only the collision-shaped slice ever creates separately (round-2 revision after codex R-003 correctly held the spec against round 1's "accept the window").** REQ-007/AC-012 demand that re-adding the same book returns the existing work — so `add_fast` gains the local bridge lookup (§2.4): isbn/asin hit + `title_verdict` Same/Grey → existing work returned synchronously, `created=false`. This is NOT a bridge-anchor-policy violation: the policy (`work_service.rs:258-260`, insight 52) forbids bridge EQUALITY acting as merge evidence over disagreeing titles — here disagreeing titles (Veto/Different) explicitly refuse the dedup and create separately, which is precisely the collision-safe behavior the policy wants. Residual window (deliberate): a bridge-only re-add whose seed title genuinely fails the verdict against the stored work (true collision, or a cross-script retitle the verdicts cannot equate) creates a visible Pending work that background identity anchors; resolution = the standing grey-never-absorbs philosophy (visible duplicate + EXISTING merge-works action). That residual is the slice where auto-dedup would risk a wrong-book merge — the worse failure. Today's pre-create fan-out has the same collision exposure (providers resolve the isbn to one of the colliding works); batch doors already live with the full window (M9).

## 6. TDD directives (behavioral tests per unit — red before implementation)

- **U-A:** slow-stubbed providers ⇒ add returns fast for all three seed shapes (AC-007); `enriching` true-then-false lifecycle over add (AC-009); duplicate add ⇒ `created=false`, one pipeline (AC-012) — including the bridge-only shape: isbn-only re-add of an existing isbn-carrying work with `title_verdict Same/Grey` + `author_verdict ≠ Disagree` ⇒ `created=false`, one row; bridge COLLISION shapes (same isbn with `TitleVerdict::Different`/`::VetoVolume`, or `AuthorVerdict::Disagree`) ⇒ separate work created (the policy arm); restart-simulation ⇒ registry empty, convergence completes (AC-013, harness restarts service); identity-parked work keeps badge, pill-state fields don't mask (AC-010); Thin ⇒ complete state (AC-009).
- **U-C1/C2:** grid payload requests thumb URLs, detail requests full (AC-001); versioned response carries immutable header, unversioned keeps no-cache+ETag, 404 no-store (AC-003/005); cover change ⇒ URL changes (mtime bump) and old cache entry unused (AC-004). (Header assertions are Rust handler tests; grid-request assertions are Playwright.)
- **U-B1:** two background passes within TTL ⇒ zero provider HTTP second pass via call records (AC-014); error/not-found never cached (AC-014); user refresh bypasses + rewrites (AC-014); TOML TTL/cap honored + eviction (AC-015).
- **U-B5:** N works with one failing ⇒ sweep completes, counts correct, pacing intervals respected in queue log (AC-019).
- **U-B4:** existing SSRF behavioral tests stay green (safe client still rejects private IPs) + full workspace green (AC-018).

## 7. Compliance

- No new crates; no new service traits — FOUR methods added to the existing `WorkService` trait (`resolve_identity_local`, `add_fast`, `complete_add`, `is_enriching` — §2.8) plus two narrow DB-trait additions in livrarr-db (`find_works_by_bridge` for §2.4's verdict-gated dedup; the U-B1 cache read/write methods); `add` keeps its signature and synchronous semantics for all existing callers; compile wall untouched (handlers keep calling trait methods only); no SQL outside livrarr-db; all spec REQ-IDs map to units (§1 table); `chrono` for cache timestamps; migrations append-only (072 new).
- Landmines respected: outbound queue/pacing untouched (ST-001); SSRF split untouched (ST-011); no OL UA changes; snapshot-first ops rule applies to any real-data bulk run during testing (insight 49).
