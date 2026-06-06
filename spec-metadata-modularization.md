---
feature: "metadata-modularization"
stage: spec
status: draft
version: 1
req_ids: [REQ-001, REQ-002, REQ-003, REQ-004, REQ-005, REQ-006, REQ-007, REQ-008, REQ-009, REQ-010, REQ-011, REQ-012, REQ-013, REQ-014, REQ-015, REQ-016, REQ-017, REQ-018, REQ-019, REQ-020, REQ-021, REQ-022, REQ-023, REQ-024]
decision_ref: docs/decision-metadata-modularization-sequencing.md
design_ref: design-metadata-modularization.md
folds: work-creation-consistency   # Track-2 feature REQs are re-homed from WCC spec v5 (REQ-024/030/031/032 + D-018)
related_issues: [97]
---

# Spec: metadata-modularization

Split the tangled work-creation + metadata pipeline — today all fused in `livrarr-metadata`'s `enrich_work` spine — into **three concerns on a shared substrate**, with identity flowing **one way** into enrichment, by **structured extraction (C′)**: extract the stable foundation and its ports first, then cut each feature-touched seam *as part of building the feature that refactors it*, landing the result in the target crate — never back in the monolith. This is **reorganization, not a rebuild**.

## Delivery Status (as-built — 2026-06-06)

> **Reconciled to the as-built worktree** (`feat/metadata-modularization` @ `40b6445`, NOT yet merged — PO call). This section is the authoritative as-built map: what shipped, what is deferred (and why), the intentional debt. The REQ/AC bodies below are unchanged **design intent**; the §6 checkboxes mark delivery. Sourced from the committed, cross-family-reviewed chunks; the status enums + completion classifier were code-verified this session.

**Shipped this arc:** 3 of the 4 crate seams extracted (`livrarr-external-data`, `livrarr-identity`, `livrarr-enrichment`) + the two-state-machine status split (4b) + the four WCC Track-2 feature chunks (D → A → B → C). The branch is merge-ready pending PO; `livrarr-materialize` and four named feature-completions are explicitly **deferred** (designed-later, not bugs).

### Delivered

