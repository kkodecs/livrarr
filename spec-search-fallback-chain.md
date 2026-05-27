---
feature: search-fallback-chain
stage: spec
status: draft
version: 2
req_ids: [REQ-001, REQ-002, REQ-003, REQ-004, REQ-005, REQ-006, REQ-007, REQ-008, REQ-009, REQ-010, REQ-011, REQ-012, REQ-013, REQ-014, REQ-015, REQ-016, REQ-017, REQ-018, REQ-019]
---

# Spec: search-fallback-chain

## 0a. Design Principles

- **Discovery and enrichment are separate concerns.** Discovery finds the work fast. Enrichment assembles the best metadata afterward. They use different providers, different timing, and different quality bars.
- **No single provider is a single point of failure.** OL down, GB quota exhausted, HC token missing — the system degrades, never blanks.
- **User selection is the identity anchor.** The user picks the work and the cover. The system enriches around their choice, never overrides it.
- **ISBN is the cross-provider bridge where available.** When a provider returns an ISBN, enrichment providers that support ISBN lookup use it for exact matching. Providers that don't support ISBN (Audnexus, Audible) continue using their native identifiers (ASIN) or title+author search.
- **Fast add, background enrich.** The user should never wait for enrichment. Create the work immediately; metadata fills in behind them.
- **Multi-user scoping is inherited.** All new search, cover-picker, work-creation, and background enrichment paths are scoped by `user_id`, using the same per-user isolation enforced by the existing architecture (Principle 4).

## 0b. System Truths

| ID | Source | Guarantee | Forbids | Confidence |
|----|--------|-----------|---------|------------|
| ST-001 | Google Books API | 1000 req/day default quota with API key. Fast (200-500ms). Returns ISBN, title, author, cover, language. No stable work-level ID. | Using GB as sole/primary persistent metadata source (ToS §5.e.1 prohibits building databases from responses). | High |
| ST-002 | Google Books API | Published dates reflect arbitrary editions (edition collapsing). Year data is unreliable. | Displaying or relying on GB year/date fields. | High |
| ST-003 | Audible catalog API | Unauthenticated search by title+author at `api.audible.com/1.0/catalog/products`. Returns ASIN, narrators, series, runtime, high-quality covers. Not an officially documented public API — reverse-engineered, same endpoint used by Audiobookshelf. | Treating as a stable contracted API. Must handle unexpected schema changes, 403/429 responses, and potential anti-bot measures gracefully. | Medium |
| ST-004 | OpenLibrary | `/isbn/{isbn}.json` returns an Edition with a `works` key pointing to the OL Work. ISBN → Edition → Work is a supported path. | N/A | High |
| ST-005 | Hardcover | GraphQL `search(query)` accepts free-text including ISBNs. Typesense-backed. 60 req/min, requires user token. Tested: ISBN-13 query returns exact match. | N/A | High (verified live 2026-05-27) |
| ST-006 | Goodreads | `/search?q=isbn:X` accepts ISBN as query. Scraping-only, LLM required for disambiguation. DataDome anti-bot. | N/A | Medium |
| ST-007 | Audnexus | ASIN-based lookup only. Does not accept ISBN. Provides chapters and author metadata that Audible catalog API does not. | Using Audnexus for ISBN-based discovery. | High |
| ST-008 | Existing merge engine | Batch merge with field-level priority model, CAS via merge_generation, provenance tracking. Works correctly today. | Incremental/streaming merge (would require rewrite). | High |
| ST-009 | Existing cover system | Cover proxy validates URLs: HTTPS only, no private/loopback IPs, no embedded credentials, size limit (5MB), SSRF-safe resolver for runtime-derived URLs. | N/A | High |

## 1. Problem Statement

When OpenLibrary is down (403, timeout, anti-bot), add-work search returns empty results. Users cannot add any work to their library during OL outages, even though Livrarr has multiple other metadata providers (HC, GB, GR, Audnexus) that have the same data.

