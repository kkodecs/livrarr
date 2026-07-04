# Metadata System Audit — 2026-06-28

**Scope:** the metadata subsystem — providers, identity, enrichment, merge, materialize, and the service layer that drives them (8 crates).
**Type:** read-only. Zero code changed.
**Method:** five parallel inventory agents (Serena/LSP-driven), then the orchestrator re-opened every load-bearing `path:line` before writing. Citations the orchestrator personally opened are marked ✓-verified; the rest rest on subagent symbol-reference checks and are flagged as such.
**Reconciliation:** this extends the 2026-06-07 lifecycle audit (`docs/metadata-lifecycle-audit-*.md`). Where that audit's findings are now fixed, I say so and move on. The new ground here is the **transport fork** and the **matching fork** — which the prior audit explicitly did not cover.

> **Line numbers:** orchestrator-opened citations use 1-based editor lines (exact). Subagent-sourced citations may be ±a few lines (Serena reports 0-based). Every claim states which.

---

## Executive summary — the 8 worst problems

Plain-English, no Rust knowledge needed.

1. **The "is this the same book?" check exists in two brains that can disagree.** The identity engine has its own private copy of the title-matching logic. It keeps little words ("the", "of"), keeps accents, and chops every title at the first colon — the shared, canonical version does the opposite on all three. Same 0.75 cutoff, different yes/no on the same two titles. This is a **correctness** risk: it decides which books are merged into one. *(High)*

2. **One of the three ways we call book websites has no speed limit at all.** Calls go out through three separate doors. Two have brakes (each its own, uncoordinated). The third — the identity lookup that fires when a book is added or refreshed — has **no brake**. That's the real cause of getting blocked (HTTP 429 / bans) during bulk operations, not the door the earlier signal pointed at. *(High)*

3. **There is no single place that limits how fast we hit each website.** Rate limiting is scattered across two live mechanisms that don't share state, one unlimited path, and **two dead mechanisms left lying around** — including a "Goodreads rate limiter" that is built at startup and then never used. Goodreads can be hit from three places at once. *(High)*

4. **The Hardcover website call is copy-pasted 4–5 times, and the copies have drifted.** Same query, pasted into 4 files, now with different page sizes and different quoting — so the same book can get different results depending on which copy ran. Only one copy has a speed limit. *(Med–High)*

5. **Each provider is parsed twice, in two different crates.** The "search/discovery" path was bolted onto the big service file instead of reusing the provider code next door. OpenLibrary, for example, has a second hand-written JSON parser that lives only in the service. *(Med)*

6. **The core service file is a 3,684-line god object — and it grew since the last audit.** It mixes identity, per-provider lookups, enrichment orchestration, CRUD, covers, and refresh in one file. Pure maintainability drag; it hides the duplication above. *(Med)*

