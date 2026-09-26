# Architecture Insights

Crate layout, entity model, and system-level structure.

### 1. **17-crate workspace**

1. **17-crate workspace.** All deps point toward `livrarr-domain`. `livrarr-server` is the composition root. `livrarr-handlers` owns all route handlers behind a compile wall. See [overview](architecture/overview.md). Core application crates (14): domain, http, db, matching, external-data, identity, enrichment, materialize, metadata, download, library, tagwrite, handlers, server. Plus: `livrarr-jobs` (JobService trait — compile-wall-safe job triggering from handlers without depending on livrarr-server), `livrarr-cli` (stub), `livrarr-behavioral` (test harness).

### 2. **BIG7 entities**

2. **BIG7 entities:** Author, Series, Work, Release, Grab, LibraryItem, List. See [big7](domain/big7.md).

### 3. **Work-first, not author-first**

3. **Work-first, not author-first.** The Work is the primary entity everywhere.

### 4. **One app, both formats**

4. **One app, both formats.** Ebooks and audiobooks. Per-media-type monitoring (`monitor_ebook`, `monitor_audiobook`).

### 6. **Collections = root folders**

6. **Collections = root folders.** 1:1 mapping. A collection IS a root folder with a name and shared toggle.

### 38. **Per-crate reference docs in `wiki/crates/`**

38. **Per-crate reference docs in `wiki/crates/`.** `handlers.md` has the full `Has*` trait table and route handler inventory. `domain.md`, `server.md`, `db.md` cover their respective crates. These pages are NOT linked from `wiki/index.md` — navigate to `wiki/crates/` directly.

### 48. **The canonical architecture model is live at…**

48. **The canonical architecture model is live at `docs/canonical-model.yaml`** (authored 2026-06-10; moved from `architecture/` in the 2026-06-29 cleanup). The entity spine (17 concepts — HistoryEvent added by the work-history architecture amendment, 2026-07-18), legal crate seams (full `livrarr-*` names), data_flow + invariants, and the amendments log. Gate rules: a feature IR v1 must carry `domain_entities` (names-only list of the spine concepts it introduces/touches; `[]` is valid) — only that marked set is spine-gated; IR crate-level dependency edges must be legal per `seams` (conceptual feature-internal edges are ungated); `verify.py canonical` reverse-checks every entity name against `pub struct/enum` in source. Any spine change (entity names or seam edges) requires a new `amendments[]` row. Deliberate not-yet-conformed reds stay ≤2 and logged — currently 1: `Release` (code carries `ReleaseSearchResult`; rename = issue #141). The live `library→tagwrite` edge is off-model (intent: save via `materialize` — amendment S1). Conformance backlog is tracked: issue #141 (Release rename), issue #143 (library→materialize cutover). The model↔code audit tool is `python3 ~/Projects/kk-build/audit_canonical.py /mnt/opt/livrarr` (report-only; exit 1 = hard drift; `--full`/`--json` for detail). First-audit baseline (2026-06-10, post-merge ab28699): entity_coverage 0.9375 (Release), seam_conformance 0.9787 (library→tagwrite), surface_sanction_ratio 0.024 (652 unsanctioned pub types, 154 true-noun candidates in livrarr-domain) — both hard-drift hits are exactly the two on-record deltas. **During the first feature through the armed gates, report every point of gate friction verbatim** (unclear error, awkward convention, a check firing on something legitimate) to the kk-build side instead of working around it — never soften a gate locally; the escape is an amendment or a kk-build-side fix.

### 64. **Discovery/search is its own service…**

64. **Discovery/search is its own service (work-service-split, 2026-07-11, commits `2734fd02..0094e805`).** `DiscoveryService` (3 methods: `lookup`, `lookup_filtered`, `eager_match_by_author`) lives in `livrarr-domain/src/services/discovery.rs`; `DiscoveryServiceImpl<C,H,L=StubNoLlm>` in `livrarr-metadata/src/discovery_service.rs` owns the 4 provider lookups, the LLM tail-filter, the 15-min lookup cache, and the resolver fast-path, implemented as free fns over a Copy `DiscoveryCtx` borrow-context. `WorkService` is 17 methods; `WorkServiceImpl<D,E,H>` (7 fields — no llm, no lookup_cache, no merge/tag/http_client) keeps the lifecycle (add/refresh/enrich/CRUD/merge-works) — its resolver field serves ONLY the identity legs (`settle_identity`). Handlers bind `HasDiscoveryService` for search; AppState carries `Arc<LiveDiscoveryService>`; main.rs's list-service and author-monitor WorkServiceImpl instances have no discovery/LLM wiring at all (they never search). `new_with_all` is gone — `WorkServiceImpl::new(db, enrichment, http, data_dir)` is the constructor. A second `HttpFetcherImpl` instance for discovery is safe: all fetchers share the process-global outbound queue (insight 30).

### 66. **Interactive add is fast-path + background completion…**

66. **Interactive add is fast-path + background completion (responsiveness Lane A, 2026-07-11).** `POST /works` no longer blocks on providers: the handler derives identity LOCALLY (`WorkService::resolve_identity_local` — sync, zero network: work anchor → Confirmed, bridge/none → Pending), calls `add_fast` (all of old `add` through phase-1 cover + badge persist, PLUS a verdict-gated bridge dedup: `find_works_by_bridge(user, isbn, asin)` hits dedup iff `title_verdict ∈ {Same, Grey}` AND `author_verdict ≠ Disagree` — collision shapes create separately per the bridge-anchor policy), then spawns `complete_add` (the identity fan-out + enrichment + cover gates, wrapped in the in-memory `enriching` registry guard; +5s anchor top-up refresh CHAINED after completion, never parallel). `ensure_identity_and_enrichment` remains ONE gated road — Pending/Conflict/NeedsReview blocks enrichment for every caller including complete_add (an implementing agent's bypass variant was reverted; the behavioral test pins the gate). `is_enriching` is process-local by design (false after restart; convergence owns durable recovery); `WorkDetailResponse.enriching` + `AddWorkResponse.created` are the API surface; the detail page pill/poll (1.5s→5s, 60s cap) + skeletons consume it. Existing sync callers of `add` are UNCHANGED (`add` = `add_fast` + awaited `complete_add`). Measured: add POST 19.8s/3.1s/2.1s → 3.0s/7ms/513ms (ceiling = the 3s phase-1 cover budget). Grids serve the existing 300px thumbs via `BookCover variant` (default thumb, detail hero full) — 3.65× lighter first view. _U-C2 (2026-07-12):_ all four image routes honor a `?v=` token (`CoverQuery` in `livrarr-handlers/src/mediacover.rs`) — versioned requests return `Cache-Control: public, max-age=31536000, immutable` (repeat views = zero image requests; the UI already sends `?v=<cover_mtime>`, and a cover change bumps the mtime → new URL), unversioned keep `no-cache`+ETag, missing covers stay `no-store`. Tests: `tests/behavioral/test_responsiveness_add.rs` (8). Baseline/after: `docs/speed-baseline-2026-07-11-responsiveness.md`.