Root cause: discovery is OL-first and sequential. Enrichment is already multi-provider and parallel, but discovery is a single point of failure.

Secondary problems discovered during investigation:
- Audible catalog API (unauthenticated, rich audiobook metadata, ASINs) is not used at all. Livrarr has no reliable ASIN source — a bug in `apply_enrichment_merge` (bare `SET asin = ?` instead of `COALESCE`) means ASINs from Audnexus are lost on re-enrichment.
- User-selected covers can be overridden by enrichment because `cover_manual` is not set during the add flow.
- Add is synchronous — enrichment blocks the response for 5-10s while providers are queried.
- ISBN is available from GB at discovery time but enrichment providers don't use it for lookup (they all do title+author fuzzy search instead).

## 2. Requirements

### Discovery

- **REQ-001**: Google Books is the primary discovery provider. When the user searches for a work, GB is queried first. Results are displayed as a simple list (title, author, small thumbnail). No year displayed (ST-002). All search requests are scoped by `user_id`.
- **REQ-002**: If GB fails or returns empty, fall back to OL, then HC (if token configured), then GR (foreign-language only). Sequential first-success. The search must never return empty if any provider has results.
- **REQ-003**: GB quota exhaustion (403) is non-fatal. Log a warning, continue to the next provider in the fallback chain.

### Cover Selection

- **REQ-004**: After selecting a search result, the user is presented with a cover picker before the work is created. The GB cover is shown as the default.
- **REQ-005**: Cover alternatives are fetched via a dedicated pre-add cover search endpoint (`GET /api/v1/work/preadd-covers?title=X&author=Y&lang=Z`). The endpoint queries available providers (HC, OL, GR, Audnexus, Audible, ISBN-based) and returns a list of cover candidates with source labels and proxy URLs. The frontend polls this single endpoint once; results include all providers that responded within a 10s timeout.
- **REQ-006**: Audible covers must be included in the cover picker. They are high-quality Amazon CDN images and are often the best available cover for audiobooks. Cover URLs pass through the existing cover proxy which validates HTTPS, blocks private IPs, and enforces size limits (ST-009).
- **REQ-007**: The user can accept the default, pick an alternative, or choose "skip" (no cover). Clicking Add creates the work. "Skip" means "I don't want to wait" — it leaves `cover_manual = false` so enrichment can populate a cover later.
- **REQ-008**: When the user actively selects a specific cover (not "skip"), the cover is marked `cover_manual = true` at creation time. Enrichment must not override a manual cover. However, if the initial cover download fails (transient network error), background retry must still attempt to download the same URL — the retry checks whether the cover *file exists on disk*, not just the `cover_manual` flag.

### Async Add

- **REQ-009**: Work creation returns immediately after the DB insert and phase-1 cover download (3s budget, existing behavior). The response includes title, author, cover, and `enrichment_status: Unenriched`. Enrichment fans out to all providers in the background via the existing `tokio::spawn` + `provider_retry_state` mechanism. The background enrichment job is durable — `provider_retry_state` rows survive process restart and the existing background retry job recovers incomplete enrichment.
- **REQ-010**: The UI navigates to the work detail page after add. Empty metadata fields show a shimmer/skeleton state. The frontend polls `GET /api/v1/work/{id}` every 3s until `enrichment_status` transitions from `Unenriched` to a terminal state (`Enriched`, `Failed`, `Conflict`). Fields populate in a single batch when the poll detects the transition. No incremental field updates.

### ISBN Bridge

- **REQ-011**: When GB returns an ISBN at discovery time, it is stored on the Work at creation (`work.isbn_13`). GB may return multiple ISBNs — prefer ISBN-13 directly, fall back to ISBN-10→13 conversion (existing `isbn10_to_isbn13`).
- **REQ-012**: ISBN-based lookup is added to OpenLibrary (`/isbn/{isbn}.json` → Work), Goodreads (`/search?q=isbn:X`), and Hardcover (ISBN as free-text query). Each provider tries ISBN first when `work.isbn_13` is populated. If the ISBN lookup returns empty or NotFound, the provider falls back to title+author search within the same dispatch task before reporting a terminal outcome.
- **REQ-017**: ISBN bridge applies only to providers that support it (OL, HC, GR, GB). Audnexus and Audible use their native identifiers (ASIN) or title+author search. The design principle "ISBN is the cross-provider bridge" applies where technically feasible, not universally.