| Unit | What landed | REQ (primary) | Commit |
|---|---|---|---|
| Canary — `external-data` | Shared substrate extracted; `NormalizedWorkDetail`/`ProviderOutcome` relocated; no back-edge (incl. no `livrarr-db`); D-014 shims deleted | REQ-004/005/006/008/011 | `2917490`, `a882dbd` |
| `identity` crate | english/async/bulk/title_cleanup → `livrarr-identity`; one-way graph (`cargo tree` zero back-edges) | REQ-001/002 (3 of 4 crates) | `a79237f` |
| `enrichment` crate | merge/enrich engine + queue/validator/cover-policy carved out of the monolith | REQ-001/002/022 | `808d47a` |
| 4b — two state machines | `IdentityStatus`{Pending/Confirmed/Provisional/Conflict/NeedsReview} + `EnrichmentStatus`{Unenriched/Enriched/Thin} split; "Book Information" tab (Identity + Details sections, in-context badges) | REQ-014, §3 | 4b steps 1–6 |
| 4b — completion classifier | `Enriched` iff ≥1 meaningful **text** field (desc/subtitle/series/genres/publisher); cover never gates (`enrichment/lib.rs:866`). **This is WCC REQ-030.** | REQ-019 (classification) | 4b |
| 4b — de-facto identity | ISBN/ASIN bridge w/ no work anchor → `Provisional` (`IdentityState::derived_identity_status`); enriches; upgrades to `Confirmed` later | REQ-016 | chunk D + 4b |
| Chunk D — winner rule | `run_quorum` anchored-cluster rule: anchored outranks anchorless **only** when an anchored cluster exists; all-anchorless → Resolved→Provisional, not a false Pending | REQ-016 (spine) | `6f1e5c3` |
| Chunk A — discovery fan-out | Add-Work *search* box: 3-way → 4-way (`+Goodreads`), interleave/cap/tail-filter; **GR via `/book/auto_complete`** (no LLM) — carries the `a21c643` GR-autocomplete reconcile onto `external-data` | REQ-005/017; REQ-018 (autocomplete rung only) | `f19dc76` |
| Chunk B — cached-payload reuse | `merge_from_cached` promoted to the `MergeEngine` trait; `try_reuse_cached_payloads`; consume-once/pass-scoped cache; `candidate_id` threaded end-to-end; "trust-the-pick" zero-network add | REQ-017/023 | `e181dc2` |
| Chunk C — Tier-A auto-match (#97) | `eager_match_by_author` (author-grouped GB+OL, ISBN-beats-title), `SuggestedMatch`, per-file suggestions + Confirmed import reusing the cache; OL-only legacy machinery removed | REQ-016 (import path), issue #97 | `40b6445` |

### Design §3 deviations (intentional, PO-approved)

| Deviation | What | Why |
|---|---|---|
| **Sync enrichment KEPT** | Chunk C kept **synchronous** enrichment — NOT the stranded WCC deferred-enrichment / fast-return / post-add-cover / author-skip path | PO Design-§3 scope: core threading + MatchCluster harvest only; the async deferral added complexity without a manual-import payoff |
| **Goodreads NOT in the eager pass** | `eager_match_by_author` queries **GB + OL only**; GR stays in the *interactive* Add-Work search (chunk A) | GR autocomplete isn't author-scoped/batch-friendly; its anti-bot fragility isn't worth it in a bulk background pass |
| **MatchCluster harvest ADDED** | Import `MatchCluster` now harvests file **isbn/asin/year/language** from the parser's primary (feat had hardcoded `None`) | Threads real edition identifiers through the import path so cache-reuse + Provisional identity actually fire |

### Deferred (designed-later; NOT built — do not treat as bugs)

| Item | REQ / AC | Why deferred | Gate before code |
|---|---|---|---|
| `livrarr-materialize` crate + sync/async entry points | REQ-001/003/019(materialize)/021; AC-001(4th crate)/003/017 | Cover↔`work_service` bidirectional cycle (seam #3/#7) — not a clean Track-1 move. Cover/tag projection still lives in `enrichment`. | Design the cover-decouple seam first |
| **Status-backport-drop** | REQ-014 follow-up | `EnrichmentStatus` still carries `{Conflict, IdentityPending, NeedsReview}` duplicating `IdentityStatus`; the merge engine still **writes** `EnrichmentStatus::Conflict` as an interim signal (a seam-2 one-way violation). Redundant, not broken. | **Pseudocode + PO approval.** See `wiki/livrarr/gotchas.md` (dual-status). **Pin first:** `EnrichmentStatus::Conflict` semantics — enum comment says "LLM identity-validation provider mismatch" but memory `project_enrichment_conflict_semantics` says "LLM rejected all provider payloads, not an identity dispute." |
| Full GR anti-bot ladder + `gr_key` verify | REQ-018; AC-014 (← WCC REQ-032/024) | Only the `/book/auto_complete` rung shipped (chunk A). The cost-ordered ladder (TLS-impersonation HTML → LLM-locator → give-up), `WillRetry`-not-`NotFound`, and `/book/show` verify-before-persist are unbuilt. | Discovery-seam feature |
| Bug #2 — identifier-change invalidation | REQ-020; AC-016 (← WCC REQ-031) | A dependent provider's terminal failure is not yet invalidated when its prerequisite id resolves `None → Some`. | Its own feature unit |
| Clean-overwrite provenance | REQ-024; AC-020 | Already design-stage in §6. | (design-stage) |

## 0a. Design Principles

The boundary invariants. If a requirement conflicts, the principle wins. (These extend — never override — `build/foundation/principles.md`.)

1. **One-way identity.** Identity → Enrichment only, **compile-enforced**. Enrichment never mutates identity. The two are non-dependent siblings; they communicate solely through the `EstablishedIdentity` contract type in `livrarr-domain`.
2. **Monotonic anchors.** A Work's federated identity is append-only. Three write paths, only one mutates an *existing* anchor: **ADD** (auto, append a new anchor type), **CONFLICT** (auto, raise for the user — never silently change), **EDIT** (user, the only path that mutates an established anchor).
3. **Dumb pipe, smart policy.** One shared `livrarr-external-data` substrate owns the *mechanism* (HTTP, normalize, rate-limit/GCRA, circuit-break, anti-bot ladder, the payload cache). Each consumer owns its own *selection policy* (Identity: who resolves an id; Enrichment: who has the best fields + language routing).
4. **Payloads are facts, not identity.** Metadata flows via the shared cache; identity flows via the contract. Never carry "the description Identity happened to fetch" inside the contract — that re-couples the subsystems.
5. **Reorganize, don't rebuild.** The existing code is good, just un-walled (~73% is cleanly factored). Extraction is **behavior-preserving**; tests stay green.
6. **Cut seams via features, into the target crate.** The three messy couplings (discovery, cover, status) are *exactly* what three planned features refactor — cut-the-seam and build-the-feature are one act, done in the target crate. Never pour new work back into the monolith.
7. **Falsifiable first move.** The first extraction (`livrarr-external-data`) is a cheap, reversible experiment that *proves or kills* the plan before any large commitment.
8. **Display never blocks; enrich gates.** Always show the best metadata in hand (never blocked on the network); the gated provider fan-out (enrich) runs only once Identity has a deterministic addressing key.

## 0b. Baseline truths (the code as-is)

Facts about the current structure, verified this cycle (Serena + code-index). These ground the migration — they are *what we are reorganizing*, not aspirations. (Confidence: High = verified in code.)

| ID | Fact | Confidence |
|----|------|------------|
| BT-001 | `livrarr-metadata` ≈ 20,600 LOC / 37 modules. ~73% is genuinely well-factored — the add→enrich→materialize spine is orchestration over injected `DB/E/Q/ME/V/L/T` services. The "swamp" is **localized** to `work_service`'s inline discovery + resolver-composition. | High |
| BT-002 | **Two** provider-access paths: (a) the **enrichment** path (`provider_queue → provider_client`) is trait-based + injected — clean; (b) the **discovery** path (`WorkService::lookup_filtered`, work_service.rs:1378-1564) is **inline** parallel fetch via concrete `lookup_google_books/openlibrary/hardcover/goodreads` — not trait-routed. | High |
| BT-003 | The stable contract types `NormalizedWorkDetail` and `ProviderOutcome` currently live in `livrarr-metadata`'s `lib.rs`. They must relocate to `livrarr-external-data` first — that relocation *is* the canary (§ REQ-011). | High |
| BT-004 | `transport_cache()` (work_service.rs:2640) reaches the **concrete** `LiveEnglishIdentityResolver` cache (`Option<Arc<LiveEnglishIdentityResolver>>`) — a leaky boundary, not a clean crate seam. | High |
| BT-005 | `add` **enriches `Pending` works today** (work_service.rs:816) — contradicts the target enrich-gate (REQ-015). A behavior delta the migration must make explicit. | High |
| BT-006 | The identity-federation foundation (`f78f3bc`) is **already on `main`**; the WCC branch carries the in-flight metadata-pipeline work whose feature REQs are re-homed here as Track-2. | High |
| BT-007 | The compile-wall pattern already exists (`livrarr-handlers` is walled off `livrarr-db/metadata/tagwrite/download`; verified via `cargo tree`). The new crate boundaries reuse this exact mechanism — no new infrastructure. | High |

## 1. Problem Statement

`livrarr-metadata` fuses four distinct concerns into one ~20k-LOC crate with no internal walls:

- **Identity resolution** ("what work is this?") is interleaved with **enrichment** ("fill the work's metadata record") inside one `enrich_work` spine and one `WorkService`. Nothing structurally prevents enrichment from mutating identity, or identity from leaking fetched metadata — both are real coupling risks today (BT-004, BT-005).
- **Provider access** (the dumb-pipe mechanism: transport, normalize, rate-limit, cache) is split across two incompatible shapes (BT-002) — a clean trait-based enrichment path and an inline, concrete discovery path — so the discovery front-half cannot reuse the enrichment substrate, and `#97` (a single-provider-first-hit chain) lives in the inline path.
- **Materialize** (projecting the canonical record onto physical artifacts — cover image + file tags) is folded into enrichment, so a fragile, disk-bound, 1:N concern (the audio-tag OOM class) shares a crate and a failure domain with the network-bound, 1:1 enrichment concern.
- The **contract types** that *should* be a stable shared vocabulary (`NormalizedWorkDetail`, `ProviderOutcome`) sit in `lib.rs` (BT-003), so any consumer of "a normalized provider payload" must depend on the whole monolith.

The result: a crate that cannot be reasoned about or evolved one concern at a time; a discovery path that re-fetches what enrichment already has (and vice-versa); a state model (`EnrichmentStatus`) that conflates *identity*-uncertainty with *enrichment*-thinness; and a materialize failure (audio-tag OOM) that can stall the whole pipeline.

This feature makes the boundaries **explicit and compiler-enforced**: four crates (`livrarr-external-data`, `livrarr-identity`, `livrarr-enrichment`, `livrarr-materialize`) on a shared substrate, a one-way `EstablishedIdentity` contract in `domain`, and the three feature-touched seams (discovery / cover / status) cut by the three features that refactor them — extracted by **structured extraction (C′)**, validated by a **falsifiable canary** as the first move.

## 2. Requirements

### Crate decomposition & boundaries

- **REQ-001**: The pipeline MUST be decomposed into four library crates plus a contract in the foundation crate: **`livrarr-external-data`** (shared substrate), **`livrarr-identity`** ("what work is this?"), **`livrarr-enrichment`** ("fill the record"), **`livrarr-materialize`** ("project onto artifacts"), and the **`EstablishedIdentity` contract type in `livrarr-domain`** (the leaf). Each crate owns one concern; no crate re-implements another's responsibility.
- **REQ-002**: **Identity → Enrichment is one-way and compile-enforced.** `livrarr-identity` and `livrarr-enrichment` MUST be non-dependent siblings — neither names the other in its dependency graph (`cargo tree`-verifiable). They communicate **only** through the `EstablishedIdentity` contract in `domain`. Enrichment MUST NOT mutate identity; identity convergence (a later pass that upgrades an anchor) is owned *inside* `livrarr-identity`. **When enrichment *detects* a remote identity contradiction** (a fetched anchor disagrees with the established one), it MUST NOT write identity state — it MUST emit a domain `IdentityContradiction` event/error that the composition root (`livrarr-server`) routes to `livrarr-identity` to transition the Work to `Conflict` (round-1 review: without this path a detected conflict would be silently dropped). Enrichment reports evidence; identity owns the decision.
- **REQ-003**: **Materialize is a separate downstream crate (three boxes), not folded into Enrichment.** The decisive reasons are cardinality (Enrichment is 1:1 with the Work — logical, network-bound, slow; Materialize is 1:N with physical files — disk-bound, fast, fragile), **failure isolation** (the audio-tag OOM class MUST NOT be able to stall enrichment), per-format on/off policy (EPUB on, m4b off), and the requirement that the import/library path can depend on Materialize **without** pulling Enrichment's network/LLM dependencies.
- **REQ-004**: **`livrarr-external-data` is the shared, STATELESS substrate** both Identity and Enrichment consume. It owns the *mechanism* only: HTTP transport, normalization, rate-limit/GCRA, circuit-breaker, and the in-memory payload cache. It MUST NOT own *selection policy* (which providers to consult — that is each consumer's) **nor durable orchestration state**: the DB-backed provider retry queue (`provider_queue`/`provider_retry_state`) and the GR anti-bot *access ladder* (a discovery policy) are NOT part of the dumb pipe — they live in `livrarr-enrichment` and `livrarr-identity` respectively (round-1 review: a DB-stateful queue in the substrate would force a `external-data → livrarr-db` edge and break the no-back-edge invariant, REQ-006).
- **REQ-005**: **`livrarr-external-data` MUST expose both a search/discovery surface and an enrichment-fetch surface** — not just `ProviderClient::fetch`. The enrichment shape (`fetch(&Work, &EnrichmentContext) -> ProviderOutcome<NormalizedWorkDetail>`) is **not** the discovery shape, so the inline discovery path (`lookup_filtered`) cannot consume a fetch-only crate. The stable contract types **`NormalizedWorkDetail` and `ProviderOutcome` MUST move out of `livrarr-metadata`'s `lib.rs` into `livrarr-external-data`** (BT-003).
- **REQ-006**: **No back-edge.** `livrarr-external-data` MUST NOT depend on `livrarr-metadata`, `livrarr-identity`, `livrarr-enrichment`, **or `livrarr-db`** (the DB edge is the canary-killer — round-1 review). The permitted resolution is narrow and bounded: only a **genuinely pure value type** (e.g. `LookupResult`, a provider-key newtype) may be relocated into `domain` and retried (≤3 such relocations). `EnrichmentContext` and the queue/retry traits are **NOT relocatable** — they encode enrichment policy/state, so external-data consumes a external-data-local mechanical `FetchRequest` instead and the queue stays in `livrarr-enrichment` (REQ-004/022). Any back-edge that survives the bounded pure-type relocation — including any `external-data → livrarr-db` edge — is the canary's **NO-GO** (REQ-011). The acyclic, one-way dependency graph MUST be `cargo tree`-verifiable against **all four** forbidden targets.
- **REQ-007**: **The contract is pure identity, no metadata.** `EstablishedIdentity` carries `{ work_id, anchors+bridges (ol/gr/hc/isbn/asin), title, author, language, state, method, generation }` and nothing more. Field/metadata data MUST flow via the shared cache, never inside the contract — Enrichment depends on the *contract*, not on Identity (Principle 4).
- **REQ-008**: **The provider payload cache key MUST be `(provider, provider-key)`** — e.g. `(Goodreads, "12345")` — never `work_id`. Keying by `work_id` leaks the Work concept into the provider layer, and `work_id` does not exist yet during discovery.

### Migration discipline (C′ — structured extraction)

- **REQ-009**: **Track 1 (stable foundation) extraction MUST be behavior-preserving.** Extracting `livrarr-external-data`, `livrarr-materialize`, and the `EstablishedIdentity` contract MUST NOT change runtime logic — only relocate it and fix imports. `cargo build` + `cargo test` MUST stay green at each extraction step.
- **REQ-010**: **Feature-touched seams MUST be cut via their feature, into the target crate.** The three couplings (discovery, cover, status) are refactored *as part of building* the feature that touches them (discovery → GR ladder; cover → cover-decouple; status → two-state-machines) and the result lands in the target crate (`external-data`/`identity`/`materialize`) — **never back in `livrarr-metadata`**. Walling a feature-touched interface *before* its feature reshapes it (a throwaway abstraction) is forbidden.
- **REQ-011**: **The first move is the falsifiable canary.** `livrarr-external-data` is extracted first; the outcome is binary and decisive: **GO** if `livrarr-metadata → livrarr-external-data` compiles with no back-edge (REQ-006) and tests pass — C′ confirmed, proceed; **NO-GO** if extraction forces an unresolvable back-edge — stop, isolate the offending contract type into `domain`/`external-data`, or fall back to plan A (build-in-place-then-extract). Either outcome is a complete, reportable result.

### Identity model (one-way, monotonic, two state machines)

- **REQ-012**: **Identity anchors are monotonic.** ADD (auto) appends a *new* anchor type and never touches an existing one; CONFLICT (auto) raises a contradicting anchor for the user and never silently changes an established one; EDIT (user) is the **only** path that mutates an established anchor. (Generalizes WCC REQ-028 / REQ-020 to the crate boundary.)
- **REQ-013**: **A user ID-edit re-fires enrichment.** The EDIT path is the operational sequence: (1) the user changes an established anchor; (2) `livrarr-identity` validates + writes the new anchor (the only sanctioned mutation of an established anchor) and **bumps the identity `generation`**; (3) the generation bump re-fires the enrichment gate (REQ-015) so enrichment re-runs against the corrected identity (the old metadata may belong to the wrong book); (4) the re-merge **preserves user-set field values** (Principle: User > Provider > System) — a provider value never overwrites a user-set field, even on a forced re-enrichment. ADD/CONFLICT (the auto paths) MUST NOT bump generation in a way that mutates an established anchor — only EDIT does (REQ-012). *(This operational design backs AC-009; the full user-ID-edit UX is a Track-2 feature.)*
- **REQ-014**: **Two independent state machines replace the one overloaded `EnrichmentStatus`.** **Identity:** `Pending → { Confirmed | Provisional | Conflict | NeedsReview }`. **Enrichment:** `Unenriched → { Enriched | Thin }`. This structurally kills the conflation between *identity*-NeedsReview ("which book is this?") and *enrichment*-Thin ("we know the book, found no info") — two different problems, two different user actions, two tracks. (Re-homes WCC REQ-026/030 status work as the **two-state-machine** split.) **User-facing labels + the "Book Information" tab layout: see §3.**
- **REQ-015**: **Enrichment gates on a deterministic addressing key (Q4 gating).** Display (best-in-hand) is always shown and never gated; **enrich** (the provider fan-out) runs per the Identity-state table: **Confirmed → enrich; Provisional (ISBN resolved, no work anchor) → enrich; Pending (fuzzy title+author, no key) → hold (converge first); Conflict → pause (user resolves); NeedsReview → hold (user resolves).** Enrichment re-fires on an identity `generation` bump (a new anchor → better providers; a user edit → redo). This makes BT-005 (today's "enrich Pending works") an explicit, corrected behavior delta.
- **REQ-016**: **De-facto identity — an ISBN-only resolution is Tier-A `Provisional` identity, not Pending.** An ISBN that resolves to ≥1 provider but yields no work anchor (e.g. only Google Books — no work anchor, ST/AC-019 intact) MUST settle as a **`Provisional`** identity keyed on `isbn_13` (Tier-A, no confirmation prompt — `Provisional` is exactly "ISBN resolved, no work anchor" per REQ-015, and it enriches), NOT a `Pending`/identity-pending limbo. Background convergence **upgrades it to `Confirmed`** when a work anchor later appears. A GB `volumeId` is **never** an identity. (Re-homes WCC D-018; round-2 review: aligned to `Provisional` so REQ-015/REQ-016/D-013 agree.)
- **REQ-017**: **Payloads are retained for the pass; Enrichment tops up only what's missing.** Identity's discovery fan-out warms the `(provider, provider-key)` cache (REQ-008); Enrichment reads it and issues a network query only for fields not already in hand — discovery and enrichment MUST NOT each independently re-fetch the same provider for the same data within a pass. Payloads are not discarded *prematurely* (not before enrichment consumes them); once the pass persists the canonical data they are evicted (REQ-023). (Generalizes WCC REQ-014/015 across the new crate seam.)

### Track-2 feature seams (the forcing functions)

- **REQ-018**: **GR anti-bot access ladder (the discovery seam → `external-data`/`identity`).** Goodreads discovery MUST walk a cost-ordered ladder of access paths (e.g. `/book/auto_complete` JSON → TLS-impersonation HTML `/search` → LLM-locator → give-up) so no single path is load-bearing. An access **block** (challenge / format failure) MUST be a *transient* outcome (`WillRetry`) that escalates to the next rung — **never** a terminal `NotFound` that suppresses the provider on later passes. A persisted `gr_key` obtained by any path other than direct harvesting MUST be **verified** against a stable `/book/show` detail page before persistence. (Re-homes WCC REQ-024/032; lands the ladder in the extracted crates, not the monolith.)
- **REQ-019**: **Cover-decouple (the cover seam → `materialize`).** Enrichment **completion** MUST be classified on identity + meaningful text metadata, NOT on cover presence: a confirmed-identity Work carrying ≥1 meaningful text field is `Enriched`; the cover is a **lazy backfill asset** that MUST NOT gate completion and is **never an identifier** (a different cover ≠ a different work). `livrarr-materialize` MUST expose two entry points: a **synchronous** `materialize_cover(work_id)` the add path can block on, and an **async queued** `materialize_tags(...)` for the heavy/fragile disk writes. (Re-homes WCC REQ-030; lands cover/tag projection in `materialize`.)
- **REQ-020**: **Identifier-change invalidation (Bug #2).** When an enrichment merge resolves a dependent identifier `None → Some` (e.g. an `asin` that Audnexus requires), the dependent provider's **terminal** failure state MUST be invalidated within the same merge transaction so the next background pass re-queries it and converges. A provider MUST NOT remain permanently terminal solely because its prerequisite identifier was resolved after its first attempt. (Re-homes WCC REQ-031.)
- **REQ-021**: **Materialize idempotency.** `livrarr-materialize` MUST rewrite a physical file only when `merge_generation > tag_generation` (self-healing, trivially retriable). This makes the materialize step idempotent and decouples its retry cadence from enrichment's.

### Provider substrate retention

- **REQ-022**: **The resilience machinery is reused, not rewritten** (Principle 5), and partitioned by statefulness: the **stateless** GCRA rate-limiter and circuit-breaker live in `livrarr-external-data` (the substrate exposes them; consumers do not re-implement pacing or breaking). The **stateful, DB-backed** retry queue (`provider_queue`, durable `provider_retry_state`) and the CAS-merge mechanics live in `livrarr-enrichment`/`livrarr-db` — NOT in the substrate (round-1 review: their `livrarr-db` dependency must not enter `livrarr-external-data`, REQ-006).
- **REQ-023**: **The payload cache is consume-once / pass-scoped** (PO-clarified 2026-06-03). Its sole purpose is to avoid repeated provider lookups *within a single identity→enrichment pass*: discovery warms it, enrichment consumes it, and once the pass persists the canonical data to the Work's DB record the entry MUST be **evicted** (the "good" data now lives in the DB, not the cache). A short safety **TTL** reaps entries from passes that abandon before consuming. There is therefore **no long-lived cache and no unbounded-growth problem** — the cache holds only in-flight passes. (Resolves the former design-stage retention question.)
- **REQ-024**: **Provisional write-amplification is a clean overwrite.** Enriching a `Provisional` Work fills an edition record that may later upgrade to `Confirmed`; when the identity `generation` bumps, the merge MUST do a **clean overwrite** (no orphaned provenance rows) rather than accreting stale provenance. (Mechanics are design-stage.)

## 3. UI/Interface Design

The two-state-machine split (REQ-014) gets a concrete home (PO-decided 2026-06-03): the book detail view's **"Book Information"** tab — **renamed from "Metadata"** so the word "metadata" disappears from the user's view. It carries **two stacked sections**, each owning its own status badge shown **in context under its header** (the header supplies the "what is Pending/Confirmed" that a bare floating badge lacks):

- **Identity** section — IDs / edition / match info, with the identity-track badge:
  - **Pending** — still matching; only a fuzzy title/author guess so far
  - **Confirmed** — locked to a master catalog record
  - **Provisional** — identified by ISBN (barcode); no master record yet, may later upgrade to Confirmed
  - **Conflict** — sources disagree; needs the user
  - **Needs Review** — couldn't match; needs the user
- **Details** section — description / genres / series / cover, with the details-track badge:
  - **Pending** — details not fetched yet
  - **Enriched** — real info present (≥1 meaningful field; the cover is a separate lazy asset per REQ-019, so "Enriched" does **not** require a cover)
  - **Sparse** — known book, but providers returned almost nothing (a *settled* outcome, not "still loading")

"Pending" intentionally appears in **both** tracks — the section header disambiguates it (PO design call). These badge strings are the canonical **user-facing** labels; internal enum names may differ (e.g. the details-track `Unenriched`→"Pending", `Thin`→"Sparse").

The cover-decouple (REQ-019) means a cover may populate *after* the Details badge already reads "Enriched" (fast-then-upgrade). No other new screens; an HTML mockup of the two-section tab is the one design-stage UI artifact to produce.

## 4. Non-Requirements

Explicit scope exclusions:

- **No rebuild.** The existing enrich spine, merge engine, retry/CAS/GCRA/breaker machinery is **reused** (Principle 5, REQ-022). This is extraction + targeted untangle, not a rewrite.
- **No new providers; no new third-party dependencies.** The provider set is unchanged (Audnexus is repositioned by the folded WCC work, not removed). The crate split reuses the existing compile-wall pattern (BT-007) — no new infra crates.
- **No behavior change outside the named seams.** Track-1 extraction is behavior-preserving (REQ-009). The only intentional behavior deltas are the three feature-cuts (discovery/cover/status) and the explicit corrections they carry (REQ-015 Pending-holds, REQ-016 de-facto identity, REQ-019 cover-decouple, REQ-020 Bug #2) — each requires explicit tests.
- **Author identity / author-side modularization.** Out of scope; this feature is the work-creation + metadata pipeline.
- **The full Track-2 feature *implementations*.** This spec establishes the *boundaries* and *seam-cut contracts*; the GR ladder, cover-decouple, two-state-machine, and Bug #2 features are delivered incrementally against these boundaries. The **canary (REQ-011)** is the only implementation this cycle commits to landing.
- **Convergence-resolver re-architecture.** The async/bulk convergence resolver is relocated into `livrarr-identity` behavior-preserving; redesigning its retry/backoff is out of scope.

## 5. Open Questions

| ID | Question | Status | Resolution |
|----|----------|--------|------------|
| Q-001 | Does `livrarr-identity` own the convergence (async/bulk) resolver, or does that stay a Background-tier service composed in `livrarr-server`? (Bears on whether the resolver-cache leak BT-004 is cut at the identity boundary or the server boundary.) | open | PO leaning: identity owns it (convergence is an identity concern, Decision 1). |
| Q-002 | Cache retention policy (REQ-023): TTL, reference-count, or LRU-bounded? | resolved | PO 2026-06-03: none of those — the cache is **consume-once / pass-scoped**. Evict on consume (canonical data persisted to DB); short safety TTL for abandoned passes. No long-lived cache. |
| Q-003 | Does `EstablishedIdentity` live as a brand-new type in `domain`, or is the existing `IdentityState`/`CapturedIdentity` (already federated by WCC) promoted to *be* the contract? | open | Leaning: promote the existing federated `CapturedIdentity` (avoids a parallel type — DRY, WCC D-001). |
| Q-004 | Sequencing of the three Track-2 feature-cuts relative to each other (discovery-first vs cover-first) once the canary is GO. | open | (post-canary) |

## 6. Acceptance Criteria

**Scope tiers (round-1 review):** the ACs are tagged by when they must hold.
- **[CANARY]** — provable by the first move (the `livrarr-external-data` extraction). The ONLY acceptance this cycle commits to landing: **AC-004, AC-005, AC-007**.
- **[TRACK-1]** — the behavior-preserving foundation extractions (materialize, contract): **AC-001, AC-002, AC-003, AC-006, AC-018, AC-021**.
- **[TRACK-2]** — delivered incrementally as each feature-cut lands (status / discovery / cover): **AC-008..AC-017**.
- **[DEFERRED]** — design-stage; explicitly out of Phase-1 acceptance: **AC-020** (clean-overwrite) — see REQ-024. *(AC-019 resolved 2026-06-03 → now TRACK-2.)*

> **As-built (2026-06-06):** checkboxes below reflect delivery on `feat/metadata-modularization` @ `40b6445`; the **Delivery Status (as-built)** section near the top of this spec is the authoritative map. `[x]` = delivered; `[~]` = partial (as-built note inline); `[ ]` = deferred (see Delivery Status).

- [~] **AC-001** [TRACK-1] *(as-built: 3 of 4 crates — external-data/identity/enrichment; materialize deferred)* (REQ-001): The workspace contains four new library crates — `livrarr-external-data`, `livrarr-identity`, `livrarr-enrichment`, `livrarr-materialize` — and an `EstablishedIdentity` contract type in `livrarr-domain`; each compiles as a workspace member.
- [x] **AC-002** [CANARY/TRACK-1] (REQ-002, REQ-006): `cargo tree -p livrarr-identity` does not list `livrarr-enrichment`, and `cargo tree -p livrarr-enrichment` does not list `livrarr-identity`; `cargo tree -p livrarr-external-data` lists **none** of `livrarr-metadata`, `livrarr-identity`, `livrarr-enrichment`, **or `livrarr-db`**. The one-way graph is machine-verified.
- [ ] **AC-003** *(as-built: deferred — materialize crate not extracted)* (REQ-003): `livrarr-library` (or the import path) can depend on `livrarr-materialize` without transitively pulling `livrarr-enrichment` (verified by `cargo tree`); a panic/OOM in a materialize tag-write does not abort an in-flight enrichment (verified by a fault-injection test).
- [x] **AC-004** (REQ-005): `NormalizedWorkDetail` and `ProviderOutcome` are defined in `livrarr-external-data` and imported by `livrarr-metadata`/`livrarr-enrichment` (`use livrarr_external_data::…`), not the reverse; `livrarr-external-data` exposes both a discovery/search entry point and an enrichment-fetch entry point.
- [x] **AC-005** [CANARY] (REQ-011): Extracting `livrarr-external-data` (with the DB-stateful `provider_queue` left in `livrarr-enrichment`) and rebuilding yields **GO** — `livrarr-metadata` compiles against it with `lookup_filtered` and `enrich_work` as consumers, and `cargo tree -p livrarr-external-data` shows **no** edge to `livrarr-metadata`, `livrarr-identity`, `livrarr-enrichment`, **or `livrarr-db`** — OR a documented **NO-GO** naming the exact offending edge. Within the predeclared bound (≤3 pure-value-type relocations into `domain`; `EnrichmentContext`/queue traits NOT relocatable — REQ-006 fallback): exceeding the bound is a NO-GO, not a scope expansion. (The canary.)
- [x] **AC-006** (REQ-007, REQ-008): The `EstablishedIdentity` contract carries no `NormalizedWorkDetail`/metadata field (compile-checked by its definition); the provider cache is keyed on `(provider, provider-key)`, demonstrated by two Works sharing one provider key hitting one cache entry.
- [x] **AC-007** (REQ-009): After each Track-1 extraction commit, `cargo test --workspace` passes with the same test set green as the pre-extraction baseline (behavior-preserving).
- [~] **AC-008** *(as-built: status + discovery seams landed in target crates, monolith code removed; cover seam deferred w/ materialize)* (REQ-010): Each feature-touched seam (discovery/cover/status) lands its post-refactor code in the target crate (`external-data`/`materialize`/`identity`/`enrichment`), with `livrarr-metadata`'s corresponding monolith code removed — not duplicated.
- [~] **AC-009** *(as-built: auto ADD/CONFLICT monotonic-anchor paths built; full user-EDIT re-fire UX is a Track-2 feature per REQ-013)* (REQ-012, REQ-013): An auto ADD appends a new anchor type without altering an existing one; a user EDIT of an established anchor mutates it AND re-fires enrichment, while a user-set field value survives the re-enrichment.
- [x] **AC-010** (REQ-014): A Work's identity state and enrichment state are independently representable — a `Confirmed`/`Thin` Work and a `NeedsReview`/`Unenriched` Work are both expressible and visually distinct; no single status conflates the two.
- [~] **AC-011** *(as-built: Provisional-enriches + de-facto identity delivered (chunk D/4b); Pending-hold + generation-bump-re-fire per REQ-015 partially wired — verify before claiming)* (REQ-015): A `Pending` (fuzzy, no key) Work is NOT enriched until it converges; a `Provisional` (ISBN, no anchor) Work IS enriched; an identity `generation` bump re-fires enrichment.
- [x] **AC-012** [TRACK-2] (REQ-016): An ISBN resolving only to Google Books creates a `Provisional` Work keyed on `isbn_13` (no confirmation prompt, no work anchor; enriches), not an identity-pending Work; it upgrades to `Confirmed` if a work anchor later appears; no GB `volumeId` is persisted as a work anchor.
- [x] **AC-013** (REQ-017): Discovery of a Work warms the provider cache; the subsequent enrichment issues zero network calls for a provider whose payload discovery already cached (verified by a provider-call spy).
- [ ] **AC-014** *(as-built: deferred — only the `/book/auto_complete` rung shipped (chunk A); cost-ordered ladder + WillRetry-not-NotFound + `/book/show` verify unbuilt)* (REQ-018): A Goodreads discovery whose first rung is anti-bot-blocked escalates to the next rung and is not marked terminal-`NotFound`; a `gr_key` is persisted only after a stable `/book/show` page verifies title+author.
- [~] **AC-015** *(as-built: completion classification delivered — `Enriched` on text not cover (`enrichment/lib.rs:866`, = WCC REQ-030); `materialize_cover`/`materialize_tags` entry points deferred w/ materialize crate)* (REQ-019): An OpenLibrary-anchored Work with a description but no cover is classified `Enriched` and its file tags are written; the cover populates later via `materialize_cover` without changing the `Enriched` status; the add path can call `materialize_cover` synchronously and `materialize_tags` is queued.
- [ ] **AC-016** *(as-built: deferred — Bug #2 identifier-change invalidation unbuilt)* (REQ-020): A Work whose `asin` is resolved by Hardcover/Google Books *after* Audnexus terminally returned `NotFound` re-queries Audnexus on the next background pass and converges — with no manual refresh.
- [ ] **AC-017** *(as-built: deferred — materialize idempotency lands w/ the materialize crate)* (REQ-021): A materialize pass rewrites a file only when `merge_generation > tag_generation`; a second pass with no generation change is a no-op.
- [x] **AC-018** (REQ-022): The GCRA rate-limiter and circuit-breaker live in `livrarr-external-data` and are invoked by both Identity discovery and Enrichment fetch — neither consumer re-implements pacing.
- [ ] **AC-019** [TRACK-2] (REQ-023): A cache entry is evicted once its identity→enrichment pass consumes it (canonical data persisted to the Work); an entry from a pass that abandons before consuming is reaped by the safety TTL — so the cache holds only in-flight passes (verified by a consume-once + TTL test).
- [ ] **AC-020** [DEFERRED — design-stage, out of Phase-1 acceptance] (REQ-024): Re-enriching a `Provisional` Work after its `generation` bumps to `Confirmed` leaves no orphaned provenance rows (clean overwrite).
- [ ] **AC-021** [TRACK-1] (REQ-010, D-014 deletion gate): At the end of each extraction slice, `livrarr-metadata` exposes **no** `pub use livrarr_{external_data,materialize,identity,enrichment}::` re-export shims (verified by a direct-import scan) — the migration scaffold is removed, not left as a permanent facade.
