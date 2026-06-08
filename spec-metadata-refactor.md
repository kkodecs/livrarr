---
feature: "metadata-refactor"
stage: spec
status: draft
version: 4
req_ids: [REQ-001, REQ-002, REQ-003, REQ-004, REQ-005, REQ-006, REQ-007, REQ-008, REQ-009, REQ-010, REQ-011, REQ-012, REQ-013, REQ-014, REQ-015]
---

# Spec: metadata-refactor

Rebuild metadata enrichment as **one simple pipeline** that every entry path funnels through, replacing today's tangle of parallel add/enrich/cover roads.

## 0a. Design Principles

Choices committed to. If a requirement conflicts, the principle wins.

- **P-A — One road.** There is exactly one enrichment pipeline. Every door (Add box, author page, manual import, list import, refresh, background) funnels into it. Doors differ only in the *seed* they hand in, never in the road they take.
- **P-B — Convergence.** The same book yields the same final metadata regardless of which door created it. Destination is identical; only starting richness differs.
- **P-C — Deterministic.** The merge is pure and deterministic. No LLM participates in field selection or in enrichment at all.
- **P-D — One home per policy.** Each cross-cutting decision (provider selection, pacing, saving) lives in exactly one place the pipeline is forced through — enforced by crate boundaries, not convention.
- **P-E — Simplest thing that works.** Prefer the minimal mechanism. Add structure only where a real failure (proven, not anticipated) demands it.
- **P-F — Language is sacred.** A book's metadata is sourced only from providers appropriate to its language. No cross-language contamination, ever.
- **P-G — Partial beats wrong beats empty.** A provider being down yields fewer fields, never a wrong value, and never a blocked add.

## 0b. System Truths

Facts about the environment we don't control. (Provider audiobook capabilities verified online 2026-06-08.)

| ID | Source | Guarantee | Forbids | Confidence |
|----|--------|-----------|---------|------------|
| ST-001 | Hardcover GraphQL API | Exposes audiobook data: `audio_seconds` (duration), `edition_format`/`physical_format`, narrators via `Contributions` | Assuming "only audio-native providers return audio fields" → a single all-fields priority list | High |
| ST-002 | Goodreads book pages | Carry audiobook editions with narrator + listening length (community-entered, unstructured) | Same as ST-001 | High |
| ST-003 | Google Books API | `volumeInfo` has **no** narrator/duration/audiobook fields | Relying on GB for any audio field | High |
| ST-004 | Google Books API | Hard ~1,000 requests/day quota; API key required (keyless quota = 0) | Unbounded GB calls; any design without a daily budget + backoff-to-reset | High |
| ST-005 | Goodreads (DataDome) | Anti-bot; rate-sensitive; no official API | Hammering GR; high call volume | High |
| ST-006 | Data model (`works` table) | A work has **no** `media_type`; it carries `monitor_ebook`/`monitor_audiobook` flags + two cover slots (`cover_url`, `audiobook_cover_url`). `media_type` lives on the file (`LibraryItem`). | Treating "format" as a single per-work property | High |
| ST-007 | `work_metadata_provenance` table | Records per-field setter; `setter = User` marks a user-locked field | A second/parallel user-lock mechanism | High |
| ST-008 | Seed (`AddWorkRequest`) | Guarantees only `title` + `author`; `language` is optional; no format field | Assuming the seed always carries language/format today | High |

## 1. Problem Statement

Metadata enrichment has drifted into several half-different roads. The add-from-search/author-page path reuses pre-add provider data and **skips the cover/save step entirely** (`try_reuse_cached_payloads`), so freshly added books arrive with no cover while a later refresh fixes them. The merge has an LLM override layer that adds latency and nondeterminism and blocks the whole record on a single identity dissent. The "save" step (cover download, tag write) has no single home — it's spread across three crates, which is how the cover bug slipped through. The net symptom: **the same book lands differently depending on which door you used.**

This rebuild collapses everything to one pipeline with one home per policy, so divergence-by-door becomes structurally impossible.

## 2. Requirements

