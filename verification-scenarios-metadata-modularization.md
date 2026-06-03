# Verification Scenarios — metadata-modularization (pre-build design check)

**Purpose.** Before building, stress-test the design against the cases that actually break the *current* system. Every scenario is grounded in a REAL work from the live dev DB (`testdata/livrarr.db`, 143 works, read-only). For each, trace it through the four boxes (external-data / identity / enrichment / materialize) and answer three questions:
- **Q1 — correct data?** does the design produce the right identity + metadata?
- **Q2 — no waiting?** is the user never left blocked or in indefinite limbo?
- **Q3 — clean boundary?** does any box have to reach *across* a boundary to handle it?

A scenario that forces a boundary violation means **the cut is wrong — fix it on paper, before building.** This is the cheapest possible verification.

**Why this is the right test set.** These 143 works are the current system's actual output, and it mishandles them in *measurable* ways (below). The new design must handle every one correctly. If it can't, it isn't the right design — and we'd rather learn that now than in redesign #N+1.

---

## Real-data baseline — the current system's failure surface (today, measured)

| Signal | Count | Meaning |
|---|---|---|
| Total works | 143 | 93 enriched · 25 unenriched · 15 identity_pending · 8 failed · 2 conflict |
| **`conflict` status but conflict-store = 0 rows** | **2** | conflict is unobservable + unresolvable — the unwired path (REQ-020), LIVE |
| **`merge_generation ≥ 400`** (max **522**) | **21** | re-merged hundreds of times, never settling — the churn (REQ-030) |
| **Audnexus stuck `not_found`** | **125** | terminal despite many having an `asin` — Bug #2 pattern at scale (REQ-031) |
| **ISBN-10 stored in the `asin` field** | ≥6 | e.g. Abaddon's Gate `asin=1549142194` → feeds Audnexus garbage (REQ-004) |
| **ISBN present, no work anchor** | 15 | scattered across failed/unenriched/identity_pending — de-facto-identity limbo (REQ-016) |
| **`unenriched` yet has a description** | 19 | text present, still not "enriched" — the cover/status conflation (REQ-030) |
| Foreign works (fr 6 · pl 5 · es 4) | 15 | incl. *both* conflict works — language routing (REQ-027) |
| gr_key set / non-numeric | 36 / **0** | clean — gr_key drift (REQ-002) is NOT a live problem in this data |

> The current system re-merges 21 works 400+ times, leaves 125 Audnexus lookups terminally dead, and shows 2 unresolvable conflicts. That is the bar to beat.

---

## Scenarios — trace each through the design

### Group A — Correct data (identity)

**A1 — Conflict with no resolution path.** REAL: `La Nuit Des Temps` (ol=OL8551775W) and `Die Krone Der Sterne` (ol=OL20909706W), both `status=conflict`, conflict-store empty.
- Q1: Does a conflict now produce an **observable, resolvable** Identity state — a Conflict badge + a conflict-store row + a user action — not a dead status with nothing behind it?
- Q3: Enrichment must NOT set this (it can't write identity). Trace: enrichment emits `IdentityContradiction` → server → identity raises the conflict. Does that path actually land an observable row?

**A2 — ISBN, no work anchor.** REAL: `El Problema De Los Tres Cuerpos` (isbn 9788413143750, *unenriched*); `The Ender Quintet` (isbn 9780765376824, *identity_pending*); `Wiedźmin` (isbn, *failed*).
- Q1: Does each settle to **Provisional** (Tier-A, enriches, no prompt) — instead of scattering across three different wrong states? Does it upgrade to Confirmed if a work anchor later appears?

**A3 — Duplicate work from title variance.** REAL: `The Dragon Reborn` AND `The Dragon Reborn (The Wheel of Time, Book 3)` — same book, **two** works, both identity_pending, same isbn 9780356525235.
- Q1: Same ISBN → does dedup/quorum collapse these to ONE work? Today they are two because the series suffix dodged normalized-title matching. Does the design's anchor-merge + normalized match catch it?

**A4 — ISBN-10 stored as ASIN.** REAL: `Abaddon's Gate` asin=1549142194; `Tiamat's Wrath` asin=1980006520.
- Q1: Does REQ-004 normalization convert these to `isbn_13` and clear `asin`, so they stop being sent to Audnexus as bogus ASINs (a root cause of the 125 not_founds)?

**A5 — Foreign work + English providers.** REAL: both conflict works are non-English with OL anchors; 15 foreign works total.
- Q1: Does language routing stop OL/HC English metadata from winning a foreign record (REQ-027), while still giving the book a Provisional/Confirmed identity?

### Group B — No waiting / convergence

**B1 — The 522-generation churn.** REAL: `Cibola Burn`, `Persepolis Rising`, `HP Sorcerer's Stone` at merge_generation=522; 21 works ≥400.
- Q2: In the two-state-machine, does a confirmed-identity work with metadata settle to **Enriched** (or **Sparse**) and **stop re-merging** — instead of re-merging every background pass forever? *(Biggest single waste in the live data.)*

**B2 — Audnexus terminal despite ASIN.** REAL: 125 works; e.g. `Cibola Burn` asin=B00K7PP15W.
- Q2: When an asin is present/resolved, does Bug #2 invalidation re-query Audnexus and converge — instead of 125 works dead forever? *(Caveat: Audnexus currently region-blocks ASINs — a live service issue — but the design path must self-heal once it clears, and must not have left them terminal.)*

**B3 — Sparse vs. still-loading.** REAL: 19 works `unenriched` that already have a description.
- Q2: Does a known work with thin metadata land in **Sparse** (a settled, surfaced state) rather than `unenriched` limbo that looks like "still loading" and never resolves?

**B4 — Interactive add never blocks.** Any Add Work.
- Q2: Trace the interactive path — does it return fully-formed from cache + deterministic resolution, never blocking on a provider fan-out or an LLM call? (display-vs-enrich + consume-once cache + latency tiers.)

### Group C — Clean architecture

**C1 — The canary (build it).** Extract `livrarr-external-data`; `cargo tree -p livrarr-external-data` shows **no** edge to metadata / identity / enrichment / db. Empirical proof the boundary holds (converts the analytical GO into a built GO).

**C2 — Boundary stress.** For A1–B4, confirm no box reaches across a boundary — especially: enrichment never *writes* identity (A1), materialize never pulls enrichment (covers), external-data never touches db (the canary-killer).

---

## How to run this verification (next session, fresh context)

1. **Focused cross-family stress-test** (paper, highest signal): feed the design (spec + ir-v1 + ir-v2) + this scenario list to Gemini + Codex with the ask: *"Trace each scenario through the four-box design. For each, where does the design force a boundary violation, produce wrong data, or make the user wait? Cite the box + the rule."* (This is distinct from the round-1/2 reviews, which hunted for flaws in the artifacts; this pressure-tests against reality.) Dispatch reviewers with cwd inside the worktree; gemini pinned to `gemini-3.1-pro-preview`; codex `--dangerously-bypass-approvals-and-sandbox`.
2. **Build the canary** (C1) — the empirical boundary proof; not throwaway (it's step 1 of the build).
3. **Pre-mortem** — one paragraph: "It's 6 months out and we're redesigning metadata *again*. Why?" Check the design against whatever you write.

**Pass bar:** every scenario above traces cleanly (correct data, no indefinite wait, no boundary reach) AND the canary builds GREEN with no back-edge. If all hold, this is a design safe to *build incrementally* — and because the boundaries are compile-enforced, future changes stay local edits, not wholesale redesigns. That is the actual end-condition for the redesign cycle.