7. **A large pile of dead code misleads readers.** Whole unused rate-limiter module, dead LLM modules, an orphaned bulk resolver, dead config fields. The danger isn't that it runs — it's that it lies: someone reading the code believes Goodreads is throttled (it isn't, on that path). *(Med)*

8. **Audiobook cover sizes are computed and then thrown away.** The width/height are decoded but never saved for the audiobook slot, so they read as zero forever. Small, but a real never-written-field bug. *(Low–Med)*

**The good news (state it plainly, so the report isn't alarmist):**
- The **merge core is sound and deterministic** — fixed tie-break ordering, priority-list walk, and a null-guard so empty data never overwrites real data. ✓-verified.
- The earlier audit's top-priority bug (#133, foreign-language data leaking through one of two paths) is **fixed** — the language drop now lives at a single merge chokepoint that both paths go through. ✓-verified.
- **Every work-creation door funnels through one gate** (`ensure_identity_and_enrichment`) — no bypass. The earlier audit's door-wiring worry is closed.
- The enrichment fan-out **is** parallel and rate-limited (just not coordinated with the other two paths).

**Bottom line:** the *pipeline shape* is healthier than the 2026-06-07 audit (doors unified, merge centralized, foreign-drop fixed). The remaining rot is in two layers that audit didn't reach: **transport** (forked, fragmented rate limiting) and **matching** (two diverging brains). Those are the two things to fix, in that order.

**Pass 2 (post-access tail, L3–L8) added three more High-severity problems** — these are *correctness/identity-lifecycle* bugs, distinct from the transport/matching rot above (full bodies in "Post-access correctness findings — Pass 2"):
- **M-010** — Readarr import data is injected, then silently dropped before merge; Readarr contributes nothing.
- **M-019** — a work that hits an identity `Conflict` can never leave it: user resolve/dismiss clear the conflict row but not the status badge, and re-resolution is permanently blocked. Stuck + un-enrichable forever (only delete+re-add escapes).
- **M-020** — affirming a pending identity guess is silently ineffective: the badge never reaches `Confirmed` and the work lands in `NeedsReview` limbo (the work still gets metadata; the identity badge is the casualty).

---

## Cross-family review — corrections & additions (2026-06-28, post-audit)

Both **Gemini** and **Codex** independently re-verified this report against source (each opened the cited code itself; neither trusted the report's ✓ marks). **All eight findings M-001…M-008 were CONFIRMED in source by both families.** The spine stands. The corrections below refine it — **apply these when planning; they override the original counts/scope where they conflict.**

**Confirmed exactly as written:** M-002, M-005, M-007, M-008.

**Corrections:**

- **M-001 — the unthrottled-surface count is too low: it is 6+, not 3.** Beyond the identity fan-out, these also reach providers with no shared throttle:
  - `cover_alternatives.rs:61-90` — `join_all` over `client.fetch`, timeout only (re-confirmed in source).
  - `preadd_cover_service.rs:39-49` — spawns provider fetches, timeout only.
  - `cover_service.rs:408-424` — direct `client.fetch(work)`.
  - **Audnexus and Audible bypass the `HttpFetcher` abstraction entirely** (raw `reqwest`, `audnexus.rs:118`). This is the **root cause** of M-006's "dead `RateBucket::Audnexus/Audible`" — those buckets are unreachable because those clients never go through the fetcher that would honor them.
  → **Fixing only the identity path will NOT stop the 429s.** Phase C must cover every direct-fetch surface and move Audnexus/Audible onto `HttpFetcher` first.

- **M-001 calibration — soften the localization, not the mechanism.** "60% this is THE 429 cause" over-localizes to identity. Rate-fragmentation is real and likely a primary driver, but it is spread across 6+ surfaces (and anti-bot/DataDome is a separate factor). Reframe: high confidence rate-fragmentation is a primary cause; identity is one of several contributors, not the sole one.

- **M-003 — the count is exactly 4, not "4–5."** Both families enumerated the same four: `hardcover.rs:73`, `hardcover.rs:477`, `cover.rs:319`, `work_service.rs:2599`. Lock to 4.

- **M-004 — scope is narrower than stated.** Only **OpenLibrary** has a genuine second inline parser in the service (`work_service.rs:2372-2479`). Goodreads delegates to `parse_autocomplete_json` and Google Books to `fetch_gb_volumes` — they build queries in the service but reuse the provider crate's parser. Restate from "each provider parsed twice" → "OL parsed twice; GR/GB build queries in the service but share parsers."

- **M-006 — one item is NOT safe to delete.** `bulk_resolver::resolve_bulk` has live callers in ignored behavioral tests (`tests/behavioral/test_ewl_bulk_resolver.rs:6, :100-109`); deleting the source alone breaks test compilation. A1 must migrate or remove those tests first. (All other M-006 items confirmed dead by both families.)

- **M-006 — add one misleading-comment item.** `main.rs:351-367` says the merge engine uses LLM arbitration; `DefaultMergeEngine::new_with_llm` (`enrichment/lib.rs:412-421`) accepts and **discards** the LLM caller — the merge is deliberately deterministic (REQ-005). Behavior is correct; the comment lies. Same "misleads the reader" class as the dead rate limiter.

- **Title-normalizer count — ≥6, not 5.** Add `normalize_title_variants` (`matching/lib.rs`) and `work_dedup`'s second normalizer. Strengthens M-002: matching is even more forked than stated.

**New finding:**

- **M-009 — the discovery rate limiter is per-instance, not process-global. Severity: High.** `RateLimiterMap` is a field on `HttpFetcherImpl` (`fetcher.rs:129-154`), and the server constructs **multiple** `HttpFetcherImpl` instances (`main.rs:573, :706, :795`). So even the one path the report called "throttled" can run several uncoordinated per-provider buckets at once. The Phase C limiter must be a **single shared instance** injected everywhere — the true single source of truth.

**Roadmap deltas:**

- **A1**: before deleting `bulk_resolver`, migrate/remove its ignored behavioral tests.
- **C1**: widen to **all** direct-fetch surfaces (identity + the 3 cover paths), make the limiter **process-global** (one shared instance, M-009), and move **Audnexus/Audible onto `HttpFetcher`** so their buckets become reachable. This is materially bigger than the report's original "throttle the identity fan-out."
- **D1**: the existing C1 colon test (`english_identity_resolver.rs:919`) is a **false green** — it passes only because the two payloads carry conflicting anchors, which trip the anchor-guard (`agree`'s `opt_differs`) *before* the buggy title logic runs. It does **not** guard the real failure mode (keyless payloads). Re-confirmed in source. Before the Phase D fix, rewrite it with keyless payloads so it actually goes red first — your TDD gate depends on it.

---

## System map — entities, states, data flow

**Entities (the spine that matters here):** Work, the unit of identity. A Work carries *anchors* (provider keys: `isbn_13`, `gr_key`, `hc_key`, `ol_key`, `asin`) and an *identity status* (Pending / Confirmed / Provisional / NeedsReview).

**The canonical flow** (all ✓-verified at the cited entry points):

```
   ADD / REFRESH / MONITOR / READARR / LIST / MANUAL-IMPORT  (the doors)
                              │
                              ▼
  (1) IDENTITY     settle_identity → resolver.resolve
                   english_identity_resolver.rs:60
                   fan out to providers (join_all, per-call timeout) — NO RATE LIMIT  :94-113 ✓
                   run_quorum arbitrates (sorted, deterministic)      :298 / :307 ✓
                   → writes anchors + status; caches payloads (5-min TransportCache)
                              │
                              ▼
  (2) ENRICH       run_unified_enrichment → enrich_work
                   work_service.rs:3086 / enrichment/lib.rs:1328
                   cache hit?  merge_from_cached (zero network)
                   cache miss? dispatch_enrichment scatter:
                     JoinSet + per-provider Semaphore + TokenBucket  provider_queue.rs:537-539 ✓
                              │
                              ▼
  (3) MERGE        DefaultMergeEngine::merge      enrichment/lib.rs:424
                   drop_language_incompatible_providers (ONE chokepoint) :476 ✓
                   merge_impl: priority-list walk, null-guard  :789-806
                              │
                              ▼
  (4) MATERIALIZE  download_cover_to_disk (decode once, atomic write) materialize/lib.rs:22
                   ebook dims persisted; audiobook dims dropped (gap)  work_service.rs:3249-3251
                              │
                              ▼
  (5) PERSIST      apply_enrichment_merge (CAS retry x3); no anchor columns written here
```

**Three independent "call a website" surfaces — the heart of the problem:**

| Surface | Where | Rate limiter | Mechanism |
|---|---|---|---|
| Discovery (UI search) | `work_service.rs` `lookup_*` (e.g. :2268) | per-bucket, 1s spacing | `RateLimiterMap` in `livrarr-http/fetcher.rs:40-81` ✓ |
| Enrichment scatter | `provider_queue.rs:537` ✓ | per-provider token bucket (1/s, concurrency 2) | `TokenBucket` (GCRA) in `livrarr-enrichment` |
| **Identity fan-out** | `english_identity_resolver.rs:94-113` ✓ | **none** | bare `join_all` + timeout |

These three do **not** share state. Goodreads can be hit by all three at once.

---

## Pipeline lifecycle — coverage map & open stages

A piece of metadata moves through the stages below. Pass 1 (2026-06-28) covered **L1 access** and **L2 identity/matching** hard, and **L5 materialize** partially. **Pass 2 (2026-06-28) audited L3, L4, L6, L7, L8** — the post-access correctness tail — producing M-010…M-021 (full bodies in "Post-access correctness findings — Pass 2"). L5 tag-writing remains the only partial stage. New findings continue the `M-0NN` sequence.

Legend: ✅ audited · ◐ partial · ⬜ not yet audited

### L0 — Doors / entry points
*What:* add · refresh · monitor · readarr · list · manual-import — how a work enters the pipeline.
*Status:* ✅ (wiring only). All doors funnel through one creation gate (`ensure_identity_and_enrichment`); no bypass (prior-audit door-wiring worry closed — see Reconciliation).
*Findings:* none open.

### L1 — Access / transport (calling the provider sites)
*What:* HTTP/GraphQL to Goodreads / Hardcover / OpenLibrary / Google Books / Audnexus / Audible; rate limiting; (proposed) gateways.
*Status:* ✅ audited.
*Findings:* **M-001** (fragmented rate limiting, 6+ unthrottled surfaces), **M-003** (Hardcover query copied ×4), **M-009** (per-instance limiter), **M-006** (dead rate-limiter code). Full surface inventory in the corrections section.

### L2 — Identity (which work is this?)
*What:* `settle_identity` → `resolve` → provider fan-out → `run_quorum` → write anchors.
*Status:* ✅ audited.
*Findings:* **M-001** (identity fan-out unthrottled), **M-002** (matching fork — the colon bug). Quorum determinism confirmed sound.

### L3 — Enrich (fetch full per-provider payloads)
*What:* `run_unified_enrichment` → `enrich_work` → cache hit/miss → dispatch scatter (JoinSet + semaphore + token bucket).
*Status:* ✅ audited (Pass 2).
*Examined:* cache correctness (5-min `TransportCache` confirmed sound; `metadata_cache`/migration 056 confirmed DEAD — M-011); `candidate_id` zero-network reuse (anchor-validated, sound — but cover gate skipped, M-012); scatter dispatch (panic isolation + null-guard sound); provider abstention (NotFound correctly skipped).
*Findings:* M-001 (access), **M-010** (Readarr drop, High), **M-011** (dead 24h cache), **M-012** (cover gate skipped on reuse).

### L4 — Merge (combine payloads into one record)
*What:* `DefaultMergeEngine::merge` → `drop_language_incompatible_providers` chokepoint → priority-walk + null-guard.
*Status:* ✅ audited (Pass 2). Field-level priority rules now covered; the Pass-1 structural soundness (determinism, ordering, null-guard, single language chokepoint) holds.
*Examined:* per-field priority is language-aware and correct; per-field fallthrough is correct; user-locks and anchor-field exclusion (REQ-007) sound. The one gap is empty-Vec fields (M-013). LLM-arbitration comment re-confirmed stale (M-006 class).
*Findings:* **M-013** (HC empty-genres array blocks lower providers, Med).

### L5 — Materialize (covers, files, tags on disk)
*What:* `download_cover_to_disk` (decode once, atomic write); cover dimensions; tag writing.
*Status:* ◐ partial — covers audited; tag-writing and non-cover materialize not.
*Examine next:* tag-write path (EPUB vs m4b/mp3); phase-1 → phase-2 cover upgrade correctness; cover trust/quality gates beyond M-008.
*Findings:* **M-007** (audiobook dims dropped), **M-008** (cover gate 0.6 vs identity 0.75). _Further findings: (empty — fill in next pass)_

### L6 — Persist (write to the database)
*What:* `apply_enrichment_merge` (CAS retry ×3); anchor columns; identity-status write.
*Status:* ✅ audited (Pass 2).
*Examined:* main merge write is fully atomic (single tx) and idempotent (COALESCE guards) — sound. CAS is app-level only (M-014). Anchor columns confirmed NOT written here (REQ-007 ✓); anchors move via the identity track. `supersede_anchor` drops 3 of 5 column syncs but is dead (M-015).
*Findings:* **M-014** (CAS no DB-level generation predicate, Med/low-risk), **M-015** (supersede_anchor partial sync, latent), **M-016** (raise_identity_conflict not transactional, Med).

### L7 — Convergence (cross-cutting: re-resolve incomplete works over time)
*What:* `converge_work` / `retry_all_incomplete` / background convergence job.
*Status:* ✅ audited (Pass 2). The 4 cases were traced in source: Pending±chaseable and Confirmed-not-chaseable are correct; M9 silent-limbo is resolved (anchorless/exhausted Pending → NeedsReview on first tick, sound). The Confirmed+chaseable case is the bug (M-017).
*Findings:* **M-017** (`Completed` clears clock but Branch 3 re-selects — bounded extra sweeps; + no-backoff-on-error), **M-018** (sweep amplifies M-001's unthrottled fan-out at batch scale, Med).

### L8 — State machine (cross-cutting: identity-status transitions)
*What:* Pending → Confirmed → Provisional → NeedsReview, and what drives each move.
*Status:* ✅ audited (Pass 2). `derived_identity_status` and the monotonic-raise rules are sound; the transition map has no illegal downgrades. The defects are in the **exits** from terminal states, not the entries.
*Findings:* **M-019** (Conflict is an irresolvable terminal — resolve/dismiss never clear the badge, High), **M-020** (affirm is silently ineffective; identity lands in NeedsReview limbo, High/conditional), **M-021** (`identity_not_found` dropped by refresh + converge — **latent**, producer dormant), plus minor `NotFound`-gate and `user_id`-guard hardening items.

---

## Findings

Severity = blast radius × likelihood. Confidence = how sure I am the claim is true.

> ⚠ **The "Cross-family review — corrections & additions" section above overrides the counts/scope below for M-001, M-003, M-004, and M-006, and adds M-009. Read both together before planning.**

---

### M-001 — The identity lookup has no rate limit; rate limiting is fragmented across 5 places
**Severity: High. Confidence: 95% (code) / 60% (that this is *the* live 429 cause).**

**Claim.** Outbound provider calls happen on three surfaces. Only two are throttled, by two *separate* limiters; the identity fan-out is throttled by nothing.
- Identity fan-out: `english_identity_resolver.rs:94-113` ✓ — `join_all` over `client.fetch()`, wrapped only in a per-call `timeout`. No bucket, no semaphore.
- Enrichment: `provider_queue.rs:537-539` ✓ — acquires a `Semaphore` permit, then a `TokenBucket` token, before each fetch.
- Discovery: `fetcher.rs:40-81` ✓ — `RateLimiterMap`, 1-second min interval per provider.

Plus **two dead** rate-limiter mechanisms that throttle nothing:
- `goodreads_rate_limiter: Arc<GoodreadsRateLimiter>` — built at `main.rs:612`, **zero readers** (`find_referencing_symbols` returned only the constructor) ✓. `GoodreadsClient` has no rate-limiter field. So this field is pure decoration.
- The entire `livrarr-http/src/rate_limit.rs` (`DefaultRateLimiter`, `RateLimitContract`, `ProviderKind`) — zero external callers (subagent-sourced, high confidence).

**Root cause (distinct from symptom).** Symptom = 429s / Goodreads bans under load. Root cause = rate limiting was added per-path as each path was built, never unified to a per-provider single source of truth. Two live limiters give Goodreads two independent 1/s budgets; the identity path adds an uncapped third stream. The wiki already records "we're 5–7× over the polite Goodreads floor" (`wiki/integrations/goodreads.md`) and a known "per-instance GR throttle, no global cap" — consistent with this.

**Correction to the starting signal.** The brief said provider clients "bypass the throttle" via raw `HttpClient`, causing bulk-import 429s. That's incomplete: in *enrichment*, those same raw calls **are** throttled upstream by the queue's token bucket (`provider_queue.rs:537` ✓). The genuinely unthrottled burst is the **identity fan-out**, not the enrichment transport. Worth correcting because it changes where the fix goes.

**Blast radius.** Every add/refresh/monitor/convergence run. Bulk refresh (serial across works) multiplies it.

**Caveat (calibration).** I read the code; I did **not** reproduce a live 429 this session. The mechanism is certain; "this is the dominant cause of the bans you see" is 60% — there could be additional anti-bot factors (DataDome) independent of rate.

---

### M-002 — "Same book?" is forked: the identity engine's matcher diverges from the canonical one
**Severity: High. Confidence: 95% (divergence exists) / 65% (real-world mis-merge rate).**

**Claim.** Two title-matchers, same 0.75 threshold, different answers:

| | Canonical — `text_norm::title_tokens` | Identity — `normalize_match_title` + `token_set` |
|---|---|---|
| Location ✓ | `livrarr-domain/text_norm.rs:48-72` | `livrarr-identity/english_identity_resolver.rs:736-774` |
| Stop-words ("the/of/and") | **removed** (`:7,69`) | **kept** (no filter) |
| Accents (café/cafe) | **stripped** (NFKD, `:63`) | **kept** (lowercase only) |
| Punctuation ("Philosopher's") | split on non-alphanumeric (`:67`) | split on whitespace only — apostrophe stays inside token |
| Colon / subtitle | regex series-marker strip | **breaks at first `:`** (`:744`) — "Dresden Files: Summer Knight" and "Dresden Files: Dead Beat" collapse to the same string |

The colon-truncation case is the known **C1** bug; the identity crate even ships a test documenting it (`english_identity_resolver.rs:~918`, subagent-sourced). All four divergences are confirmed by reading both function bodies ✓.

**Also forked (subagent-sourced, high confidence):** Google Books (`google_books.rs:426`, `MIN_TITLE_JACCARD=0.75`) and Audible (`audible.rs:273`) both use the *canonical* `text_norm` — good. The cover gate uses **0.6** (`cover_gate.rs:2`), and the matching crate has two more private `normalize` copies (`work_dedup.rs:2,204`; `m4_scoring.rs:171`). So there are **5 distinct title-normalizers** in the tree.

**Root cause.** The identity resolver grew a private matcher instead of depending on `livrarr-domain::text_norm`. Symptom = wrong-book merges and missed merges (foreign titles with accents, series with colons). Root cause = no single matching authority; the most identity-critical caller uses the weakest normalizer.

**Blast radius.** Identity clustering / quorum decisions — i.e. which provider payloads are treated as the same work. Directly affects the F1-class "wrong book adopted" damage the project has fought before.

**Confidence note.** That the code diverges: near-certain. That it causes a meaningful number of *real* mis-merges: 65% — depends on the catalog. The colon case is the most likely to bite (series-heavy libraries).

---

### M-003 — Hardcover's GraphQL call is copy-pasted 4–5 times and has drifted
**Severity: Med–High. Confidence: 90%.**

**Claim.** The same Hardcover `SearchBooks` query is independently built and POSTed in (at least) four places, with divergence:

| Location | per_page | Title quoting | Rate-limited? |
|---|---|---|---|
| `hardcover.rs:73-102` ✓ | 25 | quoted (exact) | no (raw; throttled by queue when reached via enrichment) |
| `cover.rs:319-339` ✓ | 10 | quoted (exact) | no |
| `work_service.rs lookup_hardcover :2568` | 15 | **unquoted** (partial matches) | **yes** (discovery limiter) |
| `hardcover.rs:470 query_hardcover_by_isbn` | 10 | ISBN term | no |

I personally diffed the first two (`hardcover.rs:73-102` vs `cover.rs:319-339`) ✓ — near-identical: same query string, same parenthetical-strip, same quoted term, same `Bearer {token}` header, same endpoint constant. The other two are subagent-sourced.

**Root cause.** No single Hardcover "gateway." Symptom = the cover search and the discovery search return *different* hits for the same title (one quotes, one doesn't; different page sizes). Root cause = four owners of one transport concern.

**Blast radius.** Hardcover results inconsistency + 4× the maintenance surface. A token-format or endpoint change must be made in four places.

---

### M-004 — Provider transport/parsing is split between the provider crate and the service god-object
**Severity: Med. Confidence: 85%.**

**Claim.** `work_service.rs` carries `lookup_goodreads/openlibrary/google_books/hardcover` (`:2268–2699`), the rate-limited "discovery" path. These re-implement query-building and parsing that *also* lives in `livrarr-external-data`. Worst case is OpenLibrary: `lookup_openlibrary` (`:2369-2479`) hand-parses the search JSON inline — a **second** OL reader that exists only in the service, separate from the enrichment client. (Subagent-sourced; method bodies read by the inventory agent, not diffed against the external-data client by me — hence 85%.)

**Root cause.** The discovery/search path was attached to the service rather than to the provider crate where the enrichment path lives. This is *why* the god-object (M-005) is so large and why M-003's Hardcover query has a fourth copy.

---

### M-005 — `work_service.rs` is a 3,684-line god object (grew since last audit)
**Severity: Med (maintainability). Confidence: 99%.**

**Claim.** `crates/livrarr-metadata/src/work_service.rs` = 3,684 lines (subagent-measured; prior audit recorded 3,383 — it grew ~300 lines). One struct, 12 fields, mixing: identity (`add`, `resolve_identity`, `converge_work`), four inline provider lookups, enrichment orchestration (`run_unified_enrichment`), CRUD, cover upload/download, and refresh/bulk-refresh. This is the prior audit's **R8**, still open and larger.

**Root cause.** Accretion — every new metadata concern landed here. Pure cleanup; no behavior change. Splitting it is the lever that makes M-002/M-003/M-004 fixable without fear.

---

### M-006 — A large pile of dead code that misleads readers
**Severity: Med (clarity, not runtime). Confidence: 90% (subagent `find_referencing_symbols`; I spot-verified `goodreads_rate_limiter` ✓).**

Zero-caller / inert items found:
- `goodreads_rate_limiter` field — dead (verified ✓).
- `livrarr-http/rate_limit.rs` — entire module dead.
- `RateBucket::Audnexus`, `RateBucket::Audible` — defined, never constructed (those clients use the raw path).
- `bulk_resolver::resolve_bulk` — zero callers.
- `EnrichmentServiceImpl.validator` and `.llm` fields — stored, never invoked (REQ-005 removed LLM from enrichment).
- `llm_ewl::ask_same_book` (whole module) — zero callers.
- `ResolverConfig::confirm_title_jaccard` — set in `default()`, never read; the resolver uses the hard-coded const instead.
- `trigger_monitor` — empty stub, zero callers (`author_monitor_workflow.rs:253-255`); the trait still mandates it.

**Root cause.** Two big refactors (REQ-005 LLM removal; the identity restructure) left orphans behind. Risk is low at runtime but real for comprehension — e.g. the dead `goodreads_rate_limiter` actively misleads a reader into thinking Goodreads is throttled there.

---

### M-007 — Audiobook cover dimensions are computed and then dropped
**Severity: Low–Med. Confidence: 90%.**

**Claim.** Materialize decodes width/height for both covers, but `update_cover_dimensions` is called only for the **ebook** slot (`work_service.rs:3253/3280`); the code comment at `:3249-3251` admits "the audiobook slot has no dims writer today." So `audiobook_cover_width/height` are always 0. This is the residual of the prior audit's **F7** (the ebook half was fixed; the audiobook half wasn't).

---

### M-008 — Cover gate (0.6) is looser than identity (0.75): a cover can attach to a book identity won't confirm
**Severity: Low–Med. Confidence: 80% (mechanism) / unknown (intended?).**

**Claim.** A Goodreads cover passes the gate at Jaccard ≥ 0.6 (`cover_gate.rs:2`), but the same title pair needs ≥ 0.75 to be accepted by the identity quorum. So a work can wear a GR cover from an entry that identity does **not** consider the same book. May be a deliberate "covers are low-stakes" call — flagged as an open question, not asserted as a bug.

---

## Post-access correctness findings — Pass 2 (L3–L8), 2026-06-28

**Method.** Five parallel Serena/LSP-driven inventory agents (one per stage: L3 enrich, L4 merge field-rules, L6 persist, L7 convergence, L8 state machine), then the orchestrator personally re-opened every load-bearing `path:line` below before writing (✓ = orchestrator-opened this pass; "subagent-sourced" = relied on the inventory agent's symbol read). Same discipline as Pass 1. New findings continue the `M-0NN` sequence.

**Three new High-severity findings** — all in the post-access tail Pass 1 didn't reach:
- **M-010** — Readarr import data is injected, then silently discarded before the merge.
- **M-019** — an identity `Conflict` can never be cleared; resolve/dismiss don't touch the badge.
- **M-020** — affirming a pending anchor is silently ineffective (badge never reaches Confirmed; lands in `NeedsReview` limbo).

**Cross-family review (2026-06-28).** Both **Codex** and **Gemini** independently re-opened the cited code. **M-010–M-018 CONFIRMED by both, as written.** Three corrections (applied to the bodies below):
- **M-019** — core stands (a `Conflict` badge can never be cleared), but **not "un-enrichable forever":** a manual refresh still enriches via `run_unified_enrichment` (`work_service.rs:3147` — no identity-status gate ✓); it just never clears the badge. Stays High.
- **M-020** — core stands (affirm is silently ineffective), but **not "blocks all enrichment":** the affirm-spawned refresh does enrich the work; the identity badge is the casualty (Pending → terminalized to `NeedsReview` on the next convergence tick). High but conditional. *(Codex: overstated; Gemini: confirmed as-written — corrected toward Codex.)*
- **M-021** — **downgraded to Low / latent.** `identity_not_found` is hard-set `false` at every construction site (`enrichment/lib.rs:1439,1515,1724`) — a documented system truth (spec ST-004; REQ-014 removed the merge's identity signal). The producer is dormant, so the dropped-signal asymmetry has zero impact until the LLM identity-validator is re-wired. (Verified by full enumeration + ST-004.)

**Codex addition:** `set_identity_pending` (`sqlite_work_identity.rs:405`) is a 4th bare-`work_id` status writer — folded into the `user_id`-guard item. **Gemini-proposed M-022 (NOT confirmed):** a pending-anchor "shadowing"/status-downgrade gap — Gemini's citation (`sqlite_work_identity.rs:427-437`) points at the status setters, not `record_pending_anchor`, and the mechanism is unclear. Left as an **unverified open item**, not a finding.

---

### M-010 — Readarr import data is injected, then dropped before the merge
**Severity: High (Readarr-import door only). Confidence: 95%.**

**Claim.** `enrich_work` Step 4.5 adds the pre-injected Readarr payload to the scatter outcomes as `Success` (`enrichment/lib.rs:1475-1478` ✓), but Step 8 reads every `Success` payload back from the DB via `get_retry_state` instead of using the in-memory box — the match arm binds `Success(_)` and discards it (`enrichment/lib.rs:1529-1546` ✓). Readarr is never scattered (`provider_queue.rs:94 → None` ✓), and only scattered providers persist a `normalized_payload_json` row (`provider_queue.rs:745-758` ✓), so Readarr has no DB row → `payload = None`. Readarr contributes **zero fields**, despite the Step 4.5 comment claiming "merge engine arbitrates field selection."

**Root cause.** Step 8's "re-read from DB for restart-safety" pattern was applied uniformly to all `Success` outcomes; Readarr was bolted in at Step 4.5 as an in-memory bypass that Step 8 doesn't honor.

**Blast radius.** Every Readarr import: the user's existing Readarr metadata is silently ignored; fields come only from OL/GR/HC/GB. Status still resolves via `provider_outcomes`, so the drop is invisible. **Possibly a regression** (the bypass may have worked before the restart-safety refactor) — not git-confirmed.

---

### M-011 — The 24-hour persistent enrichment cache (`metadata_cache`, migration 056) is dead/unwired
**Severity: Low (missing optimization + dead code). Confidence: 95% (subagent `find_referencing_symbols`, corroborated by wiki insight 55 and `spec-sprint-e-refresh-gate.md`).**

**Claim.** The `metadata_cache` table (migration 056), the `MetadataCacheDb` trait (`db/lib.rs:1630`), and its `SqliteDb` impl (`sqlite_metadata_cache.rs`) all exist, but nothing in the enrichment pipeline holds a `MetadataCacheDb` bound or calls `metadata_cache_get/put` — the only references are the impl itself and a direct unit test. The live cache is the **5-minute in-memory `TransportCache`** (candidate-reuse only, `transport_cache.rs` ✓); REQ-009's 24h persistent cache is unimplemented.

**Resolves Pass-1 open question #3** ("`metadata_cache` — dead or wired?"): **dead**, confirmed. No runtime harm; it misleads readers and the restart re-enrichment optimization isn't realized. Same dead/misleading class as M-006.

---

### M-012 — The Goodreads cover-quality gate is skipped on the candidate-reuse (cache) path
**Severity: Low–Med. Confidence: 80% (early-return ✓; gate location subagent-sourced).**

**Claim.** On a `candidate_id` cache hit, `enrich_work` early-returns from the reuse block (`enrichment/lib.rs:1455-1457` ✓) before the GR cover-Jaccard gate (REQ-017) runs (`~:1617-1666`, subagent-sourced). The reuse path calls `merge_from_cached`, which applies the language drop but not the cover gate — so a cached GR cover from a different edition can attach to an English work. **Self-corrects** on the next network enrichment (gated path).

**Blast radius.** First enrichment of a UI-search add (candidate path), English work with an OL key. Cosmetic, transient.

---

### M-013 — A Hardcover empty-genres array blocks every lower-priority provider's genres
**Severity: Med. Confidence: 90% (code path ✓); trigger frequency unknown.**

**Claim.** HC normalization yields `genres = Some(vec![])` when the API returns `"genres": []` — or when every entry is filtered out by `.filter(|s| !s.contains('|'))` (`hardcover.rs:218-224` ✓), copied unguarded into `NormalizedWorkDetail` (`provider_client.rs:619` ✓). The merge's `FieldValue::Strings(Some(vec![])).is_some()` returns `true` (`enrichment/lib.rs:550` ✓), and the priority walk breaks on the first `is_some()` winner (`enrichment/lib.rs:857-862` ✓) — so HC "wins" genres with an empty list and GR/OL never contribute. GR correctly guards this (`provider_client.rs:1375-1378 → None` ✓); HC does not. Google Books has the same latent pattern (`google_books.rs:499`, subagent-sourced).

**Root cause.** `extract_provider_field` applies `non_blank()` to string fields but there is no `non_empty_vec()` guard for `Strings` fields; the guard belongs at the provider normalizer (as GR has it) or at extract-time.

**Blast radius.** Any work where HC returns empty/pipe-only genres loses all genre data even when GR/OL have real lists; doesn't self-heal while HC stays eligible.

---

### M-014 — The enrichment-merge CAS is application-level only; the UPDATE has no generation predicate
**Severity: Med (defense-in-depth); low practical risk today. Confidence: 92%.**

**Claim.** `apply_enrichment_merge` reads `merge_generation` and compares it in Rust inside the tx (`sqlite_work.rs:868-878` ✓), but the committing UPDATE keys on `WHERE id = ? AND user_id = ?` with no `AND merge_generation = ?` (`sqlite_work.rs:924` ✓). The compare-and-swap is enforced by read-then-check, not by the write — the guarantee rests on transaction isolation plus the application layer, not a DB-level conditional write.

**Mitigation (why low risk now).** The per-work Tokio mutex in `enrich_work` and the process PID lock serialize writers in production, so the race is currently unreachable. It becomes real if any future caller invokes `apply_enrichment_merge` outside that mutex. The CAS retry/re-read loop itself is sound (Pass-1 ✓).

---

### M-015 — `supersede_anchor` syncs only OL/GR back to `works.*`; HC/ISBN/ASIN dropped — latent (dead code)
**Severity: Low / latent. Confidence: 99%.**

**Claim.** `supersede_anchor`'s denormalized-column sync handles `OL_WORK` and `GR_WORK`, then `_ => {}` (`sqlite_work_identity.rs:178-195` ✓) — unlike `confirm_anchor`, which syncs all five (`:69-111` ✓). A superseded HC/ISBN/ASIN anchor would leave `works.hc_key/isbn_13/asin` stale. **But `supersede_anchor` has zero callers** — `find_referencing_symbols` on the trait method (`livrarr-domain/services/work_identity.rs`) returns only the impl ✓. Dead today; a landmine for when GR-redirect/supersession wiring is activated.

---

### M-016 — `raise_identity_conflict` is not transactional: partial-write window + TOCTOU duplicate
**Severity: Med. Confidence: 95%.**

**Claim.** The dedup-SELECT (`sqlite_work_identity.rs:332-341` ✓), the conflict-row INSERT (`create_identity_conflict`, `:347-357` ✓), and the badge UPDATE to `'conflict'` (`:361-366` ✓) are three separate `self.pool()` calls with no enclosing transaction. Two failure modes: (1) a crash between INSERT and the badge UPDATE leaves an open conflict row while `works.identity_status` is still non-conflict → the identity gate doesn't block, and enrichment proceeds on a disputed work; (2) two concurrent callers (add + converge on the same work) both see "no existing conflict" and both INSERT, violating the REQ-020 "one open conflict per (work, kind)" invariant.

**Root cause.** Missing `BEGIN/COMMIT` around the check→insert→badge sequence (contrast `set_identity_pending`, which is transactional ✓).

---

### M-017 — `converge_work` reports `Completed` while the work is still re-selectable; extra unthrottled sweeps
**Severity: Med–Low (bounded). Confidence: 95%.**

**Claim.** `converge_work` returns `Completed` for any Confirmed/Provisional + Enriched/Thin work (`work_service.rs:2185-2192` ✓) regardless of whether chaseable missing anchors remain; the job then clears `next_convergence_at` (`convergence.rs:87` ✓). But `list_convergence_due` Branch 3 re-selects on any chaseable anchor with the time-gate `next_convergence_at IS NULL` passing (`sqlite_work.rs:1115-1133` ✓) — so the work is re-picked on the **next tick** and re-runs `settle_identity` (the unthrottled fan-out, M-001). Step 3's dead-end bump (`:2172-2177` ✓) caps this at ~`threshold` (default 3) sweeps before the anchor stops being chaseable. The `convergence.rs:84` comment ("Completed works stop being selected") is therefore wrong.

**Blast radius.** Up to `batch_size × threshold` extra identity fan-outs per cadence for Confirmed works with a sub-threshold missing anchor; fires immediately on restart (NULL time-gate). Bounded, not infinite. *(Minor, same file: on a `converge_work` `Err`, the job `continue`s and skips `set_next_convergence_at` (`convergence.rs:75-81` vs `:86-94` ✓) — no backoff, so a persistently-failing work re-selects every tick.)*

---

### M-018 — A convergence sweep amplifies M-001: batch × 6 unthrottled identity calls, no pacing
**Severity: Med. Confidence: 90%.**

**Claim.** Each `converge_work` with a chaseable anchor fires `settle_identity` → `resolver.resolve` → all eligible providers via `join_all` with no rate limit (M-001; `english_identity_resolver.rs:90-103`, subagent-sourced), and the job loops over up to `batch_size` (default 25) works with **no inter-work sleep** (`convergence.rs:64-82` ✓). One sweep can fire ~25 × 6 = 150 provider calls in rapid succession, Goodreads (WAF-sensitive) hit hardest. This is the convergence-scale face of M-001 and compounds with M-017's extra sweeps.

**Fix dependency.** Resolved by the M-001 process-global limiter (Phase C); until then convergence is the most likely live 429 trigger.

---

### M-019 — An identity `Conflict` is an irresolvable terminal: resolve/dismiss never clear the badge
**Severity: High. Confidence: 98%.**

**Claim.** `raise_identity_conflict` sets `works.identity_status = 'conflict'` (`sqlite_work_identity.rs:361` ✓). But `resolve_identity_conflict` and `dismiss_identity_conflict` update only the `work_identity_conflicts` row, never `works.identity_status` (`sqlite_identity_conflict.rs:143-175` ✓); the service layer calls only those (`identity_conflict_service.rs:92-95` ✓). Meanwhile `settle_identity`'s terminal guard refuses to re-resolve a `Conflict` work (`async_resolver.rs:118-128` ✓), and `reset_for_manual_refresh` deliberately leaves `conflict` untouched (`sqlite_work.rs:1057, 1064-1068` ✓). **Net: once a work is `Conflict`, nothing clears it** — not user resolve, not user dismiss, not refresh, not convergence (the selection query excludes `conflict`). Only delete + re-add escapes.

**Root cause.** The conflict-resolution path was wired against the conflict-row table only; the row-state→badge seam was never connected for the exit direction.

**Blast radius.** Every work that ever hits an anchor conflict is permanently stuck in the `Conflict` badge and excluded from background convergence, even after the user resolves/dismisses — the user's action is a no-op. *(Corrected per cross-family review: a **manual refresh** can still update its metadata via `run_unified_enrichment`, which has no identity-status gate (`work_service.rs:3147` ✓) — so it is not "un-enrichable"; the badge simply never clears.)* The most severe Pass-2 finding.

---

### M-020 — Affirming the last chaseable anchor leaves the work stuck `Pending`; `NeedsReview` is unconditionally stuck
**Severity: High (conditional). Confidence: 90%.**

**Claim.** `affirm_pending_anchor` confirms the anchor (syncing `works.*`) and fires a background `refresh`; it never writes the badge directly (`work.rs:944-957` ✓), and `confirm_anchor` writes anchor columns but not `identity_status` (`sqlite_work_identity.rs:55-117` ✓). `refresh` only re-settles identity when `chaseable_anchor_types` is non-empty (`work_service.rs:1402` ✓), and that function excludes any anchor whose `works.*` column is now set (`:240-266` ✓). So if the just-affirmed anchor was the **last** chaseable one, `settle_identity` never runs and the badge stays `Pending`. For a `NeedsReview` work it's worse: `settle_identity`'s terminal guard blocks re-resolution unconditionally (`async_resolver.rs:118-128` ✓), and nothing clears `NeedsReview`.

**Blast radius.** A user affirms "yes, this is the right book," but the badge never reaches `Confirmed`. *(Corrected per cross-family review: the affirm-spawned refresh **does** enrich the work — `run_unified_enrichment` has no identity gate — so metadata is not blocked.)* The casualty is identity: the work stays `Pending`, and the next convergence tick terminalizes a Pending+unchaseable work to `NeedsReview` (`work_service.rs:2106-2110` ✓), for which no user resolution path was found (M-019-class limbo); the `NeedsReview` sub-case is stuck immediately via the terminal guard. Likelihood is real because affirm targets works with one or two fuzzy guesses — exactly the last-chaseable case.

---

### M-021 — The `identity_not_found` signal is dropped by `refresh` and `converge_work` — latent (producer dormant)
**Severity: Low / latent (downgraded in cross-family review). Confidence: 97% (asymmetry is real) / the signal never fires today.**

**Claim.** The handling is asymmetric: the add door reads `run_unified_enrichment`'s `identity_not_found` bool and writes `NotFound` (`work_service.rs:3026-3031` ✓), but `refresh` binds the whole tuple to `_enrichment_status` (`:1433` ✓) and `converge_work` Step 2 binds it to `_` (`:2148` ✓) — both drop it.

**Why latent.** `identity_not_found` is **hard-set `false` at every construction site** (`enrichment/lib.rs:1439, 1515, 1724`; all `work_service.rs` sites) — a documented system truth (`spec-unified-identity-path.md` ST-004; REQ-014 removed the merge's identity signal, and the old LLM-validator producer was deleted, `design-metadata-refactor-road.md:26`). A behavioral test asserts `!result.identity_not_found`. So the bool is always `false` today: the add door's `NotFound`-write never fires, and dropping it in refresh/converge has **zero impact**.

**Blast radius (when re-armed).** If the LLM identity-validator is ever re-wired to produce `identity_not_found = true`, this asymmetry resurfaces: a refreshed unidentifiable work would surface with a confident-looking badge instead of `NotFound`/"Unverified". Fix the propagation at the same time the producer is restored.

---

### Pass-2 minor / hardening items (low severity, mostly latent)
- **Stale `priority_model` constructor arg** — `DefaultMergeEngine::new`/`new_with_llm` accept `_priority_model` and discard it; the language-aware model is built per-merge (`enrichment/lib.rs:404-420, 1671` ✓). Misleading API, no runtime effect. Same dead/misleading class as M-006; the stale `main.rs:351-353` LLM-arbitration comment is re-confirmed here.
- **`NotFound` missing from two enrichment gates** — `ensure_identity_and_enrichment` (`:2865-2869`) and `converge_work` (`:2139-2142`) block on `Pending|Conflict|NeedsReview` but not `NotFound` (subagent-sourced); benign for convergence (the selection query never fetches `not_found`), but a re-added NotFound work that dedup-matches could waste an enrichment pass.
- **Four identity-status writers lack a `user_id` guard** — `set_identity_confirmed/provisional` and `set_needs_review` (`sqlite_work_identity.rs:417-438` ✓), plus `set_identity_pending`'s status UPDATE (`:405` ✓, Codex), use `WHERE id = ?1` only, unlike the other setters. Defense-in-depth only (`work_id` is globally unique); needs a trait-signature change to fix.
- **Unchecked semaphore permit** — `provider_queue.rs:537` binds `acquire_owned().await` as a `Result` without unwrapping (subagent-sourced); harmless unless a semaphore is ever closed (never today).

---

## Duplication matrix

| Concern | Implementations (`path:line`) | Canonical | Verdict |
|---|---|---|---|
| **HTTP transport — Hardcover** | `hardcover.rs:73` ✓, `hardcover.rs:470`, `cover.rs:319` ✓, `work_service.rs:2568` | a single `HardcoverGateway` (none exists yet) | collapse 4→1; delete copies |
| **HTTP transport — OL/GB/GR** | provider client *and* `work_service.rs lookup_*` (`:2268–2699`) | the `livrarr-external-data` client | move discovery into the client; delete the `lookup_*` parsers |
| **Rate limiting** | `RateLimiterMap` (fetcher.rs:40 ✓), `TokenBucket` (provider_queue.rs:537 ✓), identity = none ✓, `rate_limit.rs` (dead), `goodreads_rate_limiter` (dead ✓) | **one** per-provider limiter shared by all 3 surfaces | unify to 1 live; delete 2 dead; throttle identity |
| **Title normalize** | `text_norm.rs:48` ✓ (canonical), `english_identity_resolver.rs:736` ✓, `work_dedup.rs:2`, `work_dedup.rs:204`, `m4_scoring.rs:171` | `text_norm::title_tokens` | route identity + dedup through it; keep m4 only if release-matching genuinely differs |
| **"Same book?" scorer** | identity `title_matches` (`:765` ✓), GB `score_candidates` (`:426`), Audible `score_provider_candidates` (`:273`), HC read-count+LLM (`hardcover.rs:61`), cover gate 0.6 (`:2`) | one shared scorer over the canonical normalizer | unify threshold + normalizer; HC/LLM stays as a tie-breaker layer, not a separate normalizer |

---

## Recommended target architecture (concrete, this codebase)

1. **One gateway struct per provider** in `livrarr-external-data`, wrapping a private fetcher. All Hardcover GraphQL goes through `HardcoverGateway`; `cover.rs` and `work_service.rs` call it instead of pasting the query. (This is the already-designed "gateway per provider" workstream — it correctly covers M-003/M-004, but **not** M-002.)
2. **One per-provider rate limiter**, owned where the gateways live, shared by *all three* surfaces (discovery, enrichment, identity). The identity fan-out must acquire a token before each `client.fetch()`. Delete `rate_limit.rs` and `goodreads_rate_limiter`. This is the single source of truth principle applied to throttling.
3. **One matching module** in `livrarr-domain`: `text_norm::title_tokens` is the only normalizer; one `same_work(a, b) -> bool` at one threshold. Identity, work-dedup, and the cover gate all call it. Provider-specific cleverness (Hardcover read-count, LLM disambiguation) sits *on top* as a ranking/tie-break layer, never as a second normalizer.
4. **Shrink `work_service.rs`** by moving the four `lookup_*` into their gateways and `converge_work`/`retry_all_incomplete` into a `convergence`/jobs module — mirroring the existing `author_monitor_workflow` pattern.
5. **Keep what's already right:** the merge chokepoint, the deterministic quorum, the single creation gate. Don't touch them except to feed them the unified matcher.

---

## Remediation roadmap (sequenced; cleanup vs behavior-change separated)

**Phase A — pure cleanup, no behavior change (low risk, do first):**
- A1. Delete dead code from M-006 (`rate_limit.rs`, `goodreads_rate_limiter`, dead enrichment LLM fields, `bulk_resolver`, `llm_ewl`, `confirm_title_jaccard`, `trigger_monitor` or wire it). *Effort: S. Risk: low.*
- A2. Wire the audiobook cover-dims writer (M-007). *Effort: S. Risk: low. Behavior: fills a zero field.*
- A3. Split `work_service.rs` along the seams above (M-005). *Effort: L. Risk: low if mechanical. Behavior: none.*

**Phase B — transport consolidation (the designed gateway work):**
- B1. `HardcoverGateway`, collapse the 4 copies (M-003). *Effort: M. Risk: med — pick the canonical `per_page`/quoting; this **changes which hits return** for cover vs discovery, so it is a behavior change masquerading as cleanup. Needs a before/after diff on real titles.*
- B2. Move `lookup_*` into the provider gateways (M-004). *Effort: M. Risk: low–med.*

**Phase C — rate-limiter unification (behavior-changing, identity-touching):**
- C1. One per-provider limiter shared by all three surfaces; throttle the identity fan-out (M-001). *Effort: M. Risk: med — adds latency to add/refresh (identity calls now wait their turn). This is a deliberate correctness-over-speed trade; measure the add/refresh time delta.*

**Phase D — matching unification (highest blast radius; gated):**
- D1. Route identity + dedup through `text_norm::title_tokens`; one `same_work` (M-002). *Effort: M–L. Risk: **high** — this changes identity decisions (which books merge). Must follow the project's TDD gate: behavioral tests first (red), cross-family review, snapshot the DB, and diff identity outcomes on the real library before/after. The C1 colon case is the canary.*

**Sequence rationale:** cleanup (A) makes the rest legible; transport (B) is the already-scoped, lower-risk consolidation; rate-limiter (C) is the highest-value correctness/perf fix and is contained; matching (D) is last because it is the riskiest and benefits from A/B/C being settled first.

---

## Open questions / could not verify

1. **Is the 0.6 cover gate vs 0.75 identity threshold intentional (M-008)?** Product call. I did not find a written decision.
2. **Is the live 429/ban rate dominated by M-001 (rate) or by anti-bot (DataDome)?** I did not reproduce a 429 this session. 60% it's rate-driven.
3. ~~**`metadata_cache` (migration 056) — dead or wired?**~~ **RESOLVED (Pass 2): dead/unwired (M-011).** Table, trait, and impl exist; nothing in the enrichment pipeline calls them. The live cache is the 5-min in-memory `TransportCache`.
4. **OL inline parser in `lookup_openlibrary` — exact duplication scope (M-004).** Read by a subagent, not diffed line-by-line against the external-data OL client by me. 85%.
5. **Real-world mis-merge rate from M-002.** The divergence is certain; its frequency on the actual catalog is not measured. The colon (C1) case is the most likely to matter.
6. **C1 test line (`english_identity_resolver.rs:~918`)** — subagent-sourced; I confirmed the *behavior* (colon break at `:744` ✓) but did not open the test itself.

---

## Reconciliation with the 2026-06-07 audit (what changed)

| Prior finding | Status now |
|---|---|
| **#133 / F1** foreign data leaks through one of two paths | **FIXED** — `drop_language_incompatible_providers` is a single chokepoint both paths cross (`enrichment/lib.rs:476` ✓). |
| **F2** no shared seed builder | **FIXED** — `livrarr-domain/seed.rs` is the one seed home (insight 53). |
| **F6** `series_id` never back-filled | **FIXED** — `series_backfill` job + `reconcile_work_series` write it. |
| **F7** cover dims uncalled | **PARTLY FIXED** — ebook done; audiobook still dropped (M-007). |
| **R8 / F8** god object + dead `MetadataProvider` *trait* | god object **STILL OPEN, grew** (M-005); the dead *trait* was deleted (insight 53 F8) — note `MetadataProvider` today is a live *enum*, a different thing. |
| Door wiring worry | **CLOSED** — no bypass found; all doors hit one gate. |
| **NEW (not in prior audit):** transport fork (M-003/M-004), fragmented + unthrottled rate limiting (M-001), matching fork (M-002) | the live priorities. |
