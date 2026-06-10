# Design Seed — Metadata Pipeline Modularization

**Status:** Pre-spec design seed. Decisions locked in a PO design session on **2026-06-02** (Q1–Q7, one at a time). This is **not a feature yet** — kick it off as the `metadata-modularization` feature **after work-creation-consistency (WCC) lands**. Captured so the thinking isn't re-derived.

**Relationship to WCC:** WCC is **phase 1 of this**. It already builds the seams this arc extracts — the federated `IdentityResolver`, the payload cache, the GR anti-bot ladder, and the status split (Fork F / NeedsReview) — but *inside* `livrarr-metadata`. This arc makes the boundary explicit: separate crates, a one-way contract, and a clean three-box split.

---

## The one idea

Split the tangled work-creation + metadata pipeline (today all fused in `livrarr-metadata`'s `enrich_work` spine) into **three concerns on a shared substrate**, with identity flowing **one way** into enrichment:

```
livrarr-domain        <- leaf: shared types + the EstablishedIdentity contract + traits
   ^        ^
livrarr-providers     <- shared "dumb pipe + cache": transport, normalize, rate-limit,
   |        |            circuit-break, anti-bot resilience ladder, per-(provider,key) cache
   +--------+--------+
livrarr-identity   livrarr-enrichment ---> livrarr-materialize
 "what work is        "fill the work's        "project the record onto
  this?"               metadata record"         artifacts: cover image + file tags"
   |                     |                        |
   +---------------------+------------------------+
                  livrarr-server (composition root: wires the flow via work state + jobs)
```

- **Identity** and **Enrichment** are **non-dependent sibling crates** — neither imports the other. They communicate only through the `EstablishedIdentity` contract type, which lives in `domain`. One-way is therefore **compile-enforced**.
- **Provider-Access** is the shared substrate both consume.
- **Materialize** is downstream of Enrichment (projects the canonical DB record onto physical artifacts).

---

## The 7 decisions

### 1. Direction — one-way, monotonic identity
Identity -> Enrichment only. Enrichment never mutates identity. Convergence (a later pass that upgrades identity) is owned *inside* Identity.
Anchors are **monotonic** — three write-paths, only one changes an *existing* anchor:
- **ADD** (auto) — a lookup finds a *new* anchor type -> append. Never touches an existing one.
- **CONFLICT** (auto) — a lookup finds a *contradicting* anchor -> raise for the user; never silently change.
- **EDIT** (user) — the only path that mutates an established anchor. A user ID-edit **re-fires enrichment** (old metadata may belong to the wrong book), preserving user-set field values.

### 2. Shared substrate — dumb pipe + cache
One `livrarr-providers` layer owns the *mechanism* (HTTP, normalization, rate-limit/GCRA, circuit breaker, the anti-bot resilience ladder, and the payload cache). Each consumer owns its own *selection policy* (Identity: who resolves an id; Enrichment: who has the best fields + language routing).
Payloads are **never discarded** — Identity's discovery fan-out warms the cache; Enrichment reads it and tops up only what's missing.
**Cache key = `(provider, provider-key)`** (e.g. `(Goodreads, "12345")`), NOT `work_id` — keying by `work_id` leaks the Work concept into the provider layer, and `work_id` doesn't even exist yet during discovery. *(confer catch, Gemini)*

### 3. Contract — pure identity, no metadata
Identity hands Enrichment a thin record: `{ work_id, anchors+bridges (ol/gr/hc/isbn/asin), title, author, language, state, method, generation }`. **No metadata** — field data flows via the shared cache. The contract type lives in `domain` (the leaf), so Enrichment depends on the *contract*, not on Identity.
**The trap to avoid:** don't pass "the description Identity happened to fetch" inside the contract — that re-couples the two subsystems. Metadata -> cache; identity -> contract.

### 4. Gating — enrich on a deterministic key
Separate **display** (always show the best in hand — never blocked) from **enrich** (the gated provider fan-out).
Enrichment runs once Identity has a deterministic addressing key:

| Identity state | Enrichment |
|---|---|
| Confirmed | enrich |
| Provisional (ISBN resolved, no work anchor) | enrich — the whole point of Provisional |
| Pending (fuzzy title+author, no key) | hold (converge first) |
| Conflict | pause (user resolves) |
| NeedsReview | hold (user resolves) |

Re-fires on an identity `generation` bump (a new anchor -> better providers; a user edit -> redo).

### 5. State model — two state machines
Replace the one overloaded `EnrichmentStatus` with two independent tracks:
- **Identity:** Pending -> { Confirmed | Provisional | Conflict | NeedsReview }
- **Enrichment:** Unenriched -> { Enriched | Thin }

This structurally kills the conflation between *identity*-NeedsReview ("which book is this?") and *enrichment*-Thin ("we know the book, found no info") — two different problems, two different user actions, two tracks.

### 6. Physical form — separate crates, three boxes
- **6a:** separate crates, boundary **compiler-enforced** (the existing compile-wall pattern). Identity & Enrichment as non-dependent siblings; contract in `domain`.
- **6b:** **three boxes** — Materialize is its own downstream crate, not folded into Enrichment. Decisive reason *(Gemini)*: **cardinality** — Enrichment is 1:1 with the Work (logical, DB, network-bound, slow); Materialize is 1:N with physical files (disk-bound, fast, fragile). Plus: failure isolation (the audio-tag OOM mess can't stall enrichment), per-format on/off policy (EPUB on, m4b off), and the import/library path can depend on Materialize **without** pulling Enrichment's network/LLM deps.
- **Cover** is a **special, top-priority Enrichment class** — fetched first/eagerly so it's present at work-creation, user-pickable, and **never an identifier** (a different cover != a different work). Materialize exposes two entry points: a **synchronous** `materialize_cover(work_id)` the add-handler blocks on, and an **async queued** `materialize_tags(...)` for the heavy/dangerous disk writes. *(Gemini)*

### 7. Sequencing — see the decision brief (C′ structured extraction)
**Superseded 2026-06-02** by a read-only code trace + two cross-family confer rounds. The sequencing is NOT "land WCC first, then evolve." It is **C′ — structured extraction**: stabilize the in-flight work → extract the stable foundation + ports first (the clean ~73%: providers, materialize, the contract) → cut each feature-touched seam (discovery→GR ladder, cover→cover-decouple, status→two-state-machines) as part of building that feature, in the target crate, never back in the monolith. The foundation is "well-factored but un-walled" → reorganization, not rebuild. First move = extract `livrarr-providers` as a **falsifiable canary** (GO if no back-edge `providers→metadata`; else fall back to plan A). Full decision record + rationale: `docs/decision-metadata-modularization-sequencing.md` (v3 FINAL).

---

## Open items to resolve when formalizing (confer red flags)

1. **Cache keying** — confirm `(provider, provider-key)`, not `work_id` (Decision 2).
2. **Provisional write-amplification** — enriching a `Provisional` work fills an edition that may later upgrade to `Confirmed`; ensure the merge does a **clean overwrite** (no orphaned provenance) when the identity `generation` bumps.
3. **Cache TTL / eviction** — "never discard payloads" -> DB bloat over time; needs a retention policy for stale, unreferenced keys.
4. **`tag_generation` vs `merge_generation`** — Materialize idempotency: rewrite a file only when `merge_generation > tag_generation`. Self-healing, trivially retriable. (`merge_generation` already exists.) *(Gemini)*

---

## Provenance

Decisions locked in a PO design session, 2026-06-02 (Q1–Q7, one node at a time). Cross-family confer on 6b: **Gemini concurred (3 boxes)**; Codex was quota-blocked. Kick off as feature `metadata-modularization` after WCC closes — `/kk-session-start metadata-modularization`, with this file as the design input (the way `diagrams/metadata-lifecycle.html` fed WCC).