- **REQ-001**: Single pipeline. All entry paths produce metadata exclusively through one enrichment pipeline: *ask sources → merge → save*. No path may write enrichable metadata, covers, or tags by any other route.
- **REQ-002**: Seed contract. The pipeline accepts a seed guaranteeing `title` and `author`. `language` selects the provider policy; if a seed arrives without a language, the pipeline treats it as English (belt-and-suspenders interim guard). The durable fix — enforcing language-required at the add boundary — is tracked but not in this refactor. The pipeline assumes the work may be **both** ebook and audiobook and enriches for both; it never branches on a per-work format.
- **REQ-003**: Provider policy by language. Which providers are consulted, and in what priority, is data — a policy table keyed by **language**. Each language has **two self-contained priority lists**: an **ebook** list (shared content fields + ebook cover + page count) and an **audiobook** list (narrator, duration, narration type, abridged, audiobook cover). Lists are used as written: no fallback, no concatenation, no provider appearing twice **within a list** (a provider may legitimately appear in *both* the ebook and audiobook lists — e.g., Hardcover/Goodreads, per ST-001/ST-002). A language absent from the table receives a defined **generic** row (a standalone row, used instead of a dedicated one, never appended).
- **REQ-004**: Deterministic merge. For each field, the winner is the highest-priority provider (per the field's list) that returns a non-empty value. Shared content fields source from the ebook list; audiobook-specific fields from the audiobook list.
- **REQ-005**: No LLM in enrichment. The merge and the enrichment pipeline make zero LLM calls.
- **REQ-006**: Covers. Each cover slot (ebook, audiobook) is filled by the highest-priority source that returns that kind of cover. Covers are **not** ranked by resolution/dimensions. Something beats nothing (no minimum-size floor). A user-locked cover is never overwritten. A refresh that yields no cover never blanks an existing cover (the same non-destructive rule as identifiers, REQ-007).
- **REQ-007**: Identifiers accumulate. Provider keys/identifiers (`isbn_13`, `asin`, `ol_key`, `gr_key`, `hc_key`) are accumulated; a known identifier is never blanked by a merge in which no provider supplied that identifier.
- **REQ-008**: User edits preserved. A field whose provenance is `setter = User` — series name, series position (identity title/author are already locked) — is never overwritten by enrichment. **A user-chosen cover is preserved via `cover_trust = User`**, the canonical cover lock the merge honors (`resolve_cover` bails on `User`); it is set whenever a user picks or uploads a cover and is kept in sync with the legacy `cover_manual` flag. The merge additionally honors provenance `CoverUrl = User`. So provenance is the single lock for metadata *fields*, while *covers* lock on `cover_trust`. (As-built reconciliation: collapsing the overlapping cover signals — `cover_trust`, `cover_manual`, cover provenance — into one, retiring `cover_manual` to a derived mirror, is a tracked post-refactor cleanup, **not** part of this refactor.)
- **REQ-009**: Caching is transparent. From the pipeline's view, "ask a provider" = check the `(work, provider)` cache (≤24h) → hit: use it; miss: fetch (through pacing) → store. A **refresh** bypasses the cache. Cache population, keying, and search-box behavior are outside this pipeline's concern.
- **REQ-010**: Pacing. All provider network calls pass through one shared gate that enforces: per-provider rate limit, per-provider **daily budget** (GB), and **foreground-before-background** priority (a user waiting outranks background work). The gate records each call's outcome (ok / rate-limited / quota-exhausted-until-T / blocked) to the provider **status page**. Today nothing is written there.
- **REQ-011**: Status model. Stored enrichment status is the existing enum — `Unenriched` / `Enriched` / `Thin` / `Failed` (no new persisted states, no migration). Status resolves by outcome: **`Enriched`** if ≥1 usable field was saved; else **`Thin`** if at least one provider responded successfully (even with zero usable fields); else **`Failed`** (no provider returned a usable response — all errored, blocked, or rate-limited). A *mixed* outcome (one provider responds empty, others error) is `Thin`, not `Failed` — a successful response occurred. **"In Progress" is not stored** — it is derived live from queue membership (a job is queued/running for the work) and shown transiently in the UI; deriving it self-heals if a process dies mid-run. On `Failed`, the user sees a notification with a **retry link**. There is **no automatic retry** (no background job re-dispatching works); recovery after a bulk failure is a user-triggered **"Retry all failed"** action that re-enqueues failed works through the same queue and daily budget.
- **REQ-012**: Save (materialize). Cover download and tag writing happen in one place, performed only when the merged metadata actually changed. Re-running enrichment that changes nothing does not re-download covers or rewrite tags.
- **REQ-013**: Never block; partial beats empty. Provider failures, an unconfigured provider, or zero results never fail the add. The work is saved with whatever was obtained (possibly just the seed) and the status reflects the outcome.
- **REQ-014**: Language integrity. A work is enriched only from providers appropriate to its language. Foreign-language works exclude Hardcover and OpenLibrary (intentional policy). No English-edition data is written onto a foreign record, on any path (add or refresh).
- **REQ-015**: Covers independent of identity. Held/unverified works (identity not confirmed) still receive covers. Cover resolution is not gated behind identity confirmation.

## 3. UI/Interface Design

Minimal, building on the existing two-section Book Information tab:
- Status badge reflects the **stored** enrichment status (`Unenriched` / `Enriched` / `Thin` / `Failed`); the UI may display `Unenriched` as "Pending." A transient "In Progress" / "enriching…" indicator is derived from **queue membership**, not from the status enum.
- On `Failed`, a toast/notification with a **Retry** link (manual, single-work). A **"Retry all failed"** action provides bulk recovery (re-enqueues failed works through the same queue + budget). No auto-retry.
- Provider status page shows per-provider health from REQ-010 (e.g., "Google Books — quota exhausted until 00:00 PT", "Goodreads — blocked").

No new pages. Full mockups deferred unless the PO wants them.

## 4. Non-Requirements

Explicit exclusions:

- **Identity verification** ("is this the right book?") — a separate subsystem. Enrichment trusts the seed's identity and does not check it.
- **Series-entity creation/linking** — separate subsystem. Enrichment sets the `series_name`/`series_position` strings only. **F6 / #58 / #111 / #112 / #52 (Series-entity creation, FK back-fill, language-aware series dedup) remain unaddressed and will still be broken after this refactor** — a deliberate scope boundary, not a fix.
- **Tag-writing internals** — owned by the materialize component; this spec does not specify tag formats or writers (audio tag writers are currently disabled for OOM reasons; that stays as-is).
- **Seed language population fix** — the upstream bug where doors hardcode `"en"` and audiobook files aren't read for language is deferred. Interim guard: REQ-002's English default. Durable fix (tracked, not in this refactor): enforce language-required at the add boundary.
- **Cover dimensions (#134)** — not captured. Size-based ranking was dropped (REQ-006), so `cover_width`/`cover_height` stay unused by design.
- **Search-box / discovery caching** — how candidate data is fetched and cached pre-pick is out of scope; the pipeline only checks "does a cache entry exist."
- **mp3-specific audiobook handling** — deprioritized (m4b matters; mp3 audiobooks do not).
- **Pacing numeric tuning** — exact worker counts, per-provider rates, GB budget value, and backoff durations are an Architecture/Design concern, not a spec requirement.

## 5. Open Questions

| ID | Question | Status | Resolution |
|----|----------|--------|------------|
| Q-001 | Per-provider eligibility: is a provider's *search* payload complete enough to pre-seed the enrichment cache? | resolved | Per-provider property, decided in each provider's module. GR autocomplete = thin (don't seed); GB ≈ full (may seed). Detail, not a spec requirement. |
| Q-002 | Pacing values (workers, rates, GB daily budget, backoff-to-reset window) | resolved | Deferred to Architecture/Design — numeric tuning, not a spec-blocking ambiguity. |
| Q-003 | Generic-row provider list contents (which providers for unlisted languages) | resolved | Deferred to Architecture — the generic row's membership is an architecture decision. |

## 6. Acceptance Criteria

- [ ] **AC-001** (REQ-001, REQ-002): The same book added via two different doors (e.g., Add box vs author page) converges to an identical final field set, identifiers, and both cover slots.
- [ ] **AC-002** (REQ-001, REQ-012, REQ-015): Adding a book from the author page results in a cover present on the work (the originating bug — no door skips the save).
- [ ] **AC-003** (REQ-005): A full enrichment completes with **zero** LLM calls.
- [ ] **AC-004** (REQ-007): A refresh in which no provider returns a `gr_key` leaves the work's existing `gr_key` intact (not blanked).
- [ ] **AC-005** (REQ-008): A user-set series name / position / cover survives an automatic refresh unchanged.
- [ ] **AC-006** (REQ-014): A foreign-language work is never written any Hardcover or OpenLibrary field, on both the add and refresh paths.
- [ ] **AC-007** (REQ-003, REQ-004): Audiobook fields (narrator, duration, audio cover) resolve from the audiobook list's priority (Audible-first), even though Hardcover/Goodreads also return audio data.
- [ ] **AC-008** (REQ-010, REQ-013): With the Google Books daily budget exhausted, GB calls stop, other providers still run, the work is saved with partial data, and the provider status page shows GB exhausted.
- [ ] **AC-009** (REQ-011): A failed enrichment sets stored status `Failed`, surfaces a retry notification, and triggers **no** automatic retry. While a work is queued/running, the UI shows a transient "in progress" indicator that is not a stored status.
- [ ] **AC-010** (REQ-012): An enrichment run that produces no metadata change does not re-download the cover or rewrite file tags.
- [ ] **AC-011** (REQ-009): A refresh bypasses the 24h cache and re-fetches; a non-refresh enrichment within 24h uses the cache (no network call for that provider).
- [ ] **AC-012** (REQ-006): When two providers return covers of different sizes, the higher-*priority* source wins regardless of pixel dimensions.
- [ ] **AC-013** (REQ-002): A seed with no language is enriched via the English policy (interim behavior).
- [ ] **AC-014** (REQ-003): An unlisted language uses the generic row alone — no other list is appended to it.
- [ ] **AC-015** (REQ-003): No provider appears twice within a single list; a provider present in both the ebook and audiobook lists is accepted (not rejected as a duplicate).
- [ ] **AC-016** (REQ-010): A foreground add's provider calls execute ahead of an already-queued background work's calls.
- [ ] **AC-017** (REQ-013): An unconfigured provider is skipped; the remaining providers still enrich and the work is saved.
- [ ] **AC-018** (REQ-011, REQ-013): When all providers return no usable data, the work is saved with its seed and lands `Thin` (or `Failed` if every attempt errored), never blocking the add.
- [ ] **AC-019** (REQ-011): "Retry all failed" re-enqueues failed works and triggers no background retry loop.
- [ ] **AC-020** (REQ-011): A work where one provider responds with no usable fields and the rest error lands `Thin` (a successful response occurred), not `Failed`.

## 7. PO-Locked Architectural Decisions (carry into Stage 2)

These are HOW-decisions the PO locked during spec conversation. Recorded here so they aren't lost; they belong to Architecture, not the behavioral REQs above.

- **Materialize crate.** The save step (cover download + tag write) becomes its own crate — the single home all paths are forced through (realizes P-D + REQ-012).
- **Provider-policy table cached in memory.** DB is source of truth; loaded into an in-memory snapshot at startup; rebuilt + atomically swapped on edit. Lookup is a memory read (REQ-003).
- **Shared priority queue for pacing.** Unit of work = one provider call. Fan-out = submit-N-and-join. Two lanes: foreground (drains first) and background (REQ-010).
- **Cache key = `(work, provider)`, 24h TTL.** Lives inside the provider layer; pipeline treats it as a black box (REQ-009).
- **No background retry job.** Removing it is a deliberate consequence of REQ-011 and the primary fix for the GB quota churn (ST-004).
- **Cutover strategy.** Build the new pipeline, migrate one door at a time, delete each old road as it goes dead (compiler-guided). Not a scorched-earth rewrite.
- **Execution context.** Foreground-vs-background is a flag on the enrichment *invocation* (a call parameter), not part of the seed — set by the caller (interactive add/refresh = foreground; bulk / queue-drain = background). Backs REQ-010's priority semantics.