### Audible Provider

- **REQ-013**: Audible catalog API is added as a new enrichment provider. It searches by ASIN (direct lookup) or title+author (search). It provides: ASIN, narrator names, series name+position, runtime, publisher, cover URL, language. The endpoint is reverse-engineered (ST-003) — the provider must handle unexpected responses, schema changes, and anti-bot measures gracefully (circuit breaker, retry budget, WillRetry on unexpected errors).
- **REQ-014**: Audible is the highest-priority provider for audio fields (ASIN, narrator, duration) in the merge priority model, above Audnexus. For non-audio fields (series, cover, description), Audible is lower priority than HC/GR.
- **REQ-015**: Audible is dispatched for all works (English and foreign). Default storefront is `.com`. For round one, `.com` is best-effort for foreign works — if no result, the provider returns NotFound. Regional storefront selection (`.co.jp`, `.de`, etc.) is a follow-up enhancement.
- **REQ-018**: Audible rate limiting uses a dedicated `RateBucket::Audible` at 150ms interval (matching Audiobookshelf's observed safe rate).

### ASIN Bug Fix

- **REQ-016**: The merge engine's last-known-good fallback already prevents resolving to None when a current value exists. The DB write path in `apply_enrichment_merge` should not use bare `SET asin = ?` — use the merge engine's resolved value which includes the last-known-good fallback. If the resolved value is explicitly None (no provider succeeded AND no current value exists), writing NULL is correct. The fix ensures the merge engine's intent is faithfully written, not that the field can never be cleared.

### Cover Download Safety

- **REQ-019**: All cover URLs from external providers (GB, Audible, HC, OL, GR) pass through the existing cover proxy and SSRF-safe resolver for validation. Cover downloads enforce: HTTPS only, no private/loopback IPs after DNS resolution, no embedded credentials, redirect limit, 5MB size limit, accepted image MIME types, 10s per-request timeout. These constraints are already implemented (ST-009) and apply to all new cover sources without modification.

## 3. UI/Interface Design

### Search Page (replaces `/search`)

- Search bar with language selector (existing pattern)
- Results as compact list rows: small thumbnail (40x56px), title, author, plus icon
- No year column (GB years are unreliable)
- Click a result → transition to cover picker

### Cover Picker (inline, same page)

- Header: selected work title + author + current cover preview + "Add to Library" button
- Grid of cover options: GB cover (default), alternatives from HC/OL/GR/Audnexus/Audible/ISBN
- Loading placeholders while alternatives fetch (single request, 10s timeout)
- "Skip" option (no cover — enrichment will add one later)
- Selected cover has visual indicator (border highlight + checkmark)
- "Add to Library" button creates the work and navigates to work detail

### Work Detail (post-add)

- Shimmer/skeleton on empty metadata fields while `enrichment_status == Unenriched`
- All fields populate in one batch when enrichment completes (poll detects status transition)
- Cover is the user's selection if they picked one (`cover_manual = true`, not overridden)
- If user skipped cover, enrichment-provided cover appears when enrichment completes

## 4. Non-Requirements

- **Edition-level identity.** Works remain the primary entity. No Edition layer between Work and LibraryItem.
- **Multi-provider result aggregation for discovery.** Discovery returns first-success, not a merged result set from all providers.
- **Incremental/streaming merge.** Enrichment merge remains batch (all providers complete, then one merge pass). UI updates once, not per-provider.
- **Audible as a discovery provider.** Audible is enrichment-only. GB handles discovery.
- **Audible regional storefront selection (round one).** Default to `.com`. Regional support (`.co.jp`, `.de`, etc.) is a follow-up.
- **GB data persistence beyond cache headers.** GB data is used for transient display (search results) and ISBN bridging. Persistent metadata comes from enrichment providers.
- **WebSocket/SSE for enrichment progress.** Polling is sufficient for the single-work add flow.

## 5. Open Questions

| ID | Question | Status | Resolution |
|----|----------|--------|------------|
| Q-001 | Should the existing search page (`/search`) be replaced, or should `/work/add-new` coexist as an alternative? | resolved | Replace. One search flow, GB-first. |
| Q-002 | What is the Audible catalog API rate limit? ABS throttles to 1 req/150ms. Do we need a dedicated rate bucket? | resolved | Match ABS at 150ms. Dedicated `RateBucket::Audible`. |
| Q-003 | Should the cover picker fire enrichment providers directly, or use a dedicated "pre-add cover search" endpoint? | resolved | Dedicated endpoint — enrichment requires a work_id which doesn't exist yet. |
| Q-004 | HC ISBN search — does it reliably return the right work when queried with an ISBN string? | resolved | Yes. Tested live: ISBN-13 query returns exact match (Dune, HC ID 312460). |
| Q-005 | Should "no cover" in the picker block enrichment from adding a cover later? | resolved | No. "Skip" means "I don't want to wait." `cover_manual` stays false; enrichment can populate a cover. Only an active cover selection sets `cover_manual = true`. |

## 6. Acceptance Criteria

- [ ] **AC-001** (REQ-001): Searching "Dune" returns GB results with title, author, and cover thumbnails. No year displayed.
- [ ] **AC-002** (REQ-002): With GB API key invalid (simulated), search falls back to OL and returns results.
- [ ] **AC-003** (REQ-003): With GB returning 403, a warning is logged and the next provider is tried. No error shown to user.
- [ ] **AC-004** (REQ-004, REQ-007): Clicking a search result shows a cover picker with the GB cover as default and an "Add to Library" button.
- [ ] **AC-005** (REQ-005, REQ-006): Pre-add cover endpoint returns candidates from multiple providers within 10s. Audible covers appear for works with audiobook editions.
- [ ] **AC-006** (REQ-008): After adding with a user-selected cover, `cover_manual = true` in DB. Manual refresh does not change the cover.
- [ ] **AC-007** (REQ-007): After adding with "skip" (no cover), `cover_manual = false` in DB. Background enrichment populates a cover.
- [ ] **AC-008** (REQ-009): Add returns with `enrichment_status = Unenriched`. Work row exists in DB with title, author, cover (if selected). Enrichment completes in background within 30s (provider_retry_state rows populated).
- [ ] **AC-009** (REQ-010): Work detail page polls until `enrichment_status` transitions. Fields update in a single batch. No intermediate partial state visible.
- [ ] **AC-010** (REQ-011, REQ-012): A work added with ISBN from GB has `isbn_13` populated. Provider retry state shows ISBN-based lookups attempted for OL/HC/GR. If ISBN lookup fails, title+author fallback is attempted (visible in retry state).
- [ ] **AC-011** (REQ-013, REQ-014): After enrichment, a work on Audible has `asin`, `narrator`, and `duration_seconds` populated.
- [ ] **AC-012** (REQ-015): A foreign-language work triggers Audible enrichment (provider_retry_state row for Audible exists).
- [ ] **AC-013** (REQ-016): Re-enriching a work that has an ASIN retains the ASIN when Audnexus/Audible returns WillRetry. Verify: ASIN in DB is unchanged after re-enrichment with simulated provider failure.
- [ ] **AC-014** (REQ-019): Cover URLs from Audible pass through cover proxy. A test with a private-IP cover URL is rejected.
- [ ] **AC-015** (REQ-009): After process restart with incomplete enrichment, the background retry job picks up and completes enrichment (provider_retry_state rows with WillRetry are retried).
